//! Measurement probe for the EVE chat log folder.
//!
//! Questions it answers before the real design is built on them:
//!   1. Are directory events prompt, or do they depend on something else
//!      touching the file (OneDrive / NTFS lazy size notifications)?
//!   2. Can the live file per (character, channel) be found from filenames
//!      alone, without trusting directory metadata?
//!   3. What does a real burst of chat look like, across two characters?
//!
//! Usage: probe [--no-poll] [chatlogs-dir]
//!   --no-poll  Do not poll tracked files. Only directory events wake us up.
//!              Compare `age` on LINES entries against a normal run.
//! Without a dir it tries %USERPROFILE%\OneDrive\Documents first, then
//! %USERPROFILE%\Documents. Everything is echoed to probe-log.txt.

use notify::{Config, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use std::collections::HashMap;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// Sessions created longer ago than this are not considered live.
const LIVE_WINDOW: Duration = Duration::from_secs(14 * 24 * 3600);
const TICK: Duration = Duration::from_millis(500);
const STALE_AFTER: Duration = Duration::from_secs(3);
const LAG_REPORT_MS: u128 = 50;
const STATUS_EVERY: Duration = Duration::from_secs(30);

static LOG: OnceLock<Mutex<File>> = OnceLock::new();
static START: OnceLock<Instant> = OnceLock::new();

fn utc_secs_of_day() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs() % 86400).unwrap_or(0)
}

fn out(msg: impl AsRef<str>) {
    let t = START.get().map(|s| s.elapsed().as_secs_f64()).unwrap_or(0.0);
    let s = utc_secs_of_day();
    let line = format!(
        "[{:02}:{:02}:{:02}Z {t:>8.3}] {}",
        s / 3600,
        s % 3600 / 60,
        s % 60,
        msg.as_ref()
    );
    println!("{line}");
    if let Some(f) = LOG.get() {
        if let Ok(mut f) = f.lock() {
            let _ = writeln!(f, "{line}");
        }
    }
}

#[derive(Default, Clone)]
struct Header {
    channel_id: String,
    channel_name: String,
    listener: String,
}

struct Tracked {
    header: Option<Header>,
    /// (channel, character id) from the filename, plus the session-start stamp.
    key: Option<(String, String)>,
    stamp: String,
    /// Byte offset of the first byte not yet consumed (always at a line start).
    offset: u64,
    /// File length observed the last time we read it.
    seen_len: u64,
    /// Set when the poller sees growth that no event has explained yet.
    pending_growth_since: Option<Instant>,
    stale_reported: bool,
    events: u64,
    lines: u64,
}

struct State {
    tracked: HashMap<PathBuf, Tracked>,
    polling: bool,
    events: u64,
    poll_first: u64,
}

fn default_dir() -> Option<PathBuf> {
    let home = PathBuf::from(std::env::var_os("USERPROFILE")?);
    [home.join("OneDrive").join("Documents"), home.join("Documents")]
        .into_iter()
        .map(|d| d.join("EVE").join("logs").join("Chatlogs"))
        .find(|p| p.is_dir())
}

fn all_digits(s: &str, n: Option<usize>) -> bool {
    !s.is_empty() && n.is_none_or(|n| s.len() == n) && s.bytes().all(|b| b.is_ascii_digit())
}

/// `<Channel>_<YYYYMMDD>_<HHMMSS>[_<characterId>].txt` -> (channel, id, stamp).
fn parse_name(path: &Path) -> Option<(String, String, String)> {
    let stem = path.file_stem()?.to_string_lossy().into_owned();
    let p: Vec<&str> = stem.split('_').collect();
    let n = p.len();
    if n >= 4 && all_digits(p[n - 3], Some(8)) && all_digits(p[n - 2], Some(6)) && all_digits(p[n - 1], None) {
        Some((p[..n - 3].join("_"), p[n - 1].to_string(), format!("{}_{}", p[n - 3], p[n - 2])))
    } else if n >= 3 && all_digits(p[n - 2], Some(8)) && all_digits(p[n - 1], Some(6)) {
        Some((p[..n - 2].join("_"), String::new(), format!("{}_{}", p[n - 2], p[n - 1])))
    } else {
        None
    }
}

