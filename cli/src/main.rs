//! Headless runner: follows the EVE chat logs and prints what would be shown.
//!
//!   chatter [--dir <Chatlogs>] [--keyword <text>]... [--regex <pattern>]...
//!           [--suppress focused|visible|none] [--delivery auto|overlay|toast|both|sound]
//!           [--settings <file.json>] [--pilots <file.json>] [--verbose] [--seconds <n>]
//!
//! Without --dir it finds Documents\EVE\logs\Chatlogs through the Windows
//! known-folder API (this follows OneDrive). Settings are layered (global,
//! channel kind, channel, pilot...) and start from the built-in defaults by
//! kind, or from --settings; --keyword, --regex, --suppress and --delivery
//! adjust the global layer. Each alert is routed through presence (which EVE
//! client has focus, which are on screen) and the rate caps, and printed with
//! the decision: suppressed, overlay (and where), toast, sound, or held back.
//! Nothing is ever written to the log folder. --pilots keeps the registry of
//! characters between runs (otherwise it lives in memory only).

use eve_chatterer_core::engine::{Alert, Engine, EngineConfig, Event};
use eve_chatterer_core::governor::Governor;
use eve_chatterer_core::liveset::Discovery;
use eve_chatterer_core::paths;
use eve_chatterer_core::pilots::PilotRegistry;
use eve_chatterer_core::prefs::{DeliveryMode, OverCap, Suppression};
use eve_chatterer_core::presence::Snapshot;
use eve_chatterer_core::router::{self, Anchor, Decision, Delivery, Outcome, RouterConfig, SuppressedBy};
use eve_chatterer_core::rules::Reason;
use eve_chatterer_core::settings::{Settings, SettingsBook};
use std::path::PathBuf;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const SAMPLE_EVERY: Duration = Duration::from_millis(250);
const TICK_EVERY: Duration = Duration::from_millis(500);

struct Args {
    dir: Option<PathBuf>,
    keywords: Vec<String>,
    regexes: Vec<String>,
    pilots: Option<PathBuf>,
    settings: Option<PathBuf>,
    suppress: Option<Suppression>,
    delivery: Option<DeliveryMode>,
    verbose: bool,
    seconds: Option<u64>,
}

fn parse_args() -> Result<Args, String> {
    let mut a = Args {
        dir: None,
        keywords: vec![],
        regexes: vec![],
        pilots: None,
        settings: None,
        suppress: None,
        delivery: None,
        verbose: false,
        seconds: None,
    };
    let mut it = std::env::args().skip(1);
    while let Some(flag) = it.next() {
        let mut value = |what: &str| it.next().ok_or_else(|| format!("{flag} needs {what}"));
        match flag.as_str() {
            "--dir" => a.dir = Some(PathBuf::from(value("a folder")?)),
            "--keyword" => a.keywords.push(value("a word")?),
            "--regex" => a.regexes.push(value("a pattern")?),
            "--pilots" => a.pilots = Some(PathBuf::from(value("a file")?)),
            "--settings" => a.settings = Some(PathBuf::from(value("a file")?)),
            "--seconds" => a.seconds = Some(value("a number")?.parse().map_err(|_| "--seconds needs a number".to_string())?),
            "--suppress" => {
                a.suppress = Some(match value("focused, visible or none")?.as_str() {
                    "focused" => Suppression::FocusedOnly,
                    "visible" => Suppression::VisibleOnScreen,
                    "none" => Suppression::AllowAll,
                    other => return Err(format!("unknown --suppress {other}")),
                })
            }
            "--delivery" => {
                a.delivery = Some(match value("auto, overlay, toast, both or sound")?.as_str() {
                    "auto" => DeliveryMode::Auto,
                    "overlay" => DeliveryMode::Overlay,
                    "toast" => DeliveryMode::Toast,
                    "both" => DeliveryMode::Both,
                    "sound" => DeliveryMode::SoundOnly,
                    other => return Err(format!("unknown --delivery {other}")),
                })
            }
            "--verbose" => a.verbose = true,
            "-h" | "--help" => return Err(String::new()),
            other => return Err(format!("unknown option {other}")),
        }
    }
    Ok(a)
}

