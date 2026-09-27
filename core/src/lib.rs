//! EVE chat notifier core: log tracking, layered settings, rules, presence and
//! the alert router. No UI.

pub mod channel;
pub mod engine;
pub mod governor;
pub mod liveset;
pub mod logfmt;
pub mod merge;
pub mod paths;
pub mod pilots;
pub mod prefs;
pub mod presence;
pub mod router;
pub mod rules;
pub mod settings;
pub mod tailer;
pub mod time;
#[cfg(windows)]
pub mod winapi;
