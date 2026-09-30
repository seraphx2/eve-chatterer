//! Rate caps: the annoyance ceilings.
//!
//! Every level's cap applies to an alert (`Prefs::caps`), so a channel cannot
//! lift a pilot's ceiling. Each cap counts, per pilot, the alerts admitted in
//! the last minute at that level. A limited alert is not counted, so the
//! window recovers as time passes, and it is either dropped or folded into the
//! count badge of the alert already showing, whichever the strictest exceeded
//! cap asks for. Suppressed alerts never reach the governor: only alerts that
//! would really be shown count.
//!
//! A mention of the character's own name is never capped and never counted
//! (owner decision 2026-09-30): it's the one alert that must always get its
//! Beacon, notification and sound, however busy the channel is.

use crate::prefs::{LayerKey, OverCap, RateCap};
use crate::router::{Decision, Outcome};
use crate::rules::Reason;
use std::collections::{HashMap, VecDeque};
use std::time::{Duration, Instant};

const WINDOW: Duration = Duration::from_secs(60);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    Allow,
    Limited(OverCap),
}

#[derive(Default)]
pub struct Governor {
    windows: HashMap<(String, LayerKey), VecDeque<Instant>>,
}

impl Governor {
    pub fn new() -> Governor {
        Governor::default()
    }

    /// Decides whether an alert for `pilot` fits under all of `caps`, and
    /// counts it if so.
    pub fn admit(&mut self, pilot: &str, caps: &[(LayerKey, RateCap)], now: Instant) -> Verdict {
        let mut over: Option<OverCap> = None;
        for (key, cap) in caps {
            let w = self.windows.entry((pilot.to_string(), key.clone())).or_default();
            while w.front().is_some_and(|t| now.duration_since(*t) >= WINDOW) {
                w.pop_front();
            }
            if w.len() as u32 >= cap.per_minute {
                // Dropping is stricter than folding.
                over = Some(if over == Some(OverCap::Drop) || cap.over == OverCap::Drop { OverCap::Drop } else { OverCap::Fold });
            }
        }
        if let Some(o) = over {
            return Verdict::Limited(o);
        }
        for (key, _) in caps {
            if let Some(w) = self.windows.get_mut(&(pilot.to_string(), key.clone())) {
                w.push_back(now);
            }
        }
        Verdict::Allow
    }

