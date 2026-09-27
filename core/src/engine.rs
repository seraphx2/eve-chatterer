//! The headless core: follow the live logs, register pilots, merge duplicate
//! lines across characters and turn matches into alerts.
//!
//! The caller drives it: `tick()` about every 500 ms. Deciding *whether* and
//! *how* to show an alert (focus suppression, overlay vs toast) belongs to the
//! router, which knows about windows; an `Alert` carries everything that
//! decision needs: who matched and why, each target's resolved preferences,
//! and which characters' logs contained the line.

use crate::channel::{classify, ChannelKind};
use crate::liveset::{Discovery, LiveConfig, LiveSet};
use crate::logfmt::ChatLine;
use crate::merge::{Listener, MergedLine, Merger};
use crate::pilots::{Observation, Pilot, PilotRegistry};
use crate::prefs::Prefs;
use crate::rules::{LineCtx, Reason};
use crate::settings::SettingsBook;
use crate::time::Stamp;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone)]
pub struct EngineConfig {
    pub live: LiveConfig,
    /// How long a line in a shared channel waits for the other character's copy.
    pub hold: Duration,
    pub merge_tolerance_secs: i64,
    pub rescan_every: Duration,
    /// A session created this recently counts as a live pilot even before it
    /// has produced a line.
    pub new_session_is_live: Duration,
    /// A client window whose character has no log after this long means chat
    /// logging is probably off in EVE's settings.
    pub logging_off_after: Duration,
}

