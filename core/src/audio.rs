//! Which sound plays, and whether it plays right now.
//!
//! The Audio page decides the file (none, one shared, or one per character).
//! Whether an alert asks for a sound at all is each channel's own Sound
//! setting (`prefs::Prefs::sound`); this module only answers the rest.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AudioMode {
    /// Silent everywhere, whatever the channels say.
    Off,
    /// One sound for every character.
    #[default]
    Shared,
    /// A sound per character; a character with none uses the shared one.
    PerCharacter,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct AudioSettings {
    pub mode: AudioMode,
    /// The shared sound; `None` means the built-in alert tone.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shared_file: Option<String>,
    /// By character id.
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub pilot_files: BTreeMap<String, String>,
    /// 0 to 100.
    pub volume: u8,
    /// The quiet time after a sound before the next one may play. Mentions
    /// ignore it.
    pub cooldown_secs: u32,
}

impl Default for AudioSettings {
    fn default() -> Self {
        AudioSettings { mode: AudioMode::Shared, shared_file: None, pilot_files: BTreeMap::new(), volume: 50, cooldown_secs: 10 }
    }
}

/// What to play.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Source {
    BuiltIn,
    File(String),
}

impl AudioSettings {
    /// The sound for an alert to this character, or `None` when audio is off.
    pub fn source_for(&self, pilot_id: Option<&str>) -> Option<Source> {
        let own = || pilot_id.and_then(|id| self.pilot_files.get(id)).filter(|f| !f.trim().is_empty());
        let shared = || self.shared_file.as_ref().filter(|f| !f.trim().is_empty());
        match self.mode {
            AudioMode::Off => None,
            AudioMode::Shared => Some(shared().map_or(Source::BuiltIn, |f| Source::File(f.clone()))),
            AudioMode::PerCharacter => Some(own().or_else(shared).map_or(Source::BuiltIn, |f| Source::File(f.clone()))),
        }
    }

    pub fn cooldown(&self) -> Duration {
        Duration::from_secs(u64::from(self.cooldown_secs))
    }

    pub fn gain(&self) -> f32 {
        gain_for(self.volume)
    }
}

/// The volume slider (0 to 100) as an amplitude. Squared, because loudness is
/// heard roughly logarithmically: a straight line made 30% still loud and left
/// the bottom of the slider useless.
pub fn gain_for(volume: u8) -> f32 {
    let v = f32::from(volume.min(100)) / 100.0;
    v * v
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Play {
    /// Nothing is playing: start it.
    Start,
    /// Stop what is playing and start this one.
    Interrupt,
    /// Stay quiet.
    Skip,
}

/// One sound at a time, and a quiet time after each: a busy channel makes one
/// sound, not a stutter. A mention always plays, cutting off anything else.
#[derive(Debug, Default)]
pub struct SoundGate {
    last: Option<Instant>,
}

impl SoundGate {
    pub fn new() -> Self {
        SoundGate::default()
    }

    /// Decides one alert's sound, and counts it as played unless skipped.
    pub fn check(&mut self, now: Instant, cooldown: Duration, mention: bool, playing: bool) -> Play {
        let play = if mention {
            if playing { Play::Interrupt } else { Play::Start }
        } else if playing || self.last.is_some_and(|t| now.saturating_duration_since(t) < cooldown) {
            Play::Skip
        } else {
            Play::Start
        };
        if play != Play::Skip {
            self.last = Some(now);
        }
        play
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEN: Duration = Duration::from_secs(10);

    #[test]
    fn the_cooldown_keeps_a_busy_channel_to_one_sound() {
        let t0 = Instant::now();
        let mut g = SoundGate::new();
        assert_eq!(g.check(t0, TEN, false, false), Play::Start);
        assert_eq!(g.check(t0 + Duration::from_secs(3), TEN, false, false), Play::Skip);
        assert_eq!(g.check(t0 + Duration::from_secs(9), TEN, false, false), Play::Skip);
        // Skipped alerts don't extend it: measured from the last sound played.
        assert_eq!(g.check(t0 + Duration::from_secs(10), TEN, false, false), Play::Start);
    }

    #[test]
    fn a_mention_plays_through_the_cooldown_and_cuts_off_a_sound() {
        let t0 = Instant::now();
        let mut g = SoundGate::new();
        assert_eq!(g.check(t0, TEN, false, false), Play::Start);
        assert_eq!(g.check(t0 + Duration::from_secs(1), TEN, true, true), Play::Interrupt);
        assert_eq!(g.check(t0 + Duration::from_secs(2), TEN, true, false), Play::Start);
        // And it restarts the quiet time.
        assert_eq!(g.check(t0 + Duration::from_secs(11), TEN, false, false), Play::Skip);
    }

    #[test]
    fn only_one_sound_at_a_time_even_without_a_cooldown() {
        let t0 = Instant::now();
        let mut g = SoundGate::new();
        assert_eq!(g.check(t0, Duration::ZERO, false, false), Play::Start);
        assert_eq!(g.check(t0, Duration::ZERO, false, true), Play::Skip);
        assert_eq!(g.check(t0, Duration::ZERO, false, false), Play::Start);
    }

    #[test]
    fn which_file_plays() {
        let mut a = AudioSettings::default();
        assert_eq!(a.source_for(Some("1")), Some(Source::BuiltIn));
        a.shared_file = Some("C:\\s.wav".into());
        a.pilot_files.insert("1".into(), "C:\\one.wav".into());
        assert_eq!(a.source_for(Some("1")), Some(Source::File("C:\\s.wav".into())), "shared ignores per-character files");
        a.mode = AudioMode::PerCharacter;
        assert_eq!(a.source_for(Some("1")), Some(Source::File("C:\\one.wav".into())));
        assert_eq!(a.source_for(Some("2")), Some(Source::File("C:\\s.wav".into())), "unset falls back to shared");
        a.shared_file = None;
        assert_eq!(a.source_for(None), Some(Source::BuiltIn));
        a.mode = AudioMode::Off;
        assert_eq!(a.source_for(Some("1")), None);
    }

    #[test]
    fn an_old_settings_file_gets_the_default_audio() {
        let a: AudioSettings = serde_json::from_str("{}").unwrap();
        assert_eq!(a, AudioSettings::default());
        assert_eq!(a.cooldown_secs, 10);
    }
}
