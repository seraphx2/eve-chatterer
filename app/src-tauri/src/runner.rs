//! Drives the core on a background thread: follow the logs, sample presence,
//! route alerts and hand them to the overlay windows. Nothing here draws.

use crate::audio;
use crate::overlay;
use crate::overlay::{Fold, OverlayAlert};
use crate::state::{AppState, PilotView};
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
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Manager};
use crate::toast::{self, ChatToast};

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
/// (a synthetic test alert, or mid-startup) by falling back to the
/// name-derived tag either way.
pub fn tag_for(app: &AppHandle, pilot_id: Option<&str>, pilot_name: &str) -> String {
    let state = app.state::<AppState>();
    let guard = state.engine.lock().unwrap();
    pilot_id
        .and_then(|id| guard.as_ref()?.pilots().get(id))
        .map(eve_chatterer_core::pilots::Pilot::display_tag)
        .unwrap_or_else(|| tag_from_name(pilot_name))
}

/// This character's saved overlay position/width, if it has one — looked up
/// fresh on every alert rather than cached, since it can change at any time
/// via reposition mode.
pub fn placement_for(app: &AppHandle, pilot_id: Option<&str>) -> Option<eve_chatterer_core::pilots::OverlayPlacement> {
    let state = app.state::<AppState>();
    let guard = state.engine.lock().unwrap();
    pilot_id.and_then(|id| guard.as_ref()?.pilots().get(id)?.placement)
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

/// Plain-language reason and the tone (color) it takes.
pub fn reason_text(r: &Reason) -> (String, &'static str) {
    match r {
        Reason::OwnName => ("Mentioned you".to_string(), "mention"),
        Reason::Keyword(k) => (format!("Keyword: {k}"), "keyword"),
        Reason::Regex(_) => ("Matched a pattern".to_string(), "keyword"),
        Reason::AlwaysChannel(c) => (format!("Always alert: {c}"), "always"),
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

/// An app-level Windows notification (not a chat alert).
fn notify(_app: &AppHandle, title: &str, body: &str) {
    toast::plain(title, body);
}

struct Runner {
    app: AppHandle,
    /// Shared with `AppState::engine` so settings commands can reach in
    /// (`get_settings_data` / `save_settings`); locked briefly once per tick.
    engine: Arc<Mutex<Option<Engine>>>,
    sampler: Sampler,
    governor: Governor,
    router_cfg: RouterConfig,
    pilots_path: PathBuf,
    last_client_log: Option<Instant>,
}

pub fn spawn(app: AppHandle) {
    let result = std::thread::Builder::new().name("core-runner".into()).spawn(move || {
        if let Err(e) = run(app.clone()) {
            eprintln!("the core stopped: {e}");
            notify(&app, "EVE Chatterer is not watching", &e);
            app.state::<AppState>().status.lock().unwrap().running = false;
        }
    });
    if let Err(e) = result {
        eprintln!("could not start the core thread: {e}");
    }
}

fn run(app: AppHandle) -> Result<(), String> {
    let cfg_dir = crate::storage::config_dir();
    let settings = Settings::load(&cfg_dir.join("settings.json")).unwrap_or_else(|e| {
        eprintln!("could not read settings.json, using the defaults: {e}");
        Settings::with_defaults()
    });
    let pilots_path = cfg_dir.join("pilots.json");
    let pilots = PilotRegistry::load(&pilots_path).unwrap_or_else(|e| {
        eprintln!("could not read pilots.json, starting empty: {e}");
        PilotRegistry::default()
    });
    let dir = paths::chatlogs_dir().ok_or("Could not find Documents\\EVE\\logs\\Chatlogs. Is chat logging turned on in EVE?")?;
    let engine_slot = app.state::<AppState>().engine.clone();
    *engine_slot.lock().unwrap() = Some(Engine::new(&dir, EngineConfig::default(), pilots, SettingsBook::new(settings)));
    {
        let state = app.state::<AppState>();
        let mut st = state.status.lock().unwrap();
        st.log_folder = Some(dir.display().to_string());
        st.running = true;
    }

    let mut r = Runner {
        engine: engine_slot,
        sampler: Sampler::new(),
        governor: Governor::new(),
        router_cfg: RouterConfig::default(),
        pilots_path,
        app,
        last_client_log: None,
    };
    r.run_loop()
}

impl Runner {
    fn run_loop(&mut self) -> ! {
        let mut last_tick: Option<Instant> = None;
        loop {
            let now = Instant::now();
            let snap = self.sampler.sample(now);
            *self.app.state::<AppState>().last_snapshot.lock().unwrap() = Some(snap.clone());
            self.app.state::<AppState>().overlays.follow(&self.app, &snap);
            if last_tick.is_none_or(|t| now.duration_since(t) >= TICK_EVERY) {
                last_tick = Some(now);
                let names: Vec<&str> = snap.clients.iter().map(|c| c.character.as_str()).collect();
                if self.last_client_log.is_none_or(|t| now.duration_since(t) >= Duration::from_secs(30)) {
                    self.last_client_log = Some(now);
                    println!("clients: {}  focused: {:?}", snap.clients.iter().map(|c| format!("{}[{}]", c.character, if c.on_screen() {"on-screen"} else if c.minimized {"minimized"} else {"hidden"})).collect::<Vec<_>>().join(", "), snap.focused);
                }
                let events = {
                    let mut guard = self.engine.lock().unwrap();
                    let engine = guard.as_mut().expect("engine is set before run_loop starts");
                    let mut events = engine.observe_clients(&names, now);
                    events.extend(engine.tick(now));
                    events
                };
                for ev in events {
                    self.handle(ev, &snap, now);
                }
                self.publish_pilots();
            }
            self.app.state::<AppState>().overlays.reap();
            std::thread::sleep(SAMPLE_EVERY);
        }
    }

    fn publish_pilots(&self) {
        let state = self.app.state::<AppState>();
        let guard = self.engine.lock().unwrap();
        let engine = guard.as_ref().expect("engine is set before run_loop starts");
        let mut st = state.status.lock().unwrap();
        st.pilots = engine.pilots().iter().map(|p| PilotView { id: p.id.clone(), name: p.name.clone(), live: p.live }).collect();
        st.pilots.sort_by(|a, b| b.live.cmp(&a.live).then_with(|| a.name.cmp(&b.name)));
        st.alerts_shown = state.alerts_shown.load(Ordering::Relaxed);
        st.overlay_windows = state.overlays.window_count();
    }

    fn handle(&mut self, ev: Event, snap: &Snapshot, now: Instant) {
        match ev {
            Event::Alert(a) => {
                println!(
                    "ALERT  {} ({:?})  {}: {}   seen by: {}",
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
                notify(&self.app, &format!("New character: {}", p.name), "Chat alerts are on for this character. Open EVE Chatterer from the tray to adjust them.");
            }
            Event::PilotInLogs(_) | Event::PilotUpdated { .. } => self.save_pilots(),
            Event::ChatLoggingOff { name } => {
                notify(&self.app, &format!("No chat log for {name}"), "Turn on \"Log chat to file\" in EVE's chat settings so alerts can work for this character.");
            }
            Event::Discovery(_) => {}
        }
    }

    fn save_pilots(&self) {
        let guard = self.engine.lock().unwrap();
        let engine = guard.as_ref().expect("engine is set before run_loop starts");
        if let Err(e) = engine.pilots().save(&self.pilots_path) {
            eprintln!("could not save pilots.json: {e}");
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
        let settings = {
            let guard = self.engine.lock().unwrap();
            guard.as_ref().expect("engine is set before run_loop starts").settings().settings().audio.clone()
        };
        if let Some(source) = settings.source_for(d.pilot_id.as_deref()) {
            audio::alert(source, settings.gain(), settings.cooldown(), matches!(d.reason, Reason::OwnName));
        }
    }

    fn deliver(&self, alert: &Alert, d: &Decision, snap: &Snapshot) {
        let state = self.app.state::<AppState>();
        match &d.outcome {
            Outcome::Deliver(deliveries) => {
                for delivery in deliveries {
                    match delivery {
                        Delivery::Overlay { anchor, style } => {
                            let Some((monitor, region, owner)) = placement(&self.app, *anchor, snap) else {
                                println!("       could not place the overlay: anchor={anchor:?} had no monitor");
                                continue;
                            };
                            println!("       showing {style:?} on monitor ({},{})-({},{})", monitor.left, monitor.top, monitor.right, monitor.bottom);
                            let (reason, tone) = reason_text(&d.reason);
                            let saved = placement_for(&self.app, d.pilot_id.as_deref());
                            let width = saved.map(|p| p.width).unwrap_or(overlay::DEFAULT_OVERLAY_WIDTH);
                            // Relative to whatever region the router picked (the
                            // client window, or its monitor when fullscreen), so it
                            // always lands inside the game (overlay.rs, OverlayPlacement).
                            let custom_pos = saved;
                            let key = overlay::overlay_key(d.pilot_id.as_deref(), &d.pilot_name);
                            state.overlays.show(
                                &self.app,
                                &key,
                                overlay::Placement { monitor, region, width, custom_pos, owner },
                                OverlayAlert {
                                    id: state.overlays.next_id(),
                                    style: style_name(*style),
                                    pilot: d.pilot_name.clone(),
                                    tag: tag_for(&self.app, d.pilot_id.as_deref(), &d.pilot_name),
                                    accent: accent_for(&d.pilot_name),
                                    channel: alert.channel_name.clone(),
                                    sender: alert.line.sender.clone(),
                                    text: alert.line.text.clone(),
                                    reason,
                                    tone,
                                    lifetime_ms: lifetime_ms(*style),
                                    count: 1,
                                    stack_up: false,
                                },
                            );
                            state.alerts_shown.fetch_add(1, Ordering::Relaxed);
                        }
                        Delivery::Toast { switch_to, style } => {
                            let (reason, tone) = reason_text(&d.reason);
                            toast::chat(ChatToast {
                                key: toast::key_for(switch_to, &alert.channel_name, matches!(d.reason, Reason::OwnName)),
                                pilot: switch_to.clone(),
                                tag: tag_for(&self.app, d.pilot_id.as_deref(), switch_to),
                                accent: accent_for(switch_to),
                                tone,
                                sender: alert.line.sender.clone(),
                                channel: alert.channel_name.clone(),
                                text: alert.line.text.clone(),
                                reason,
                                style: style_name(*style),
                                mention: matches!(d.reason, Reason::OwnName),
                            });
                            state.alerts_shown.fetch_add(1, Ordering::Relaxed);
                        }
                        Delivery::Sound => {} // once per alert, not per character: `sound`
                    }
                }
            }
            Outcome::Limited(OverCap::Fold) => {
                state.overlays.fold(&self.app, Fold { pilot: d.pilot_name.clone(), channel: alert.channel_name.clone() });
                // And the notification, if the recent alerts went there.
                toast::fold(&toast::key_for(&d.pilot_name, &alert.channel_name, matches!(d.reason, Reason::OwnName)));
            }
            Outcome::Limited(OverCap::Drop) | Outcome::Suppressed(_) => {}
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