impl Default for EngineConfig {
    fn default() -> Self {
        EngineConfig {
            live: LiveConfig::default(),
            hold: Duration::from_millis(750),
            merge_tolerance_secs: 2,
            rescan_every: Duration::from_secs(5),
            new_session_is_live: Duration::from_secs(120),
            logging_off_after: Duration::from_secs(90),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AlertTarget {
    pub pilot_id: Option<String>,
    pub pilot_name: String,
    pub reason: Reason,
    /// This pilot's resolved presentation choices for this channel.
    pub prefs: Prefs,
}

#[derive(Debug, Clone)]
pub struct Alert {
    pub channel_id: String,
    pub channel_name: String,
    /// What kind of channel this is (from the id; see `channel`).
    pub kind: ChannelKind,
    pub line: ChatLine,
    /// The characters whose settings matched.
    pub targets: Vec<AlertTarget>,
    /// Every character whose log contained the line (a superset of the targets).
    pub seen_by: Vec<Listener>,
}

#[derive(Debug)]
pub enum Event {
    Discovery(Discovery),
    /// First live sighting of a pilot: create its config and tell the user.
    NewPilot(Pilot),
    /// A pilot seen only in old logs, registered silently.
    PilotInLogs(Pilot),
    /// A client window for this character has existed for a while but no chat
    /// log has appeared: "log chat to file" is probably off in EVE. Reported once.
    ChatLoggingOff { name: String },
    Alert(Alert),
}

pub struct Engine {
    live: LiveSet,
    merger: Merger,
    pilots: PilotRegistry,
    book: SettingsBook,
    cfg: EngineConfig,
    last_rescan: Option<Instant>,
    /// Client windows whose character has no log yet, and since when.
    client_wait: HashMap<String, Instant>,
    logging_off_reported: HashSet<String>,
}

fn wall_clock() -> Stamp {
    Stamp(SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0))
}

impl Engine {
    pub fn new(dir: impl Into<PathBuf>, cfg: EngineConfig, pilots: PilotRegistry, book: SettingsBook) -> Engine {
        Engine {
            live: LiveSet::new(dir, cfg.live.clone()),
            merger: Merger::new(cfg.hold, cfg.merge_tolerance_secs),
            pilots,
            book,
            cfg,
            last_rescan: None,
            client_wait: HashMap::new(),
            logging_off_reported: HashSet::new(),
        }
    }

    pub fn pilots(&self) -> &PilotRegistry {
        &self.pilots
    }

    pub fn settings(&self) -> &SettingsBook {
        &self.book
    }

    /// Edit the settings through the book so its cache is invalidated.
    pub fn settings_mut(&mut self) -> &mut SettingsBook {
        &mut self.book
    }

    pub fn live(&self) -> &LiveSet {
        &self.live
    }

    /// Presence (a running client window) says this pilot is live.
    pub fn mark_live(&mut self, id: &str, name: &str) -> Option<Event> {
        match self.pilots.observe(id, name, true, wall_clock()) {
            Observation::NewLive(p) => Some(Event::NewPilot(p)),
            Observation::NewHistoric(p) => Some(Event::PilotInLogs(p)),
            Observation::Known => None,
        }
    }

    /// Feed the names of the running EVE client windows (from presence).
    ///
    /// A pilot already known by name is marked live at once. A character the
    /// registry has never seen cannot be registered from the window alone (the
    /// title has the name, not the id); its log supplies the id within seconds
    /// of login, and the normal tick registers it then. If no log ever shows up,
    /// report that chat logging looks off, once.
    pub fn observe_clients(&mut self, names: &[&str], now: Instant) -> Vec<Event> {
        let mut events = vec![];
        for name in names {
            let has_log = self.live.sessions().any(|s| s.header.is_some_and(|h| h.listener.eq_ignore_ascii_case(name)));
            if let Some(id) = self.pilots.by_name(name).map(|p| p.id.clone()) {
                events.extend(self.mark_live(&id, name));
                self.client_wait.remove(&name.to_lowercase());
            } else if has_log {
                self.client_wait.remove(&name.to_lowercase()); // the next tick registers it
            } else {
                let since = *self.client_wait.entry(name.to_lowercase()).or_insert(now);
                if now.duration_since(since) >= self.cfg.logging_off_after && self.logging_off_reported.insert(name.to_lowercase()) {
                    events.push(Event::ChatLoggingOff { name: name.to_string() });
                }
            }
        }
        // A closed client no longer waits for a log.
        let open: HashSet<String> = names.iter().map(|n| n.to_lowercase()).collect();
        self.client_wait.retain(|k, _| open.contains(k));
        events
    }

    pub fn tick(&mut self, now: Instant) -> Vec<Event> {
        let mut events = vec![];

        if self.last_rescan.is_none_or(|t| now.duration_since(t) >= self.cfg.rescan_every) {
            self.last_rescan = Some(now);
            if let Ok(found) = self.live.rescan() {
                events.extend(found.into_iter().map(Event::Discovery));
            }
        }

        let poll = self.live.poll();
        events.extend(poll.discoveries.into_iter().map(Event::Discovery));

        let mut merged: Vec<MergedLine> = vec![];
        for ev in &poll.events {
            let shared = ev.header.as_ref().is_some_and(|h| self.live.listeners_in_channel(&h.channel_id) > 1);
            merged.extend(self.merger.push(ev, now, shared));
        }
        merged.extend(self.merger.flush(now));

        self.observe_pilots(&mut events);

        for m in merged {
            if let Some(alert) = self.evaluate(m) {
                events.push(Event::Alert(alert));
            }
        }
        events
    }

    fn observe_pilots(&mut self, events: &mut Vec<Event>) {
        let recent = self.cfg.new_session_is_live;
        let seen: Vec<(String, String, bool)> = self
            .live
            .sessions()
            .filter_map(|s| {
                let id = s.char_id?.to_string();
                let name = s.header?.listener.clone();
                let live = s.lines_seen > 0 || s.created.and_then(|c| c.elapsed().ok()).is_some_and(|age| age < recent);
                Some((id, name, live))
            })
            .collect();
        let now = wall_clock();
        for (id, name, live) in seen {
            match self.pilots.observe(&id, &name, live, now) {
                Observation::NewLive(p) => events.push(Event::NewPilot(p)),
                Observation::NewHistoric(p) => events.push(Event::PilotInLogs(p)),
                Observation::Known => {}
            }
        }
    }

    /// Evaluates the line for every character that saw it, each under its own
    /// resolved settings for this channel.
    fn evaluate(&mut self, m: MergedLine) -> Option<Alert> {
        let kind = classify(&m.channel_id, &m.channel_name);
        let mut targets = vec![];
        for l in &m.seen_by {
            let resolved = self.book.resolved(l.char_id.as_deref(), kind, &m.channel_id);
            let ctx = LineCtx { pilot_name: &l.name, channel_name: &m.channel_name, sender: &m.line.sender, text: &m.line.text };
            if let Some(reason) = resolved.rules.evaluate_mode(&ctx, resolved.mode) {
                targets.push(AlertTarget { pilot_id: l.char_id.clone(), pilot_name: l.name.clone(), reason, prefs: resolved.prefs.clone() });
            }
        }
        if targets.is_empty() {
            return None;
        }
        Some(Alert { channel_id: m.channel_id, channel_name: m.channel_name, kind, line: m.line, targets, seen_by: m.seen_by })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::logfmt::testutil::*;
    use crate::prefs::{Mode, OverCap, RateCap};
    use crate::settings::{Layer, Settings};
    use std::fs::{self, OpenOptions};
    use std::io::Write;
    use std::path::Path;

    /// Channel ids as they appear in real headers.
    fn chan_id(channel: &str) -> &'static str {
        match channel {
            "Local" => "local",
            "Corp" => "corp",
            "Fleet" => "fleet_1063112310668",
            c if c.starts_with("Private") => "private_92740cb0b9ee11f19b5a3a68dd86f9e7",
            _ => "system_1_2",
        }
    }

    fn session(dir: &Path, channel: &str, id: &str, listener: &str) -> PathBuf {
        let p = dir.join(format!("{channel}_20260926_100000_{id}.txt"));
        fs::write(&p, file_bytes(&header(chan_id(channel), channel, listener))).unwrap();
        p
    }

    fn append(path: &Path, text: &str) {
        let mut f = OpenOptions::new().append(true).open(path).unwrap();
        f.write_all(&text.encode_utf16().flat_map(|u| u.to_le_bytes()).collect::<Vec<u8>>()).unwrap();
    }

    fn cfg() -> EngineConfig {
        EngineConfig {
            // Files present at startup are joined at their end; rescan on every tick.
            live: LiveConfig { fresh_window: Duration::ZERO, ..LiveConfig::default() },
            rescan_every: Duration::ZERO,
            new_session_is_live: Duration::ZERO,
            ..EngineConfig::default()
        }
    }

    /// A test engine whose settings are just one global layer (no kind defaults).
    fn engine(dir: &Path, global: Layer) -> Engine {
        engine_with(dir, Settings { global, ..Settings::default() })
    }

    fn engine_with(dir: &Path, settings: Settings) -> Engine {
        Engine::new(dir, cfg(), PilotRegistry::default(), SettingsBook::new(settings))
    }

    fn keywords(words: &[&str]) -> Layer {
        Layer { keywords: Some(words.iter().map(|w| w.to_string()).collect()), ..Layer::default() }
    }

    fn alerts(events: &[Event]) -> Vec<&Alert> {
        events.iter().filter_map(|e| if let Event::Alert(a) = e { Some(a) } else { None }).collect()
    }

    #[test]
    fn end_to_end_own_name_mention_and_new_pilot_come_in_order() {
        let dir = tempfile::tempdir().unwrap();
        let p = session(dir.path(), "Local", "1", "Jarna");
        let mut e = engine(dir.path(), Layer::default());
        let t0 = Instant::now();

        let first = e.tick(t0);
        assert!(first.iter().any(|x| matches!(x, Event::PilotInLogs(p) if p.name == "Jarna")), "old session registers silently: {first:?}");
        assert!(alerts(&first).is_empty());

        append(&p, &line("2026.09.26 10:00:05", "Bob", "hey Jarna, got a minute?"));
        let ev = e.tick(t0 + Duration::from_millis(500));
        let new_pos = ev.iter().position(|x| matches!(x, Event::NewPilot(_))).expect("first line makes the pilot live");
        let alert_pos = ev.iter().position(|x| matches!(x, Event::Alert(_))).expect("alert");
        assert!(new_pos < alert_pos, "the pilot is announced before its first alert");
        let a = alerts(&ev)[0];
        assert_eq!(a.line.sender, "Bob");
        assert_eq!(a.kind, ChannelKind::Local);
        assert_eq!(a.targets[0].reason, Reason::OwnName);
        assert_eq!(a.targets[0].pilot_id.as_deref(), Some("1"));

        // A second line does not announce the pilot again.
        append(&p, &line("2026.09.26 10:00:09", "Bob", "Jarna?"));
        let ev = e.tick(t0 + Duration::from_secs(1));
        assert!(!ev.iter().any(|x| matches!(x, Event::NewPilot(_))));
        assert_eq!(alerts(&ev).len(), 1);
    }

    #[test]
    fn two_characters_in_one_channel_produce_one_alert_listing_both() {
        let dir = tempfile::tempdir().unwrap();
        let a = session(dir.path(), "Local", "1", "Jarna");
        let b = session(dir.path(), "Local", "2", "Psianna Archeia");
        let mut e = engine(dir.path(), keywords(&["jita"]));
        let t0 = Instant::now();
        e.tick(t0);

        // The same line, stamped a second apart in the two logs, arrives on different ticks.
        append(&a, &line("2026.09.26 10:00:05", "Bob", "selling in Jita"));
        assert!(alerts(&e.tick(t0 + Duration::from_millis(500))).is_empty(), "held for the other copy");
        append(&b, &line("2026.09.26 10:00:06", "Bob", "selling in Jita"));
        assert!(alerts(&e.tick(t0 + Duration::from_millis(1000))).is_empty());
        let ev = e.tick(t0 + Duration::from_millis(1400));
        let got = alerts(&ev);
        assert_eq!(got.len(), 1, "one alert, not two: {ev:?}");
        assert_eq!(got[0].seen_by.len(), 2);
        assert_eq!(got[0].targets.len(), 2, "the keyword rule matches for both pilots");
    }

    #[test]
    fn own_name_targets_only_the_named_pilot_but_records_both_viewers() {
        let dir = tempfile::tempdir().unwrap();
        let a = session(dir.path(), "Local", "1", "Jarna");
        let b = session(dir.path(), "Local", "2", "Psianna Archeia");
        let mut e = engine(dir.path(), Layer::default());
        let t0 = Instant::now();
        e.tick(t0);
        append(&a, &line("2026.09.26 10:00:05", "Bob", "Jarna, look here"));
        append(&b, &line("2026.09.26 10:00:05", "Bob", "Jarna, look here"));
        e.tick(t0 + Duration::from_millis(500));
        let ev = e.tick(t0 + Duration::from_millis(1300));
        let got = alerts(&ev);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].targets.len(), 1);
        assert_eq!(got[0].targets[0].pilot_name, "Jarna");
        assert_eq!(got[0].seen_by.len(), 2, "the router needs this to know Psianna's screen showed it too");
    }

