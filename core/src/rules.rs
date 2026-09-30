//! Deciding whether a chat line deserves an alert for one pilot.
//!
//! Per-channel behavior (mute, mentions only, everything) is a `Mode` chosen by
//! the settings layers, not a list here; this module holds the content rules
//! and applies a mode to them.

use crate::prefs::Mode;
use regex::Regex;
use serde::{Deserialize, Serialize};

/// One tracked keyword or regex, already scoped to the current channel by
/// `Settings::resolve()` (a `settings::TrackedRule`'s channel targeting is
/// resolved away before it becomes this) — all that survives into matching is
/// the text and whether it's still allowed to fire on a muted (`Mode::Nothing`)
/// channel.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct TrackedTerm {
    pub text: String,
    pub even_when_muted: bool,
}

impl Default for TrackedTerm {
    fn default() -> Self {
        TrackedTerm { text: String::new(), even_when_muted: true }
    }
}

/// Lets existing plain-string construction (`vec!["jita".into()]`) keep
/// working: a bare string means "always fires, even when muted" — today's
/// original, only behavior before the per-entry toggle existed.
impl From<&str> for TrackedTerm {
    fn from(s: &str) -> Self {
        TrackedTerm { text: s.to_string(), even_when_muted: true }
    }
}

/// The content rules, after the settings layers have been merged. Matching is case-insensitive.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct RuleSet {
    /// Alert when the listening pilot's own name appears in the text.
    pub own_name: bool,
    pub keywords: Vec<TrackedTerm>,
    /// Advanced: regular expressions matched against the text.
    pub regexes: Vec<TrackedTerm>,
    pub ignore_own_messages: bool,
    /// Senders that are never alerted on, chiefly the login message-of-the-day.
    pub ignore_system: bool,
    pub system_senders: Vec<String>,
    pub ignore_senders: Vec<String>,
    /// Alert on every line from these senders.
    pub always_senders: Vec<String>,
}

