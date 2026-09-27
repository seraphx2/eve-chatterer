//! Headless runner: follows the EVE chat logs and prints alerts.
//!
//!   chatter [--dir <Chatlogs>] [--keyword <text>]... [--regex <pattern>]...
//!           [--pilots <file.json>] [--verbose] [--seconds <n>]
//!
//! Without --dir it finds Documents\EVE\logs\Chatlogs through the Windows
//! known-folder API (this follows OneDrive). Own-name mentions always alert;
//! --keyword and --regex add rules. --pilots keeps the registry of characters
//! between runs (otherwise it lives in memory only). Nothing is ever written
//! to the log folder.

use eve_chatterer_core::engine::{Alert, Engine, EngineConfig, Event};
use eve_chatterer_core::liveset::Discovery;
use eve_chatterer_core::paths;
use eve_chatterer_core::pilots::PilotRegistry;
use eve_chatterer_core::rules::{Reason, RuleBook, RuleSet};
use std::path::PathBuf;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

struct Args {
    dir: Option<PathBuf>,
    keywords: Vec<String>,
    regexes: Vec<String>,
    pilots: Option<PathBuf>,
    verbose: bool,
    seconds: Option<u64>,
}

fn parse_args() -> Result<Args, String> {
    let mut a = Args { dir: None, keywords: vec![], regexes: vec![], pilots: None, verbose: false, seconds: None };
    let mut it = std::env::args().skip(1);
    while let Some(flag) = it.next() {
        let mut value = |what: &str| it.next().ok_or_else(|| format!("{flag} needs {what}"));
        match flag.as_str() {
            "--dir" => a.dir = Some(PathBuf::from(value("a folder")?)),
            "--keyword" => a.keywords.push(value("a word")?),
            "--regex" => a.regexes.push(value("a pattern")?),
            "--pilots" => a.pilots = Some(PathBuf::from(value("a file")?)),
            "--seconds" => a.seconds = Some(value("a number")?.parse().map_err(|_| "--seconds needs a number".to_string())?),
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

fn print_alert(a: &Alert) {
    let targets: Vec<String> = a.targets.iter().map(|t| format!("{} ({})", t.pilot_name, reason(&t.reason))).collect();
    let viewers: Vec<&str> = a.seen_by.iter().map(|l| l.name.as_str()).collect();
    println!("[{}] ALERT  {}  {}: {}", clock(), a.channel_name, a.line.sender, a.line.text);
    println!("             for: {}   seen by: {}", targets.join(", "), viewers.join(", "));
}

fn main() {
    let args = match parse_args() {
        Ok(a) => a,
        Err(msg) => {
            if !msg.is_empty() {
                eprintln!("{msg}\n");
            }
            eprintln!("usage: chatter [--dir <Chatlogs>] [--keyword <text>]... [--regex <pattern>]... [--pilots <file.json>] [--verbose] [--seconds <n>]");
            std::process::exit(if msg.is_empty() { 0 } else { 2 });
        }
    };
    let Some(dir) = args.dir.clone().or_else(paths::chatlogs_dir) else {
        eprintln!("Could not find Documents\\EVE\\logs\\Chatlogs. Pass it with --dir.");
        std::process::exit(1);
    };

    let rules = RuleSet { keywords: args.keywords.clone(), regexes: args.regexes.clone(), ..RuleSet::default() };
    let book = match RuleBook::new(&rules) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("bad --regex: {e}");
            std::process::exit(2);
        }
    };
    let pilots = match &args.pilots {
        Some(p) => PilotRegistry::load(p).unwrap_or_else(|e| {
            eprintln!("could not read {}: {e}", p.display());
            std::process::exit(1);
        }),
        None => PilotRegistry::default(),
    };

    println!("[{}] watching {}", clock(), dir.display());
    let mut engine = Engine::new(&dir, EngineConfig::default(), pilots, book);
    let start = Instant::now();
    loop {
        for ev in engine.tick(Instant::now()) {
            match ev {
                Event::Alert(a) => print_alert(&a),
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
        std::thread::sleep(Duration::from_millis(500));
    }
}

fn save(args: &Args, engine: &Engine) {
    if let Some(p) = &args.pilots {
        if let Err(e) = engine.pilots().save(p) {
            eprintln!("could not save {}: {e}", p.display());
        }
    }
}
