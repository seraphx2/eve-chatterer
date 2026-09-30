//! Collapsing the same chat line as seen by several of the user's characters.
//!
//! Two characters in the same channel write the same line to separate log
//! files, sometimes with timestamps one second apart (docs/FINDINGS.md #3). A
//! line is merged when (channel, sender, text) match, the stamps are within a
//! tolerance, and the copies come from *different* listeners. The same
//! listener saying the same thing twice is two lines. The first copy is held
//! briefly, only when another listener is following the same channel, so the
//! second copy can join it. A copy that arrives after the line already went
//! out still joins it, and comes back to be evaluated for its own listener
//! alone: that listener's rules (its own name above all) never saw the line.

use crate::liveset::LineEvent;
use crate::logfmt::ChatLine;
use std::time::{Duration, Instant};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Listener {
    pub char_id: Option<String>,
    pub name: String,
}

#[derive(Debug, Clone)]
pub struct MergedLine {
    pub channel_id: String,
    pub channel_name: String,
    pub line: ChatLine,
    /// Every character whose log contained this line.
    pub seen_by: Vec<Listener>,
    /// The characters to evaluate the line for: all of `seen_by`, except for
    /// a copy that arrived after the line went out, which is evaluated for
    /// its own character alone (the others already were).
    pub evaluate_for: Vec<Listener>,
}

struct Entry {
    merged: MergedLine,
    first_seen: Instant,
    emitted: bool,
}

pub struct Merger {
    hold: Duration,
    tolerance_secs: i64,
    retain: Duration,
    entries: Vec<Entry>,
}

impl Merger {
    pub fn new(hold: Duration, tolerance_secs: i64) -> Merger {
        // Keep emitted entries long enough for a slow second copy to be recognised.
        Merger { hold, tolerance_secs, retain: hold + Duration::from_secs(10), entries: vec![] }
    }

    /// `shared` is whether another character is following the same channel.
    /// If not, the line is returned immediately with no hold. A copy joining
    /// a line that already went out is returned too, to be evaluated for its
    /// own character (see `MergedLine::evaluate_for`).
    pub fn push(&mut self, ev: &LineEvent, now: Instant, shared: bool) -> Option<MergedLine> {
        let (channel_id, listener_name) = match &ev.header {
            Some(h) => (h.channel_id.clone(), h.listener.clone()),
            None => (String::new(), String::new()),
        };
        let listener = Listener { char_id: ev.char_id.clone(), name: listener_name };
        let same_channel = |m: &MergedLine| {
            if channel_id.is_empty() {
                m.channel_name == ev.channel_name
            } else {
                m.channel_id == channel_id
            }
        };
        let tol = self.tolerance_secs;
        if let Some(e) = self.entries.iter_mut().find(|e| {
            let m = &e.merged;
            same_channel(m)
                && m.line.sender == ev.line.sender
                && m.line.text == ev.line.text
                && (m.line.stamp.since(ev.line.stamp)).abs() <= tol
                && !m.seen_by.contains(&listener)
        }) {
            e.merged.seen_by.push(listener.clone());
            if !e.emitted {
                return None; // it goes out with the others when the hold ends
            }
            // Too late to go out with the others: evaluate it for this
            // listener alone, or a mention of this character would be lost.
            return Some(MergedLine { evaluate_for: vec![listener], ..e.merged.clone() });
        }
        let merged = MergedLine {
            channel_id,
            channel_name: ev.channel_name.clone(),
            line: ev.line.clone(),
            seen_by: vec![listener.clone()],
            evaluate_for: vec![listener],
        };
        self.entries.push(Entry { merged: merged.clone(), first_seen: now, emitted: !shared });
        (!shared).then_some(merged)
    }