fn decode_utf16le(bytes: &[u8]) -> String {
    let units: Vec<u16> = bytes
        .chunks_exact(2)
        .map(|c| u16::from_le_bytes([c[0], c[1]]))
        .collect();
    String::from_utf16_lossy(&units)
}

fn read_header(path: &Path) -> Option<Header> {
    let mut f = File::open(path).ok()?;
    let mut buf = vec![0u8; 4096];
    let n = f.read(&mut buf).ok()?;
    let text = decode_utf16le(&buf[..n]);
    let mut h = Header::default();
    let mut found = 0;
    for line in text.lines() {
        let line = line.trim_start_matches('\u{feff}').trim();
        if let Some(v) = line.strip_prefix("Channel ID:") {
            h.channel_id = v.trim().to_string();
            found += 1;
        } else if let Some(v) = line.strip_prefix("Channel Name:") {
            h.channel_name = v.trim().to_string();
            found += 1;
        } else if let Some(v) = line.strip_prefix("Listener:") {
            h.listener = v.trim().to_string();
            found += 1;
        }
    }
    (found >= 3).then_some(h)
}

/// Index just past the last complete UTF-16LE line in `buf`, 0 if none.
fn last_line_end(buf: &[u8]) -> usize {
    let mut end = 0;
    let mut i = 0;
    while i + 1 < buf.len() {
        if buf[i] == 0x0A && buf[i + 1] == 0 {
            end = i + 2;
        }
        i += 2;
    }
    end
}

/// Offset just past the last complete line, so adopting an existing file
/// never replays history or starts mid-line.
fn last_newline_boundary(path: &Path) -> io::Result<u64> {
    let mut f = File::open(path)?;
    let len = f.metadata()?.len();
    let start = len.saturating_sub(8192) & !1;
    f.seek(SeekFrom::Start(start))?;
    let mut buf = vec![0u8; (len - start) as usize];
    f.read_exact(&mut buf)?;
    Ok(start + last_line_end(&buf) as u64)
}

/// Reads complete lines appended since `offset`. std opens files with
/// FILE_SHARE_READ | WRITE | DELETE on Windows, so we never block EVE.
fn read_new(path: &Path, offset: &mut u64) -> io::Result<(Vec<String>, u64)> {
    let mut f = File::open(path)?;
    let len = f.metadata()?.len();
    if len < *offset {
        out(format!("TRUNC   {} shrank ({} -> {len}), resetting", name(path), *offset));
        *offset = 0;
    }
    if len == *offset {
        return Ok((vec![], len));
    }
    f.seek(SeekFrom::Start(*offset))?;
    let mut buf = vec![0u8; (len - *offset) as usize];
    f.read_exact(&mut buf)?;
    let end = last_line_end(&buf);
    if end == 0 {
        return Ok((vec![], len));
    }
    *offset += end as u64;
    let text = decode_utf16le(&buf[..end]);
    Ok((text.lines().map(str::to_string).collect(), len))
}

fn name(path: &Path) -> String {
    path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
}

fn is_log(path: &Path) -> bool {
    path.extension().is_some_and(|e| e.eq_ignore_ascii_case("txt"))
}

fn describe(h: &Option<Header>) -> String {
    match h {
        Some(h) => format!("listener={:?} channel={:?} id={}", h.listener, h.channel_name, h.channel_id),
        None => "header=<not yet readable>".to_string(),
    }
}

fn truncate(s: &str, n: usize) -> String {
    let t: String = s.chars().take(n).collect();
    if s.chars().count() > n { format!("{t}…") } else { t }
}

/// How old a `[ YYYY.MM.DD HH:MM:SS ] ...` line is right now, in seconds
/// (both sides UTC). This is the true delivery lag, independent of the poller.
fn line_age_secs(line: &str) -> Option<i64> {
    let t = line.get(13..21)?;
    let h: i64 = t.get(0..2)?.parse().ok()?;
    let m: i64 = t.get(3..5)?.parse().ok()?;
    let s: i64 = t.get(6..8)?.parse().ok()?;
    let mut d = utc_secs_of_day() as i64 - (h * 3600 + m * 60 + s);
    if d < -43200 {
        d += 86400;
    } else if d > 43200 {
        d -= 86400;
    }
    Some(d)
}

