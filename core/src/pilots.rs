//! Pilots (EVE characters), keyed by character id.
//!
//! Names never change in EVE, so the name is a stable secondary key, but the
//! id is what the config hangs off. A pilot first seen only in old log files
//! is registered silently; the first time it is seen live is the moment to
//! create its config and tell the user (docs/DESIGN.md, "Pilots").

use crate::channel::ChannelKind;
use crate::time::Stamp;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::io;
use std::path::Path;

/// A channel this pilot has been seen in, whose specific instance the
/// settings UI needs to list individually — in practice only Public (and
/// Unknown) channels, since Local/Corp/Alliance/Fleet/Private are always one
/// row regardless of which system, corp, alliance, fleet or conversation it
/// actually is (docs/DESIGN.md, "Settings screen").
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KnownChannel {
    pub name: String,
    pub kind: ChannelKind,
    /// Unix seconds when this pilot was last seen active in it — the only way
    /// to tell an abandoned channel from a current one, since the entry
    /// itself is otherwise kept forever (docs/BACKLOG.md, "Known channels
    /// never get pruned").
    pub last_seen: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Pilot {
    pub id: String,
    pub name: String,
    /// Has been seen running, not just in historical log files.
    pub live: bool,
    /// Unix seconds when first registered.
    pub first_seen: i64,
    /// Keyed by channel id.
    #[serde(default)]
    pub channels: BTreeMap<String, KnownChannel>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Observation {
    /// First live sighting (also when a pilot known only from old logs starts running).
    NewLive(Pilot),
    /// First sighting, from historical logs only. Register silently.
    NewHistoric(Pilot),
    Known,
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct PilotRegistry {
    pilots: BTreeMap<String, Pilot>,
}

impl PilotRegistry {
    pub fn observe(&mut self, id: &str, name: &str, live: bool, now: Stamp) -> Observation {
        match self.pilots.get_mut(id) {
            None => {
                let p = Pilot { id: id.to_string(), name: name.to_string(), live, first_seen: now.0, channels: BTreeMap::new() };
                self.pilots.insert(id.to_string(), p.clone());
                if live {
                    Observation::NewLive(p)
                } else {
                    Observation::NewHistoric(p)
                }
            }
            Some(p) => {
                if p.name.is_empty() {
                    p.name = name.to_string();
                }
                if live && !p.live {
                    p.live = true;
                    Observation::NewLive(p.clone())
                } else {
                    Observation::Known
                }
            }
        }
    }

    pub fn get(&self, id: &str) -> Option<&Pilot> {
        self.pilots.get(id)
    }

    /// Names are unique and permanent in EVE, so this is a safe join key.
    pub fn by_name(&self, name: &str) -> Option<&Pilot> {
        self.pilots.values().find(|p| p.name.eq_ignore_ascii_case(name))
    }

    pub fn iter(&self) -> impl Iterator<Item = &Pilot> {
        self.pilots.values()
    }

    pub fn len(&self) -> usize {
        self.pilots.len()
    }

    pub fn is_empty(&self) -> bool {
        self.pilots.is_empty()
    }

    /// Records that `id` is active in this channel right now — always bumps
    /// `last_seen`, even when the name/kind are unchanged. A no-op for an
    /// unknown pilot id (observe it first).
    pub fn note_channel(&mut self, id: &str, channel_id: &str, channel_name: &str, kind: ChannelKind, now: Stamp) {
        if let Some(p) = self.pilots.get_mut(id) {
            p.channels.insert(channel_id.to_string(), KnownChannel { name: channel_name.to_string(), kind, last_seen: now.0 });
        }
    }

    /// Forgets a channel outright — the UI only offers this when nothing is
    /// configured for it, so it never silently discards a rule.
    pub fn remove_channel(&mut self, id: &str, channel_id: &str) {
        if let Some(p) = self.pilots.get_mut(id) {
            p.channels.remove(channel_id);
        }
    }

    /// A missing file is an empty registry (first run).
    pub fn load(path: &Path) -> io::Result<PilotRegistry> {
        match std::fs::read_to_string(path) {
            Ok(s) => serde_json::from_str(&s).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e)),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(PilotRegistry::default()),
            Err(e) => Err(e),
        }
    }

    /// Writes to a temporary file and renames it over the target, so a crash
    /// mid-write cannot leave a truncated registry.
    pub fn save(&self, path: &Path) -> io::Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_vec_pretty(self).map_err(io::Error::other)?)?;
        std::fs::rename(&tmp, path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const T: Stamp = Stamp(1_790_000_000);

    #[test]
    fn historic_then_live_notifies_once() {
        let mut r = PilotRegistry::default();
        assert!(matches!(r.observe("1", "Hinata Sunji", false, T), Observation::NewHistoric(_)));
        assert_eq!(r.observe("1", "Hinata Sunji", false, T), Observation::Known);
        // The old alt logs in for the first time: this is the moment to notify.
        assert!(matches!(r.observe("1", "Hinata Sunji", true, T), Observation::NewLive(p) if p.live));
        assert_eq!(r.observe("1", "Hinata Sunji", true, T), Observation::Known);
        assert_eq!(r.len(), 1);
    }

    #[test]
    fn a_brand_new_live_pilot_notifies_immediately() {
        let mut r = PilotRegistry::default();
        assert!(matches!(r.observe("2", "Jarna", true, T), Observation::NewLive(_)));
    }

    #[test]
    fn lookup_by_name_ignores_case() {
        let mut r = PilotRegistry::default();
        r.observe("3", "Psianna Archeia", true, T);
        assert_eq!(r.by_name("psianna archeia").unwrap().id, "3");
        assert!(r.by_name("nobody").is_none());
    }

    #[test]
    fn known_channels_are_recorded_and_last_seen_is_bumped_every_time() {
        let mut r = PilotRegistry::default();
        r.observe("1", "Jarna", true, T);
        r.note_channel("1", "system_1_2", "EVE University", ChannelKind::Public, T);
        assert_eq!(r.get("1").unwrap().channels.len(), 1);
        let c = &r.get("1").unwrap().channels["system_1_2"];
        assert_eq!((c.name.as_str(), c.last_seen), ("EVE University", T.0));

        // A rename updates in place, not a second entry, and last_seen moves forward.
        let later = Stamp(T.0 + 3600);
        r.note_channel("1", "system_1_2", "EVE University Renamed", ChannelKind::Public, later);
        assert_eq!(r.get("1").unwrap().channels.len(), 1);
        let c = &r.get("1").unwrap().channels["system_1_2"];
        assert_eq!((c.name.as_str(), c.last_seen), ("EVE University Renamed", later.0));

        // A second distinct channel adds, not replaces.
        r.note_channel("1", "system_9_9", "Help", ChannelKind::Public, later);
        assert_eq!(r.get("1").unwrap().channels.len(), 2);
        // An unknown pilot id is a no-op, not a panic.
        r.note_channel("nobody", "system_1_2", "EVE University", ChannelKind::Public, later);
        assert!(r.get("nobody").is_none());
    }

    #[test]
    fn a_channel_can_be_removed() {
        let mut r = PilotRegistry::default();
        r.observe("1", "Jarna", true, T);
        r.note_channel("1", "system_1_2", "EVE University", ChannelKind::Public, T);
        r.remove_channel("1", "system_1_2");
        assert!(r.get("1").unwrap().channels.is_empty());
        // Removing something absent, or from an unknown pilot, is a no-op.
        r.remove_channel("1", "system_1_2");
        r.remove_channel("nobody", "system_1_2");
    }

    #[test]
    fn round_trips_through_disk_and_a_missing_file_is_empty() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sub").join("pilots.json");
        assert!(PilotRegistry::load(&path).unwrap().is_empty());
        let mut r = PilotRegistry::default();
        r.observe("1", "Jarna", true, T);
        r.note_channel("1", "system_1_2", "EVE University", ChannelKind::Public, T);
        r.save(&path).unwrap();
        let back = PilotRegistry::load(&path).unwrap();
        assert_eq!(back.get("1"), r.get("1"));
        std::fs::write(&path, "not json").unwrap();
        assert!(PilotRegistry::load(&path).is_err());
    }

    #[test]
    fn a_pilots_json_from_before_known_channels_existed_still_loads() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("pilots.json");
        std::fs::write(&path, r#"{"pilots":{"1":{"id":"1","name":"Jarna","live":true,"firstSeen":1}}}"#).unwrap();
        let r = PilotRegistry::load(&path).unwrap();
        assert!(r.get("1").unwrap().channels.is_empty());
    }
}
