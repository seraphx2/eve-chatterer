//! Drives the core on a background thread: follow the logs, sample presence,
//! route alerts and hand them to the overlay windows. Nothing here draws.

use crate::audio;
use crate::overlay;
use crate::overlay::OverlayAlert;
use crate::state::{self, AppState};
use crate::toast::{self, ChatToast};
use eve_chatterer_core::channel::ChannelKind;
use eve_chatterer_core::engine::{Alert, Engine, EngineConfig, Event};
use eve_chatterer_core::governor::Governor;
use eve_chatterer_core::paths;
use eve_chatterer_core::pilots::{tag_from_name, PilotRegistry};
use eve_chatterer_core::prefs::{OverCap, OverlayStyle};
use eve_chatterer_core::presence::{Rect, Sampler, Snapshot};
use eve_chatterer_core::router::{self, Anchor, Decision, Delivery, Outcome, RouterConfig};
use eve_chatterer_core::rules::Reason;
use eve_chatterer_core::settings::{Settings, SettingsBook};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Manager};

const SAMPLE_EVERY: Duration = Duration::from_millis(250);
const TICK_EVERY: Duration = Duration::from_millis(500);

/// Per-pilot accent colors, picked by hashing the name so a pilot keeps its color.
const ACCENTS: [&str; 6] = ["#62d1a5", "#a58af0", "#6cb6f0", "#e58aa8", "#d9c15a", "#9bd45f"];

pub fn accent_for(name: &str) -> String {
    let h = name.to_lowercase().bytes().fold(0xcbf29ce4u32, |h, b| (h ^ u32::from(b)).wrapping_mul(0x0100_0193));
    ACCENTS[h as usize % ACCENTS.len()].to_string()
}

/// The Strip badge text for an alert: the pilot's own tag if the registry has
/// one for it, otherwise derived from the name. Works with no matching pilot
/// (a synthetic test alert) by falling back to the name-derived tag.
pub fn tag_for(app: &AppHandle, pilot_id: Option<&str>, pilot_name: &str) -> String {
    let state = app.state::<AppState>();
    let engine = state.engine.lock().unwrap();
    pilot_id
        .and_then(|id| engine.pilots().get(id))
        .map(eve_chatterer_core::pilots::Pilot::display_tag)
        .unwrap_or_else(|| tag_from_name(pilot_name))
}

/// This character's saved overlay position/width, if it has one — looked up
/// fresh on every alert rather than cached, since it can change at any time
/// via reposition mode.
pub fn placement_for(app: &AppHandle, pilot_id: Option<&str>) -> Option<eve_chatterer_core::pilots::OverlayPlacement> {
    let state = app.state::<AppState>();
    let engine = state.engine.lock().unwrap();
    pilot_id.and_then(|id| engine.pilots().get(id)?.placement)
}

pub fn style_name(s: OverlayStyle) -> &'static str {
    match s {
        OverlayStyle::Panel => "panel",
        OverlayStyle::Strip => "strip",
        OverlayStyle::Beacon => "beacon",
    }
}

/// How long an alert stays up, by style.
pub fn lifetime_ms(s: OverlayStyle) -> u32 {
    match s {
        OverlayStyle::Panel => 9_000,
        OverlayStyle::Strip => 6_000,
        OverlayStyle::Beacon => 12_000,
    }
}

/// The channel as the player knows it. EVE names private conversation logs
/// "Private Chat (N)", a name the game never shows (its window is titled
/// with the other person, who is the alert's sender anyway).
pub fn channel_label(kind: ChannelKind, name: &str) -> String {
    match kind {
        ChannelKind::Private => "Private chat".to_string(),
        _ => name.to_string(),
    }
}

/// Plain-language reason and the tone (color) it takes.
pub fn reason_text(r: &Reason) -> (String, &'static str) {
    match r {
        Reason::OwnName => ("Mentioned you".to_string(), "mention"),
        Reason::Keyword(k) => (format!("Keyword: {k}"), "keyword"),
        Reason::Regex(_) => ("Matched a pattern".to_string(), "keyword"),
        // The channel is already shown beside the reason everywhere.
        Reason::AlwaysChannel(_) => ("Every message".to_string(), "always"),
        Reason::AlwaysSender(s) => (format!("Always alert: {s}"), "always"),
    }
}

pub fn monitor_rects(app: &AppHandle) -> Vec<Rect> {
    app.available_monitors()
        .unwrap_or_default()
        .iter()
        .map(|m| Rect {
            left: m.position().x,
            top: m.position().y,
            right: m.position().x + m.size().width as i32,
            bottom: m.position().y + m.size().height as i32,
        })
        .collect()
}