    /// Emits lines whose hold has run out and forgets old entries.
    pub fn flush(&mut self, now: Instant) -> Vec<MergedLine> {
        let mut out = vec![];
        for e in self.entries.iter_mut().filter(|e| !e.emitted) {
            if now.duration_since(e.first_seen) >= self.hold {
                e.emitted = true;
                out.push(MergedLine { evaluate_for: e.merged.seen_by.clone(), ..e.merged.clone() });
            }
        }
        let retain = self.retain;
        self.entries.retain(|e| now.duration_since(e.first_seen) < retain);
        out.sort_by_key(|m| m.line.stamp);
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::logfmt::Header;
    use crate::time::Stamp;
    use std::path::PathBuf;

    fn ev(char_id: &str, name: &str, ts: i64, sender: &str, text: &str) -> LineEvent {
        LineEvent {
            path: PathBuf::new(),
            char_id: Some(char_id.to_string()),
            channel_name: "Local".to_string(),
            header: Some(Header { channel_id: "local".into(), channel_name: "Local".into(), listener: name.into(), session_started: None, instance: None }),
            line: ChatLine { stamp: Stamp(ts), sender: sender.into(), text: text.into() },
        }
    }

    const HOLD: Duration = Duration::from_millis(750);

    #[test]
    fn the_same_line_from_two_characters_becomes_one_with_both_listeners() {
        let mut m = Merger::new(HOLD, 2);
        let t0 = Instant::now();
        assert!(m.push(&ev("1", "Holden", 100, "Bob", "buying"), t0, true).is_none());
        // Naomi's copy carries a stamp one second later and arrives on the next poll.
        assert!(m.push(&ev("2", "Naomi", 101, "Bob", "buying"), t0 + Duration::from_millis(500), true).is_none());
        assert!(m.flush(t0 + Duration::from_millis(600)).is_empty(), "still holding");
        let out = m.flush(t0 + HOLD);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].seen_by.len(), 2);
        assert!(m.flush(t0 + Duration::from_secs(5)).is_empty(), "emitted once");
    }

    #[test]
    fn a_lone_line_in_a_shared_channel_is_emitted_after_the_hold() {
        let mut m = Merger::new(HOLD, 2);
        let t0 = Instant::now();
        m.push(&ev("1", "Holden", 100, "Bob", "hi"), t0, true);
        let out = m.flush(t0 + HOLD);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].seen_by.len(), 1);
    }

    #[test]
    fn unshared_channels_are_not_delayed() {
        let mut m = Merger::new(HOLD, 2);
        assert!(m.push(&ev("1", "Holden", 100, "Bob", "hi"), Instant::now(), false).is_some());
    }

    #[test]
    fn the_copies_that_join_during_the_hold_are_all_evaluated() {
        let mut m = Merger::new(HOLD, 2);
        let t0 = Instant::now();
        m.push(&ev("1", "Holden", 100, "Bob", "hi"), t0, true);
        m.push(&ev("2", "Naomi", 100, "Bob", "hi"), t0, true);
        let out = m.flush(t0 + HOLD);
        let names = |v: &[Listener]| v.iter().map(|l| l.name.clone()).collect::<Vec<_>>();
        assert_eq!(names(&out[0].evaluate_for), ["Holden", "Naomi"]);
    }

    #[test]
    fn a_late_second_copy_is_evaluated_for_its_own_character_only() {
        let mut m = Merger::new(HOLD, 2);
        let t0 = Instant::now();
        m.push(&ev("1", "Holden", 100, "Bob", "Naomi, dock up"), t0, true);
        assert_eq!(m.flush(t0 + HOLD).len(), 1);
        let late = m.push(&ev("2", "Naomi", 101, "Bob", "Naomi, dock up"), t0 + Duration::from_secs(2), true).expect("returned for Naomi");
        assert_eq!(late.evaluate_for, vec![Listener { char_id: Some("2".into()), name: "Naomi".into() }]);
        assert_eq!(late.seen_by.len(), 2, "both logs had it");
        assert!(m.flush(t0 + Duration::from_secs(3)).is_empty(), "the line itself doesn't go out again");
        // The same text again in Naomi's log is Bob repeating himself: a line
        // of its own, held like any other.
        assert!(m.push(&ev("2", "Naomi", 101, "Bob", "Naomi, dock up"), t0 + Duration::from_secs(2), true).is_none());
    }

    #[test]
    fn the_same_listener_repeating_itself_is_two_lines() {
        let mut m = Merger::new(HOLD, 2);
        let t0 = Instant::now();
        m.push(&ev("1", "Holden", 100, "Bob", "spam"), t0, true);
        m.push(&ev("1", "Holden", 101, "Bob", "spam"), t0, true);
        assert_eq!(m.flush(t0 + HOLD).len(), 2);
    }

    #[test]
    fn different_text_or_far_apart_stamps_do_not_merge() {
        let mut m = Merger::new(HOLD, 2);
        let t0 = Instant::now();
        m.push(&ev("1", "Holden", 100, "Bob", "a"), t0, true);
        m.push(&ev("2", "Naomi", 100, "Bob", "b"), t0, true);
        m.push(&ev("2", "Naomi", 110, "Bob", "a"), t0, true);
        assert_eq!(m.flush(t0 + HOLD).len(), 3);
    }
}
