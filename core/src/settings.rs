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

use crate::audio::AudioSettings;
use crate::channel::ChannelKind;
use crate::prefs::{DeliveryMode, LayerKey, Mode, OverCap, OverlayStyle, Prefs, RateCap, Suppression};
use crate::rules::{CompiledRules, RuleSet, TrackedTerm};
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::io;
use std::path::Path;
use std::sync::Arc;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TrackedKind {
    Keyword,
    Regex,
}

/// A tracked keyword or regex. Stored once per character (or once for
/// Defaults), not once per channel layer — which channels it applies to is a
/// property of the entry itself (`only_in`), not of where the list lives.
/// Chosen over a per-channel-layer list (owner decision 2026-09-27): "the
/// same string/regex in 3 of 5 channels" gets unmanageable fast as separate
/// per-channel lists, but is one entry with a 3-channel scope here.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TrackedRule {
    pub text: String,
    pub kind: TrackedKind,
    /// Empty means every channel kind; otherwise just these.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub only_in: Vec<ChannelKind>,
    /// Still fires on a channel whose Mode is "Nothing". Default true,
    /// matching tracked matching's original, only behavior.
    #[serde(default = "TrackedRule::default_even_when_muted")]
    pub even_when_muted: bool,
}

impl TrackedRule {
    fn default_even_when_muted() -> bool {
        true
    }

    fn applies_to(&self, kind: ChannelKind) -> bool {
        self.only_in.is_empty() || self.only_in.contains(&kind)
    }
}

