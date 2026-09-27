//! Deciding whether a chat line deserves an alert for one pilot.
//!
//! Per-channel behavior (mute, mentions only, everything) is a `Mode` chosen by
//! the settings layers, not a list here; this module holds the content rules
//! and applies a mode to them.

use crate::prefs::Mode;
use regex::Regex;
use serde::{Deserialize, Serialize};

/// The content rules, after the settings layers have been merged. Matching is case-insensitive.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct RuleSet {
    /// Alert when the listening pilot's own name appears in the text.
    pub own_name: bool,
    pub keywords: Vec<String>,
    /// Advanced: regular expressions matched against the text.
    pub regexes: Vec<String>,
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
    keywords: Vec<(String, String)>,
    regexes: Vec<(String, Regex)>,
}

fn lower(v: &[String]) -> Vec<String> {
    v.iter().map(|s| s.trim().to_lowercase()).filter(|s| !s.is_empty()).collect()
}

impl CompiledRules {
    pub fn compile(r: &RuleSet) -> Result<CompiledRules, regex::Error> {
        let regexes = r
            .regexes
            .iter()
            .filter(|p| !p.trim().is_empty())
            .map(|p| Regex::new(&format!("(?i){p}")).map(|re| (p.clone(), re)))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(CompiledRules {
            own_name: r.own_name,
            ignore_own: r.ignore_own_messages,
            ignore_system: r.ignore_system,
            system_senders: lower(&r.system_senders),
            ignore_senders: lower(&r.ignore_senders),
            always_senders: lower(&r.always_senders),
            keywords: r
                .keywords
                .iter()
                .filter(|k| !k.trim().is_empty())
                .map(|k| (k.trim().to_string(), k.trim().to_lowercase()))
                .collect(),
            regexes,
        })
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
    /// Checked under every `Mode`, `Nothing` included — this is the pilot
    /// explicitly asking to hear about one thing regardless of how the
    /// channel is otherwise configured (docs/DESIGN.md, "Notification modes").
    fn tracked(&self, c: &LineCtx) -> Option<Reason> {
        if self.ignored(c) {
            return None;
        }
        let sender = c.sender.trim().to_lowercase();
        if self.always_senders.contains(&sender) {
            return Some(Reason::AlwaysSender(c.sender.trim().to_string()));
        }
        let text = c.text.to_lowercase();
        if let Some((k, _)) = self.keywords.iter().find(|(_, lk)| text.contains(lk)) {
            return Some(Reason::Keyword(k.clone()));
        }
        if let Some((p, _)) = self.regexes.iter().find(|(_, re)| re.is_match(c.text)) {
            return Some(Reason::Regex(p.clone()));
        }
        None
    }

    fn mentioned(&self, c: &LineCtx) -> Option<Reason> {
        (!self.ignored(c) && self.own_name && self.mentions_own_name(c)).then_some(Reason::OwnName)
    }

    /// Own name plus the tracking layer — everything that can fire outside of
    /// `Mode::Everything`. Kept public for callers that just want "does this
    /// line match anything", independent of a channel's mode. Own name is
    /// checked before keywords/regexes, so a line matching both is reported
    /// as a mention.
    pub fn evaluate(&self, c: &LineCtx) -> Option<Reason> {
        self.mentioned(c).or_else(|| self.tracked(c))
    }

    /// Applies a channel `Mode`. The tracking layer (`tracked`) runs under
    /// every mode; `Nothing` gets *only* that layer, `Mentions` adds the
    /// pilot's own name, `Everything` alerts regardless but still prefers a
    /// specific tracked reason over the generic "every line" one.
    pub fn evaluate_mode(&self, c: &LineCtx, mode: Mode) -> Option<Reason> {
        match mode {
            Mode::Nothing => self.tracked(c),
            Mode::Mentions => self.evaluate(c),
            Mode::Everything => self
                .tracked(c)
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
        assert_eq!(eval(d(), "Jarna", "Bob", "hey JARNA are you there"), Some(Reason::OwnName));
        assert_eq!(eval(d(), "Jarna", "Bob", "nothing to see"), None);
        assert_eq!(eval(d(), "Jarna", "Jarna", "I said my own name Jarna"), None);
        assert_eq!(eval(RuleSet { ignore_own_messages: false, ..d() }, "Jarna", "Jarna", "Jarna"), Some(Reason::OwnName));
    }

    #[test]
    fn login_motd_from_eve_system_never_alerts() {
        let text = "Channel MOTD: contacts: Jarna, Psianna";
        assert_eq!(eval(RuleSet::default(), "Jarna", "EVE System", text), None);
        assert_eq!(eval(RuleSet { ignore_system: false, ..RuleSet::default() }, "Jarna", "EVE System", text), Some(Reason::OwnName));
    }

    #[test]
    fn keywords_and_regexes() {
        let r = RuleSet { keywords: vec!["Jita".into(), " ".into()], regexes: vec![r"\bgank(ed|ing)?\b".into()], ..RuleSet::default() };
        assert_eq!(eval(r.clone(), "Jarna", "X", "going to jita 4-4"), Some(Reason::Keyword("Jita".into())));
        assert_eq!(eval(r.clone(), "Jarna", "X", "we got ganked"), Some(Reason::Regex(r"\bgank(ed|ing)?\b".into())));
        assert_eq!(eval(r, "Jarna", "X", "gankster"), None);
    }

    #[test]
    fn ignored_senders_beat_always_senders_beat_content() {
        let r = RuleSet { ignore_senders: vec!["Spammer".into()], always_senders: vec!["Boss".into()], ..RuleSet::default() };
        assert_eq!(eval(r.clone(), "Jarna", "Spammer", "hello Jarna"), None);
        assert_eq!(eval(r.clone(), "Jarna", "Boss", "hi"), Some(Reason::AlwaysSender("Boss".into())));
        assert_eq!(eval(r, "Jarna", "Bob", "Jarna?"), Some(Reason::OwnName));
    }

    #[test]
    fn bad_regex_is_reported_not_swallowed() {
        assert!(CompiledRules::compile(&RuleSet { regexes: vec!["(".into()], ..RuleSet::default() }).is_err());
    }

    #[test]
    fn rule_sets_deserialize_with_missing_fields() {
        let r: RuleSet = serde_json::from_str(r#"{"keywords":["a"]}"#).unwrap();
        assert!(r.own_name && r.ignore_system);
        assert_eq!(r.keywords, ["a"]);
    }

    fn mode(rules: RuleSet, m: Mode, sender: &str, text: &str) -> Option<Reason> {
        CompiledRules::compile(&rules).unwrap().evaluate_mode(&ctx("Jarna", "Fleet", sender, text), m)
    }

    #[test]
    fn tracked_keywords_fire_under_every_mode_including_nothing() {
        // A pilot mutes a busy channel but still wants to hear about "jita" —
        // muting general chatter must not silence what was explicitly tracked
        // (owner correction 2026-09-27: tracking is independent of the mode).
        let r = || RuleSet { keywords: vec!["jita".into()], ..RuleSet::default() };
        assert_eq!(mode(r(), Mode::Nothing, "Bob", "Jarna!"), None, "a plain mention still gets nothing");
        assert_eq!(mode(r(), Mode::Nothing, "Bob", "selling in jita"), Some(Reason::Keyword("jita".into())));
        assert_eq!(mode(r(), Mode::Mentions, "Bob", "hello there"), None);
        assert_eq!(mode(r(), Mode::Mentions, "Bob", "Jarna?"), Some(Reason::OwnName));
        assert_eq!(mode(r(), Mode::Mentions, "Bob", "selling in jita"), Some(Reason::Keyword("jita".into())));
        assert_eq!(mode(r(), Mode::Everything, "Bob", "hello"), Some(Reason::AlwaysChannel("Fleet".into())), "no tracked reason applies, so the generic one does");
        assert_eq!(mode(r(), Mode::Everything, "Bob", "selling in jita"), Some(Reason::Keyword("jita".into())), "a tracked reason is more specific than the generic one");
    }

    #[test]
    fn everything_still_skips_own_system_and_ignored_lines() {
        let r = || RuleSet { ignore_senders: vec!["Spammer".into()], ..RuleSet::default() };
        assert_eq!(mode(r(), Mode::Everything, "Jarna", "my own line"), None);
        assert_eq!(mode(r(), Mode::Everything, "EVE System", "MOTD"), None);
        assert_eq!(mode(r(), Mode::Everything, "Spammer", "buy stuff"), None);
        assert_eq!(mode(r(), Mode::Mentions, "Spammer", "Jarna"), None);
    }
}
