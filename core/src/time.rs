//! Timestamps. EVE writes UTC everywhere in the logs (file names and lines).

/// Seconds since the Unix epoch, UTC.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Stamp(pub i64);

impl Stamp {
    pub fn from_civil(y: i64, m: u32, d: u32, hh: u32, mm: u32, ss: u32) -> Stamp {
        Stamp(days_from_civil(y, m, d) * 86_400 + i64::from(hh) * 3600 + i64::from(mm) * 60 + i64::from(ss))
    }

    /// Parses the in-log form `2026.09.27 01:37:31`.
    pub fn parse_log(s: &str) -> Option<Stamp> {
        let (date, time) = s.trim().split_once(' ')?;
        let mut d = date.split('.');
        let (y, mo, da) = (d.next()?.parse().ok()?, d.next()?.parse().ok()?, d.next()?.parse().ok()?);
        if d.next().is_some() {
            return None;
        }
        let mut t = time.trim().split(':');
        let (h, mi, se): (u32, u32, u32) = (t.next()?.parse().ok()?, t.next()?.parse().ok()?, t.next()?.parse().ok()?);
        if t.next().is_some() || !(1..=12).contains(&mo) || !(1..=31).contains(&da) || h > 23 || mi > 59 || se > 60 {
            return None;
        }
        Some(Stamp::from_civil(y, mo, da, h, mi, se))
    }

    /// Whole seconds from `earlier` to `self` (negative if `self` is earlier).
    pub fn since(self, earlier: Stamp) -> i64 {
        self.0 - earlier.0
    }
}

/// Days since 1970-01-01 for a proleptic Gregorian date (Hinnant's algorithm).
fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y.rem_euclid(400);
    let mp = (i64::from(m) + 9) % 12; // March = 0
    let doy = (153 * mp + 2) / 5 + i64::from(d) - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn epoch_and_known_dates() {
        assert_eq!(Stamp::from_civil(1970, 1, 1, 0, 0, 0), Stamp(0));
        assert_eq!(Stamp::from_civil(2000, 3, 1, 0, 0, 0), Stamp(951_868_800));
        // The Local line seen in the owner's logs.
        assert_eq!(Stamp::parse_log("2026.09.27 01:37:31"), Some(Stamp(1_790_473_051)));
    }

    #[test]
    fn leap_days_and_ordering() {
        let feb29 = Stamp::from_civil(2024, 2, 29, 12, 0, 0);
        let mar1 = Stamp::from_civil(2024, 3, 1, 12, 0, 0);
        assert_eq!(mar1.since(feb29), 86_400);
    }

    #[test]
    fn rejects_garbage() {
        for bad in ["", "2026.09.27", "2026-09-27 01:37:31", "2026.13.01 00:00:00", "2026.09.27 25:00:00", "a.b.c 1:2:3"] {
            assert_eq!(Stamp::parse_log(bad), None, "{bad:?}");
        }
    }
}