    #[test]
    fn login_motd_is_not_an_alert() {
        let dir = tempfile::tempdir().unwrap();
        let a = session(dir.path(), "Corp", "1", "Jarna");
        let mut e = engine(dir.path(), Layer::default());
        let t0 = Instant::now();
        e.tick(t0);
        append(&a, &line("2026.09.26 10:00:05", "EVE System", "Channel MOTD: welcome Jarna"));
        assert!(alerts(&e.tick(t0 + Duration::from_millis(500))).is_empty());
    }

    #[test]
    fn a_pilot_seen_only_in_old_logs_is_announced_when_it_first_speaks_live() {
        let dir = tempfile::tempdir().unwrap();
        let p = session(dir.path(), "Local", "9", "Hinata Sunji");
        let mut e = engine(dir.path(), Layer::default());
        let t0 = Instant::now();
        assert!(e.tick(t0).iter().any(|x| matches!(x, Event::PilotInLogs(_))));
        assert!(e.pilots().get("9").is_some_and(|p| !p.live));
        append(&p, &line("2026.09.26 10:00:05", "Bob", "hello"));
        assert!(e.tick(t0 + Duration::from_millis(500)).iter().any(|x| matches!(x, Event::NewPilot(p) if p.id == "9")));
    }

    #[test]
    fn the_kind_defaults_apply_every_private_and_fleet_line_but_only_matches_in_local() {
        let dir = tempfile::tempdir().unwrap();
        let pm = session(dir.path(), "Private Chat (2)", "1", "Jarna");
        let fleet = session(dir.path(), "Fleet", "1", "Jarna");
        let local = session(dir.path(), "Local", "1", "Jarna");
        let mut e = engine_with(dir.path(), Settings::with_defaults());
        let t0 = Instant::now();
        e.tick(t0);
        append(&pm, &line("2026.09.26 10:00:05", "Friend", "got a sec?")); // no keyword, no name
        append(&fleet, &line("2026.09.26 10:00:06", "FC", "align to the gate")); // no keyword, no name
        append(&local, &line("2026.09.26 10:00:07", "Bob", "selling stuff")); // no keyword, no name
        let ev = e.tick(t0 + Duration::from_millis(500));
        let got = alerts(&ev);
        assert_eq!(got.len(), 2, "the private message and the fleet callout alert; plain Local chatter does not: {ev:?}");
        assert_eq!(got[0].kind, ChannelKind::Private);
        assert!(matches!(got[0].targets[0].reason, Reason::AlwaysChannel(_)));
        assert_eq!(got[0].targets[0].prefs.style, Some(crate::prefs::OverlayStyle::Beacon));
        assert_eq!(got[1].kind, ChannelKind::Fleet);
        assert!(matches!(got[1].targets[0].reason, Reason::AlwaysChannel(_)));
    }