impl Default for RuleSet {
    fn default() -> Self {
        RuleSet {
            own_name: true,
            keywords: vec![],
            regexes: vec![],
            ignore_own_messages: true,
            ignore_system: true,
            system_senders: vec!["EVE System".to_string()],
            ignore_senders: vec![],
            always_senders: vec![],
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reason {
    /// Every line alerts in this channel (mode `Everything`).
    AlwaysChannel(String),
    AlwaysSender(String),
    OwnName,
    Keyword(String),
    Regex(String),
}

pub struct LineCtx<'a> {
    pub pilot_name: &'a str,
    pub channel_name: &'a str,
    pub sender: &'a str,
    pub text: &'a str,
}

pub struct CompiledRules {
    own_name: bool,
    ignore_own: bool,
    ignore_system: bool,
    system_senders: Vec<String>,
    ignore_senders: Vec<String>,
    always_senders: Vec<String>,
    /// (original text, lowercased text, fires even when the channel is muted)
    keywords: Vec<(String, String, bool)>,
    /// (original text, compiled pattern, fires even when the channel is muted)
    regexes: Vec<(String, Regex, bool)>,
}

fn lower(v: &[String]) -> Vec<String> {
    v.iter().map(|s| s.trim().to_lowercase()).filter(|s| !s.is_empty()).collect()
}

/// Compiles a tracked pattern exactly as matching uses it (case-insensitive).
/// The one place that decides whether a pattern is valid: the settings
/// screen checks new patterns through it too.
pub fn compile_pattern(pattern: &str) -> Result<Regex, regex::Error> {
    Regex::new(&format!("(?i){pattern}"))
}

impl CompiledRules {
    /// Fails on the first pattern that doesn't compile.
    pub fn compile(r: &RuleSet) -> Result<CompiledRules, regex::Error> {
        let (rules, bad) = CompiledRules::compile_skipping_bad(r);
        match bad.into_iter().next() {
            Some((_, e)) => Err(e),
            None => Ok(rules),
        }
    }

    /// Compiles every pattern once, leaving out (and returning) the ones that
    /// don't compile, so one bad entry never disables the rest.
    pub fn compile_skipping_bad(r: &RuleSet) -> (CompiledRules, Vec<(String, regex::Error)>) {
        let mut regexes = vec![];
        let mut bad = vec![];
        for t in r.regexes.iter().filter(|t| !t.text.trim().is_empty()) {
            match compile_pattern(&t.text) {
                Ok(re) => regexes.push((t.text.clone(), re, t.even_when_muted)),
                Err(e) => bad.push((t.text.clone(), e)),
            }
        }
        (CompiledRules::with_regexes(r, regexes), bad)
    }

    fn with_regexes(r: &RuleSet, regexes: Vec<(String, Regex, bool)>) -> CompiledRules {
        CompiledRules {
            own_name: r.own_name,
            ignore_own: r.ignore_own_messages,
            ignore_system: r.ignore_system,
            system_senders: lower(&r.system_senders),
            ignore_senders: lower(&r.ignore_senders),
            always_senders: lower(&r.always_senders),
            keywords: r
                .keywords
                .iter()
                .filter(|k| !k.text.trim().is_empty())
                .map(|k| (k.text.trim().to_string(), k.text.trim().to_lowercase(), k.even_when_muted))
                .collect(),
            regexes,
        }
    }

    /// Lines that never alert whatever the mode: the pilot's own messages,
    /// system messages (the login MOTD) and ignored senders.
    fn ignored(&self, c: &LineCtx) -> bool {
        let sender = c.sender.trim().to_lowercase();
        (self.ignore_own && !c.pilot_name.trim().is_empty() && sender == c.pilot_name.trim().to_lowercase())
            || (self.ignore_system && self.system_senders.contains(&sender))
            || self.ignore_senders.contains(&sender)
    }

    fn mentions_own_name(&self, c: &LineCtx) -> bool {
        let name = c.pilot_name.trim().to_lowercase();
        !name.is_empty() && c.text.to_lowercase().contains(&name)
    }

    /// The independent tracking layer: "always" senders, keywords, regexes.
    /// Checked under every `Mode` by default, `Nothing` included — this is
    /// the pilot explicitly asking to hear about one thing regardless of how
    /// the channel is otherwise configured (docs/DESIGN.md, "Notification
    /// modes") — except a keyword/regex whose own `even_when_muted` is false,
    /// which a muted channel silences like everything else. `muted` is only
    /// ever true for `Mode::Nothing`; "always" senders are unaffected, since
    /// that's a different, always-on feature.
    fn tracked(&self, c: &LineCtx, muted: bool) -> Option<Reason> {
        if self.ignored(c) {
            return None;
        }
        let sender = c.sender.trim().to_lowercase();
        if self.always_senders.contains(&sender) {
            return Some(Reason::AlwaysSender(c.sender.trim().to_string()));
        }
        let text = c.text.to_lowercase();
        if let Some((k, _, _)) = self.keywords.iter().find(|(_, lk, even)| (*even || !muted) && text.contains(lk)) {
            return Some(Reason::Keyword(k.clone()));
        }
        if let Some((p, _, _)) = self.regexes.iter().find(|(_, re, even)| (*even || !muted) && re.is_match(c.text)) {
            return Some(Reason::Regex(p.clone()));
        }
        None
    }

    fn mentioned(&self, c: &LineCtx) -> Option<Reason> {
        (!self.ignored(c) && self.own_name && self.mentions_own_name(c)).then_some(Reason::OwnName)
    }

    /// Own name plus the tracking layer — everything that can fire outside of
    /// `Mode::Everything`. Kept public for callers that just want "does this
    /// line match anything", independent of a channel's mode; not muted,
    /// since there's no channel mode here to be muted. Own name is checked
    /// before keywords/regexes, so a line matching both is reported as a
    /// mention.
    pub fn evaluate(&self, c: &LineCtx) -> Option<Reason> {
        self.mentioned(c).or_else(|| self.tracked(c, false))
    }

    /// Applies a channel `Mode`. The tracking layer (`tracked`) runs under
    /// every mode; `Nothing` gets *only* that layer (muted, so a term with
    /// `even_when_muted: false` is silent too), `Mentions` adds the pilot's
    /// own name, `Everything` alerts regardless but still prefers a specific
    /// tracked reason over the generic "every line" one.
    pub fn evaluate_mode(&self, c: &LineCtx, mode: Mode) -> Option<Reason> {
        match mode {
            Mode::Nothing => self.tracked(c, true),
            Mode::Mentions => self.evaluate(c),
            Mode::Everything => self
                .tracked(c, false)
                .or(self.mentioned(c))
                .or_else(|| (!self.ignored(c)).then(|| Reason::AlwaysChannel(c.channel_name.trim().to_string()))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx<'a>(pilot: &'a str, channel: &'a str, sender: &'a str, text: &'a str) -> LineCtx<'a> {
        LineCtx { pilot_name: pilot, channel_name: channel, sender, text }
    }

    fn eval(rules: RuleSet, pilot: &str, sender: &str, text: &str) -> Option<Reason> {
        CompiledRules::compile(&rules).unwrap().evaluate(&ctx(pilot, "Local", sender, text))
    }

    #[test]
    fn own_name_matches_case_insensitively_and_ignores_own_messages() {
        let d = RuleSet::default;
        assert_eq!(eval(d(), "Holden", "Bob", "hey HOLDEN are you there"), Some(Reason::OwnName));
        assert_eq!(eval(d(), "Holden", "Bob", "nothing to see"), None);
        assert_eq!(eval(d(), "Holden", "Holden", "I said my own name Holden"), None);
        assert_eq!(eval(RuleSet { ignore_own_messages: false, ..d() }, "Holden", "Holden", "Holden"), Some(Reason::OwnName));
    }

    #[test]
    fn login_motd_from_eve_system_never_alerts() {
        let text = "Channel MOTD: contacts: Holden, Naomi";
        assert_eq!(eval(RuleSet::default(), "Holden", "EVE System", text), None);
        assert_eq!(eval(RuleSet { ignore_system: false, ..RuleSet::default() }, "Holden", "EVE System", text), Some(Reason::OwnName));
    }

    #[test]
    fn keywords_and_regexes() {
        let r = RuleSet { keywords: vec!["Jita".into(), " ".into()], regexes: vec![r"\bgank(ed|ing)?\b".into()], ..RuleSet::default() };
        assert_eq!(eval(r.clone(), "Holden", "X", "going to jita 4-4"), Some(Reason::Keyword("Jita".into())));
        assert_eq!(eval(r.clone(), "Holden", "X", "we got ganked"), Some(Reason::Regex(r"\bgank(ed|ing)?\b".into())));
        assert_eq!(eval(r, "Holden", "X", "gankster"), None);
    }

    #[test]
    fn ignored_senders_beat_always_senders_beat_content() {
        let r = RuleSet { ignore_senders: vec!["Spammer".into()], always_senders: vec!["Boss".into()], ..RuleSet::default() };
        assert_eq!(eval(r.clone(), "Holden", "Spammer", "hello Holden"), None);
        assert_eq!(eval(r.clone(), "Holden", "Boss", "hi"), Some(Reason::AlwaysSender("Boss".into())));
        assert_eq!(eval(r, "Holden", "Bob", "Holden?"), Some(Reason::OwnName));
    }

    #[test]
    fn bad_regex_is_reported_not_swallowed() {
        assert!(CompiledRules::compile(&RuleSet { regexes: vec!["(".into()], ..RuleSet::default() }).is_err());
    }

    #[test]
    fn rule_sets_deserialize_with_missing_fields() {
        let r: RuleSet = serde_json::from_str(r#"{"keywords":[{"text":"a"}]}"#).unwrap();
        assert!(r.own_name && r.ignore_system);
        assert_eq!(r.keywords, vec![TrackedTerm { text: "a".into(), even_when_muted: true }], "a missing even_when_muted defaults to true");
    }

    fn mode(rules: RuleSet, m: Mode, sender: &str, text: &str) -> Option<Reason> {
        CompiledRules::compile(&rules).unwrap().evaluate_mode(&ctx("Holden", "Fleet", sender, text), m)
    }

    #[test]
    fn tracked_keywords_fire_under_every_mode_including_nothing() {
        // A pilot mutes a busy channel but still wants to hear about "jita" —
        // muting general chatter must not silence what was explicitly tracked
        // (owner correction 2026-09-27: tracking is independent of the mode).
        let r = || RuleSet { keywords: vec!["jita".into()], ..RuleSet::default() };
        assert_eq!(mode(r(), Mode::Nothing, "Bob", "Holden!"), None, "a plain mention still gets nothing");
        assert_eq!(mode(r(), Mode::Nothing, "Bob", "selling in jita"), Some(Reason::Keyword("jita".into())));
        assert_eq!(mode(r(), Mode::Mentions, "Bob", "hello there"), None);
        assert_eq!(mode(r(), Mode::Mentions, "Bob", "Holden?"), Some(Reason::OwnName));
        assert_eq!(mode(r(), Mode::Mentions, "Bob", "selling in jita"), Some(Reason::Keyword("jita".into())));
        assert_eq!(mode(r(), Mode::Everything, "Bob", "hello"), Some(Reason::AlwaysChannel("Fleet".into())), "no tracked reason applies, so the generic one does");
        assert_eq!(mode(r(), Mode::Everything, "Bob", "selling in jita"), Some(Reason::Keyword("jita".into())), "a tracked reason is more specific than the generic one");
    }

    #[test]
    fn even_when_muted_false_opts_a_term_out_of_the_default_nothing_override() {
        // Owner request 2026-09-27: not every tracked term should have to
        // override a muted channel — a per-term toggle, default on (matching
        // the original, only behavior above).
        let r = || RuleSet {
            keywords: vec![TrackedTerm { text: "jita".into(), even_when_muted: false }],
            regexes: vec![TrackedTerm { text: r"\bgank\b".into(), even_when_muted: false }],
            ..RuleSet::default()
        };
        assert_eq!(mode(r(), Mode::Nothing, "Bob", "selling in jita"), None, "muted, and this term doesn't override it");
        assert_eq!(mode(r(), Mode::Nothing, "Bob", "we got gank"), None);
        // The same channel un-muted still tracks it normally.
        assert_eq!(mode(r(), Mode::Mentions, "Bob", "selling in jita"), Some(Reason::Keyword("jita".into())));
        assert_eq!(mode(r(), Mode::Everything, "Bob", "we got gank"), Some(Reason::Regex(r"\bgank\b".into())));
    }

    #[test]
    fn everything_still_skips_own_system_and_ignored_lines() {
        let r = || RuleSet { ignore_senders: vec!["Spammer".into()], ..RuleSet::default() };
        assert_eq!(mode(r(), Mode::Everything, "Holden", "my own line"), None);
        assert_eq!(mode(r(), Mode::Everything, "EVE System", "MOTD"), None);
        assert_eq!(mode(r(), Mode::Everything, "Spammer", "buy stuff"), None);
        assert_eq!(mode(r(), Mode::Mentions, "Spammer", "Holden"), None);
    }
}
