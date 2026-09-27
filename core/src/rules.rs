//! Deciding whether a chat line deserves an alert for one pilot.

use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// User-editable rule settings. Matching is case-insensitive.
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
    pub ignore_channels: Vec<String>,
    pub ignore_senders: Vec<String>,
    /// Alert on every line in these channels / from these senders.
    pub always_channels: Vec<String>,
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
            ignore_channels: vec![],
            ignore_senders: vec![],
            always_channels: vec![],
            always_senders: vec![],
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reason {
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
    ignore_channels: Vec<String>,
    ignore_senders: Vec<String>,
    always_channels: Vec<String>,
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
            ignore_channels: lower(&r.ignore_channels),
            ignore_senders: lower(&r.ignore_senders),
            always_channels: lower(&r.always_channels),
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

    /// `None` means no alert. Ignores win over everything, then "always"
    /// rules, then the content rules.
    pub fn evaluate(&self, c: &LineCtx) -> Option<Reason> {
        let channel = c.channel_name.trim().to_lowercase();
        let sender = c.sender.trim().to_lowercase();
        if self.ignore_channels.contains(&channel) {
            return None;
        }
        if self.ignore_own && !c.pilot_name.is_empty() && sender == c.pilot_name.trim().to_lowercase() {
            return None;
        }
        if self.ignore_system && self.system_senders.contains(&sender) {
            return None;
        }
        if self.ignore_senders.contains(&sender) {
            return None;
        }
        if self.always_senders.contains(&sender) {
            return Some(Reason::AlwaysSender(c.sender.trim().to_string()));
        }
        if self.always_channels.contains(&channel) {
            return Some(Reason::AlwaysChannel(c.channel_name.trim().to_string()));
        }
        let text = c.text.to_lowercase();
        if self.own_name && !c.pilot_name.trim().is_empty() && text.contains(&c.pilot_name.trim().to_lowercase()) {
            return Some(Reason::OwnName);
        }
        if let Some((k, _)) = self.keywords.iter().find(|(_, lk)| text.contains(lk)) {
            return Some(Reason::Keyword(k.clone()));
        }
        if let Some((p, _)) = self.regexes.iter().find(|(_, re)| re.is_match(c.text)) {
            return Some(Reason::Regex(p.clone()));
        }
        None
    }
}

/// A default rule set plus per-pilot overrides (keyed by character id).
pub struct RuleBook {
    default: CompiledRules,
    per_pilot: HashMap<String, CompiledRules>,
}

impl RuleBook {
    pub fn new(default: &RuleSet) -> Result<RuleBook, regex::Error> {
        Ok(RuleBook { default: CompiledRules::compile(default)?, per_pilot: HashMap::new() })
    }

    pub fn set_pilot(&mut self, pilot_id: &str, rules: &RuleSet) -> Result<(), regex::Error> {
        self.per_pilot.insert(pilot_id.to_string(), CompiledRules::compile(rules)?);
        Ok(())
    }

    pub fn for_pilot(&self, pilot_id: Option<&str>) -> &CompiledRules {
        pilot_id.and_then(|id| self.per_pilot.get(id)).unwrap_or(&self.default)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn eval(rules: RuleSet, pilot: &str, channel: &str, sender: &str, text: &str) -> Option<Reason> {
        CompiledRules::compile(&rules).unwrap().evaluate(&LineCtx { pilot_name: pilot, channel_name: channel, sender, text })
    }

    #[test]
    fn own_name_matches_case_insensitively_and_ignores_own_messages() {
        let d = RuleSet::default;
        assert_eq!(eval(d(), "Jarna", "Local", "Bob", "hey JARNA are you there"), Some(Reason::OwnName));
        assert_eq!(eval(d(), "Jarna", "Local", "Bob", "nothing to see"), None);
        assert_eq!(eval(d(), "Jarna", "Local", "Jarna", "I said my own name Jarna"), None);
        assert_eq!(eval(RuleSet { ignore_own_messages: false, ..d() }, "Jarna", "Local", "Jarna", "Jarna"), Some(Reason::OwnName));
    }

    #[test]
    fn login_motd_from_eve_system_never_alerts() {
        let text = "Channel MOTD: contacts: Jarna, Psianna";
        assert_eq!(eval(RuleSet::default(), "Jarna", "Corp", "EVE System", text), None);
        assert_eq!(eval(RuleSet { ignore_system: false, ..RuleSet::default() }, "Jarna", "Corp", "EVE System", text), Some(Reason::OwnName));
    }

    #[test]
    fn keywords_and_regexes() {
        let r = RuleSet { keywords: vec!["Jita".into(), " ".into()], regexes: vec![r"\bgank(ed|ing)?\b".into()], ..RuleSet::default() };
        assert_eq!(eval(r.clone(), "Jarna", "Local", "X", "going to jita 4-4"), Some(Reason::Keyword("Jita".into())));
        assert_eq!(eval(r.clone(), "Jarna", "Local", "X", "we got ganked"), Some(Reason::Regex(r"\bgank(ed|ing)?\b".into())));
        assert_eq!(eval(r, "Jarna", "Local", "X", "gankster"), None);
    }

    #[test]
    fn ignore_beats_always_beats_content() {
        let r = RuleSet {
            always_channels: vec!["Fleet".into()],
            ignore_senders: vec!["Spammer".into()],
            always_senders: vec!["Boss".into()],
            ignore_channels: vec!["Sales".into()],
            ..RuleSet::default()
        };
        assert_eq!(eval(r.clone(), "Jarna", "Fleet", "Anyone", "hello"), Some(Reason::AlwaysChannel("Fleet".into())));
        assert_eq!(eval(r.clone(), "Jarna", "Fleet", "Spammer", "hello Jarna"), None);
        assert_eq!(eval(r.clone(), "Jarna", "Local", "Boss", "hi"), Some(Reason::AlwaysSender("Boss".into())));
        assert_eq!(eval(r, "Jarna", "Sales", "Boss", "Jarna"), None);
    }

    #[test]
    fn bad_regex_is_reported_not_swallowed() {
        assert!(CompiledRules::compile(&RuleSet { regexes: vec!["(".into()], ..RuleSet::default() }).is_err());
    }

    #[test]
    fn per_pilot_overrides_fall_back_to_the_default() {
        let mut book = RuleBook::new(&RuleSet::default()).unwrap();
        book.set_pilot("2", &RuleSet { own_name: false, keywords: vec!["fleet".into()], ..RuleSet::default() }).unwrap();
        let ctx = LineCtx { pilot_name: "Psianna", channel_name: "Local", sender: "X", text: "Psianna fleet up" };
        assert_eq!(book.for_pilot(Some("2")).evaluate(&ctx), Some(Reason::Keyword("fleet".into())));
        assert_eq!(book.for_pilot(Some("1")).evaluate(&ctx), Some(Reason::OwnName));
        assert_eq!(book.for_pilot(None).evaluate(&ctx), Some(Reason::OwnName));
    }

    #[test]
    fn rule_sets_deserialize_with_missing_fields() {
        let r: RuleSet = serde_json::from_str(r#"{"keywords":["a"]}"#).unwrap();
        assert!(r.own_name && r.ignore_system);
        assert_eq!(r.keywords, ["a"]);
    }
}
