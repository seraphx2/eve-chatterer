//! Channel kinds. EVE's channel id, read from the log header, carries a prefix
//! that says what kind of channel it is (docs/FINDINGS.md #3):
//!
//! | id                       | kind                    |
//! |--------------------------|-------------------------|
//! | `local`                  | Local                   |
//! | `corp`                   | Corp                    |
//! | `alliance` (assumed)     | Alliance                |
//! | `fleet_1368512310460`    | Fleet                   |
//! | `private_<32 hex>`       | Private message         |
//! | `system_263238_263361`   | Public channel          |
//!
//! Settings hang off the kind. Fleet and private ids change every time, so
//! only their kind can be configured; Local and Corp are one channel each per
//! character; a public channel has a stable id and can also be configured by id.

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ChannelKind {
    Local,
    Corp,
    /// The alliance channel; only exists while in an alliance. The id `alliance`
    /// is an assumption until a real alliance log has been seen.
    Alliance,
    Fleet,
    Private,
    /// A public channel: CCP's (Help, Recruitment...) or player-made (EVE University...).
    Public,
    /// An id we do not recognise. Treated like Public by the settings, but worth surfacing.
    Unknown,
}

impl ChannelKind {
    /// Does the id identify the same channel next time? Fleet and private ids
    /// are new every time, so settings for them can only be per kind.
    pub fn has_stable_id(self) -> bool {
        !matches!(self, ChannelKind::Fleet | ChannelKind::Private)
    }
}

/// Classifies a channel from its header id, falling back to its name when the
/// id is missing (an old or damaged header).
pub fn classify(channel_id: &str, channel_name: &str) -> ChannelKind {
    let id = channel_id.trim().to_ascii_lowercase();
    if !id.is_empty() {
        return match id.as_str() {
            "local" => ChannelKind::Local,
            "corp" => ChannelKind::Corp,
            "alliance" => ChannelKind::Alliance,
            _ if id.starts_with("fleet_") => ChannelKind::Fleet,
            _ if id.starts_with("private_") => ChannelKind::Private,
            _ if id.starts_with("system_") => ChannelKind::Public,
            _ => ChannelKind::Unknown,
        };
    }
    let name = channel_name.trim();
    match name.to_ascii_lowercase().as_str() {
        "local" => ChannelKind::Local,
        "corp" => ChannelKind::Corp,
        "alliance" => ChannelKind::Alliance,
        "fleet" => ChannelKind::Fleet,
        n if n.starts_with("private chat") => ChannelKind::Private,
        _ => ChannelKind::Unknown,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_the_ids_seen_in_real_logs() {
        assert_eq!(classify("local", "Local"), ChannelKind::Local);
        assert_eq!(classify("corp", "Corp"), ChannelKind::Corp);
        assert_eq!(classify("fleet_1368512310460", "Fleet"), ChannelKind::Fleet);
        assert_eq!(classify("private_92740cb0b9ee11f19b5a3a68dd86f9e7", "Private Chat (2)"), ChannelKind::Private);
        assert_eq!(classify("system_263238_263361", "EVE University"), ChannelKind::Public);
    }

    #[test]
    fn the_alliance_guess_and_unknown_ids() {
        assert_eq!(classify("alliance", "Alliance"), ChannelKind::Alliance);
        assert_eq!(classify("mystery_42", "Something"), ChannelKind::Unknown);
    }

    #[test]
    fn falls_back_to_the_name_without_an_id() {
        assert_eq!(classify("", "Local"), ChannelKind::Local);
        assert_eq!(classify("  ", "Private Chat (3)"), ChannelKind::Private);
        assert_eq!(classify("", "Whatever"), ChannelKind::Unknown);
    }

    #[test]
    fn ids_are_case_insensitive_and_stability_is_reported() {
        assert_eq!(classify("LOCAL", ""), ChannelKind::Local);
        assert!(ChannelKind::Local.has_stable_id());
        assert!(ChannelKind::Public.has_stable_id());
        assert!(!ChannelKind::Fleet.has_stable_id());
        assert!(!ChannelKind::Private.has_stable_id());
    }
}
