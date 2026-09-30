//! Synthetic alerts for trying the overlays without any EVE activity. They go
//! straight to the overlay windows on every monitor.

use crate::overlay::OverlayAlert;
use crate::runner::{accent_for, lifetime_ms, monitor_rects, style_name, tag_for};
use crate::state::AppState;
use eve_chatterer_core::prefs::OverlayStyle;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Manager};

const SAMPLES: [(&str, &str); 6] = [
    ("Amos Burton", "Holden, are you on for the fleet tonight? We are forming up in Jita at 20:00."),
    ("Mr Nodes", "HyperNet offer: Zirnitra 4/8 cheap nodes"),
    ("Alex", "Anyone up for a mining op?"),
    ("FC Voss", "Align to the gate. Primary is the Loki, broadcast for reps."),
    ("Bob", "selling in Jita 4-4"),
    ("Teramalk", ".Zirnitra --> 11mil"),
];

fn alert(app: &AppHandle, pilot: &str, style: OverlayStyle, i: usize) -> OverlayAlert {
    let (sender, text) = SAMPLES[i % SAMPLES.len()];
    let (reason, tone) = match style {
        OverlayStyle::Beacon => ("Mentioned you", "mention"),
        OverlayStyle::Panel => ("Keyword: fleet", "keyword"),
        OverlayStyle::Strip => ("Every message", "always"),
    };
    OverlayAlert {
        id: app.state::<AppState>().overlays.next_id(),
        style: style_name(style),
        pilot: pilot.to_string(),
        tag: tag_for(app, pilot_id(app, pilot).as_deref(), pilot),
        accent: accent_for(pilot),
        channel: "Local".to_string(),
        channel_id: "local".to_string(),
        sender: sender.to_string(),
        text: text.to_string(),
        reason: reason.to_string(),
        tone,
        lifetime_ms: lifetime_ms(style),
        count: 1,
        stack_up: false,
    }
}

/// A real registry id where possible, so a preview matches what the pilot
/// would actually see if it has its own tag set.
fn pilot_id(app: &AppHandle, name: &str) -> Option<String> {
    app.state::<AppState>().engine.lock().unwrap().pilots().by_name(name).map(|p| p.id.clone())
}

/// A character to show test alerts for: one that has played, else any known one.
fn pilot_name(app: &AppHandle) -> String {
    let state = app.state::<AppState>();
    let engine = state.engine.lock().unwrap();
    engine.pilots().iter().min_by_key(|p| (!p.live, p.name.clone())).map(|p| p.name.clone()).unwrap_or_else(|| "Holden".to_string())
}

/// One test window per monitor: (key, placement). A distinct key per monitor,
/// not the real per-pilot key: this sends the "same" synthetic alert to every
/// monitor's own window at once, which a real per-pilot key (one window
/// total) could not do.
fn monitor_windows(app: &AppHandle) -> Vec<(String, crate::overlay::Placement)> {
    monitor_rects(app)
        .into_iter()
        .map(|m| (format!("test-{}-{}", m.left, m.top), crate::overlay::Placement { monitor: m, region: m, width: crate::overlay::DEFAULT_OVERLAY_WIDTH, custom_pos: None, owner: None }))
        .collect()
}

fn on_every_monitor(app: &AppHandle, style: OverlayStyle, i: usize) {
    let pilot = pilot_name(app);
    for (key, p) in monitor_windows(app) {
        app.state::<AppState>().overlays.show(app, &key, p, alert(app, &pilot, style, i));
    }
}

/// A line past its rate cap, in `channel`, on every monitor's window: it
/// bumps the count of the newest alert for that pilot and channel if one is
/// showing, or shows as a Strip (docs/CODE-REVIEW.md, H4).
fn fold_on_every_monitor(app: &AppHandle, channel: &str, channel_id: &str) {
    let pilot = pilot_name(app);
    for (key, p) in monitor_windows(app) {
        let a = OverlayAlert { channel: channel.into(), channel_id: channel_id.into(), ..alert(app, &pilot, OverlayStyle::Strip, 2) };
        app.state::<AppState>().overlays.fold(app, &key, p, a);
    }
}