    /// Runs delivered decisions through the caps. Suppressed decisions,
    /// deliveries with nothing to show and own-name mentions pass through
    /// untouched.
    pub fn apply(&mut self, decisions: Vec<Decision>, now: Instant) -> Vec<Decision> {
        decisions
            .into_iter()
            .map(|mut d| {
                if d.caps.is_empty() || d.reason == Reason::OwnName {
                    return d;
                }
                if let Outcome::Deliver(v) = &mut d.outcome {
                    if !v.is_empty() {
                        let who = d.pilot_id.clone().unwrap_or_else(|| d.pilot_name.clone());
                        if let Verdict::Limited(over) = self.admit(&who, &d.caps, now) {
                            let deliveries = std::mem::take(v);
                            d.outcome = Outcome::Limited { over, deliveries };
                        }
                    }
                }
                d
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::channel::ChannelKind;
    use crate::router::{Delivery, SuppressedBy};
    use crate::rules::Reason;

    fn cap(n: u32, over: OverCap) -> RateCap {
        RateCap { per_minute: n, over }
    }

    fn local() -> LayerKey {
        LayerKey::Kind(ChannelKind::Local)
    }

    #[test]
    fn allows_up_to_the_cap_then_limits_and_recovers() {
        let mut g = Governor::new();
        let caps = [(local(), cap(3, OverCap::Fold))];
        let t0 = Instant::now();
        for i in 0..3 {
            assert_eq!(g.admit("1", &caps, t0 + Duration::from_secs(i)), Verdict::Allow);
        }
        assert_eq!(g.admit("1", &caps, t0 + Duration::from_secs(10)), Verdict::Limited(OverCap::Fold));
        // The limited alert was not counted, and after the window slides the oldest are gone.
        assert_eq!(g.admit("1", &caps, t0 + Duration::from_secs(61)), Verdict::Allow);
    }

    #[test]
    fn each_pilot_has_its_own_window() {
        let mut g = Governor::new();
        let caps = [(local(), cap(1, OverCap::Drop))];
        let t0 = Instant::now();
        assert_eq!(g.admit("1", &caps, t0), Verdict::Allow);
        assert_eq!(g.admit("1", &caps, t0), Verdict::Limited(OverCap::Drop));
        assert_eq!(g.admit("2", &caps, t0), Verdict::Allow);
    }

    #[test]
    fn every_level_applies_and_drop_beats_fold() {
        let mut g = Governor::new();
        let caps = [(local(), cap(6, OverCap::Fold)), (LayerKey::Pilot, cap(2, OverCap::Drop))];
        let t0 = Instant::now();
        assert_eq!(g.admit("1", &caps, t0), Verdict::Allow);
        assert_eq!(g.admit("1", &caps, t0), Verdict::Allow);
        // The kind's cap of 6 has room, but the pilot's cap of 2 does not.
        assert_eq!(g.admit("1", &caps, t0), Verdict::Limited(OverCap::Drop));
        // Only the kind cap exceeded: fold.
        let mut g = Governor::new();
        let caps = [(local(), cap(1, OverCap::Fold)), (LayerKey::Pilot, cap(9, OverCap::Drop))];
        g.admit("1", &caps, t0);
        assert_eq!(g.admit("1", &caps, t0), Verdict::Limited(OverCap::Fold));
    }

    #[test]
    fn a_limited_alert_does_not_use_up_the_other_levels_budget() {
        let mut g = Governor::new();
        let caps = [(local(), cap(1, OverCap::Fold)), (LayerKey::Pilot, cap(2, OverCap::Fold))];
        let t0 = Instant::now();
        assert_eq!(g.admit("1", &caps, t0), Verdict::Allow); // pilot window: 1, kind window: 1
        assert_eq!(g.admit("1", &caps, t0), Verdict::Limited(OverCap::Fold)); // kind is full
        // Had the limited alert been counted, the pilot window would now be full at 2.
        let only_pilot = [(LayerKey::Pilot, cap(2, OverCap::Fold))];
        assert_eq!(g.admit("1", &only_pilot, t0), Verdict::Allow);
    }

    #[test]
    fn no_caps_means_no_limit_and_a_zero_cap_blocks_everything() {
        let mut g = Governor::new();
        let t0 = Instant::now();
        for _ in 0..100 {
            assert_eq!(g.admit("1", &[], t0), Verdict::Allow);
        }
        assert_eq!(g.admit("1", &[(local(), cap(0, OverCap::Drop))], t0), Verdict::Limited(OverCap::Drop));
    }

    fn decision(reason: Reason, outcome: Outcome, caps: Vec<(LayerKey, RateCap)>) -> Decision {
        Decision { pilot_name: "Holden".into(), pilot_id: Some("1".into()), reason, outcome, caps }
    }

    fn keyword() -> Reason {
        Reason::Keyword("jita".into())
    }

    #[test]
    fn apply_only_counts_and_limits_alerts_that_would_be_shown() {
        let mut g = Governor::new();
        let t0 = Instant::now();
        let shown = || Outcome::Deliver(vec![Delivery::Sound]);
        let caps = || vec![(local(), cap(1, OverCap::Fold))];

        // Suppressed and empty deliveries pass through and do not use the budget.
        let out = g.apply(
            vec![
                decision(keyword(), Outcome::Suppressed(SuppressedBy::FocusedPilot), caps()),
                decision(keyword(), Outcome::Deliver(vec![]), caps()),
                decision(keyword(), shown(), caps()),
                decision(keyword(), shown(), caps()),
            ],
            t0,
        );
        assert_eq!(out[0].outcome, Outcome::Suppressed(SuppressedBy::FocusedPilot));
        assert_eq!(out[1].outcome, Outcome::Deliver(vec![]));
        assert_eq!(out[2].outcome, shown(), "the first shown alert fits the cap of 1");
        assert_eq!(
            out[3].outcome,
            Outcome::Limited { over: OverCap::Fold, deliveries: vec![Delivery::Sound] },
            "the second does not, and keeps where it would have gone"
        );
    }

    #[test]
    fn a_mention_is_never_capped_and_never_uses_the_budget() {
        let mut g = Governor::new();
        let t0 = Instant::now();
        let shown = || Outcome::Deliver(vec![Delivery::Sound]);
        let caps = || vec![(local(), cap(1, OverCap::Drop))];
        let out = g.apply(
            vec![
                decision(Reason::OwnName, shown(), caps()),
                decision(Reason::OwnName, shown(), caps()),
                decision(keyword(), shown(), caps()),
                decision(Reason::OwnName, shown(), caps()),
            ],
            t0,
        );
        assert!(out.iter().all(|d| d.outcome == shown()), "{out:?}");
    }
}