fn primary_rect(app: &AppHandle) -> Option<Rect> {
    let m = app.primary_monitor().ok()??;
    Some(Rect {
        left: m.position().x,
        top: m.position().y,
        right: m.position().x + m.size().width as i32,
        bottom: m.position().y + m.size().height as i32,
    })
}

/// (monitor, region to place in, the EVE client that region belongs to).
/// A monitor anchor (fullscreen client) still belongs to the on-screen
/// client covering that monitor, preferring the focused one, so the overlay
/// can be owned by it (overlay.rs, `Placement::owner`).
fn placement(app: &AppHandle, anchor: Anchor, snap: &Snapshot) -> Option<(Rect, Rect, Option<isize>)> {
    match anchor {
        Anchor::Monitor(m) => {
            let on_m = |c: &&eve_chatterer_core::presence::ClientState| c.on_screen() && c.monitor == Some(m);
            let owner = snap
                .clients
                .iter()
                .filter(on_m)
                .find(|c| snap.focused.as_deref().is_some_and(|f| f.eq_ignore_ascii_case(&c.character)))
                .or_else(|| snap.clients.iter().find(on_m));
            Some((m, owner.and_then(|c| c.rect).unwrap_or(m), owner.map(|c| c.hwnd)))
        }
        Anchor::FollowWindow { hwnd } => snap.clients.iter().find(|c| c.hwnd == hwnd).and_then(|c| {
            let m = c.monitor?;
            Some((m, c.rect.unwrap_or(m), Some(hwnd)))
        }),
        Anchor::Unknown => primary_rect(app).map(|m| (m, m, None)),
    }
}

/// The engine, built at startup whether or not EVE's chat log folder exists
/// yet: its rescan starts following the folder as soon as it appears, and
/// Settings works in the meantime.
pub fn build_engine(settings: Settings, pilots: PilotRegistry) -> Engine {
    // Only if Windows can't name the Documents folder at all: an empty path
    // is never a folder, so nothing is followed (`LogFolder` says why).
    let dir = paths::chatlogs_path().unwrap_or_default();
    Engine::new(dir, EngineConfig::default(), pilots, SettingsBook::new(settings))
}

/// Whether EVE's chat log folder is there yet. A new player usually hasn't
/// turned on "Log chat to file", so the folder may appear long after startup:
/// say once that the app is waiting, and once when it starts.
struct LogFolder {
    path: Option<PathBuf>,
    found: bool,
}

impl LogFolder {
    fn new() -> LogFolder {
        let path = paths::chatlogs_path();
        let found = path.as_ref().is_some_and(|p| p.is_dir());
        match &path {
            None => toast::plain(
                "EVE Chatterer can't find EVE's chat logs",
                "Windows didn't say where your Documents folder is, so EVE's chat logs can't be found.",
            ),
            Some(_) if !found => toast::plain(
                "Waiting for EVE's chat logs",
                "Turn on \"Log chat to file\" in EVE's chat settings. Alerts start as soon as EVE writes its first chat log.",
            ),
            Some(p) => println!("watching {}", p.display()),
        }
        LogFolder { path, found }
    }

    fn check(&mut self) {
        if self.found {
            return;
        }
        if let Some(p) = self.path.as_ref().filter(|p| p.is_dir()) {
            self.found = true;
            println!("chat log folder appeared: {}", p.display());
            toast::plain("Found EVE's chat logs", "Chat alerts are on.");
        }
    }
}

struct Runner {
    app: AppHandle,
    /// Shared with `AppState::engine` so settings commands can reach in;
    /// locked briefly once per tick.
    engine: Arc<Mutex<Engine>>,
    sampler: Sampler,
    governor: Governor,
    router_cfg: RouterConfig,
    logs: LogFolder,
    last_client_log: Option<Instant>,
}

pub fn spawn(app: AppHandle) {
    let result = std::thread::Builder::new().name("core-runner".into()).spawn(move || {
        let engine = app.state::<AppState>().engine.clone();
        let mut r = Runner {
            app,
            engine,
            sampler: Sampler::new(),
            governor: Governor::new(),
            router_cfg: RouterConfig::default(),
            logs: LogFolder::new(),
            last_client_log: None,
        };
        r.run_loop()
    });
    if let Err(e) = result {
        eprintln!("could not start the core thread: {e}");
        toast::plain("EVE Chatterer is not watching", &format!("Its background thread couldn't start: {e}"));
    }
}