fn clock() -> String {
    let s = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs() % 86_400).unwrap_or(0);
    format!("{:02}:{:02}:{:02}Z", s / 3600, s % 3600 / 60, s % 60)
}

fn reason(r: &Reason) -> String {
    match r {
        Reason::OwnName => "own name".into(),
        Reason::Keyword(k) => format!("keyword {k:?}"),
        Reason::Regex(p) => format!("regex {p:?}"),
        Reason::AlwaysChannel(c) => format!("always: channel {c}"),
        Reason::AlwaysSender(s) => format!("always: sender {s}"),
    }
}

fn delivery(d: &Delivery) -> String {
    match d {
        Delivery::Overlay { anchor, style } => {
            let style = format!("{style:?}").to_lowercase();
            match anchor {
                Anchor::Monitor(m) => format!("{style} overlay on monitor ({},{})-({},{})", m.left, m.top, m.right, m.bottom),
                Anchor::FollowWindow { hwnd } => format!("{style} overlay following window {hwnd:#x}"),
                Anchor::Unknown => format!("{style} overlay (no known monitor)"),
            }
        }
        Delivery::Toast { switch_to } => format!("toast, action: switch to {switch_to}"),
        Delivery::Sound => "sound".into(),
    }
}

fn print_alert(a: &Alert, decisions: &[Decision]) {
    let viewers: Vec<&str> = a.seen_by.iter().map(|l| l.name.as_str()).collect();
    println!(
        "[{}] ALERT  {} ({:?})  {}: {}   (seen by: {})",
        clock(),
        a.channel_name,
        a.kind,
        a.line.sender,
        a.line.text,
        viewers.join(", ")
    );
    for d in decisions {
        let what = match &d.outcome {
            Outcome::Suppressed(SuppressedBy::FocusedPilot) => "suppressed (you are on this client)".to_string(),
            Outcome::Suppressed(SuppressedBy::VisibleOnScreen) => "suppressed (this client is on screen)".to_string(),
            Outcome::Limited(OverCap::Drop) => "over its rate cap: dropped".to_string(),
            Outcome::Limited(OverCap::Fold) => "over its rate cap: folded into the count badge".to_string(),
            Outcome::Deliver(v) if v.is_empty() => "nothing to show".to_string(),
            Outcome::Deliver(v) => v.iter().map(delivery).collect::<Vec<_>>().join(" + "),
        };
        println!("             {} ({}) -> {what}", d.pilot_name, reason(&d.reason));
    }
}

/// Presence exists only on Windows; elsewhere every alert sees an empty desktop.
struct Presence {
    #[cfg(windows)]
    sampler: eve_chatterer_core::presence::Sampler,
    snap: Snapshot,
}

impl Presence {
    fn new() -> Presence {
        Presence {
            #[cfg(windows)]
            sampler: eve_chatterer_core::presence::Sampler::new(),
            snap: Snapshot { clients: vec![], focused: None, foreground: None, idle: Duration::ZERO, notifications_ok: true },
        }
    }

    fn sample(&mut self, _now: Instant) {
        #[cfg(windows)]
        {
            self.snap = self.sampler.sample(_now);
        }
    }
}

