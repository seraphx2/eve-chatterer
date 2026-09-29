//! Deciding, per alert target, whether to show it and how.
//!
//! The preferences come from the layered settings and travel on each
//! `AlertTarget` (`prefs`). Suppression (docs/DESIGN.md, "When to alert"): by
//! default an alert is suppressed only for the pilot whose client the user is
//! looking at; every other pilot's alerts go through, and everything goes
//! through when focus is on a non-EVE window. Delivery: overlay while an EVE
//! client has focus, a native toast when none does, none is on screen, or the
//! user is away, with fallbacks. Rate caps are applied afterwards by the `Governor`, which needs
//! memory; this module is pure.

use crate::engine::Alert;
use crate::prefs::{DeliveryMode, LayerKey, OverCap, OverlayStyle, Prefs, RateCap, Suppression};
use crate::presence::{ClientState, Rect, Snapshot};
use crate::rules::Reason;
use std::time::Duration;

/// Which style each kind of match gets when the settings do not force one.
/// The overlay manager may still fold a burst of alerts into a strip stack;
/// that needs memory of recent alerts and lives above this pure router.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StyleMap {
    pub own_name: OverlayStyle,
    pub keyword: OverlayStyle,
    pub regex: OverlayStyle,
    pub always: OverlayStyle,
}

impl Default for StyleMap {
    fn default() -> Self {
        StyleMap {
            own_name: OverlayStyle::Beacon,
            keyword: OverlayStyle::Panel,
            regex: OverlayStyle::Panel,
            always: OverlayStyle::Strip,
        }
    }
}

impl StyleMap {
    fn for_reason(&self, r: &Reason) -> OverlayStyle {
        match r {
            Reason::OwnName => self.own_name,
            Reason::Keyword(_) => self.keyword,
            Reason::Regex(_) => self.regex,
            Reason::AlwaysChannel(_) | Reason::AlwaysSender(_) => self.always,
        }
    }
}

/// System-wide routing behavior (not per pilot or channel; that is `Prefs`).
#[derive(Debug, Clone)]
pub struct RouterConfig {
    /// Input idle for this long counts as away: nothing is suppressed and the
    /// alert goes to a toast, which persists in the Action Center.
    pub idle_after: Duration,
    pub styles: StyleMap,
}