impl Runner {
    fn run_loop(&mut self) -> ! {
        let mut last_tick: Option<Instant> = None;
        let app = self.app.clone();
        let state = app.state::<AppState>();
        loop {
            let now = Instant::now();
            let snap = self.sampler.sample(now);
            *state.last_snapshot.lock().unwrap() = Some(snap.clone());
            state.overlays.follow(&snap);
            if last_tick.is_none_or(|t| now.duration_since(t) >= TICK_EVERY) {
                last_tick = Some(now);
                self.logs.check();
                let names: Vec<&str> = snap.clients.iter().map(|c| c.character.as_str()).collect();
                if self.last_client_log.is_none_or(|t| now.duration_since(t) >= Duration::from_secs(30)) {
                    self.last_client_log = Some(now);
                    println!("clients: {}  focused: {:?}", snap.clients.iter().map(|c| format!("{}[{}]", c.character, if c.on_screen() {"on-screen"} else if c.minimized {"minimized"} else {"hidden"})).collect::<Vec<_>>().join(", "), snap.focused);
                }
                let events = {
                    let mut engine = self.engine.lock().unwrap();
                    let mut events = engine.observe_clients(&names, now);
                    events.extend(engine.tick(now));
                    events
                };
                for ev in events {
                    self.handle(ev, &snap, now);
                }
            }
            state.overlays.reap();
            std::thread::sleep(SAMPLE_EVERY);
        }
    }

    fn handle(&mut self, ev: Event, snap: &Snapshot, now: Instant) {
        match ev {
            Event::Alert(a) => {
                println!(
                    "ALERT  {} ({:?})  {}: {}   for: {}",
                    a.channel_name,
                    a.kind,
                    a.line.sender,
                    a.line.text,
                    a.targets.iter().map(|t| t.pilot_name.as_str()).collect::<Vec<_>>().join(", ")
                );
                let decisions = self.governor.apply(router::route(&a, snap, &self.router_cfg), now);
                for d in &decisions {
                    println!("       {} ({:?}) -> {:?}", d.pilot_name, d.reason, d.outcome);
                    self.deliver(&a, d, snap);
                }
                self.sound(&decisions);
            }
            Event::NewPilot(p) => {
                self.save_pilots();
                toast::plain(&format!("New character: {}", p.name), "Chat alerts are on for this character. Open EVE Chatterer from the tray to adjust them.");
            }
            Event::PilotInLogs(_) | Event::PilotUpdated { .. } => self.save_pilots(),
            Event::ChatLoggingOff { name } => {
                toast::plain(&format!("No chat log for {name}"), "Turn on \"Log chat to file\" in EVE's chat settings so alerts can work for this character.");
            }
            Event::Discovery(_) => {}
        }
    }

    fn save_pilots(&self) {
        if let Err(e) = state::save_pilots(&self.engine.lock().unwrap()) {
            eprintln!("{e}");
        }
    }

    /// One sound for one message, however many characters it alerted. A
    /// mention decides the file (and cuts through the cooldown); otherwise
    /// the first character that asked for a sound does.
    fn sound(&self, decisions: &[Decision]) {
        let wants = |d: &&Decision| matches!(&d.outcome, Outcome::Deliver(v) if v.contains(&Delivery::Sound));
        let asking: Vec<&Decision> = decisions.iter().filter(wants).collect();
        let Some(d) = asking.iter().find(|d| matches!(d.reason, Reason::OwnName)).or(asking.first()) else {
            return;
        };
        let settings = self.engine.lock().unwrap().settings().settings().audio.clone();
        if let Some(source) = settings.source_for(d.pilot_id.as_deref()) {
            audio::alert(source, settings.gain(), settings.cooldown(), matches!(d.reason, Reason::OwnName));
        }
    }

    fn pilot_id_by_name(&self, name: &str) -> Option<String> {
        self.engine.lock().unwrap().pilots().by_name(name).map(|p| p.id.clone())
    }

