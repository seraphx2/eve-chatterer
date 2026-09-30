//! Finds and follows the live log files.
//!
//! The live set is the newest file per (character id, channel), derived from
//! file names alone. Directory events are not trusted for growth and directory
//! last-write times are stale for files EVE holds open (docs/FINDINGS.md #1,
//! #2), so the caller polls this set on a timer and rescans the folder now and
//! then. Total file count in the folder does not matter.

use crate::logfmt::{ChatLine, Header};
use crate::tailer::{Start, Tailer};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

/// `<Channel>_<YYYYMMDD>_<HHMMSS>[_<characterId>].txt`, stamp in UTC.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionName {
    pub channel: String,
    pub char_id: Option<String>,
    /// `YYYYMMDD_HHMMSS`, sorts chronologically as text.
    pub stamp: String,
}

impl SessionName {
    fn key(&self) -> (String, String) {
        (self.channel.clone(), self.char_id.clone().unwrap_or_default())
    }
}

fn digits(s: &str, len: Option<usize>) -> bool {
    !s.is_empty() && len.is_none_or(|n| s.len() == n) && s.bytes().all(|b| b.is_ascii_digit())
}

pub fn parse_session_name(file_name: &str) -> Option<SessionName> {
    let (stem, ext) = file_name.rsplit_once('.')?;
    if !ext.eq_ignore_ascii_case("txt") {
        return None;
    }
    // Channel names may contain underscores and spaces, so parse from the right.
    let p: Vec<&str> = stem.split('_').collect();
    let n = p.len();
    if n >= 4 && digits(p[n - 3], Some(8)) && digits(p[n - 2], Some(6)) && digits(p[n - 1], None) {
        Some(SessionName {
            channel: p[..n - 3].join("_"),
            char_id: Some(p[n - 1].to_string()),
            stamp: format!("{}_{}", p[n - 3], p[n - 2]),
        })
    } else if n >= 3 && digits(p[n - 2], Some(8)) && digits(p[n - 1], Some(6)) {
        Some(SessionName { channel: p[..n - 2].join("_"), char_id: None, stamp: format!("{}_{}", p[n - 2], p[n - 1]) })
    } else {
        None
    }
}

/// OneDrive "Files On-Demand" stubs. Opening one downloads it, which we must
/// never do just to look at a log we may not need.
#[cfg(windows)]
fn is_cloud_placeholder(m: &fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    const OFFLINE: u32 = 0x1000;
    const RECALL_ON_OPEN: u32 = 0x4_0000;
    const RECALL_ON_DATA_ACCESS: u32 = 0x40_0000;
    m.file_attributes() & (OFFLINE | RECALL_ON_OPEN | RECALL_ON_DATA_ACCESS) != 0
}

#[cfg(not(windows))]
fn is_cloud_placeholder(_: &fs::Metadata) -> bool {
    false
}

#[derive(Debug, Clone)]
pub struct LiveConfig {
    /// Sessions created longer ago than this are not considered live.
    pub live_window: Duration,
    /// Sessions created more recently than this are read from their header
    /// (they just started); older ones are joined at their current end.
    pub fresh_window: Duration,
}

impl Default for LiveConfig {
    fn default() -> Self {
        LiveConfig { live_window: Duration::from_secs(14 * 24 * 3600), fresh_window: Duration::from_secs(60) }
    }
}

struct Session {
    name: SessionName,
    tailer: Tailer,
    created: Option<SystemTime>,
    lines_seen: u64,
}

pub struct SessionInfo<'a> {
    pub path: &'a Path,
    pub char_id: Option<&'a str>,
    pub channel: &'a str,
    pub header: Option<&'a Header>,
    pub created: Option<SystemTime>,
    pub lines_seen: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Discovery {
    Adopted { path: PathBuf, name: SessionName, from_start: bool },
    /// A newer session for the same (character, channel) replaced an older one.
    Superseded { old: PathBuf, new: PathBuf },
    /// A cloud-only OneDrive stub; not opened.
    SkippedPlaceholder(PathBuf),
    /// The file disappeared.
    Gone(PathBuf),
}

#[derive(Debug, Clone)]
pub struct LineEvent {
    pub path: PathBuf,
    pub char_id: Option<String>,
    pub channel_name: String,
    pub header: Option<Header>,
    pub line: ChatLine,
}

pub struct Poll {
    pub events: Vec<LineEvent>,
    pub discoveries: Vec<Discovery>,
}

pub struct LiveSet {
    dir: PathBuf,
    cfg: LiveConfig,
    sessions: HashMap<PathBuf, Session>,
    skipped: HashSet<PathBuf>,
}