/// `--selftest`: exercises the overlays and then exits, so a run can be
/// watched (or measured from outside) without touching the tray. Timeline:
/// alerts on every monitor at 3 s, a burst at 8 s, two rate-capped lines at
/// 15 s, exit at 90 s (the overlay windows are reaped 45 s after their last
/// alert, so their teardown is visible).
pub fn selftest(app: &AppHandle) {
    let app = app.clone();
    std::thread::spawn(move || {
        let t0 = std::time::Instant::now();
        let log = |what: &str| {
            println!("[selftest +{:>3}s] {what}", t0.elapsed().as_secs());
            crate::diag::diag(format!("selftest: {what}"));
        };
        std::thread::sleep(Duration::from_secs(3));
        log("all three styles on every monitor");
        send(&app, "all");
        std::thread::sleep(Duration::from_secs(5));
        log("burst of 12");
        send(&app, "burst");
        std::thread::sleep(Duration::from_secs(15u64.saturating_sub(t0.elapsed().as_secs())));
        log("capped Local line: the newest Local alert's count goes up by one");
        fold_on_every_monitor(&app, "Local", "local");
        std::thread::sleep(Duration::from_secs(1));
        log("capped Corp line with nothing to fold into: shows as a Strip");
        fold_on_every_monitor(&app, "Corp", "corp");
        for t in [20u64, 40, 70] {
            std::thread::sleep(Duration::from_secs(t - t0.elapsed().as_secs().min(t)));
            log(&format!("overlay windows open: {}", app.state::<AppState>().overlays.window_count()));
        }
        std::thread::sleep(Duration::from_secs(90u64.saturating_sub(t0.elapsed().as_secs())));
        log("done");
        app.exit(0);
    });
}

/// `--soak`: keeps alerts on screen continuously for a steady-state
/// measurement: a set of all three styles every 8 s for 50 s, then exit.
pub fn soak(app: &AppHandle) {
    let app = app.clone();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_secs(3));
        for _ in 0..7 {
            send(&app, "all");
            std::thread::sleep(Duration::from_secs(8));
        }
        std::thread::sleep(Duration::from_secs(2));
        app.exit(0);
    });
}

