//! Layered settings.
//!
//! Every setting is optional at each level and inherits from the one above.
//! From least to most specific:
//!
//! ```text
//! global -> channel kind -> one channel (by id) -> pilot -> pilot + kind -> pilot + channel
//! ```
//!
//! A pilot's own change overrides the defaults for a kind, because a change
//! made for one character is deliberate. Preferences (mode, rules, delivery,
//! style, sound) take the most specific value that is set; a list set at a
//! level replaces the inherited list. Limits (rate caps) are different:
//! every level's cap applies, so a channel cannot lift a pilot's ceiling.
//!
//! Fleet and private channel ids change every time, so a channel-by-id layer
//! is only consulted for kinds with stable ids.

use crate::channel::ChannelKind;
use crate::prefs::{DeliveryMode, LayerKey, Mode, OverCap, OverlayStyle, Prefs, RateCap, Suppression};
use crate::rules::{CompiledRules, RuleSet};
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::io;
use std::path::Path;
use std::sync::Arc;

/// One level of overrides. `None` means "inherit".
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Layer {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mode: Option<Mode>,

    // Content rules (a list set here replaces the inherited list).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub own_name: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub keywords: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub regexes: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ignore_own_messages: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ignore_system: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub system_senders: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ignore_senders: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub always_senders: Option<Vec<String>>,

    // How to show it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub delivery: Option<DeliveryMode>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub suppression: Option<Suppression>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub style: Option<OverlayStyle>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sound: Option<bool>,

    // Limits: applied at every level, never overridden.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rate_cap: Option<RateCap>,
}

/// Everything configured for one pilot (a character id).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PilotSettings {
    pub base: Layer,
    pub kinds: BTreeMap<ChannelKind, Layer>,
    /// By channel id; only for kinds with stable ids.
    pub channels: BTreeMap<String, Layer>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub global: Layer,
    pub kinds: BTreeMap<ChannelKind, Layer>,
    /// By channel id; only for kinds with stable ids (a public channel, Local, Corp, Alliance).
    pub channels: BTreeMap<String, Layer>,
    /// Keyed by character id.
    pub pilots: BTreeMap<String, PilotSettings>,
}

/// The fully resolved behavior for one (pilot, channel).
pub struct Resolved {
    pub mode: Mode,
    pub rules: CompiledRules,
    pub prefs: Prefs,
    /// Regexes that failed to compile and were skipped, so the UI can say so.
    pub bad_regexes: Vec<String>,
}

impl Settings {
    /// Defaults by channel kind. These are proposals for the owner to tune.
    /// Private messages, Fleet and Corp are addressed to, or shared with, a
    /// small circle on purpose, so every line is worth seeing without needing
    /// a keyword; Fleet and Corp can still get busy, so they carry a cap.
    /// Local, Alliance and public channels are full of strangers, so they stay
    /// mention/keyword only, and are capped and fold into the count badge.
    pub fn with_defaults() -> Settings {
        let cap = |n: u32| Layer { rate_cap: Some(RateCap { per_minute: n, over: OverCap::Fold }), ..Layer::default() };
        let everything = |cap_per_min: Option<u32>, style: Option<OverlayStyle>| Layer {
            mode: Some(Mode::Everything),
            style,
            rate_cap: cap_per_min.map(|n| RateCap { per_minute: n, over: OverCap::Fold }),
            ..Layer::default()
        };
        let mut s = Settings::default();
        s.kinds.insert(ChannelKind::Local, cap(6));
        s.kinds.insert(ChannelKind::Public, cap(4));
        s.kinds.insert(ChannelKind::Unknown, cap(4));
        s.kinds.insert(ChannelKind::Private, everything(None, Some(OverlayStyle::Beacon)));
        s.kinds.insert(ChannelKind::Fleet, everything(Some(6), Some(OverlayStyle::Panel)));
        // Corp and Alliance are the same relationship at a different scope
        // (owner decision 2026-09-27: any future change to Corp's defaults
        // applies to Alliance too), so they always get identical values.
        for org in [ChannelKind::Corp, ChannelKind::Alliance] {
            s.kinds.insert(org, everything(Some(6), Some(OverlayStyle::Panel)));
        }
        s
    }