    fn deliver(&self, alert: &Alert, d: &Decision, snap: &Snapshot) {
        match &d.outcome {
            Outcome::Deliver(deliveries) => {
                for delivery in deliveries {
                    match delivery {
                        Delivery::Overlay { anchor, style } => self.overlay(alert, d, snap, *anchor, *style, false),
                        Delivery::Toast { switch_to, style } => toast::chat(self.chat_toast(alert, d, switch_to, *style)),
                        Delivery::Sound => {} // once per alert, not per character: `sound`
                    }
                }
            }
            // Over a rate cap: into the count of the alert already showing
            // where this one would have gone, or shown as a Strip when
            // nothing is there to fold into, so a capped line is never lost.
            Outcome::Limited { over: OverCap::Fold, deliveries } => {
                for delivery in deliveries {
                    match delivery {
                        Delivery::Overlay { anchor, .. } => self.overlay(alert, d, snap, *anchor, OverlayStyle::Strip, true),
                        Delivery::Toast { switch_to, .. } => toast::fold(self.chat_toast(alert, d, switch_to, OverlayStyle::Strip)),
                        Delivery::Sound => {}
                    }
                }
            }
            Outcome::Limited { over: OverCap::Drop, .. } | Outcome::Suppressed(_) => {}
        }
    }

    fn overlay(&self, alert: &Alert, d: &Decision, snap: &Snapshot, anchor: Anchor, style: OverlayStyle, fold: bool) {
        let Some((monitor, region, owner)) = placement(&self.app, anchor, snap) else {
            println!("       could not place the overlay: anchor={anchor:?} had no monitor");
            return;
        };
        println!("       showing {style:?} on monitor ({},{})-({},{})", monitor.left, monitor.top, monitor.right, monitor.bottom);
        let (reason, tone) = reason_text(&d.reason);
        // Position, width and window belong to the client the alert is drawn
        // over, whoever it is for: an alert for one character shown on
        // another's screen (the one being looked at) lands in that client's
        // box, in its stack (owner decision 2026-09-30). The alerted pilot is
        // still named on the alert. No client under it: the alerted pilot's.
        let host = owner.and_then(|h| snap.clients.iter().find(|c| c.hwnd == h)).map(|c| c.character.clone());
        let (host_id, host_name) = match host {
            Some(name) => (self.pilot_id_by_name(&name), name),
            None => (d.pilot_id.clone(), d.pilot_name.clone()),
        };
        let saved = placement_for(&self.app, host_id.as_deref());
        let width = saved.map(|p| p.width).unwrap_or(overlay::DEFAULT_OVERLAY_WIDTH);
        let key = overlay::overlay_key(host_id.as_deref(), &host_name);
        // Relative to whatever region the router picked (the client window, or
        // its monitor when fullscreen), so it always lands inside the game.
        let p = overlay::Placement { monitor, region, width, custom_pos: saved, owner };
        let overlays = &self.app.state::<AppState>().overlays;
        let shown = OverlayAlert {
            id: overlays.next_id(),
            style: style_name(style),
            pilot: d.pilot_name.clone(),
            tag: tag_for(&self.app, d.pilot_id.as_deref(), &d.pilot_name),
            accent: accent_for(&d.pilot_name),
            channel: channel_label(alert.kind, &alert.channel_name),
            channel_id: alert.channel_id.clone(),
            sender: alert.line.sender.clone(),
            text: alert.line.text.clone(),
            reason,
            tone,
            lifetime_ms: lifetime_ms(style),
            count: 1,
            stack_up: false,
        };
        if fold {
            overlays.fold(&self.app, &key, p, shown);
        } else {
            overlays.show(&self.app, &key, p, shown);
        }
    }

    fn chat_toast(&self, alert: &Alert, d: &Decision, switch_to: &str, style: OverlayStyle) -> ChatToast {
        let (reason, tone) = reason_text(&d.reason);
        let mention = matches!(d.reason, Reason::OwnName);
        ChatToast {
            key: toast::key_for(switch_to, &alert.channel_name, mention),
            pilot: switch_to.to_string(),
            tag: tag_for(&self.app, d.pilot_id.as_deref(), switch_to),
            accent: accent_for(switch_to),
            tone,
            sender: alert.line.sender.clone(),
            channel: channel_label(alert.kind, &alert.channel_name),
            text: alert.line.text.clone(),
            reason,
            style: style_name(style),
            mention,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_pilot_keeps_its_accent_and_the_case_does_not_matter() {
        assert_eq!(accent_for("Holden"), accent_for("holden"));
        assert!(ACCENTS.contains(&accent_for("Naomi Nagata").as_str()));
    }

    #[test]
    fn reasons_read_as_plain_language() {
        assert_eq!(reason_text(&Reason::OwnName), ("Mentioned you".to_string(), "mention"));
        assert_eq!(reason_text(&Reason::Keyword("jita".into())).0, "Keyword: jita");
        assert_eq!(reason_text(&Reason::AlwaysChannel("Fleet".into())).1, "always");
    }
}
