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

/// Longest a tag (own or derived) is ever shown as, in the Strip overlay's badge.
const MAX_TAG_LEN: usize = 5;

/// Readable-content bounds for a saved overlay width: narrow enough to tuck
/// into a corner of the game window, wide enough that a Panel/Beacon's body
/// text doesn't wrap into a ladder. `OverlayPlacement::clamped` enforces
/// these; `app/src-tauri/src/overlay.rs` mirrors them for the window's own
/// OS-level min/max size.
pub const MIN_OVERLAY_WIDTH: f64 = 320.0;
pub const MAX_OVERLAY_WIDTH: f64 = 760.0;

/// A character's own saved overlay position and width, set by dragging it in
/// "reposition mode" (a global hotkey) instead of the default
/// centered-on-the-client-window placement.
///
/// The position is relative to the region the alert is drawn in (the EVE
/// client window, or its monitor when the client is fullscreen), as a
/// fraction of the free space: 0 is flush left/top, 1 flush right/bottom.
/// Like Discord's in-game overlay, it stays inside the game and keeps its
/// relative spot when the client is moved or resized (docs/DESIGN.md,
/// "Overlay reposition & resize").
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OverlayPlacement {
    #[serde(default = "centered")]
    pub fx: f64,
    #[serde(default)]
    pub fy: f64,
    /// Logical pixels; always within `[MIN_OVERLAY_WIDTH, MAX_OVERLAY_WIDTH]`
    /// on write (see `PilotRegistry::set_placement`).
    pub width: f64,
}

fn centered() -> f64 {
    0.5
}

impl OverlayPlacement {
    fn clamped(mut self) -> OverlayPlacement {
        self.width = self.width.clamp(MIN_OVERLAY_WIDTH, MAX_OVERLAY_WIDTH);
        self.fx = self.fx.clamp(0.0, 1.0);
        self.fy = self.fy.clamp(0.0, 1.0);
        self
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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
    /// User-set short tag for the Strip overlay's badge, in place of the
    /// name-derived one — mainly for telling apart two characters (possibly
    /// on different accounts) whose names happen to start the same way.
    /// `None` means "derive one from the name" (see `display_tag`), not
    /// "blank"; always normalized (trimmed, capped, uppercased) on write by
    /// `PilotRegistry::set_tag`, so any value found here is already valid.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tag: Option<String>,
    /// This character's own overlay position/width, if it has ever been
    /// dragged in reposition mode. `None` means "use the default centered
    /// placement", same absent-means-inherit convention as `tag`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub placement: Option<OverlayPlacement>,
}

impl Pilot {
    /// What the Strip overlay's badge actually shows: this pilot's own tag if
    /// it has one, otherwise the first letter of each word in its name.
    pub fn display_tag(&self) -> String {
        match &self.tag {
            Some(t) if !t.is_empty() => t.clone(),
            _ => tag_from_name(&self.name),
        }
    }
}

/// Derives a tag straight from a name, for a pilot with no tag of its own and
/// for synthetic/preview alerts that have no `Pilot` record at all: the first
/// letter of each word (any run of whitespace splits words; leading, trailing
/// and repeated spaces are ignored), capped at `MAX_TAG_LEN` and uppercased.
pub fn tag_from_name(name: &str) -> String {
    cap_tag(&name.split_whitespace().filter_map(|w| w.chars().next()).collect::<String>())
}

fn cap_tag(s: &str) -> String {
    s.trim().chars().take(MAX_TAG_LEN).collect::<String>().to_uppercase()
}

#[derive(Debug, Clone, PartialEq)]
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
                let p = Pilot { id: id.to_string(), name: name.to_string(), live, first_seen: now.0, channels: BTreeMap::new(), tag: None, placement: None };
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

    /// Sets this pilot's own Strip-badge tag, normalizing it (trim, cap at
    /// `MAX_TAG_LEN`, uppercase) first; a blank tag clears it back to "derive
    /// one from the name" rather than storing an empty string. A no-op for an
    /// unknown pilot id.
    pub fn set_tag(&mut self, id: &str, tag: Option<&str>) {
        if let Some(p) = self.pilots.get_mut(id) {
            p.tag = tag.map(cap_tag).filter(|t| !t.is_empty());
        }
    }