    /// The applicable layers, least specific first.
    fn layers<'a>(&'a self, pilot_id: Option<&str>, kind: ChannelKind, channel_id: &str) -> Vec<(LayerKey, &'a Layer)> {
        let by_id = kind.has_stable_id() && !channel_id.is_empty();
        let mut v = vec![(LayerKey::Global, &self.global)];
        if let Some(l) = self.kinds.get(&kind) {
            v.push((LayerKey::Kind(kind), l));
        }
        if by_id {
            if let Some(l) = self.channels.get(channel_id) {
                v.push((LayerKey::Channel(channel_id.to_string()), l));
            }
        }
        if let Some(p) = pilot_id.and_then(|id| self.pilots.get(id)) {
            v.push((LayerKey::Pilot, &p.base));
            if let Some(l) = p.kinds.get(&kind) {
                v.push((LayerKey::PilotKind(kind), l));
            }
            if by_id {
                if let Some(l) = p.channels.get(channel_id) {
                    v.push((LayerKey::PilotChannel(channel_id.to_string()), l));
                }
            }
        }
        v
    }

    pub fn resolve(&self, pilot_id: Option<&str>, kind: ChannelKind, channel_id: &str) -> Resolved {
        let mut mode = Mode::Mentions;
        let mut rules = RuleSet::default();
        let mut prefs = Prefs::default();
        for (key, l) in self.layers(pilot_id, kind, channel_id) {
            if let Some(v) = l.mode {
                mode = v;
            }
            if let Some(v) = l.own_name {
                rules.own_name = v;
            }
            if let Some(v) = &l.keywords {
                rules.keywords = v.clone();
            }
            if let Some(v) = &l.regexes {
                rules.regexes = v.clone();
            }
            if let Some(v) = l.ignore_own_messages {
                rules.ignore_own_messages = v;
            }
            if let Some(v) = l.ignore_system {
                rules.ignore_system = v;
            }
            if let Some(v) = &l.system_senders {
                rules.system_senders = v.clone();
            }
            if let Some(v) = &l.ignore_senders {
                rules.ignore_senders = v.clone();
            }
            if let Some(v) = &l.always_senders {
                rules.always_senders = v.clone();
            }
            if let Some(v) = l.delivery {
                prefs.delivery = v;
            }
            if let Some(v) = l.suppression {
                prefs.suppression = v;
            }
            if let Some(v) = l.style {
                prefs.style = Some(v);
            }
            if let Some(v) = l.sound {
                prefs.sound = v;
            }
            if let Some(cap) = l.rate_cap {
                prefs.caps.push((key, cap)); // every level's cap applies
            }
        }
        let (good, bad_regexes): (Vec<String>, Vec<String>) =
            rules.regexes.iter().cloned().partition(|p| p.trim().is_empty() || Regex::new(&format!("(?i){p}")).is_ok());
        rules.regexes = good;
        let rules = CompiledRules::compile(&rules).unwrap_or_else(|_| CompiledRules::compile(&RuleSet::default()).unwrap());
        Resolved { mode, rules, prefs, bad_regexes }
    }

    /// Every regex in every layer that does not compile, with where it is.
    pub fn invalid_regexes(&self) -> Vec<String> {
        let mut all: Vec<&Layer> = vec![&self.global];
        all.extend(self.kinds.values().chain(self.channels.values()));
        for p in self.pilots.values() {
            all.push(&p.base);
            all.extend(p.kinds.values().chain(p.channels.values()));
        }
        all.into_iter()
            .flat_map(|l| l.regexes.iter().flatten())
            .filter(|p| !p.trim().is_empty() && Regex::new(&format!("(?i){p}")).is_err())
            .cloned()
            .collect()
    }

    /// A missing file gives the built-in defaults (first run).
    pub fn load(path: &Path) -> io::Result<Settings> {
        match std::fs::read_to_string(path) {
            Ok(s) => serde_json::from_str(&s).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e)),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(Settings::with_defaults()),
            Err(e) => Err(e),
        }
    }

    /// Writes to a temporary file and renames it over the target.
    pub fn save(&self, path: &Path) -> io::Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_vec_pretty(self).map_err(io::Error::other)?)?;
        std::fs::rename(&tmp, path)
    }
}

type CacheKey = (Option<String>, ChannelKind, String);

/// Settings plus a cache of resolved behavior, because every chat line is
/// checked against its (pilot, channel) and resolving compiles regexes.
pub struct SettingsBook {
    settings: Settings,
    cache: HashMap<CacheKey, Arc<Resolved>>,
}

impl SettingsBook {
    pub fn new(settings: Settings) -> SettingsBook {
        SettingsBook { settings, cache: HashMap::new() }
    }

    pub fn settings(&self) -> &Settings {
        &self.settings
    }

    /// Edits the settings and drops the cache.
    pub fn edit<R>(&mut self, f: impl FnOnce(&mut Settings) -> R) -> R {
        let r = f(&mut self.settings);
        self.cache.clear();
        r
    }

