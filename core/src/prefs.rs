//! The vocabulary of settings: what a user can choose for how alerts behave.
//! Plain data, shared by settings, rules, the router and the governor.

use crate::channel::ChannelKind;
use serde::{Deserialize, Serialize};

/// Which lines of a channel can alert at all, named after Discord's own
/// per-channel notification levels (owner decision 2026-09-27).
///
/// Keyword/regex/always-alert-sender tracking (`RuleSet::keywords` etc.) is a
/// **separate, always-on layer, not a fourth value here** (owner correction
/// the same day: "Matching is a parallel concept... it works independently,
/// alongside the others"). It is checked under every mode, `Nothing` included
/// — muting a channel for general chatter must not silence something you
/// explicitly asked to be told about. See `rules::CompiledRules::evaluate_mode`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    /// No alert from ordinary chatter (tracked keywords/senders still fire).
    Nothing,
    /// The listening pilot's own name (plus tracked keywords/senders).
    Mentions,
    /// Alert on every line (except your own messages, system messages and ignored senders).
    Everything,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Suppression {
    /// Suppress only the pilot whose client has focus. The default.
    FocusedOnly,
    /// Suppress any pilot whose client is visible on screen.
    VisibleOnScreen,
    /// Never suppress.
    AllowAll,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeliveryMode {
    /// Overlay when a client is on screen, toast otherwise or when the user is away.
    Auto,
    Overlay,
    Toast,
    Both,
    SoundOnly,
}

/// How an overlay looks (docs/design/alert-styles.html).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OverlayStyle {
    /// EVE-window look with the reason and a lifetime meter: the default for keyword matches.
    Panel,
    /// One compact line: for high-volume channels.
    Strip,
    /// Large, with a pulse on arrival: for alerts that must not be missed.
    Beacon,
}

/// What happens to alerts beyond a rate cap.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OverCap {
    /// Discard them.
    Drop,
    /// Fold them into the count badge of the alert already showing.
    Fold,
}

/// A ceiling on alerts. Unlike preferences, caps from every applicable level
/// all apply: a channel cannot lift a pilot's cap.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RateCap {
    pub per_minute: u32,
    pub over: OverCap,
}

/// Which settings level a cap (or any value) came from.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum LayerKey {
    Global,
    Kind(ChannelKind),
    Channel(String),
    Pilot,
    PilotKind(ChannelKind),
    PilotChannel(String),
}

/// The resolved presentation choices for one alert target.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Prefs {
    pub delivery: DeliveryMode,
    pub suppression: Suppression,
    /// A style forced by the settings; `None` lets the reason choose.
    pub style: Option<OverlayStyle>,
    pub sound: bool,
    pub caps: Vec<(LayerKey, RateCap)>,
}

impl Default for Prefs {
    fn default() -> Self {
        Prefs { delivery: DeliveryMode::Auto, suppression: Suppression::FocusedOnly, style: None, sound: false, caps: vec![] }
    }
}