impl Default for RouterConfig {
    fn default() -> Self {
        RouterConfig { idle_after: Duration::from_secs(5 * 60), styles: StyleMap::default() }
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
    Overlay { anchor: Anchor, style: OverlayStyle },
    /// A native notification with a "switch to this pilot" action. `style`
    /// is what the overlay would have been: it sets how insistent the
    /// notification is (how long it stays), like the overlays' lifetimes.
    Toast { switch_to: String, style: OverlayStyle },
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
    /// Held back by a rate cap (set by the `Governor`, never by `route`).
    Limited(OverCap),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Decision {
    pub pilot_name: String,
    pub pilot_id: Option<String>,
    pub reason: Reason,
    pub outcome: Outcome,
    /// The caps to apply to a delivered alert, from every settings level.
    pub caps: Vec<(LayerKey, RateCap)>,
}

/// Where to draw the alert, in priority order:
/// 1. Wherever the user is actually focused, if that client is on screen —
///    that always wins, even when the alerted pilot's *own* client is also
///    genuinely visible (e.g. each client fixed to its own monitor in a
///    multi-boxing setup). Showing an alert on a screen the user isn't
///    looking at defeats the point of an alert. Found live (2026-09-27,
///    owner: "im on naomi, and holden's Local notifications are still
///    showing up on Holden's screen" — a follow-on to the same-day anchor fix
///    below, which only covered a *hidden* alerted client, not one visible
///    on its own separate screen while unfocused).
/// 2. Otherwise, the alerted pilot's own window if it is on screen — a
///    neutral choice when nobody in particular is focused.
/// 3. Otherwise, any other client that happens to be on screen — still
///    better than a screen nobody can see.
/// 4. Otherwise, the alerted pilot's own last known monitor, as a
///    least-bad guess (2026-09-27: originally the *only* rule, which put a
///    hidden pilot's alert on a screen nobody could see).
fn anchor_for(client: Option<&ClientState>, snap: &Snapshot) -> Anchor {
    fn on_its_own_screen(c: &ClientState) -> Anchor {
        if c.covers_monitor() { c.monitor.map_or(Anchor::Unknown, Anchor::Monitor) } else { Anchor::FollowWindow { hwnd: c.hwnd } }
    }
    if let Some(f) = snap.focused_client().filter(|f| f.on_screen()) {
        return on_its_own_screen(f);
    }
    if let Some(c) = client.filter(|c| c.on_screen()) {
        return on_its_own_screen(c);
    }
    if let Some(v) = snap.clients.iter().find(|c| c.on_screen()) {
        return on_its_own_screen(v);
    }
    client.and_then(|c| c.monitor).map_or(Anchor::Unknown, Anchor::Monitor)
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
                match t.prefs.suppression {
                    Suppression::FocusedOnly if snap.is_focused(&t.pilot_name) => Some(SuppressedBy::FocusedPilot),
                    Suppression::VisibleOnScreen if client.is_some_and(ClientState::on_screen) => Some(SuppressedBy::VisibleOnScreen),
                    _ => None,
                }
            };
            let outcome = match suppressed {
                Some(why) => Outcome::Suppressed(why),
                None => Outcome::Deliver(deliveries(&t.pilot_name, &t.reason, &t.prefs, client, snap, cfg, away)),
            };
            Decision {
                pilot_name: t.pilot_name.clone(),
                pilot_id: t.pilot_id.clone(),
                reason: t.reason.clone(),
                outcome,
                caps: t.prefs.caps.clone(),
            }
        })
        .collect()
}