    #[test]
    fn a_pilots_override_beats_the_kind_default_and_other_pilots_keep_it() {
        let dir = tempfile::tempdir().unwrap();
        let a = session(dir.path(), "Private Chat (2)", "1", "Jarna");
        let b = session(dir.path(), "Private Chat (2)", "2", "Psianna Archeia");
        let mut settings = Settings::with_defaults();
        // Psianna mutes private messages: her job is scouting, not chat.
        settings.pilots.entry("2".into()).or_default().kinds.insert(ChannelKind::Private, Layer { mode: Some(Mode::Nothing), ..Layer::default() });
        let mut e = engine_with(dir.path(), settings);
        let t0 = Instant::now();
        e.tick(t0);
        append(&a, &line("2026.09.26 10:00:05", "Friend", "hi"));
        append(&b, &line("2026.09.26 10:00:05", "Friend", "hi"));
        e.tick(t0 + Duration::from_millis(500));
        let ev = e.tick(t0 + Duration::from_millis(1300));
        let got = alerts(&ev);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].targets.len(), 1);
        assert_eq!(got[0].targets[0].pilot_name, "Jarna", "Psianna is muted, Jarna still gets it");
        assert_eq!(got[0].seen_by.len(), 2);
    }

    #[test]
    fn resolved_caps_travel_on_the_target_and_edits_take_effect_at_once() {
        let dir = tempfile::tempdir().unwrap();
        let p = session(dir.path(), "Local", "1", "Jarna");
        let mut e = engine_with(dir.path(), Settings::with_defaults());
        let t0 = Instant::now();
        e.tick(t0);
        append(&p, &line("2026.09.26 10:00:05", "Bob", "Jarna?"));
        let ev = e.tick(t0 + Duration::from_millis(500));
        assert_eq!(alerts(&ev)[0].targets[0].prefs.caps.len(), 1, "Local's default cap");

        // Editing settings through the engine invalidates the cache.
        e.settings_mut().edit(|s| {
            s.pilots.entry("1".into()).or_default().base.rate_cap = Some(RateCap { per_minute: 2, over: OverCap::Drop });
        });
        append(&p, &line("2026.09.26 10:00:09", "Bob", "Jarna again"));
        let ev = e.tick(t0 + Duration::from_secs(1));
        assert_eq!(alerts(&ev)[0].targets[0].prefs.caps.len(), 2, "the pilot's cap is now included");
    }

    #[test]
    fn a_client_without_a_log_is_reported_once_as_logging_off() {
        let dir = tempfile::tempdir().unwrap();
        let mut e = engine(dir.path(), Layer::default());
        let t0 = Instant::now();
        assert!(e.observe_clients(&["Jarna"], t0).is_empty());
        assert!(e.observe_clients(&["Jarna"], t0 + Duration::from_secs(60)).is_empty(), "still within the wait");
        let ev = e.observe_clients(&["Jarna"], t0 + Duration::from_secs(91));
        assert!(matches!(ev.as_slice(), [Event::ChatLoggingOff { name }] if name == "Jarna"));
        assert!(e.observe_clients(&["Jarna"], t0 + Duration::from_secs(200)).is_empty(), "reported only once");
    }

    #[test]
    fn a_client_whose_log_appears_is_registered_by_the_tick_not_reported() {
        let dir = tempfile::tempdir().unwrap();
        let mut e = engine(dir.path(), Layer::default());
        let t0 = Instant::now();
        assert!(e.observe_clients(&["Jarna"], t0).is_empty());
        session(dir.path(), "Local", "1", "Jarna");
        let ev = e.tick(t0 + Duration::from_secs(1));
        assert!(ev.iter().any(|x| matches!(x, Event::PilotInLogs(p) if p.name == "Jarna")));
        // Now the name resolves through the registry and the pilot goes live.
        assert!(e.observe_clients(&["Jarna"], t0 + Duration::from_secs(2)).iter().any(|x| matches!(x, Event::NewPilot(p) if p.id == "1")));
        assert!(e.observe_clients(&["Jarna"], t0 + Duration::from_secs(200)).is_empty());
    }

    #[test]
    fn a_closed_client_stops_waiting() {
        let dir = tempfile::tempdir().unwrap();
        let mut e = engine(dir.path(), Layer::default());
        let t0 = Instant::now();
        e.observe_clients(&["Jarna"], t0);
        e.observe_clients(&[], t0 + Duration::from_secs(60)); // window closed
        // Reopened later: the clock starts again rather than firing immediately.
        assert!(e.observe_clients(&["Jarna"], t0 + Duration::from_secs(100)).is_empty());
    }

    #[test]
    fn presence_can_mark_a_pilot_live() {
        let dir = tempfile::tempdir().unwrap();
        let mut e = engine(dir.path(), Layer::default());
        assert!(matches!(e.mark_live("5", "Jarna"), Some(Event::NewPilot(_))));
        assert!(e.mark_live("5", "Jarna").is_none());
    }
}
