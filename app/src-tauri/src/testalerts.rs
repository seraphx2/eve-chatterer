//! Synthetic alerts for trying the overlays without any EVE activity. They go
//! straight to the overlay windows on every monitor.

use crate::overlay::OverlayAlert;
use crate::runner::{accent_for, lifetime_ms, monitor_rects, style_name};
use crate::state::AppState;
use eve_chatterer_core::prefs::OverlayStyle;
use std::time::Duration;
use tauri::{AppHandle, Manager};

const SAMPLES: [(&str, &str); 6] = [
    ("Rilakss", "Jarna, are you on for the fleet tonight? We are forming up in Jita at 20:00."),
    ("Mr Nodes", "HyperNet offer: Zirnitra 4/8 cheap nodes"),
    ("Ceryph", "Anyone up for a mining op?"),
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
    OverlayAlert {
        id: app.state::<AppState>().overlays.next_id(),
        style: style_name(style),
        pilot: pilot.to_string(),
        accent: accent_for(pilot),
        channel: "Local".to_string(),
        sender: sender.to_string(),
        text: text.to_string(),
        reason: reason.to_string(),
        tone,
        lifetime_ms: lifetime_ms(style),
        count: 1,
    }
}

fn pilot_name(app: &AppHandle) -> String {
    let state = app.state::<AppState>();
    let st = state.status.lock().unwrap();
    st.pilots.iter().find(|p| p.live).or_else(|| st.pilots.first()).map(|p| p.name.clone()).unwrap_or_else(|| "Jarna".to_string())
}

fn on_every_monitor(app: &AppHandle, style: OverlayStyle, i: usize) {
    let pilot = pilot_name(app);
    for m in monitor_rects(app) {
        app.state::<AppState>().overlays.show(app, m, m, alert(app, &pilot, style, i));
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
        let log = |what: &str| println!("[selftest +{:>3}s] {what}", t0.elapsed().as_secs());
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