fn deliveries(
    pilot: &str,
    reason: &Reason,
    prefs: &Prefs,
    client: Option<&ClientState>,
    snap: &Snapshot,
    cfg: &RouterConfig,
    away: bool,
) -> Vec<Delivery> {
    let mode = prefs.delivery;
    let style = prefs.style.unwrap_or_else(|| cfg.styles.for_reason(reason));
    let overlay = || Delivery::Overlay { anchor: anchor_for(client, snap), style };
    let toast = || Delivery::Toast { switch_to: pilot.to_string(), style };
    let toast_ok = snap.notifications_ok;
    // A toast that Windows would hold back falls back to an overlay.
    let toast_or_overlay = || if toast_ok { toast() } else { overlay() };

    let mut out = match mode {
        DeliveryMode::Auto => {
            // An overlay lives with its EVE client (owned window, not
            // topmost), so while another app has focus it would land behind
            // that app: use the notification area then too (owner,
            // 2026-09-28). `focused` already ignores the alt-tab switcher.
            if away || snap.focused.is_none() || !snap.any_on_screen() {
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
    if prefs.sound || mode == DeliveryMode::SoundOnly {
        out.push(Delivery::Sound);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::channel::ChannelKind;
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

    fn alert_with(targets: Vec<(&str, Reason, Prefs)>) -> Alert {
        Alert {
            channel_id: "local".into(),
            channel_name: "Local".into(),
            kind: ChannelKind::Local,
            line: ChatLine { stamp: Stamp(0), sender: "Bob".into(), text: "hi".into() },
            targets: targets
                .into_iter()
                .map(|(n, reason, prefs)| AlertTarget { pilot_id: None, pilot_name: n.to_string(), reason, prefs })
                .collect(),
            seen_by: vec![],
        }
    }

    /// Own-name mentions with default preferences.
    fn alert(names: &[&str]) -> Alert {
        alert_with(names.iter().map(|n| (*n, Reason::OwnName, Prefs::default())).collect())
    }

    /// Own-name mentions with overlays forced (Overlay delivery mode), for
    /// checking where an overlay lands when Auto would use the notification
    /// area instead (nothing focused).
    fn forced_overlay(names: &[&str]) -> Alert {
        let p = || Prefs { delivery: DeliveryMode::Overlay, ..Prefs::default() };
        alert_with(names.iter().map(|n| (*n, Reason::OwnName, p())).collect())
    }

    fn two() -> Vec<ClientState> {
        vec![client("Holden", 10, MON_L), client("Naomi", 20, MON_R)]
    }

    /// Own-name mentions default to a Beacon.
    fn ov(anchor: Anchor) -> Delivery {
        Delivery::Overlay { anchor, style: OverlayStyle::Beacon }
    }

    /// Own-name mentions default to a Beacon, and a notification carries it.
    fn toast(name: &str) -> Delivery {
        Delivery::Toast { switch_to: name.into(), style: OverlayStyle::Beacon }
    }

    fn only(d: &[Decision], name: &str) -> Outcome {
        d.iter().find(|d| d.pilot_name == name).unwrap().outcome.clone()
    }

    #[test]
    fn default_suppresses_only_the_focused_pilot() {
        let s = snap(two(), Some("Holden"));
        let d = route(&alert(&["Holden", "Naomi"]), &s, &RouterConfig::default());
        assert_eq!(only(&d, "Holden"), Outcome::Suppressed(SuppressedBy::FocusedPilot));
        // Naomi's own monitor (MON_R) is genuinely on screen too, but the
        // user is looking at Holden's - the alert must land there, not on a
        // screen nobody is watching just because it happens to belong to the
        // pilot it's about.
        assert_eq!(only(&d, "Naomi"), Outcome::Deliver(vec![ov(Anchor::Monitor(MON_L))]));
    }

    #[test]
    fn everything_goes_through_when_focus_is_not_on_an_eve_client() {
        let s = snap(two(), None);
        let d = route(&alert(&["Holden", "Naomi"]), &s, &RouterConfig::default());
        assert!(d.iter().all(|d| matches!(d.outcome, Outcome::Deliver(_))));
    }

    #[test]
    fn allow_all_and_visible_policies_come_from_each_targets_prefs() {
        let s = snap(two(), Some("Holden"));
        let allow = Prefs { suppression: Suppression::AllowAll, ..Prefs::default() };
        let d = route(&alert_with(vec![("Holden", Reason::OwnName, allow)]), &s, &RouterConfig::default());
        assert!(matches!(only(&d, "Holden"), Outcome::Deliver(_)));

        let visible = || Prefs { suppression: Suppression::VisibleOnScreen, ..Prefs::default() };
        let a = alert_with(vec![("Holden", Reason::OwnName, visible()), ("Naomi", Reason::OwnName, visible())]);
        let d = route(&a, &s, &RouterConfig::default());
        assert_eq!(only(&d, "Naomi"), Outcome::Suppressed(SuppressedBy::VisibleOnScreen), "visible on the other monitor");

        // A minimized client is not visible, so it still alerts.
        let mut clients = two();
        clients[1].minimized = true;
        let d = route(&a, &snap(clients, Some("Holden")), &RouterConfig::default());
        assert!(matches!(only(&d, "Naomi"), Outcome::Deliver(_)));
    }

    #[test]
    fn with_no_eve_client_focused_auto_uses_the_notification_area() {
        // The user is in another app with the clients still visible behind
        // it: an overlay (owned by its client, not topmost) would land behind
        // that app, so Auto delivery uses the notification area instead.
        let d = route(&alert(&["Holden", "Naomi"]), &snap(two(), None), &RouterConfig::default());
        assert_eq!(only(&d, "Holden"), Outcome::Deliver(vec![toast("Holden")]));
        assert_eq!(only(&d, "Naomi"), Outcome::Deliver(vec![toast("Naomi")]));
    }

    #[test]
    fn being_away_overrides_suppression_and_uses_a_toast() {
        let mut s = snap(two(), Some("Holden"));
        s.idle = Duration::from_secs(10 * 60);
        let d = route(&alert(&["Holden"]), &s, &RouterConfig::default());
        assert_eq!(only(&d, "Holden"), Outcome::Deliver(vec![toast("Holden")]));
    }

    #[test]
    fn no_client_on_screen_means_a_toast_and_a_blocked_toast_falls_back_to_an_overlay() {
        let mut clients = two();
        for c in &mut clients {
            c.cloaked = true; // both on another virtual desktop
            c.rect = None;
        }
        let mut s = snap(clients, None);
        let d = route(&alert(&["Naomi"]), &s, &RouterConfig::default());
        assert_eq!(only(&d, "Naomi"), Outcome::Deliver(vec![toast("Naomi")]));

        s.notifications_ok = false;
        let d = route(&alert(&["Naomi"]), &s, &RouterConfig::default());
        assert_eq!(
            only(&d, "Naomi"),
            Outcome::Deliver(vec![ov(Anchor::Monitor(MON_R))]),
            "falls back to the pilot's last known monitor"
        );
    }

    #[test]
    fn windowed_clients_get_a_following_overlay_and_borderless_ones_a_monitor_overlay() {
        let mut clients = two();
        clients[0].rect = Some(Rect { left: -1500, top: 100, right: -300, bottom: 900 }); // Windowed
        let d = route(&forced_overlay(&["Holden", "Naomi"]), &snap(clients, None), &RouterConfig::default());
        assert_eq!(only(&d, "Holden"), Outcome::Deliver(vec![ov(Anchor::FollowWindow { hwnd: 10 })]));
        assert_eq!(only(&d, "Naomi"), Outcome::Deliver(vec![ov(Anchor::Monitor(MON_R))]));
    }

    #[test]
    fn a_hidden_pilots_alert_anchors_to_whichever_client_the_user_is_looking_at() {
        // Naomi is minimized (not on screen); the user is looking at Holden.
        // The alert must land on Holden's screen, not on Naomi's own,
        // last-known, currently-invisible one — that's the whole point of an
        // alert about a pilot you aren't looking at.
        let mut clients = two();
        clients[1].minimized = true;
        clients[1].rect = None; // monitor stays: last known
        let d = route(&alert(&["Naomi"]), &snap(clients, Some("Holden")), &RouterConfig::default());
        assert_eq!(only(&d, "Naomi"), Outcome::Deliver(vec![ov(Anchor::Monitor(MON_L))]));
    }

    #[test]
    fn a_hidden_pilots_alert_still_borrows_a_visible_clients_screen_without_focus() {
        // Nobody is focused (say, the user alt-tabbed to a third, non-EVE
        // app), but Holden's client is still sitting visibly on screen, and
        // overlays are forced (Auto would use the notification area here). That is strictly better to anchor to than Naomi's own,
        // definitely-invisible last known spot.
        let mut clients = two();
        clients[1].minimized = true;
        clients[1].rect = None;
        let d = route(&forced_overlay(&["Naomi"]), &snap(clients, None), &RouterConfig::default());
        assert_eq!(only(&d, "Naomi"), Outcome::Deliver(vec![ov(Anchor::Monitor(MON_L))]));
    }

    #[test]
    fn a_hidden_pilots_alert_falls_back_to_its_own_last_monitor_when_nothing_is_visible_at_all() {
        // Every client is hidden: no screen to borrow, so fall back to the
        // alerted pilot's own last known monitor as the least-bad guess.
        // (Blocked toast forces the overlay path so there is an anchor to
        // check; Auto would otherwise just toast here, same as the "no
        // client on screen" test above.)
        let mut clients = two();
        clients[0].minimized = true;
        clients[1].minimized = true;
        clients[1].rect = None;
        let mut s = snap(clients, None);
        s.notifications_ok = false;
        let d = route(&alert(&["Naomi"]), &s, &RouterConfig::default());
        assert_eq!(only(&d, "Naomi"), Outcome::Deliver(vec![ov(Anchor::Monitor(MON_R))]));
    }

    #[test]
    fn delivery_modes_and_sound_come_from_each_targets_prefs() {
        let s = snap(two(), None);
        let both = Prefs { delivery: DeliveryMode::Both, sound: true, ..Prefs::default() };
        let sound = Prefs { delivery: DeliveryMode::SoundOnly, ..Prefs::default() };
        let a = alert_with(vec![("Holden", Reason::OwnName, both), ("Naomi", Reason::OwnName, sound)]);
        let d = route(&a, &s, &RouterConfig::default());
        assert_eq!(
            only(&d, "Holden"),
            Outcome::Deliver(vec![ov(Anchor::Monitor(MON_L)), toast("Holden"), Delivery::Sound])
        );
        assert_eq!(only(&d, "Naomi"), Outcome::Deliver(vec![Delivery::Sound]));
    }

    #[test]
    fn an_unknown_client_still_borrows_a_visible_viewers_screen() {
        // No client at all is known for this pilot, but Holden's is visible;
        // same principle as a hidden pilot's alert - anchor to what the user
        // can actually see rather than nowhere in particular.
        let d = route(&forced_overlay(&["Ghost"]), &snap(two(), None), &RouterConfig::default());
        assert_eq!(only(&d, "Ghost"), Outcome::Deliver(vec![ov(Anchor::Monitor(MON_L))]));
    }

    #[test]
    fn an_unknown_client_falls_back_to_an_unknown_anchor_when_nothing_is_visible() {
        let mut clients = two();
        for c in &mut clients {
            c.minimized = true;
        }
        let mut s = snap(clients, None);
        s.notifications_ok = false;
        let d = route(&alert(&["Ghost"]), &s, &RouterConfig::default());
        assert_eq!(only(&d, "Ghost"), Outcome::Deliver(vec![ov(Anchor::Unknown)]));
    }

    fn style_of(d: &[Decision]) -> OverlayStyle {
        match &d[0].outcome {
            Outcome::Deliver(v) => match &v[0] {
                Delivery::Overlay { style, .. } => *style,
                other => panic!("expected an overlay, got {other:?}"),
            },
            other => panic!("expected a delivery, got {other:?}"),
        }
    }

    #[test]
    fn the_style_follows_why_the_alert_fired_unless_the_settings_force_one() {
        let s = snap(two(), Some("Naomi"));
        let cfg = RouterConfig::default();
        let style = |r: Reason, p: Prefs| style_of(&route(&alert_with(vec![("Holden", r, p)]), &s, &cfg));
        let d = Prefs::default;
        assert_eq!(style(Reason::OwnName, d()), OverlayStyle::Beacon);
        assert_eq!(style(Reason::Keyword("jita".into()), d()), OverlayStyle::Panel);
        assert_eq!(style(Reason::Regex("x".into()), d()), OverlayStyle::Panel);
        assert_eq!(style(Reason::AlwaysChannel("Fleet".into()), d()), OverlayStyle::Strip);
        assert_eq!(style(Reason::AlwaysSender("Boss".into()), d()), OverlayStyle::Strip);
        // A style set in the settings wins over the reason.
        let forced = Prefs { style: Some(OverlayStyle::Strip), ..Prefs::default() };
        assert_eq!(style(Reason::OwnName, forced), OverlayStyle::Strip);
    }

    #[test]
    fn the_style_map_can_be_remapped() {
        let mut cfg = RouterConfig::default();
        cfg.styles.keyword = OverlayStyle::Strip;
        let s = snap(two(), Some("Naomi"));
        let a = alert_with(vec![("Holden", Reason::Keyword("k".into()), Prefs::default())]);
        assert_eq!(style_of(&route(&a, &s, &cfg)), OverlayStyle::Strip);
    }

    #[test]
    fn decisions_carry_the_caps_for_the_governor() {
        let cap = (LayerKey::Pilot, RateCap { per_minute: 3, over: OverCap::Fold });
        let p = Prefs { caps: vec![cap.clone()], ..Prefs::default() };
        let d = route(&alert_with(vec![("Holden", Reason::OwnName, p)]), &snap(two(), None), &RouterConfig::default());
        assert_eq!(d[0].caps, vec![cap]);
    }
}