    pub fn resolved(&mut self, pilot_id: Option<&str>, kind: ChannelKind, channel_id: &str) -> Arc<Resolved> {
        let key = (pilot_id.map(str::to_string), kind, channel_id.to_string());
        if let Some(r) = self.cache.get(&key) {
            return r.clone();
        }
        let r = Arc::new(self.settings.resolve(pilot_id, kind, channel_id));
        self.cache.insert(key, r.clone());
        r
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rules::{LineCtx, Reason};

    fn kw(words: &[&str]) -> Option<Vec<String>> {
        Some(words.iter().map(|w| w.to_string()).collect())
    }

    /// Which keyword (if any) fires for this text in the resolved behavior.
    fn hit(r: &Resolved, text: &str) -> Option<String> {
        match r.rules.evaluate(&LineCtx { pilot_name: "Jarna", channel_name: "X", sender: "Bob", text }) {
            Some(Reason::Keyword(k)) => Some(k),
            _ => None,
        }
    }

    #[test]
    fn a_pilots_own_setting_beats_the_kind_default() {
        let mut s = Settings::default();
        s.global.keywords = kw(&["g"]);
        s.kinds.insert(ChannelKind::Corp, Layer { keywords: kw(&["k"]), ..Layer::default() });
        assert_eq!(hit(&s.resolve(None, ChannelKind::Corp, "corp"), "k g p pk pc"), Some("k".into()), "kind beats global");

        s.pilots.entry("1".into()).or_default().base.keywords = kw(&["p"]);
        assert_eq!(hit(&s.resolve(Some("1"), ChannelKind::Corp, "corp"), "k g p pk pc"), Some("p".into()), "pilot beats kind");

        s.pilots.get_mut("1").unwrap().kinds.insert(ChannelKind::Corp, Layer { keywords: kw(&["pk"]), ..Layer::default() });
        assert_eq!(hit(&s.resolve(Some("1"), ChannelKind::Corp, "corp"), "k g p pk pc"), Some("pk".into()), "pilot+kind beats pilot");

        s.pilots.get_mut("1").unwrap().channels.insert("corp".into(), Layer { keywords: kw(&["pc"]), ..Layer::default() });
        assert_eq!(hit(&s.resolve(Some("1"), ChannelKind::Corp, "corp"), "k g p pk pc"), Some("pc".into()), "pilot+channel is most specific");

        // Another pilot is unaffected.
        assert_eq!(hit(&s.resolve(Some("2"), ChannelKind::Corp, "corp"), "k g p pk pc"), Some("k".into()));
    }

    #[test]
    fn unset_fields_inherit_while_set_ones_override() {
        let mut s = Settings::with_defaults();
        // The pilot only turns sound on; everything else still comes from the Private default.
        s.pilots.entry("1".into()).or_default().base.sound = Some(true);
        let r = s.resolve(Some("1"), ChannelKind::Private, "private_abc");
        assert_eq!(r.mode, Mode::Everything);
        assert_eq!(r.prefs.style, Some(OverlayStyle::Beacon));
        assert!(r.prefs.sound);
        // And the pilot can override the kind default deliberately.
        s.pilots.get_mut("1").unwrap().kinds.insert(ChannelKind::Private, Layer { mode: Some(Mode::Mentions), ..Layer::default() });
        assert_eq!(s.resolve(Some("1"), ChannelKind::Private, "private_abc").mode, Mode::Mentions);
        assert_eq!(s.resolve(Some("2"), ChannelKind::Private, "private_abc").mode, Mode::Everything);
    }

    #[test]
    fn built_in_defaults_by_kind() {
        let s = Settings::with_defaults();
        let priv_ = s.resolve(None, ChannelKind::Private, "private_x");
        assert_eq!((priv_.mode, priv_.prefs.style), (Mode::Everything, Some(OverlayStyle::Beacon)));
        assert!(priv_.prefs.caps.is_empty(), "a private conversation is not capped");

        // Fleet, Corp and Alliance: every line as a Panel (visible, but not as
        // loud as a mention), capped since they can still get busy. Corp and
        // Alliance are deliberately identical (same relationship, different scope).
        for kind in [ChannelKind::Fleet, ChannelKind::Corp, ChannelKind::Alliance] {
            let r = s.resolve(None, kind, "irrelevant-for-these-kinds");
            assert_eq!(r.mode, Mode::Everything, "{kind:?}");
            assert_eq!(r.prefs.style, Some(OverlayStyle::Panel), "{kind:?}");
            assert_eq!(r.prefs.caps, vec![(LayerKey::Kind(kind), RateCap { per_minute: 6, over: OverCap::Fold })], "{kind:?}");
        }
        // Local/public: mention or keyword only, still capped.
        assert_eq!(s.resolve(None, ChannelKind::Local, "local").mode, Mode::Mentions);
        let local = s.resolve(None, ChannelKind::Local, "local");
        assert_eq!(local.prefs.caps, vec![(LayerKey::Kind(ChannelKind::Local), RateCap { per_minute: 6, over: OverCap::Fold })]);
        assert_eq!(s.resolve(None, ChannelKind::Public, "system_1_2").prefs.caps[0].1.per_minute, 4);
    }

    #[test]
    fn a_public_channel_can_be_configured_by_id_but_a_fleet_cannot() {
        let mut s = Settings::default();
        s.channels.insert("system_263238_263361".into(), Layer { mode: Some(Mode::Nothing), ..Layer::default() });
        s.channels.insert("fleet_1368512310460".into(), Layer { mode: Some(Mode::Nothing), ..Layer::default() });
        assert_eq!(s.resolve(None, ChannelKind::Public, "system_263238_263361").mode, Mode::Nothing);
        assert_eq!(s.resolve(None, ChannelKind::Public, "system_9_9").mode, Mode::Mentions, "another public channel is untouched");
        // Fleet ids change every fleet, so a per-id layer never applies.
        assert_eq!(s.resolve(None, ChannelKind::Fleet, "fleet_1368512310460").mode, Mode::Mentions);
    }

    #[test]
    fn caps_from_every_level_apply_and_cannot_be_lifted() {
        let cap = |n| Some(RateCap { per_minute: n, over: OverCap::Drop });
        let mut s = Settings::default();
        s.global.rate_cap = cap(20);
        s.kinds.insert(ChannelKind::Local, Layer { rate_cap: cap(6), ..Layer::default() });
        let p = s.pilots.entry("1".into()).or_default();
        p.base.rate_cap = cap(3);
        // A pilot+kind layer that sets no cap does not remove the others.
        p.kinds.insert(ChannelKind::Local, Layer { sound: Some(true), ..Layer::default() });
        let caps = s.resolve(Some("1"), ChannelKind::Local, "local").prefs.caps;
        let keys: Vec<_> = caps.iter().map(|(k, c)| (k.clone(), c.per_minute)).collect();
        assert_eq!(keys, vec![(LayerKey::Global, 20), (LayerKey::Kind(ChannelKind::Local), 6), (LayerKey::Pilot, 3)]);
    }

    #[test]
    fn a_bad_regex_is_skipped_and_reported() {
        let mut s = Settings::default();
        s.global.regexes = Some(vec!["(".into(), r"\bgank\b".into()]);
        let r = s.resolve(None, ChannelKind::Local, "local");
        assert_eq!(r.bad_regexes, ["("]);
        assert!(matches!(
            r.rules.evaluate(&LineCtx { pilot_name: "J", channel_name: "L", sender: "B", text: "a gank here" }),
            Some(Reason::Regex(_))
        ));
        assert_eq!(s.invalid_regexes(), ["("]);
    }

    #[test]
    fn json_round_trips_and_accepts_partial_files() {
        let mut s = Settings::with_defaults();
        s.pilots.entry("1216741999".into()).or_default().base.sound = Some(true);
        let text = serde_json::to_string_pretty(&s).unwrap();
        assert!(!text.contains("null"), "unset fields are omitted: {text}");
        assert_eq!(serde_json::from_str::<Settings>(&text).unwrap(), s);

        let partial: Settings = serde_json::from_str(r#"{"pilots":{"7":{"kinds":{"private":{"mode":"nothing"}}}}}"#).unwrap();
        assert_eq!(partial.resolve(Some("7"), ChannelKind::Private, "private_x").mode, Mode::Nothing);
    }

    #[test]
    fn save_and_load_and_a_missing_file_means_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cfg").join("settings.json");
        assert_eq!(Settings::load(&path).unwrap(), Settings::with_defaults());
        let mut s = Settings::with_defaults();
        s.global.keywords = kw(&["jita"]);
        s.save(&path).unwrap();
        assert_eq!(Settings::load(&path).unwrap(), s);
        std::fs::write(&path, "nope").unwrap();
        assert!(Settings::load(&path).is_err());
    }

    #[test]
    fn the_book_caches_and_edits_invalidate() {
        let mut b = SettingsBook::new(Settings::with_defaults());
        let a = b.resolved(Some("1"), ChannelKind::Corp, "corp");
        let again = b.resolved(Some("1"), ChannelKind::Corp, "corp");
        assert!(Arc::ptr_eq(&a, &again), "second lookup is served from the cache");
        assert_eq!(a.mode, Mode::Everything, "Corp defaults to everything");
        b.edit(|s| s.kinds.entry(ChannelKind::Corp).or_default().mode = Some(Mode::Nothing));
        let after = b.resolved(Some("1"), ChannelKind::Corp, "corp");
        assert!(!Arc::ptr_eq(&a, &after));
        assert_eq!(after.mode, Mode::Nothing);
    }
}