fn main() {
    let args = match parse_args() {
        Ok(a) => a,
        Err(msg) => {
            if !msg.is_empty() {
                eprintln!("{msg}\n");
            }
            eprintln!(
                "usage: chatter [--dir <Chatlogs>] [--keyword <text>]... [--regex <pattern>]...\n\
                 \x20              [--suppress focused|visible|none] [--delivery auto|overlay|toast|both|sound]\n\
                 \x20              [--settings <file.json>] [--pilots <file.json>] [--verbose] [--seconds <n>]"
            );
            std::process::exit(if msg.is_empty() { 0 } else { 2 });
        }
    };
    let Some(dir) = args.dir.clone().or_else(paths::chatlogs_dir) else {
        eprintln!("Could not find Documents\\EVE\\logs\\Chatlogs. Pass it with --dir.");
        std::process::exit(1);
    };

    // Built-in defaults by channel kind, or a settings file; the flags then
    // adjust the global layer.
    let mut settings = match &args.settings {
        Some(p) => Settings::load(p).unwrap_or_else(|e| {
            eprintln!("could not read {}: {e}", p.display());
            std::process::exit(1);
        }),
        None => Settings::with_defaults(),
    };
    if !args.keywords.is_empty() {
        settings.global.keywords = Some(args.keywords.clone());
    }
    if !args.regexes.is_empty() {
        settings.global.regexes = Some(args.regexes.clone());
    }
    settings.global.suppression = args.suppress.or(settings.global.suppression);
    settings.global.delivery = args.delivery.or(settings.global.delivery);
    let bad = settings.invalid_regexes();
    if !bad.is_empty() {
        eprintln!("bad regex: {}", bad.join(", "));
        std::process::exit(2);
    }
    let book = SettingsBook::new(settings);
    let pilots = match &args.pilots {
        Some(p) => PilotRegistry::load(p).unwrap_or_else(|e| {
            eprintln!("could not read {}: {e}", p.display());
            std::process::exit(1);
        }),
        None => PilotRegistry::default(),
    };
    let router_cfg = RouterConfig::default();
    let mut governor = Governor::new();

    println!("[{}] watching {}", clock(), dir.display());
    let mut engine = Engine::new(&dir, EngineConfig::default(), pilots, book);
    let mut presence = Presence::new();
    let start = Instant::now();
    let mut last_tick: Option<Instant> = None;
    let mut last_clients = String::new();
    loop {
        let now = Instant::now();
        presence.sample(now);

        if args.verbose {
            let state: Vec<String> = presence
                .snap
                .clients
                .iter()
                .map(|c| {
                    format!(
                        "{}[{}{}{}]",
                        c.character,
                        if presence.snap.is_focused(&c.character) { "focused " } else { "" },
                        if c.on_screen() { "on-screen" } else if c.minimized { "minimized" } else { "hidden" },
                        if c.on_screen() && !c.covers_monitor() { " windowed" } else { "" }
                    )
                })
                .collect();
            let line = state.join("  ");
            if line != last_clients {
                println!("[{}] clients: {}", clock(), if line.is_empty() { "none".to_string() } else { line.clone() });
                last_clients = line;
            }
        }

        let mut events = vec![];
        if last_tick.is_none_or(|t| now.duration_since(t) >= TICK_EVERY) {
            last_tick = Some(now);
            let names: Vec<&str> = presence.snap.clients.iter().map(|c| c.character.as_str()).collect();
            events.extend(engine.observe_clients(&names, now));
            events.extend(engine.tick(now));
        }
        for ev in events {
            match ev {
                Event::Alert(a) => {
                    let decisions = governor.apply(router::route(&a, &presence.snap, &router_cfg), now);
                    print_alert(&a, &decisions);
                }
                Event::NewPilot(p) => {
                    println!("[{}] NEW PILOT  {} (id {})", clock(), p.name, p.id);
                    save(&args, &engine);
                }
                Event::PilotInLogs(p) => {
                    if args.verbose {
                        println!("[{}] pilot in logs  {} (id {})", clock(), p.name, p.id);
                    }
                    save(&args, &engine);
                }
                Event::ChatLoggingOff { name } => {
                    println!("[{}] CHAT LOGGING LOOKS OFF for {name}: a client is open but no chat log has appeared", clock());
                }
                Event::Discovery(d) if args.verbose => match d {
                    Discovery::Adopted { name, from_start, .. } => println!(
                        "[{}] following  {} / {}  ({})",
                        clock(),
                        name.channel,
                        name.char_id.as_deref().unwrap_or("?"),
                        if from_start { "new session" } else { "joined at end" }
                    ),
                    Discovery::Superseded { new, .. } => println!("[{}] session replaced by {}", clock(), new.display()),
                    Discovery::SkippedPlaceholder(p) => println!("[{}] skipped cloud-only file {}", clock(), p.display()),
                    Discovery::Gone(p) => println!("[{}] file gone {}", clock(), p.display()),
                },
                Event::Discovery(_) => {}
            }
        }
        if args.seconds.is_some_and(|s| start.elapsed() >= Duration::from_secs(s)) {
            break;
        }
        std::thread::sleep(SAMPLE_EVERY);
    }
}

fn save(args: &Args, engine: &Engine) {
    if let Some(p) = &args.pilots {
        if let Err(e) = engine.pilots().save(p) {
            eprintln!("could not save {}: {e}", p.display());
        }
    }
}