/// `adopt_only` registers the file without treating the call as an event
/// (used for the startup scan, so event counts stay honest).
fn handle_change(path: &Path, created: bool, adopt_only: bool, st: &mut State) {
    if !path.exists() {
        return;
    }
    if !st.tracked.contains_key(path) {
        let header = read_header(path);
        let offset = if created { 0 } else { last_newline_boundary(path).unwrap_or(0) };
        let parsed = parse_name(path);
        let (key, stamp) = match parsed {
            Some((ch, id, stamp)) => (Some((ch, id)), stamp),
            None => (None, String::new()),
        };
        out(format!(
            "TRACK   {}  {}  ({}, offset {offset})",
            name(path),
            describe(&header),
            if created { "new file" } else { "existing file" },
        ));
        // A newer session for the same (channel, character) supersedes the old file.
        if let Some(k) = &key {
            let old: Vec<PathBuf> = st
                .tracked
                .iter()
                .filter(|(_, t)| t.key.as_ref() == Some(k) && t.stamp < stamp)
                .map(|(p, _)| p.clone())
                .collect();
            for p in old {
                st.tracked.remove(&p);
                out(format!("SUPERSEDED {}  by {}", name(&p), name(path)));
            }
        }
        st.tracked.insert(
            path.to_path_buf(),
            Tracked {
                header,
                key,
                stamp,
                offset,
                seen_len: offset,
                pending_growth_since: None,
                stale_reported: false,
                events: 0,
                lines: 0,
            },
        );
    }
    if adopt_only {
        return;
    }
    let t = st.tracked.get_mut(path).unwrap();
    t.events += 1;
    st.events += 1;
    t.stale_reported = false;
    if let Some(since) = t.pending_growth_since.take() {
        st.poll_first += 1;
        let ms = since.elapsed().as_millis();
        if ms >= LAG_REPORT_MS {
            out(format!(
                "LAG     {}  event arrived {ms} ms AFTER the poller saw the file grow",
                name(path)
            ));
        }
    }
    if t.header.is_none() {
        t.header = read_header(path);
        if t.header.is_some() {
            out(format!("HEADER  {}  {}", name(path), describe(&t.header)));
        }
    }
    match read_new(path, &mut t.offset) {
        Ok((lines, len)) => {
            t.seen_len = len;
            // EVE prefixes every line (not just the file) with a BOM.
            let body: Vec<&str> = lines
                .iter()
                .map(|l| l.trim_matches(|c: char| c == '\u{feff}' || c.is_whitespace()))
                .filter(|l| l.starts_with('['))
                .collect();
            t.lines += body.len() as u64;
            let who = t.header.as_ref().map(|h| h.listener.as_str()).unwrap_or("?");
            match body.last() {
                Some(last) => out(format!(
                    "LINES   {who}/{}  +{}  age {}s  last: {}",
                    t.key.as_ref().map(|k| k.0.as_str()).unwrap_or("?"),
                    body.len(),
                    line_age_secs(last).map(|a| a.to_string()).unwrap_or_else(|| "?".into()),
                    truncate(last, 90)
                )),
                None => out(format!("EVENT   {}  size={len}, no new complete chat lines", name(path))),
            }
        }
        Err(e) => out(format!("ERROR   reading {}: {e}", name(path))),
    }
}

/// Safety-net poll over the tracked set only, never the whole folder.
/// Uses fs::metadata, which opens a handle, so the size is the real one and
/// not the possibly stale directory entry.
fn tick(st: &mut State) {
    if !st.polling {
        return;
    }
    let now = Instant::now();
    let mut gone = vec![];
    for (path, t) in st.tracked.iter_mut() {
        match fs::metadata(path) {
            Ok(m) if m.len() > t.seen_len => {
                if t.pending_growth_since.is_none() {
                    t.pending_growth_since = Some(now);
                }
                if !t.stale_reported && now.duration_since(t.pending_growth_since.unwrap()) >= STALE_AFTER {
                    t.stale_reported = true;
                    out(format!(
                        "STALE   {}  grew {} bytes but NO event for {}s (lazy or missed notification, or a partial line)",
                        name(path),
                        m.len() - t.seen_len,
                        STALE_AFTER.as_secs()
                    ));
                }
            }
            Ok(_) => {}
            Err(_) => gone.push(path.clone()),
        }
    }
    for p in gone {
        if let Some(t) = st.tracked.remove(&p) {
            out(format!("GONE    {}  events={} lines={}", name(&p), t.events, t.lines));
        }
    }
}