/// `--freezetest` (docs/CODE-REVIEW.md, H6): do overlays owned by an EVE
/// client tie that client's input to our UI thread? Every 8 s, one alert of
/// each style over every on-screen EVE client, owned by it exactly like a
/// real alert (unlike `--selftest` and `--soak`, whose windows have no
/// owner). At 30, 60 and 90 s the UI thread is blocked for 5 s. Everything
/// the player needs is in the game, since the console may be on another
/// virtual desktop: a Beacon over each client 3 s before a freeze, one chime
/// as it starts and two as it ends (sound plays on its own thread). Move the
/// camera and type in EVE between the chimes: if EVE stalls with us, the
/// input queues are attached. Exits at 120 s. Every step, with wall-clock
/// times to line up with a PresentMon capture, is also written to
/// `%TEMP%\eve-chatterer-freezetest.log`.
pub fn freezetest(app: &AppHandle) {
    use eve_chatterer_core::audio::Source;
    use std::io::Write;
    let app = app.clone();
    std::thread::spawn(move || {
        let t0 = Instant::now();
        let log_path = std::env::temp_dir().join("eve-chatterer-freezetest.log");
        let _ = std::fs::write(&log_path, "");
        let log = |what: &str| {
            let line = format!("[freezetest +{:>5.1}s {}] {what}", t0.elapsed().as_secs_f32(), wall_clock());
            println!("{line}");
            if let Ok(mut f) = std::fs::OpenOptions::new().append(true).open(&log_path) {
                let _ = writeln!(f, "{line}");
            }
        };
        let chime = || crate::audio::preview(Source::BuiltIn, 0.5);
        let pilot = pilot_name(&app);
        let freezes = [30u64, 60, 90].map(Duration::from_secs);
        let (mut next_warning, mut next_freeze) = (0, 0);
        let mut next_alerts = Duration::from_secs(3);
        let mut round = 0;
        log(&format!("started; log in {}", log_path.display()));
        loop {
            let elapsed = t0.elapsed();
            if elapsed >= Duration::from_secs(120) {
                log("done");
                app.exit(0);
                return;
            }
            if freezes.get(next_warning).is_some_and(|&at| elapsed + Duration::from_secs(3) >= at) {
                next_warning += 1;
                let heads_up = || {
                    vec![OverlayAlert {
                        sender: "Freeze test".into(),
                        text: "EVE Chatterer freezes in 3 seconds, for 5. Move the camera and type in chat from the first chime until the two chimes.".into(),
                        reason: format!("Freeze {next_warning} of 3"),
                        ..alert(&app, &pilot, OverlayStyle::Beacon, 0)
                    }]
                };
                let n = over_every_client(&app, heads_up);
                log(&format!("heads-up for freeze {next_warning} shown over {n} client(s)"));
            }
            if freezes.get(next_freeze).is_some_and(|&at| elapsed >= at) {
                next_freeze += 1;
                log(&format!("freeze {next_freeze} of 3 starts: UI thread blocked for 5 s"));
                chime();
                let (tx, rx) = std::sync::mpsc::channel();
                let blocked = app.run_on_main_thread(move || {
                    std::thread::sleep(Duration::from_secs(5));
                    let _ = tx.send(());
                });
                if blocked.is_ok() {
                    let _ = rx.recv();
                }
                log(&format!("freeze {next_freeze} of 3 ends: UI thread free again"));
                chime();
                std::thread::sleep(Duration::from_millis(800));
                chime();
            }
            if elapsed >= next_alerts {
                next_alerts += Duration::from_secs(8);
                let styles = [(OverlayStyle::Strip, 4), (OverlayStyle::Panel, 1), (OverlayStyle::Beacon, 0)];
                let n = over_every_client(&app, || styles.iter().map(|&(style, i)| alert(&app, &pilot, style, i + round)).collect());
                log(&format!("round {round}: alerts over {n} on-screen client(s)"));
                round += 1;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    });
}

/// Shows `alerts()` over every EVE client on the current desktop, owned by
/// that client like a real alert. Returns how many clients got them.
fn over_every_client(app: &AppHandle, alerts: impl Fn() -> Vec<OverlayAlert>) -> usize {
    let clients: Vec<_> = {
        let state = app.state::<AppState>();
        let snap = state.last_snapshot.lock().unwrap();
        snap.as_ref().map(|s| s.clients.iter().filter(|c| c.on_screen()).cloned().collect()).unwrap_or_default()
    };
    for c in &clients {
        let Some(monitor) = c.monitor else { continue };
        let key = format!("test-client-{}", c.hwnd);
        let p = crate::overlay::Placement { monitor, region: c.rect.unwrap_or(monitor), width: crate::overlay::DEFAULT_OVERLAY_WIDTH, custom_pos: None, owner: Some(c.hwnd) };
        for a in alerts() {
            app.state::<AppState>().overlays.show(app, &key, p, a);
        }
    }
    clients.len()
}

#[cfg(windows)]
fn wall_clock() -> String {
    eve_chatterer_core::winapi::ts()
}

#[cfg(not(windows))]
fn wall_clock() -> String {
    String::new()
}

/// `--toasttest`: sends Windows notifications through the real path, for
/// checking their look and behavior without any chat or a second client.
/// One of each length (mention: sticky, keyword: long, always: short), then
/// ordinary chatter in the mention's channel (must not replace the sticky
/// mention), then a second mention (replaces the first, stays sticky, "2
/// lines"), then two rate-capped lines: one folds into the Local "@all"
/// notification ("2 lines", updated in place), one in Alliance has nothing to
/// fold into and shows as a new, short notification. The app keeps running
/// afterwards so "Switch to" can be clicked.
pub fn toasttest(app: &AppHandle) {
    use crate::toast::{self, ChatToast};
    use eve_chatterer_core::rules::Reason;
    let app = app.clone();
    std::thread::spawn(move || {
        let pilot = pilot_name(&app);
        let tag = tag_for(&app, pilot_id(&app, &pilot).as_deref(), &pilot);
        let make = |channel: &str, sender: &str, text: &str, reason: &Reason, style: OverlayStyle| {
            let (why, tone) = crate::runner::reason_text(reason);
            let mention = matches!(reason, Reason::OwnName);
            println!("[toasttest] {channel}: {sender}: {text}");
            ChatToast {
                key: toast::key_for(&pilot, channel, mention),
                pilot: pilot.clone(),
                tag: tag.clone(),
                accent: accent_for(&pilot),
                tone,
                sender: sender.into(),
                channel: channel.into(),
                text: text.into(),
                reason: why,
                style: style_name(style),
                mention,
            }
        };
        let send = |channel: &str, sender: &str, text: &str, reason: &Reason, style: OverlayStyle| toast::chat(make(channel, sender, text, reason, style));
        let first = pilot.split_whitespace().next().unwrap_or(&pilot).to_string();
        let pause = || std::thread::sleep(Duration::from_secs(3));
        std::thread::sleep(Duration::from_secs(4));
        send("Local", "Amos Burton", &format!("{first}, are you on for the fleet tonight?"), &Reason::OwnName, OverlayStyle::Beacon);
        pause();
        send("Fleet", "FC Voss", "Align to the gate. Primary is the Loki.", &Reason::Keyword("primary".into()), OverlayStyle::Panel);
        pause();
        send("Corp", "Jack Browning", "Some of these BPs have a much higher research cost than I thought.", &Reason::AlwaysChannel("Corp".into()), OverlayStyle::Strip);
        pause();
        send("Local", "Bob", "selling in Jita 4-4, @all", &Reason::Keyword("@all".into()), OverlayStyle::Panel);
        pause();
        send("Local", "Amos Burton", &format!("{first}? x up in fleet chat"), &Reason::OwnName, OverlayStyle::Beacon);
        println!("[toasttest] the Local mention should still be on screen, sticky, showing 2 lines");
        pause();
        println!("[toasttest] capped: folds into the Local \"@all\" notification (2 lines)");
        toast::fold(make("Local", "Bob", "still selling in Jita 4-4, @all", &Reason::Keyword("@all".into()), OverlayStyle::Strip));
        pause();
        println!("[toasttest] capped with nothing to fold into: a new short notification");
        toast::fold(make("Alliance", "Clarissa Mao", "Staging moves to Tycho tomorrow, @all", &Reason::Keyword("@all".into()), OverlayStyle::Strip));
        println!("[toasttest] done");
    });
}

/// `kind` is "panel", "strip", "beacon", "burst" or "all".
pub fn send(app: &AppHandle, kind: &str) {
    match kind {
        "panel" => on_every_monitor(app, OverlayStyle::Panel, 1),
        "strip" => on_every_monitor(app, OverlayStyle::Strip, 4),
        "beacon" => on_every_monitor(app, OverlayStyle::Beacon, 0),
        "all" => {
            on_every_monitor(app, OverlayStyle::Strip, 4);
            on_every_monitor(app, OverlayStyle::Panel, 1);
            on_every_monitor(app, OverlayStyle::Beacon, 0);
        }
        "burst" => {
            // A busy channel: twelve alerts over six seconds, mostly Strips.
            let app = app.clone();
            std::thread::spawn(move || {
                for i in 0..12 {
                    let style = match i % 6 {
                        0 => OverlayStyle::Beacon,
                        2 | 4 => OverlayStyle::Panel,
                        _ => OverlayStyle::Strip,
                    };
                    on_every_monitor(&app, style, i);
                    std::thread::sleep(Duration::from_millis(500));
                }
            });
        }
        other => eprintln!("unknown test alert kind {other:?}"),
    }
}

/// Walks the sound rules with the built-in alert tone and a 10 s cooldown,
/// printing what should be heard at each step: a sound, silence for the
/// cooldown, a mention cutting through it, and one cutting off a sound.
pub fn soundtest() {
    use eve_chatterer_core::audio::Source;
    std::thread::spawn(|| {
        let cooldown = Duration::from_secs(10);
        let steps: [(u64, bool, &str); 6] = [
            (3, false, "chat: plays"),
            (5, false, "chat 2 s later: silent (cooldown)"),
            (7, true, "mention: plays through the cooldown"),
            (9, false, "chat: silent (cooldown restarted by the mention)"),
            (17_500, false, "chat 10.5 s after the mention: plays"),
            (17_700, true, "mention 0.2 s into it: cuts it off and plays"),
        ];
        let start = std::time::Instant::now();
        for (at, mention, what) in steps {
            // Whole numbers are seconds; the last two are milliseconds.
            let at = if at > 1000 { Duration::from_millis(at) } else { Duration::from_secs(at) };
            std::thread::sleep(at.saturating_sub(start.elapsed()));
            println!("[soundtest] {:>5.1}s  {what}", start.elapsed().as_secs_f32());
            crate::audio::alert(Source::BuiltIn, 0.8, cooldown, mention);
        }
        println!("[soundtest] done");
    });
}