/// One level of overrides. `None` means "inherit".
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Layer {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mode: Option<Mode>,

    // Content rules (a list set here replaces the inherited list).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub own_name: Option<bool>,
    /// Only ever consulted on the Global and Pilot(base) layers — see
    /// `TrackedRule`'s doc comment for why channel scoping lives on the
    /// entry instead of the layer. Still a plain `Layer` field (rather than
    /// living only on `Settings`/`PilotSettings`) so it round-trips through
    /// the same `Option<Vec<_>>` "unset means inherit" convention as
    /// everything else, and `resolve()`'s single layer-walking loop can stay
    /// one loop instead of a separate pass just for this.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tracked: Option<Vec<TrackedRule>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ignore_own_messages: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ignore_system: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub system_senders: Option<Vec<String>>,
    // The sender lists merge by name across layers (see `place_sender`): a
    // more specific layer only decides the names it lists, and every other
    // name keeps what a less specific layer said.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ignore_senders: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub always_senders: Option<Vec<String>>,
    /// Names this layer puts back to normal (neither always nor ignored),
    /// undoing an entry inherited from a less specific layer; e.g. a
    /// character that should hear someone Defaults ignores, without always
    /// alerting on them.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub normal_senders: Option<Vec<String>>,

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
#[serde(default, rename_all = "camelCase")]
pub struct PilotSettings {
    pub base: Layer,
    pub kinds: BTreeMap<ChannelKind, Layer>,
    /// By channel id; only for kinds with stable ids.
    pub channels: BTreeMap<String, Layer>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Settings {
    pub global: Layer,
    pub kinds: BTreeMap<ChannelKind, Layer>,
    /// By channel id; only for kinds with stable ids (a public channel, Local, Corp, Alliance).
    pub channels: BTreeMap<String, Layer>,
    /// Keyed by character id.
    pub pilots: BTreeMap<String, PilotSettings>,
    /// Which sound plays (the Audio page); app-wide, not layered.
    pub audio: AudioSettings,
    /// App-wide options from the General page; not layered.
    pub general: GeneralSettings,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct GeneralSettings {
    /// The global hotkey that toggles overlay reposition mode, as an
    /// accelerator string ("Ctrl+Alt+O"). Registered by the app at startup
    /// and whenever it's changed.
    pub reposition_hotkey: String,
}

pub const DEFAULT_REPOSITION_HOTKEY: &str = "Ctrl+Alt+O";

impl Default for GeneralSettings {
    fn default() -> Self {
        GeneralSettings { reposition_hotkey: DEFAULT_REPOSITION_HOTKEY.to_string() }
    }
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
        // A seeded example, not just an empty list: "@all" is a common
        // convention for an FC or corp leadership calling for attention in a
        // channel that has no real @mention, so it's a genuinely useful
        // out-of-the-box demo of "watch for this no matter the channel's
        // mode" (owner request 2026-09-28).
        s.global.tracked = Some(vec![TrackedRule { text: "@all".into(), kind: TrackedKind::Keyword, only_in: vec![], even_when_muted: true }]);
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
        // Unlike everything else in this loop, a tracked list is never read
        // from a Kind/Channel/PilotKind/PilotChannel layer — only Global and
        // Pilot(base) — because each entry already carries its own channel
        // scope (`TrackedRule::only_in`), applied in the filter below.
        let mut tracked_all: Vec<TrackedRule> = vec![];
        for (key, l) in self.layers(pilot_id, kind, channel_id) {
            if let Some(v) = l.mode {
                mode = v;
            }
            if let Some(v) = l.own_name {
                rules.own_name = v;
            }
            // Merged, not replaced (owner, 2026-09-29): a character's entries
            // add to Defaults'. The same text and kind listed at both levels
            // is one entry, and the character's copy (its scope and mute
            // setting) wins.
            if matches!(key, LayerKey::Global | LayerKey::Pilot) {
                for rule in l.tracked.iter().flatten() {
                    tracked_all.retain(|t| !(t.kind == rule.kind && t.text.eq_ignore_ascii_case(&rule.text)));
                    tracked_all.push(rule.clone());
                }
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
            // Normal first, then always, then ignore: within one layer a name
            // on several lists ends up ignored, same as in `rules` itself.
            for (names, verdict) in [(&l.normal_senders, None), (&l.always_senders, Some(true)), (&l.ignore_senders, Some(false))] {
                for name in names.iter().flatten() {
                    place_sender(&mut rules, name, verdict);
                }
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
        for t in tracked_all.iter().filter(|t| t.applies_to(kind)) {
            let term = TrackedTerm { text: t.text.clone(), even_when_muted: t.even_when_muted };
            match t.kind {
                TrackedKind::Keyword => rules.keywords.push(term),
                TrackedKind::Regex => rules.regexes.push(term),
            }
        }
        let mut good = Vec::with_capacity(rules.regexes.len());
        let mut bad_regexes = Vec::new();
        for t in rules.regexes {
            if t.text.trim().is_empty() || Regex::new(&format!("(?i){}", t.text)).is_ok() {
                good.push(t);
            } else {
                bad_regexes.push(t.text);
            }
        }
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
            .flat_map(|l| l.tracked.iter().flatten())
            .filter(|t| t.kind == TrackedKind::Regex)
            .map(|t| t.text.as_str())
            .filter(|p| !p.trim().is_empty() && Regex::new(&format!("(?i){p}")).is_err())
            .map(String::from)
            .collect()
    }

    /// A missing file gives the built-in defaults (first run).
    pub fn load(path: &Path) -> io::Result<Settings> {
        match std::fs::read_to_string(path) {
            Ok(s) => {
                let mut loaded: Settings = serde_json::from_str(&s).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
                loaded.backfill_missing_kinds();
                loaded.backfill_example_tracked();
                loaded.drop_copies_of_defaults();
                Ok(loaded)
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(Settings::with_defaults()),
            Err(e) => Err(e),
        }
    }

    /// Adds the shipped default for any kind that is not present *as a key*
    /// at all — a file saved before a kind existed in `with_defaults` (or one
    /// hand-written to only mention a few kinds). Deliberately does not touch
    /// a kind that IS present, even as an empty `{}`: that is a real,
    /// ambiguous user state (either "I want the base fallback here" or an
    /// accidental clear from the UI, e.g. the Defaults-page revert bug found
    /// 2026-09-27 — docs/DESIGN.md), and guessing wrong would silently
    /// overwrite something the user set on purpose.
    fn backfill_missing_kinds(&mut self) {
        for (kind, layer) in Settings::with_defaults().kinds {
            self.kinds.entry(kind).or_insert(layer);
        }
    }

    /// Same non-destructive backfill principle as `backfill_missing_kinds`,
    /// for the "@all" example seeded in `with_defaults()`: only fills it in
    /// when `global.tracked` is absent entirely (a file saved before the
    /// example existed), never when it's present, even as an empty list —
    /// that's the user having deliberately cleared it, or added their own
    /// entries already, either way not something to silently add to.
    fn backfill_example_tracked(&mut self) {
        if self.global.tracked.is_none() {
            self.global.tracked = Settings::with_defaults().global.tracked;
        }
    }

    /// Tracked and sender lists used to be copied from Defaults into a
    /// character the first time it was edited there, and then replaced
    /// Defaults' for it; they merge now (2026-09-29). Those copies are
    /// redundant under merging, and a copied sender entry would even pin the
    /// name so a later change on Defaults never reached that character. So
    /// drop a character's entries that exactly match Defaults'. Run on load;
    /// harmless once there is nothing left to drop.
    fn drop_copies_of_defaults(&mut self) {
        let g_tracked = self.global.tracked.clone().unwrap_or_default();
        let g_always = self.global.always_senders.clone().unwrap_or_default();
        let g_ignore = self.global.ignore_senders.clone().unwrap_or_default();
        let has = |list: &[String], n: &str| list.iter().any(|x| x.eq_ignore_ascii_case(n));
        for p in self.pilots.values_mut() {
            let base = &mut p.base;
            if let Some(t) = base.tracked.as_mut() {
                t.retain(|r| !g_tracked.contains(r));
            }
            if let Some(a) = base.always_senders.as_mut() {
                a.retain(|n| !(has(&g_always, n) && !has(&g_ignore, n)));
            }
            if let Some(i) = base.ignore_senders.as_mut() {
                i.retain(|n| !has(&g_ignore, n));
            }
            none_if_empty(&mut base.tracked);
            none_if_empty(&mut base.always_senders);
            none_if_empty(&mut base.ignore_senders);
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

fn none_if_empty<T>(v: &mut Option<Vec<T>>) {
    if v.as_ref().is_some_and(Vec::is_empty) {
        *v = None;
    }
}

/// Decides one sender name: always (`Some(true)`), ignored (`Some(false)`),
/// or normal (`None`), replacing whatever a less specific layer said about
/// that name only. Names are unique in EVE, and compared without case.
fn place_sender(rules: &mut crate::rules::RuleSet, name: &str, verdict: Option<bool>) {
    rules.always_senders.retain(|n| !n.eq_ignore_ascii_case(name));
    rules.ignore_senders.retain(|n| !n.eq_ignore_ascii_case(name));
    match verdict {
        Some(true) => rules.always_senders.push(name.to_string()),
        Some(false) => rules.ignore_senders.push(name.to_string()),
        None => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rules::{LineCtx, Reason};

    #[test]
    fn sender_lists_merge_by_name_and_the_character_decides_its_own_names() {
        let mut s = Settings::default();
        s.global.ignore_senders = Some(vec!["Spammer".into(), "Bob".into()]);
        s.global.always_senders = Some(vec!["Boss".into()]);
        let base = &mut s.pilots.entry("1".into()).or_default().base;
        base.always_senders = Some(vec!["bob".into(), "Friend".into()]); // Bob: ignored by Defaults, always for this pilot
        base.normal_senders = Some(vec!["boss".into()]); // Boss: always by Defaults, normal for this pilot

        let one = |pilot: &str, sender: &str| sender_verdict(&s, pilot, sender);
        assert_eq!(one("1", "Spammer"), "ignored", "untouched Defaults entries still apply");
        assert_eq!(one("1", "Bob"), "always");
        assert_eq!(one("1", "Friend"), "always");
        assert_eq!(one("1", "Boss"), "normal");
        // Another character just follows Defaults.
        assert_eq!((one("2", "Bob"), one("2", "Boss")), ("ignored", "always"));

        // A name added to Defaults later reaches a character with its own entries too.
        s.global.ignore_senders.as_mut().unwrap().push("NewPest".into());
        assert_eq!(sender_verdict(&s, "1", "NewPest"), "ignored");
    }

    #[test]
    fn loading_drops_a_characters_old_copies_of_defaults_lists() {
        let mut s = Settings::default();
        s.global.tracked = tracked_kw(&["@all"]);
        s.global.always_senders = Some(vec!["Boss".into()]);
        s.global.ignore_senders = Some(vec!["Spammer".into()]);
        let base = &mut s.pilots.entry("1".into()).or_default().base;
        // A copy made by the old UI, plus the character's own additions.
        base.tracked = Some([tracked_kw(&["@all"]).unwrap(), tracked_kw(&["mine"]).unwrap()].concat());
        base.always_senders = Some(vec!["boss".into(), "Friend".into()]);
        base.ignore_senders = Some(vec!["Spammer".into()]);

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        s.save(&path).unwrap();
        let back = Settings::load(&path).unwrap();
        let b = &back.pilots["1"].base;
        assert_eq!(b.tracked.as_ref().unwrap().iter().map(|r| r.text.as_str()).collect::<Vec<_>>(), ["mine"]);
        assert_eq!(b.always_senders.as_deref(), Some(&["Friend".to_string()][..]));
        assert_eq!(b.ignore_senders, None, "an emptied list is removed, not left empty");
    }

    #[test]
    fn within_one_layer_a_name_on_several_sender_lists_is_ignored() {
        let mut s = Settings::default();
        s.global.always_senders = Some(vec!["Bob".into()]);
        s.global.ignore_senders = Some(vec!["Bob".into()]);
        assert_eq!(sender_verdict(&s, "1", "Bob"), "ignored");
    }

    /// How lines from `sender` resolve: "always" (a plain line alerts as an
    /// always-sender), "ignored" (not even a mention alerts), or "normal"
    /// (only a mention alerts).
    fn sender_verdict(s: &Settings, pilot: &str, sender: &str) -> &'static str {
        let r = s.resolve(Some(pilot), ChannelKind::Local, "local");
        let line = |text| r.rules.evaluate(&LineCtx { pilot_name: "Holden", channel_name: "Local", sender, text });
        match (line("nothing special"), line("hi Holden")) {
            (Some(Reason::AlwaysSender(_)), _) => "always",
            (None, None) => "ignored",
            (None, Some(Reason::OwnName)) => "normal",
            other => panic!("unexpected {other:?}"),
        }
    }

    fn tracked_kw(words: &[&str]) -> Option<Vec<TrackedRule>> {
        Some(words.iter().map(|w| TrackedRule { text: w.to_string(), kind: TrackedKind::Keyword, only_in: vec![], even_when_muted: true }).collect())
    }

    fn tracked_kw_scoped(word: &str, only_in: &[ChannelKind]) -> Option<Vec<TrackedRule>> {
        Some(vec![TrackedRule { text: word.to_string(), kind: TrackedKind::Keyword, only_in: only_in.to_vec(), even_when_muted: true }])
    }

    /// Which keyword (if any) fires for this text in the resolved behavior.
    fn hit(r: &Resolved, text: &str) -> Option<String> {
        match r.rules.evaluate(&LineCtx { pilot_name: "Holden", channel_name: "X", sender: "Bob", text }) {
            Some(Reason::Keyword(k)) => Some(k),
            _ => None,
        }
    }

    #[test]
    fn a_pilots_own_tracked_list_adds_to_defaults() {
        // Tracked entries are base-only (Global -> Pilot base), not layered
        // per kind/channel like everything else here (owner decision
        // 2026-09-27: channel targeting moved onto the entry itself instead,
        // see `TrackedRule`). A pilot's entries add to Defaults' (owner,
        // 2026-09-29: it was a full replace, so a term added to Defaults later
        // never reached a pilot with a list of its own).
        let mut s = Settings::default();
        s.global.tracked = tracked_kw(&["g"]);
        assert_eq!(hit(&s.resolve(None, ChannelKind::Corp, "corp"), "g"), Some("g".into()));

        s.pilots.entry("1".into()).or_default().base.tracked = tracked_kw(&["p"]);
        let one = s.resolve(Some("1"), ChannelKind::Corp, "corp");
        assert_eq!((hit(&one, "g"), hit(&one, "p")), (Some("g".into()), Some("p".into())), "both lists apply");
        // Another pilot has only Defaults'.
        assert_eq!(hit(&s.resolve(Some("2"), ChannelKind::Corp, "corp"), "p"), None);

        // The same text at both levels is one entry, and the pilot's copy
        // wins: here its scope keeps it to Local.
        s.global.tracked = tracked_kw(&["x"]);
        s.pilots.get_mut("1").unwrap().base.tracked = tracked_kw_scoped("X", &[ChannelKind::Local]);
        assert_eq!(hit(&s.resolve(Some("1"), ChannelKind::Corp, "corp"), "x"), None, "the pilot's scope applies");
        assert_eq!(hit(&s.resolve(Some("1"), ChannelKind::Local, "local"), "x"), Some("X".into()));

        // A kind or channel layer's tracked list, if one were ever set by
        // hand-edited JSON, has no effect - only Global/pilot base are read.
        s.kinds.insert(ChannelKind::Corp, Layer { tracked: tracked_kw(&["k"]), ..Layer::default() });
        assert_eq!(hit(&s.resolve(None, ChannelKind::Corp, "corp"), "k"), None, "a kind-layer tracked list is ignored");
    }

    #[test]
    fn a_tracked_entrys_channel_scope_limits_where_it_fires() {
        let mut s = Settings::default();
        s.global.tracked = tracked_kw_scoped("jita", &[ChannelKind::Local]);
        assert_eq!(hit(&s.resolve(None, ChannelKind::Local, "local"), "selling in jita"), Some("jita".into()));
        assert_eq!(hit(&s.resolve(None, ChannelKind::Corp, "corp"), "selling in jita"), None, "scoped to Local only");

        // Empty only_in (tracked_kw's default) means every channel kind.
        s.global.tracked = tracked_kw(&["everywhere"]);
        assert_eq!(hit(&s.resolve(None, ChannelKind::Local, "local"), "everywhere"), Some("everywhere".into()));
        assert_eq!(hit(&s.resolve(None, ChannelKind::Corp, "corp"), "everywhere"), Some("everywhere".into()));
    }

    #[test]
    fn even_when_muted_flows_through_resolve_into_the_compiled_rules() {
        // rules.rs's own tests already cover the matching behavior in depth;
        // this just proves resolve() actually carries the flag through from
        // TrackedRule into the RuleSet it hands to CompiledRules, rather
        // than dropping it along the way.
        let mut s = Settings::default();
        s.global.tracked = Some(vec![TrackedRule { text: "jita".into(), kind: TrackedKind::Keyword, only_in: vec![], even_when_muted: false }]);
        s.kinds.insert(ChannelKind::Local, Layer { mode: Some(Mode::Nothing), ..Layer::default() });
        let r = s.resolve(None, ChannelKind::Local, "local");
        assert_eq!(r.mode, Mode::Nothing);
        let ctx = LineCtx { pilot_name: "Holden", channel_name: "Local", sender: "Bob", text: "selling in jita" };
        assert_eq!(r.rules.evaluate_mode(&ctx, r.mode), None, "even_when_muted:false makes it respect the muted channel");
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
        s.global.tracked = Some(vec![
            TrackedRule { text: "(".into(), kind: TrackedKind::Regex, only_in: vec![], even_when_muted: true },
            TrackedRule { text: r"\bgank\b".into(), kind: TrackedKind::Regex, only_in: vec![], even_when_muted: true },
        ]);
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
        s.pilots.entry("2112000001".into()).or_default().base.sound = Some(true);
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
        s.global.tracked = tracked_kw(&["jita"]);
        s.save(&path).unwrap();
        assert_eq!(Settings::load(&path).unwrap(), s);
        std::fs::write(&path, "nope").unwrap();
        assert!(Settings::load(&path).is_err());
    }

    #[test]
    fn loading_backfills_a_kind_missing_entirely_but_leaves_an_empty_one_alone() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        // "fleet" is absent as a key (as the Defaults-revert bug, 2026-09-27,
        // left it) and must come back with its shipped default. "private" is
        // present but empty, which is what a deliberate "use the base
        // fallback here" edit looks like too, so it must be left exactly
        // as-is rather than guessed at.
        std::fs::write(&path, r#"{"kinds":{"private":{}}}"#).unwrap();
        let loaded = Settings::load(&path).unwrap();
        assert_eq!(loaded.resolve(None, ChannelKind::Fleet, "fleet_1").mode, Mode::Everything, "backfilled from with_defaults()");
        assert_eq!(
            loaded.resolve(None, ChannelKind::Private, "private_x").mode,
            Mode::Mentions,
            "left as the base fallback, not silently reset to with_defaults()' Everything"
        );
    }

    #[test]
    fn loading_backfills_the_all_example_only_when_tracked_is_absent_entirely() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        // No "global" key at all: tracked is absent, so the "@all" example
        // from with_defaults() should appear.
        std::fs::write(&path, r#"{}"#).unwrap();
        let loaded = Settings::load(&path).unwrap();
        assert_eq!(loaded.global.tracked.as_deref().map(|v| v.len()), Some(1));
        assert_eq!(hit_regardless_of_kind(&loaded, "hey @all form up"), Some("@all".into()));

        // A file that deliberately clears the list (an empty array, not an
        // absent key) must not have "@all" silently reappear in it.
        std::fs::write(&path, r#"{"global":{"tracked":[]}}"#).unwrap();
        let loaded = Settings::load(&path).unwrap();
        assert_eq!(loaded.global.tracked, Some(vec![]), "left exactly as saved");
    }

    fn hit_regardless_of_kind(s: &Settings, text: &str) -> Option<String> {
        hit(&s.resolve(None, ChannelKind::Local, "local"), text)
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
