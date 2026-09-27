//! Deciding, per alert target, whether to show it and how.
//!
//! Suppression (docs/DESIGN.md, "When to alert"): by default an alert is
//! suppressed only for the pilot whose client the user is looking at; every
//! other pilot's alerts go through, and everything goes through when focus is
//! on a non-EVE window. Delivery: overlay when the game is on screen, a native
//! toast when it is not or the user is away, with fallbacks.

use crate::engine::Alert;
use crate::presence::{ClientState, Rect, Snapshot};
use crate::rules::Reason;
use std::collections::HashMap;
use std::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Suppression {
    /// Suppress only the pilot whose client has focus. The default.
    FocusedOnly,
    /// Suppress any pilot whose client is visible on screen.
    VisibleOnScreen,
    /// Never suppress.
    AllowAll,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeliveryMode {
    /// Overlay when a client is on screen, toast otherwise or when the user is away.
    Auto,
    Overlay,
    Toast,
    Both,
    SoundOnly,
}

#[derive(Debug, Clone)]
pub struct RouterConfig {
    pub suppression: Suppression,
    pub delivery: DeliveryMode,
    /// Input idle for this long counts as away: nothing is suppressed and the
    /// alert goes to a toast, which persists in the Action Center.
    pub idle_after: Duration,
    /// Also play a sound with every delivered alert.
    pub sound: bool,
    pilot_delivery: HashMap<String, DeliveryMode>,
    pilot_suppression: HashMap<String, Suppression>,
}

impl Default for RouterConfig {
    fn default() -> Self {
        RouterConfig {
            suppression: Suppression::FocusedOnly,
            delivery: DeliveryMode::Auto,
            idle_after: Duration::from_secs(5 * 60),
            sound: false,
            pilot_delivery: HashMap::new(),
            pilot_suppression: HashMap::new(),
        }
    }
}

impl RouterConfig {
    pub fn set_pilot_delivery(&mut self, pilot_name: &str, mode: DeliveryMode) {
        self.pilot_delivery.insert(pilot_name.to_lowercase(), mode);
    }

    pub fn set_pilot_suppression(&mut self, pilot_name: &str, s: Suppression) {
        self.pilot_suppression.insert(pilot_name.to_lowercase(), s);
    }

    fn delivery_for(&self, pilot_name: &str) -> DeliveryMode {
        self.pilot_delivery.get(&pilot_name.to_lowercase()).copied().unwrap_or(self.delivery)
    }

    fn suppression_for(&self, pilot_name: &str) -> Suppression {
        self.pilot_suppression.get(&pilot_name.to_lowercase()).copied().unwrap_or(self.suppression)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Anchor {
    /// Fixed to a monitor (borderless-style modes, or the client is hidden and
    /// this is where it last was).
    Monitor(Rect),
    /// Follows the client window (Windowed mode).
    FollowWindow { hwnd: isize },
    /// The pilot has no known client or monitor.
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Delivery {
    Overlay { anchor: Anchor },
    /// A native notification with a "switch to this pilot" action.
    Toast { switch_to: String },
    Sound,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SuppressedBy {
    FocusedPilot,
    VisibleOnScreen,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    Suppressed(SuppressedBy),
    Deliver(Vec<Delivery>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Decision {
    pub pilot_name: String,
    pub pilot_id: Option<String>,
    pub reason: Reason,
    pub outcome: Outcome,
}

fn anchor_for(client: Option<&ClientState>) -> Anchor {
    match client {
        None => Anchor::Unknown,
        Some(c) if c.on_screen() && !c.covers_monitor() => Anchor::FollowWindow { hwnd: c.hwnd },
        Some(c) => c.monitor.map_or(Anchor::Unknown, Anchor::Monitor),
    }
}

pub fn route(alert: &Alert, snap: &Snapshot, cfg: &RouterConfig) -> Vec<Decision> {
    let away = snap.idle >= cfg.idle_after;
    alert
        .targets
        .iter()
        .map(|t| {
            let client = snap.client(&t.pilot_name);
            let suppressed = if away {
                None // nobody is looking, whatever has focus
            } else {
                match cfg.suppression_for(&t.pilot_name) {
                    Suppression::FocusedOnly if snap.is_focused(&t.pilot_name) => Some(SuppressedBy::FocusedPilot),
                    Suppression::VisibleOnScreen if client.is_some_and(ClientState::on_screen) => Some(SuppressedBy::VisibleOnScreen),
                    _ => None,
                }
            };
            let outcome = match suppressed {
                Some(why) => Outcome::Suppressed(why),
                None => Outcome::Deliver(deliveries(&t.pilot_name, client, snap, cfg, away)),
            };
            Decision { pilot_name: t.pilot_name.clone(), pilot_id: t.pilot_id.clone(), reason: t.reason.clone(), outcome }
        })
        .collect()
}

fn deliveries(pilot: &str, client: Option<&ClientState>, snap: &Snapshot, cfg: &RouterConfig, away: bool) -> Vec<Delivery> {
    let mode = cfg.delivery_for(pilot);
    let overlay = || Delivery::Overlay { anchor: anchor_for(client) };
    let toast = || Delivery::Toast { switch_to: pilot.to_string() };
    let toast_ok = snap.notifications_ok;
    // A toast that Windows would hold back falls back to an overlay.
    let toast_or_overlay = || if toast_ok { toast() } else { overlay() };

    let mut out = match mode {
        DeliveryMode::Auto => {
            if away || !snap.any_on_screen() {
                vec![toast_or_overlay()]
            } else {
                vec![overlay()]
            }
        }
        DeliveryMode::Overlay => vec![overlay()],
        DeliveryMode::Toast => vec![toast_or_overlay()],
        DeliveryMode::Both => {
            let mut v = vec![overlay()];
            if toast_ok {
                v.push(toast());
            }
            v
        }
        DeliveryMode::SoundOnly => vec![],
    };
    if cfg.sound || mode == DeliveryMode::SoundOnly {
        out.push(Delivery::Sound);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::AlertTarget;
    use crate::logfmt::ChatLine;
    use crate::time::Stamp;

    const MON_L: Rect = Rect { left: -1920, top: 0, right: 0, bottom: 1080 };
    const MON_R: Rect = Rect { left: 0, top: 0, right: 1920, bottom: 1080 };

    fn client(name: &str, hwnd: isize, mon: Rect) -> ClientState {
        ClientState { character: name.into(), pid: 1, hwnd, minimized: false, cloaked: false, visible_flag: true, rect: Some(mon), monitor: Some(mon) }
    }

    fn snap(clients: Vec<ClientState>, focused: Option<&str>) -> Snapshot {
        Snapshot { clients, focused: focused.map(String::from), foreground: None, idle: Duration::ZERO, notifications_ok: true }
    }

    fn alert(targets: &[&str]) -> Alert {
        Alert {
            channel_id: "local".into(),
            channel_name: "Local".into(),
            line: ChatLine { stamp: Stamp(0), sender: "Bob".into(), text: "hi".into() },
            targets: targets
                .iter()
                .map(|n| AlertTarget { pilot_id: None, pilot_name: n.to_string(), reason: Reason::OwnName })
                .collect(),
            seen_by: vec![],
        }
    }

    fn two() -> Vec<ClientState> {
        vec![client("Jarna", 10, MON_L), client("Psianna", 20, MON_R)]
    }

    fn only(d: &[Decision], name: &str) -> Outcome {
        d.iter().find(|d| d.pilot_name == name).unwrap().outcome.clone()
    }

    #[test]
    fn default_suppresses_only_the_focused_pilot() {
        let s = snap(two(), Some("Jarna"));
        let d = route(&alert(&["Jarna", "Psianna"]), &s, &RouterConfig::default());
        assert_eq!(only(&d, "Jarna"), Outcome::Suppressed(SuppressedBy::FocusedPilot));
        assert_eq!(only(&d, "Psianna"), Outcome::Deliver(vec![Delivery::Overlay { anchor: Anchor::Monitor(MON_R) }]));
    }

    #[test]
    fn everything_goes_through_when_focus_is_not_on_an_eve_client() {
        let s = snap(two(), None);
        let d = route(&alert(&["Jarna", "Psianna"]), &s, &RouterConfig::default());
        assert!(d.iter().all(|d| matches!(d.outcome, Outcome::Deliver(_))));
    }

    #[test]
    fn allow_all_and_visible_policies() {
        let s = snap(two(), Some("Jarna"));
        let allow = RouterConfig { suppression: Suppression::AllowAll, ..RouterConfig::default() };
        assert!(matches!(only(&route(&alert(&["Jarna"]), &s, &allow), "Jarna"), Outcome::Deliver(_)));

        let visible = RouterConfig { suppression: Suppression::VisibleOnScreen, ..RouterConfig::default() };
        let d = route(&alert(&["Jarna", "Psianna"]), &s, &visible);
        assert_eq!(only(&d, "Psianna"), Outcome::Suppressed(SuppressedBy::VisibleOnScreen), "visible on the other monitor");

        // A minimized client is not visible, so it still alerts.
        let mut clients = two();
        clients[1].minimized = true;
        let d = route(&alert(&["Psianna"]), &snap(clients, Some("Jarna")), &visible);
        assert!(matches!(only(&d, "Psianna"), Outcome::Deliver(_)));
    }

    #[test]
    fn being_away_overrides_suppression_and_uses_a_toast() {
        let mut s = snap(two(), Some("Jarna"));
        s.idle = Duration::from_secs(10 * 60);
        let d = route(&alert(&["Jarna"]), &s, &RouterConfig::default());
        assert_eq!(only(&d, "Jarna"), Outcome::Deliver(vec![Delivery::Toast { switch_to: "Jarna".into() }]));
    }

    #[test]
    fn no_client_on_screen_means_a_toast_and_a_blocked_toast_falls_back_to_an_overlay() {
        let mut clients = two();
        for c in &mut clients {
            c.cloaked = true; // both on another virtual desktop
            c.rect = None;
        }
        let mut s = snap(clients, None);
        let d = route(&alert(&["Psianna"]), &s, &RouterConfig::default());
        assert_eq!(only(&d, "Psianna"), Outcome::Deliver(vec![Delivery::Toast { switch_to: "Psianna".into() }]));

        s.notifications_ok = false;
        let d = route(&alert(&["Psianna"]), &s, &RouterConfig::default());
        assert_eq!(
            only(&d, "Psianna"),
            Outcome::Deliver(vec![Delivery::Overlay { anchor: Anchor::Monitor(MON_R) }]),
            "falls back to the pilot's last known monitor"
        );
    }

    #[test]
    fn windowed_clients_get_a_following_overlay_and_borderless_ones_a_monitor_overlay() {
        let mut clients = two();
        clients[0].rect = Some(Rect { left: -1500, top: 100, right: -300, bottom: 900 }); // Windowed
        let d = route(&alert(&["Jarna", "Psianna"]), &snap(clients, None), &RouterConfig::default());
        assert_eq!(only(&d, "Jarna"), Outcome::Deliver(vec![Delivery::Overlay { anchor: Anchor::FollowWindow { hwnd: 10 } }]));
        assert_eq!(only(&d, "Psianna"), Outcome::Deliver(vec![Delivery::Overlay { anchor: Anchor::Monitor(MON_R) }]));
    }

    #[test]
    fn a_minimized_fullscreen_client_still_gets_its_alert_on_its_last_monitor() {
        let mut clients = two();
        clients[1].minimized = true;
        clients[1].rect = None; // monitor stays: last known
        let d = route(&alert(&["Psianna"]), &snap(clients, Some("Jarna")), &RouterConfig::default());
        assert_eq!(only(&d, "Psianna"), Outcome::Deliver(vec![Delivery::Overlay { anchor: Anchor::Monitor(MON_R) }]));
    }

    #[test]
    fn delivery_modes_sound_and_per_pilot_overrides() {
        let s = snap(two(), None);
        let mut cfg = RouterConfig { sound: true, ..RouterConfig::default() };
        cfg.set_pilot_delivery("jarna", DeliveryMode::Both);
        cfg.set_pilot_delivery("Psianna", DeliveryMode::SoundOnly);
        let d = route(&alert(&["Jarna", "Psianna"]), &s, &cfg);
        assert_eq!(
            only(&d, "Jarna"),
            Outcome::Deliver(vec![
                Delivery::Overlay { anchor: Anchor::Monitor(MON_L) },
                Delivery::Toast { switch_to: "Jarna".into() },
                Delivery::Sound
            ])
        );
        assert_eq!(only(&d, "Psianna"), Outcome::Deliver(vec![Delivery::Sound]));
    }

    #[test]
    fn an_unknown_client_still_delivers_with_an_unknown_anchor() {
        let d = route(&alert(&["Ghost"]), &snap(two(), None), &RouterConfig::default());
        // Auto with clients on screen -> overlay; no client for this pilot -> unknown anchor.
        assert_eq!(only(&d, "Ghost"), Outcome::Deliver(vec![Delivery::Overlay { anchor: Anchor::Unknown }]));
    }
}
