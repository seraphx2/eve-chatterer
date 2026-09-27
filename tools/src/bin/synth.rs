//! Isolates OneDrive from plain-NTFS behavior with a synthetic EVE-like writer.
//!
//!   synth write <dir> <plain|flush> <secs> <stampfile>
//!       Creates an EVE-style UTF-16LE log, holds it open, appends one line
//!       per second (BOM before every line, as EVE does). `flush` also calls
//!       FlushFileBuffers after every write. Each write's wall-clock ms goes
//!       to <stampfile>.
//!   synth watch <dir> <secs> <eventsfile> <poll:0|1>
//!       Watches <dir> with directory events only and logs every event with a
//!       wall-clock ms. With poll=1 it also opens every .txt every 500 ms
//!       (the same nudge the probe's poller gave).

use notify::{Config, RecommendedWatcher, RecursiveMode, Watcher};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

fn now_ms() -> u128 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis()
}

/// Unix seconds -> (year, month, day, hour, minute, second), UTC.
fn civil(secs: u64) -> (i64, u32, u32, u32, u32, u32) {
    let days = (secs / 86400) as i64;
    let rem = secs % 86400;
    let z = days + 719468;
    let era = z.div_euclid(146097);
    let doe = z.rem_euclid(146097);
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = (if mp < 10 { mp + 3 } else { mp - 9 }) as u32;
    let y = yoe + era * 400 + if m <= 2 { 1 } else { 0 };
    (y, m, d, (rem / 3600) as u32, (rem % 3600 / 60) as u32, (rem % 60) as u32)
}

fn utf16(s: &str) -> Vec<u8> {
    s.encode_utf16().flat_map(|u| u.to_le_bytes()).collect()
}

fn is_txt(p: &std::path::Path) -> bool {
    p.extension().is_some_and(|e| e.eq_ignore_ascii_case("txt"))
}

fn write_mode(a: &[String]) {
    let dir = PathBuf::from(&a[0]);
    let flush = a[1] == "flush";
    let secs: u64 = a[2].parse().unwrap();
    let mut stamps = OpenOptions::new().create(true).append(true).open(&a[3]).unwrap();
    fs::create_dir_all(&dir).unwrap();
    let (y, mo, d, h, mi, s) = civil(now_ms() as u64 / 1000);
    let fname = format!("Local_{y:04}{mo:02}{d:02}_{h:02}{mi:02}{s:02}_999999999.txt");
    let mut f = OpenOptions::new().create_new(true).append(true).open(dir.join(&fname)).unwrap();
    let mut head = vec![0xFF, 0xFE];
    head.extend(utf16(&format!(
        "\r\n---------------------------------------------------------------\r\n  \
         Channel ID:      local\r\n  Channel Name:    Local\r\n  Listener:        Synth\r\n  \
         Session started: {y:04}.{mo:02}.{d:02} {h:02}:{mi:02}:{s:02}\r\n\
         ---------------------------------------------------------------\r\n"
    )));
    f.write_all(&head).unwrap();
    if flush {
        f.sync_data().unwrap();
    }
    for n in 0..secs {
        let (y, mo, d, h, mi, s) = civil(now_ms() as u64 / 1000);
        let line = format!("\u{feff}[ {y:04}.{mo:02}.{d:02} {h:02}:{mi:02}:{s:02} ] Synth > test {n}\r\n");
        f.write_all(&utf16(&line)).unwrap();
        if flush {
            f.sync_data().unwrap();
        }
        writeln!(stamps, "{n} {}", now_ms()).unwrap();
        thread::sleep(Duration::from_secs(1));
    }
    println!("{fname}");
}

fn watch_mode(a: &[String]) {
    let dir = PathBuf::from(&a[0]);
    let secs: u64 = a[1].parse().unwrap();
    let mut log = OpenOptions::new().create(true).append(true).open(&a[2]).unwrap();
    let poll = a[3] == "1";

    let (tx, rx) = mpsc::channel();
    let mut w = RecommendedWatcher::new(tx, Config::default()).unwrap();
    w.watch(&dir, RecursiveMode::NonRecursive).unwrap();

    let stop = Arc::new(AtomicBool::new(false));
    if poll {
        let (stop, dir) = (stop.clone(), dir.clone());
        thread::spawn(move || {
            while !stop.load(Ordering::Relaxed) {
                if let Ok(rd) = fs::read_dir(&dir) {
                    for e in rd.flatten().filter(|e| is_txt(&e.path())) {
                        let _ = fs::metadata(e.path()); // opens a handle, like the probe's poller
                    }
                }
                thread::sleep(Duration::from_millis(500));
            }
        });
    }

    let deadline = Instant::now() + Duration::from_secs(secs);
    while Instant::now() < deadline {
        if let Ok(Ok(ev)) = rx.recv_timeout(Duration::from_millis(100)) {
            for p in ev.paths.iter().filter(|p| is_txt(p)) {
                writeln!(log, "{} {:?}", now_ms(), ev.kind).unwrap();
                let _ = p;
            }
        }
    }
    stop.store(true, Ordering::Relaxed);
}

fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    match a.first().map(String::as_str) {
        Some("write") => write_mode(&a[1..]),
        Some("watch") => watch_mode(&a[1..]),
        _ => eprintln!("usage: synth write|watch ..."),
    }
}
