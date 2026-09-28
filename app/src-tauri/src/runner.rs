//! Drives the core on a background thread: follow the logs, sample presence,
//! route alerts and hand them to the overlay windows. Nothing here draws.

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
use tauri_plugin_notification::NotificationExt;

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
fn reason_text(r: &Reason) -> (String, &'static str) {
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

/// (monitor, region to center in).
fn placement(app: &AppHandle, anchor: Anchor, snap: &Snapshot) -> Option<(Rect, Rect)> {
    match anchor {
        Anchor::Monitor(m) => Some((m, m)),
        Anchor::FollowWindow { hwnd } => snap.clients.iter().find(|c| c.hwnd == hwnd).and_then(|c| {
            let m = c.monitor?;
            Some((m, c.rect.unwrap_or(m)))
        }),
        Anchor::Unknown => primary_rect(app).map(|m| (m, m)),
    }
}

fn notify(app: &AppHandle, title: &str, body: &str) {
    if let Err(e) = app.notification().builder().title(title).body(body).show() {
        eprintln!("notification failed: {e}");
    }
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
    let cfg_dir = app.path().app_config_dir().map_err(|e| e.to_string())?;
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
            }
            Event::NewPilot(p) => {
                self.save_pilots();
                notify(&self.app, &format!("New character: {}", p.name), "Chat alerts are on for this character. Open EVE Chatterer from the tray to adjust them.");
            }
            Event::PilotInLogs(_) => self.save_pilots(),
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

    fn deliver(&self, alert: &Alert, d: &Decision, snap: &Snapshot) {
        let state = self.app.state::<AppState>();
        match &d.outcome {
            Outcome::Deliver(deliveries) => {
                for delivery in deliveries {
                    match delivery {
                        Delivery::Overlay { anchor, style } => {
                            let Some((monitor, region)) = placement(&self.app, *anchor, snap) else {
                                println!("       could not place the overlay: anchor={anchor:?} had no monitor");
                                continue;
                            };
                            println!("       showing {style:?} on monitor ({},{})-({},{})", monitor.left, monitor.top, monitor.right, monitor.bottom);
                            let (reason, tone) = reason_text(&d.reason);
                            state.overlays.show(
                                &self.app,
                                monitor,
                                region,
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
                                },
                            );
                            state.alerts_shown.fetch_add(1, Ordering::Relaxed);
                        }
                        Delivery::Toast { switch_to } => {
                            notify(
                                &self.app,
                                &format!("{} in {}", alert.line.sender, alert.channel_name),
                                &format!("{}\n(for {switch_to})", alert.line.text),
                            );
                            state.alerts_shown.fetch_add(1, Ordering::Relaxed);
                        }
                        Delivery::Sound => {} // sound comes with the audio milestone
                    }
                }
            }
            Outcome::Limited(OverCap::Fold) => {
                state.overlays.fold(&self.app, Fold { pilot: d.pilot_name.clone(), channel: alert.channel_name.clone() });
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
        assert_eq!(accent_for("Jarna"), accent_for("jarna"));
        assert!(ACCENTS.contains(&accent_for("Psianna Archeia").as_str()));
    }

    #[test]
    fn reasons_read_as_plain_language() {
        assert_eq!(reason_text(&Reason::OwnName), ("Mentioned you".to_string(), "mention"));
        assert_eq!(reason_text(&Reason::Keyword("jita".into())).0, "Keyword: jita");
        assert_eq!(reason_text(&Reason::AlwaysChannel("Fleet".into())).1, "always");
    }
}