    /// Sets (or, given `None`, clears) this pilot's saved overlay
    /// position/width, clamping the width first. A no-op for an unknown
    /// pilot id.
    pub fn set_placement(&mut self, id: &str, placement: Option<OverlayPlacement>) {
        if let Some(p) = self.pilots.get_mut(id) {
            p.placement = placement.map(OverlayPlacement::clamped);
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
    fn display_tag_falls_back_to_initials_from_the_name_when_untagged() {
        let mut r = PilotRegistry::default();
        r.observe("1", "Psianna Archeia", true, T);
        assert_eq!(r.get("1").unwrap().display_tag(), "PA");
    }

    #[test]
    fn initials_collapse_runs_of_whitespace_and_ignore_leading_and_trailing_spaces() {
        assert_eq!(tag_from_name("  Psianna   Archeia  "), "PA");
        assert_eq!(tag_from_name("Jarna"), "J");
        assert_eq!(tag_from_name("   "), "");
    }

    #[test]
    fn a_pilots_own_tag_wins_over_the_derived_one() {
        let mut r = PilotRegistry::default();
        r.observe("1", "Psianna Archeia", true, T);
        r.set_tag("1", Some("Psi"));
        assert_eq!(r.get("1").unwrap().display_tag(), "PSI");
    }

    #[test]
    fn tags_are_trimmed_capped_at_five_and_uppercased() {
        let mut r = PilotRegistry::default();
        r.observe("1", "Jarna", true, T);
        r.set_tag("1", Some("  nightstalker  "));
        assert_eq!(r.get("1").unwrap().tag.as_deref(), Some("NIGHT"));

        // Longer than five words worth of initials is capped the same way.
        r.observe("2", "A B C D E F G", true, T);
        assert_eq!(r.get("2").unwrap().display_tag(), "ABCDE");
    }

    #[test]
    fn a_blank_tag_clears_back_to_the_derived_one_instead_of_storing_empty() {
        let mut r = PilotRegistry::default();
        r.observe("1", "Jarna", true, T);
        r.set_tag("1", Some("J-"));
        r.set_tag("1", Some("   "));
        assert_eq!(r.get("1").unwrap().tag, None);
        assert_eq!(r.get("1").unwrap().display_tag(), "J");
        // An unknown pilot id is a no-op, not a panic.
        r.set_tag("nobody", Some("X"));
    }

    #[test]
    fn a_placement_can_be_set_cleared_and_is_clamped_to_the_readable_range() {
        let mut r = PilotRegistry::default();
        r.observe("1", "Jarna", true, T);
        assert_eq!(r.get("1").unwrap().placement, None);

        r.set_placement("1", Some(OverlayPlacement { fx: 0.25, fy: 0.1, width: 5000.0 }));
        let p = r.get("1").unwrap().placement.unwrap();
        assert_eq!((p.fx, p.fy, p.width), (0.25, 0.1, MAX_OVERLAY_WIDTH));

        r.set_placement("1", Some(OverlayPlacement { fx: -3.0, fy: 7.0, width: 1.0 }));
        let p = r.get("1").unwrap().placement.unwrap();
        assert_eq!((p.fx, p.fy, p.width), (0.0, 1.0, MIN_OVERLAY_WIDTH));

        r.set_placement("1", None);
        assert_eq!(r.get("1").unwrap().placement, None);
        // An unknown pilot id is a no-op, not a panic.
        r.set_placement("nobody", Some(OverlayPlacement { fx: 0.5, fy: 0.0, width: 400.0 }));
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
        // Same for `tag` and `placement`, both added later still: absent
        // entirely, not null, and `tag` still falls back to a derived one
        // correctly.
        assert_eq!(r.get("1").unwrap().tag, None);
        assert_eq!(r.get("1").unwrap().display_tag(), "J");
        assert_eq!(r.get("1").unwrap().placement, None);
    }
}
