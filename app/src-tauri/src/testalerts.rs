//! Synthetic alerts for trying the overlays without any EVE activity. They go
//! straight to the overlay windows on every monitor.

use crate::overlay::OverlayAlert;
use crate::runner::{accent_for, lifetime_ms, monitor_rects, style_name, tag_for};
use crate::state::AppState;
use eve_chatterer_core::prefs::OverlayStyle;
use std::time::Duration;
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
        OverlayStyle::Strip => ("Always alert: Local", "always"),
    };
    // A real registry lookup where possible, so the preview matches what the
    // pilot would actually see if it has its own tag set.
    let pilot_id = app.state::<AppState>().status.lock().unwrap().pilots.iter().find(|p| p.name == pilot).map(|p| p.id.clone());
    OverlayAlert {
        id: app.state::<AppState>().overlays.next_id(),
        style: style_name(style),
        pilot: pilot.to_string(),
        tag: tag_for(app, pilot_id.as_deref(), pilot),
        accent: accent_for(pilot),
        channel: "Local".to_string(),
        sender: sender.to_string(),
        text: text.to_string(),
        reason: reason.to_string(),
        tone,
        lifetime_ms: lifetime_ms(style),
        count: 1,
        stack_up: false,
    }
}

fn pilot_name(app: &AppHandle) -> String {
    let state = app.state::<AppState>();
    let st = state.status.lock().unwrap();
    st.pilots.iter().find(|p| p.live).or_else(|| st.pilots.first()).map(|p| p.name.clone()).unwrap_or_else(|| "Holden".to_string())
}

fn on_every_monitor(app: &AppHandle, style: OverlayStyle, i: usize) {
    let pilot = pilot_name(app);
    for m in monitor_rects(app) {
        // A distinct key per monitor, not the real per-pilot key: this sends
        // the "same" synthetic alert to every monitor's own window at once,
        // which a real per-pilot key (one window total) could not do.
        let key = format!("test-{}-{}", m.left, m.top);
        let p = crate::overlay::Placement { monitor: m, region: m, width: crate::overlay::DEFAULT_OVERLAY_WIDTH, custom_pos: None, owner: None };
        app.state::<AppState>().overlays.show(app, &key, p, alert(app, &pilot, style, i));
    }
}

/// `--selftest`: exercises the overlays and then exits, so a run can be
/// watched (or measured from outside) without touching the tray. Timeline:
/// alerts on every monitor at 3 s, a burst at 8 s, exit at 90 s (the overlay
/// windows are reaped 45 s after their last alert, so their teardown is visible).
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

/// `--toasttest`: sends Windows notifications through the real path, for
/// checking their look and behavior without any chat or a second client.
/// One of each length (mention: sticky, keyword: long, always: short), then
/// ordinary chatter in the mention's channel (must not replace the sticky
/// mention), then a second mention (replaces the first, stays sticky, "2
/// lines"). The app keeps running afterwards so "Switch to" can be clicked.
pub fn toasttest(app: &AppHandle) {
    use crate::toast::{self, ChatToast};
    use eve_chatterer_core::rules::Reason;
    let app = app.clone();
    std::thread::spawn(move || {
        let pilot = pilot_name(&app);
        let pilot_id = app.state::<AppState>().status.lock().unwrap().pilots.iter().find(|p| p.name == pilot).map(|p| p.id.clone());
        let tag = tag_for(&app, pilot_id.as_deref(), &pilot);
        let send = |channel: &str, sender: &str, text: &str, reason: &Reason, style: OverlayStyle| {
            let (why, tone) = crate::runner::reason_text(reason);
            let mention = matches!(reason, Reason::OwnName);
            println!("[toasttest] {channel}: {sender}: {text}");
            toast::chat(ChatToast {
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
            });
        };
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
        println!("[toasttest] done: the Local mention should still be on screen, sticky, showing 2 lines");
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