impl LiveSet {
    pub fn new(dir: impl Into<PathBuf>, cfg: LiveConfig) -> LiveSet {
        LiveSet { dir: dir.into(), cfg, sessions: HashMap::new(), skipped: HashSet::new() }
    }

    pub fn len(&self) -> usize {
        self.sessions.len()
    }

    pub fn is_empty(&self) -> bool {
        self.sessions.is_empty()
    }

    pub fn sessions(&self) -> impl Iterator<Item = SessionInfo<'_>> {
        self.sessions.iter().map(|(path, s)| SessionInfo {
            path,
            char_id: s.name.char_id.as_deref(),
            channel: &s.name.channel,
            header: s.tailer.header(),
            created: s.created,
            lines_seen: s.lines_seen,
        })
    }

    /// Distinct characters currently followed in the same channel as `h`. More
    /// than one means the same line may arrive from several logs. Corp and
    /// Alliance share one channel id (`corp` / `alliance`) across every
    /// corporation, so when both logs name their instance, the names must
    /// match too; an unknown instance counts as the same (the safe side: a
    /// needless merge wait, never a duplicate alert).
    pub fn listeners_in_channel(&self, h: &Header) -> usize {
        if h.channel_id.is_empty() {
            return 1;
        }
        let same_instance = |o: &Header| match (&h.instance, &o.instance) {
            (Some(a), Some(b)) => a.eq_ignore_ascii_case(b),
            _ => true,
        };
        self.sessions
            .values()
            .filter(|s| s.tailer.header().is_some_and(|o| o.channel_id == h.channel_id && same_instance(o)))
            .map(|s| s.name.char_id.as_deref().unwrap_or(""))
            .collect::<HashSet<_>>()
            .len()
    }

    /// Lists the folder (names and creation times only, nothing is opened),
    /// picks the newest session per (character, channel) and starts following
    /// any that are new. Cheap enough to call every few seconds.
    pub fn rescan(&mut self) -> io::Result<Vec<Discovery>> {
        /// (session, path, created, cloud placeholder) by (character, channel).
        type Newest = HashMap<(String, String), (SessionName, PathBuf, Option<SystemTime>, bool)>;
        let mut newest: Newest = HashMap::new();
        for entry in fs::read_dir(&self.dir)? {
            let Ok(entry) = entry else { continue };
            let path = entry.path();
            let Some(name) = path.file_name().and_then(|n| n.to_str()).and_then(parse_session_name) else { continue };
            // DirEntry metadata comes from the directory listing: no handle, no hydration.
            let Ok(meta) = entry.metadata() else { continue };
            let created = meta.created().ok();
            if created.and_then(|c| c.elapsed().ok()).is_some_and(|age| age > self.cfg.live_window) {
                continue;
            }
            let placeholder = is_cloud_placeholder(&meta);
            let slot = newest.entry(name.key());
            match slot {
                std::collections::hash_map::Entry::Occupied(mut o) => {
                    if name.stamp > o.get().0.stamp {
                        o.insert((name, path, created, placeholder));
                    }
                }
                std::collections::hash_map::Entry::Vacant(v) => {
                    v.insert((name, path, created, placeholder));
                }
            }
        }

        let mut out = vec![];
        for (key, (name, path, created, placeholder)) in newest {
            if self.sessions.contains_key(&path) {
                continue;
            }
            if placeholder {
                if self.skipped.insert(path.clone()) {
                    out.push(Discovery::SkippedPlaceholder(path));
                }
                continue;
            }
            self.skipped.remove(&path);
            let fresh = created.and_then(|c| c.elapsed().ok()).is_some_and(|age| age < self.cfg.fresh_window);
            let start = if fresh { Start::Beginning } else { Start::End };
            let Ok(tailer) = Tailer::open(&path, start) else { continue };

            let older: Vec<PathBuf> = self
                .sessions
                .iter()
                .filter(|(_, s)| s.name.key() == key && s.name.stamp < name.stamp)
                .map(|(p, _)| p.clone())
                .collect();
            for old in older {
                self.sessions.remove(&old);
                out.push(Discovery::Superseded { old, new: path.clone() });
            }
            out.push(Discovery::Adopted { path: path.clone(), name: name.clone(), from_start: fresh });
            self.sessions.insert(path, Session { name, tailer, created, lines_seen: 0 });
        }
        Ok(out)
    }

    /// Reads new lines from every followed file. Call about every 500 ms.
    pub fn poll(&mut self) -> Poll {
        let mut events = vec![];
        let mut gone = vec![];
        for (path, s) in self.sessions.iter_mut() {
            match s.tailer.poll() {
                Ok(lines) => {
                    s.lines_seen += lines.len() as u64;
                    for line in lines {
                        events.push(LineEvent {
                            path: path.clone(),
                            char_id: s.name.char_id.clone(),
                            channel_name: s.name.channel.clone(),
                            header: s.tailer.header().cloned(),
                            line,
                        });
                    }
                }
                Err(e) if e.kind() == io::ErrorKind::NotFound => gone.push(path.clone()),
                Err(_) => {} // transient (sharing violation, sync in progress): try again next tick
            }
        }
        events.sort_by_key(|e| e.line.stamp);
        let discoveries = gone
            .into_iter()
            .map(|p| {
                self.sessions.remove(&p);
                Discovery::Gone(p)
            })
            .collect();
        Poll { events, discoveries }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::logfmt::testutil::*;
    use std::fs::OpenOptions;
    use std::io::Write;

    fn write_session(dir: &Path, channel: &str, stamp: &str, id: &str, listener: &str, lines: &[(&str, &str, &str)]) -> PathBuf {
        let mut body = header(if channel == "Local" { "local" } else { "chan" }, channel, listener);
        for (t, s, x) in lines {
            body += &line(t, s, x);
        }
        let p = dir.join(format!("{channel}_{stamp}_{id}.txt"));
        fs::write(&p, file_bytes(&body)).unwrap();
        p
    }

    fn append(path: &Path, text: &str) {
        let mut f = OpenOptions::new().append(true).open(path).unwrap();
        f.write_all(&text.encode_utf16().flat_map(|u| u.to_le_bytes()).collect::<Vec<u8>>()).unwrap();
    }

    fn existing() -> LiveConfig {
        // Nothing counts as fresh: every file present at startup is joined at its end.
        LiveConfig { fresh_window: Duration::ZERO, ..LiveConfig::default() }
    }

    #[test]
    fn parses_session_names() {
        let n = parse_session_name("Local_20260926_210356_2112000001.txt").unwrap();
        assert_eq!((n.channel.as_str(), n.char_id.as_deref(), n.stamp.as_str()), ("Local", Some("2112000001"), "20260926_210356"));
        let n = parse_session_name("Private Chat (2)_20260926_210933_2112000001.txt").unwrap();
        assert_eq!(n.channel, "Private Chat (2)");
        // Real names from a public channel, a fleet and an alliance member's log.
        for (file, chan) in [
            ("EVE University_20260927_041925_2112000002.txt", "EVE University"),
            ("Fleet_20260927_041837_2112000002.txt", "Fleet"),
            ("Alliance_20260925_121918_496528567.txt", "Alliance"),
        ] {
            assert_eq!(parse_session_name(file).unwrap().channel, chan, "{file}");
        }
        let n = parse_session_name("Fleet_Ops_20260926_210933_5.txt").unwrap();
        assert_eq!(n.channel, "Fleet_Ops");
        let n = parse_session_name("Local_20260926_210356.txt").unwrap();
        assert_eq!(n.char_id, None);
        for bad in ["notes.txt", "Local_20260926_210356_1.log", "Local.txt", "20260926_210356_1.txt"] {
            assert!(parse_session_name(bad).is_none(), "{bad}");
        }
    }

    #[test]
    fn live_set_is_the_newest_file_per_character_and_channel() {
        let dir = tempfile::tempdir().unwrap();
        write_session(dir.path(), "Local", "20260901_100000", "1", "Holden", &[]);
        let newest = write_session(dir.path(), "Local", "20260926_100000", "1", "Holden", &[]);
        write_session(dir.path(), "Local", "20260926_100000", "2", "Naomi", &[]);
        write_session(dir.path(), "Corp", "20260926_100000", "1", "Holden", &[]);
        fs::write(dir.path().join("readme.txt"), "x").unwrap();

        let mut live = LiveSet::new(dir.path(), existing());
        let d = live.rescan().unwrap();
        assert_eq!(live.len(), 3, "old Local for Holden and the stray txt are ignored: {d:?}");
        assert!(live.sessions().any(|s| s.path == newest));
        assert!(live.rescan().unwrap().is_empty(), "a second scan finds nothing new");
    }

    #[test]
    fn follows_appends_without_replaying_history() {
        let dir = tempfile::tempdir().unwrap();
        let p = write_session(dir.path(), "Local", "20260926_100000", "1", "Holden", &[("2026.09.26 10:00:01", "A", "old")]);
        let mut live = LiveSet::new(dir.path(), existing());
        live.rescan().unwrap();
        assert!(live.poll().events.is_empty());
        append(&p, &line("2026.09.26 10:00:09", "B", "hello"));
        let ev = live.poll().events;
        assert_eq!(ev.len(), 1);
        assert_eq!(ev[0].line.text, "hello");
        assert_eq!(ev[0].header.as_ref().unwrap().listener, "Holden");
        assert_eq!(ev[0].char_id.as_deref(), Some("1"));
    }

    #[test]
    fn a_new_session_supersedes_the_old_one_and_replays_from_its_header() {
        let dir = tempfile::tempdir().unwrap();
        let old = write_session(dir.path(), "Local", "20260926_100000", "1", "Holden", &[]);
        let mut live = LiveSet::new(dir.path(), LiveConfig::default());
        live.rescan().unwrap();
        assert_eq!(live.len(), 1);

        // Holden logs in again: a brand-new file appears with lines already in it.
        let new = write_session(dir.path(), "Local", "20260926_120000", "1", "Holden", &[("2026.09.26 12:00:02", "Bob", "first")]);
        let d = live.rescan().unwrap();
        assert!(d.contains(&Discovery::Superseded { old: old.clone(), new: new.clone() }), "{d:?}");
        assert!(d.iter().any(|x| matches!(x, Discovery::Adopted { from_start: true, .. })));
        assert_eq!(live.len(), 1);
        let ev = live.poll().events;
        assert_eq!(ev.iter().map(|e| e.line.text.as_str()).collect::<Vec<_>>(), ["first"]);
    }

    #[test]
    fn counts_distinct_listeners_in_a_channel() {
        let dir = tempfile::tempdir().unwrap();
        write_session(dir.path(), "Local", "20260926_100000", "1", "Holden", &[]);
        write_session(dir.path(), "Local", "20260926_100000", "2", "Naomi", &[]);
        write_session(dir.path(), "Corp", "20260926_100000", "1", "Holden", &[]);
        let mut live = LiveSet::new(dir.path(), existing());
        live.rescan().unwrap();
        let h = |id: &str| Header { channel_id: id.into(), ..Header::default() };
        assert_eq!(live.listeners_in_channel(&h("local")), 2);
        assert_eq!(live.listeners_in_channel(&h("chan")), 1);
        assert_eq!(live.listeners_in_channel(&h("")), 1);
    }

    #[test]
    fn characters_in_different_corps_do_not_share_corp_chat() {
        let dir = tempfile::tempdir().unwrap();
        let named = |corp: &str| [("2026.09.26 10:00:00", "EVE System", format!("Channel changed to Corp : {corp}"))];
        for (id, who, corp) in [("1", "Holden", "Rocinante"), ("2", "Naomi", "Rocinante"), ("3", "Amos Burton", "Tycho Station")] {
            let l = named(corp);
            write_session(dir.path(), "Corp", "20260926_100000", id, who, &[(l[0].0, l[0].1, l[0].2.as_str())]);
        }
        let mut live = LiveSet::new(dir.path(), existing());
        live.rescan().unwrap();
        let corp = |name: Option<&str>| Header { channel_id: "chan".into(), instance: name.map(String::from), ..Header::default() };
        assert_eq!(live.listeners_in_channel(&corp(Some("Rocinante"))), 2);
        assert_eq!(live.listeners_in_channel(&corp(Some("tycho station"))), 1, "names compare without case");
        assert_eq!(live.listeners_in_channel(&corp(None)), 3, "an unknown corp waits, as before");
    }

    #[test]
    fn a_deleted_file_is_dropped() {
        let dir = tempfile::tempdir().unwrap();
        let p = write_session(dir.path(), "Local", "20260926_100000", "1", "Holden", &[]);
        let mut live = LiveSet::new(dir.path(), existing());
        live.rescan().unwrap();
        fs::remove_file(&p).unwrap();
        let poll = live.poll();
        assert_eq!(poll.discoveries, vec![Discovery::Gone(p)]);
        assert!(live.is_empty());
    }

    #[test]
    fn events_from_several_files_are_ordered_by_time() {
        let dir = tempfile::tempdir().unwrap();
        let a = write_session(dir.path(), "Local", "20260926_100000", "1", "Holden", &[]);
        let b = write_session(dir.path(), "Corp", "20260926_100000", "1", "Holden", &[]);
        let mut live = LiveSet::new(dir.path(), existing());
        live.rescan().unwrap();
        append(&b, &line("2026.09.26 10:00:05", "X", "later"));
        append(&a, &line("2026.09.26 10:00:01", "Y", "earlier"));
        let ev = live.poll().events;
        assert_eq!(ev.iter().map(|e| e.line.text.as_str()).collect::<Vec<_>>(), ["earlier", "later"]);
    }
}