fn main() {
    START.set(Instant::now()).ok();
    let mut polling = true;
    let mut dir_arg = None;
    for a in std::env::args().skip(1) {
        if a == "--no-poll" {
            polling = false;
        } else {
            dir_arg = Some(PathBuf::from(a));
        }
    }
    let dir = match dir_arg.or_else(default_dir) {
        Some(d) if d.is_dir() => d,
        _ => {
            eprintln!("Chatlogs folder not found. Pass it as the first argument.");
            std::process::exit(1);
        }
    };
    if let Ok(f) = OpenOptions::new().create(true).append(true).open("probe-log.txt") {
        LOG.set(Mutex::new(f)).ok();
    }
    out(format!("WATCH   {}", dir.display()));
    out(format!("MODE    polling {}", if polling { "ON" } else { "OFF (events only)" }));

    let mut st = State { tracked: HashMap::new(), polling, events: 0, poll_first: 0 };

    // One-time enumeration. Names and creation times only: creation time never
    // changes, unlike last-write, which can be stale for files EVE holds open.
    let t0 = Instant::now();
    let mut total = 0usize;
    let mut mtime_recent = 0usize;
    let mut newest: HashMap<(String, String), (String, PathBuf)> = HashMap::new();
    if let Ok(rd) = fs::read_dir(&dir) {
        for e in rd.flatten() {
            let p = e.path();
            if !is_log(&p) {
                continue;
            }
            total += 1;
            let meta = e.metadata().ok();
            if meta
                .as_ref()
                .and_then(|m| m.modified().ok())
                .and_then(|m| m.elapsed().ok())
                .is_some_and(|age| age < Duration::from_secs(600))
            {
                mtime_recent += 1;
            }
            let live = meta
                .and_then(|m| m.created().ok())
                .and_then(|c| c.elapsed().ok())
                .is_none_or(|age| age < LIVE_WINDOW);
            if !live {
                continue;
            }
            if let Some((ch, id, stamp)) = parse_name(&p) {
                let slot = newest.entry((ch, id)).or_insert((String::new(), PathBuf::new()));
                if stamp > slot.0 {
                    *slot = (stamp, p);
                }
            }
        }
    }
    out(format!(
        "SCAN    {total} log files in {} ms. Directory last-write says {mtime_recent} active in the last 10 min; \
         filenames say {} live (character, channel) sessions",
        t0.elapsed().as_millis(),
        newest.len()
    ));
    for (_, (_, p)) in newest {
        handle_change(&p, false, true, &mut st);
    }

    let (tx, rx) = mpsc::channel();
    let mut watcher = RecommendedWatcher::new(tx, Config::default()).expect("create watcher");
    watcher.watch(&dir, RecursiveMode::NonRecursive).expect("watch folder");
    out("READY   waiting for events. Talk in channels, jump, open/close windows. Ctrl+C to stop.");

    let mut last_tick = Instant::now();
    let mut last_status = Instant::now();
    loop {
        match rx.recv_timeout(TICK) {
            Ok(Ok(ev)) => {
                for p in ev.paths.iter().filter(|p| is_log(p)) {
                    match ev.kind {
                        EventKind::Create(_) => handle_change(p, true, false, &mut st),
                        EventKind::Modify(_) => handle_change(p, false, false, &mut st),
                        EventKind::Remove(_) => {
                            if st.tracked.remove(p).is_some() {
                                out(format!("REMOVED {}", name(p)));
                            }
                        }
                        _ => {}
                    }
                }
            }
            Ok(Err(e)) => out(format!("WATCHER ERROR {e}  (possible buffer overflow; events may have been lost)")),
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
        if last_tick.elapsed() >= TICK {
            tick(&mut st);
            last_tick = Instant::now();
        }
        if last_status.elapsed() >= STATUS_EVERY {
            out(format!(
                "STATUS  tracking {} files; events {} (poll-first {}, event-first {}); polling {}",
                st.tracked.len(),
                st.events,
                st.poll_first,
                st.events - st.poll_first,
                if st.polling { "on" } else { "off" }
            ));
            last_status = Instant::now();
        }
    }
}
