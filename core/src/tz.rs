//! DST-aware `WallClock` over a real IANA timezone, with no Python involved.
//!
//! `FixedClock` in `api.rs` covers naive and fixed-offset time, which is all
//! the arithmetic needs when there are no transitions. This module is what
//! makes the *shipped* crate a complete standalone port: it resolves ambiguous
//! and non-existent local times the way `_add_tzinfo` (croniter.py:179) does,
//! using `chrono-tz` instead of a Python `tzinfo` object.
//!
//! The mapping from Python's `fold` to `chrono`'s `LocalResult` is exact:
//!
//! | Python | chrono | meaning |
//! |---|---|---|
//! | `fold=0` | `LocalResult::Ambiguous(earliest, _)` | first pass of a repeated hour |
//! | `fold=1` | `LocalResult::Ambiguous(_, latest)` | second pass |
//! | `datetime_exists() == False` | `LocalResult::None` | skipped by a spring-forward |
//!
//! `pybridge` deliberately does **not** use this: the original test suite
//! asserts against whichever tz database the test itself supplied
//! (`zoneinfo`, `pytz`, `dateutil`), and those disagree on ambiguous times, so
//! the bridge asks the caller's own object. See DECISIONS.md #11 and #19.

use chrono::{DateTime, Duration, LocalResult, NaiveDateTime, TimeZone, Utc};
use chrono_tz::Tz;

use crate::api::{is_successor, Resolved, WallClock};
use crate::calc::timestamp_to_naive;
use crate::error::{CroniterError, Result};

/// A `WallClock` backed by the IANA timezone database.
#[derive(Clone, Copy, Debug)]
pub struct TzClock {
    tz: Tz,
}

impl TzClock {
    pub fn new(tz: Tz) -> Self {
        TzClock { tz }
    }

    /// Look up a zone by IANA name, e.g. `"Europe/London"`.
    pub fn from_name(name: &str) -> Result<Self> {
        name.parse::<Tz>()
            .map(TzClock::new)
            .map_err(|_| CroniterError::Value(format!("unknown timezone {name:?}")))
    }

    pub fn tz(&self) -> Tz {
        self.tz
    }

    /// The UTC instant for a local reading, resolving a gap the way Python's
    /// `replace(tzinfo=...)` does: a non-existent time keeps the offset that
    /// was in force *before* the gap (equivalent to `fold=0`).
    fn instant_for(&self, wall: NaiveDateTime) -> Result<DateTime<Utc>> {
        match self.tz.from_local_datetime(&wall) {
            LocalResult::Single(d) => Ok(d.with_timezone(&Utc)),
            LocalResult::Ambiguous(earliest, _) => Ok(earliest.with_timezone(&Utc)),
            LocalResult::None => {
                // Walk back to the last local time that exists and reuse its
                // offset, which is exactly what fold=0 means inside a gap.
                for back in 1..=(24 * 60) {
                    let probe = wall - Duration::minutes(back);
                    if let Some(off) = match self.tz.from_local_datetime(&probe) {
                        LocalResult::Single(d) => Some(*d.offset()),
                        LocalResult::Ambiguous(e, _) => Some(*e.offset()),
                        LocalResult::None => None,
                    } {
                        let secs = i64::from(chrono::Offset::fix(&off).local_minus_utc());
                        return Ok(Utc.from_utc_datetime(&(wall - Duration::seconds(secs))));
                    }
                }
                Err(CroniterError::Value(
                    "could not resolve local time to an instant".to_string(),
                ))
            }
        }
    }

    fn resolved(&self, local: DateTime<Tz>, exists: bool) -> Resolved {
        let utc = local.with_timezone(&Utc);
        Resolved {
            wall: local.naive_local(),
            utc: utc.timestamp() as f64 + f64::from(utc.timestamp_subsec_micros()) / 1e6,
            offset: i64::from(chrono::Offset::fix(local.offset()).local_minus_utc()),
            exists,
        }
    }
}

impl WallClock for TzClock {
    fn to_wall(&self, ts: f64) -> Result<NaiveDateTime> {
        let utc = timestamp_to_naive(ts)
            .ok_or_else(|| CroniterError::Value("timestamp out of range".into()))?;
        Ok(Utc
            .from_utc_datetime(&utc)
            .with_timezone(&self.tz)
            .naive_local())
    }

    fn from_wall(&self, wall: NaiveDateTime) -> Result<f64> {
        let utc = self.instant_for(wall)?;
        Ok(utc.timestamp() as f64 + f64::from(utc.timestamp_subsec_micros()) / 1e6)
    }

    fn is_aware(&self) -> bool {
        true
    }

    /// `_add_tzinfo` (croniter.py:179), the `zoneinfo`/`dateutil` branch.
    fn resolve(&self, wall: NaiveDateTime, prev_utc: f64, is_prev: bool) -> Result<Resolved> {
        match self.tz.from_local_datetime(&wall) {
            LocalResult::Single(d) => Ok(self.resolved(d, true)),

            LocalResult::Ambiguous(earliest, latest) => {
                // Python: fold = 1 if is_prev else 0, i.e. when stepping
                // backwards the *later* instant is the one nearer to `prev`.
                let (closer, farther) = if is_prev {
                    (latest, earliest)
                } else {
                    (earliest, latest)
                };
                let closer_r = self.resolved(closer, true);
                if is_successor(closer_r.utc, prev_utc, is_prev) {
                    Ok(closer_r)
                } else {
                    Ok(self.resolved(farther, true))
                }
            }

            LocalResult::None => {
                // Skipped by a spring-forward: step forward a minute at a time
                // until a local time exists, and report exists=false so the
                // caller can decide whether the jump was the right answer.
                let mut probe = wall;
                for _ in 0..(24 * 60) {
                    probe += Duration::minutes(1);
                    match self.tz.from_local_datetime(&probe) {
                        LocalResult::Single(d) => return Ok(self.resolved(d, false)),
                        LocalResult::Ambiguous(e, _) => return Ok(self.resolved(e, false)),
                        LocalResult::None => continue,
                    }
                }
                Err(CroniterError::Value(
                    "could not resolve a non-existent local time".to_string(),
                ))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::{CronIterator, Options, RetType};
    use chrono::NaiveDate;

    fn wall(y: i32, m: u32, d: u32, h: u32, mi: u32) -> NaiveDateTime {
        NaiveDate::from_ymd_opt(y, m, d)
            .unwrap()
            .and_hms_opt(h, mi, 0)
            .unwrap()
    }

    fn fires(expr: &str, zone: &str, start: NaiveDateTime, n: usize) -> Vec<(String, i64)> {
        let clock = TzClock::from_name(zone).unwrap();
        let ts = clock.from_wall(start).unwrap();
        let mut it = CronIterator::new(
            expr,
            ts,
            Options {
                ret_type: RetType::DateTime,
                ..Default::default()
            },
        )
        .unwrap();
        (0..n)
            .map(|_| {
                let (w, t) = it.get_next(&clock).unwrap();
                // The offset the engine actually chose, derived from the pair
                // it returned -- not re-resolved, which would lose the fold.
                let off = (crate::calc::naive_to_timestamp(w) - t).round() as i64;
                (w.format("%Y-%m-%d %H:%M:%S").to_string(), off)
            })
            .collect()
    }

    #[test]
    fn unknown_zone_is_rejected() {
        assert!(TzClock::from_name("Mars/Olympus_Mons").is_err());
        assert!(TzClock::from_name("Europe/London").is_ok());
    }

    #[test]
    fn fall_back_repeats_the_ambiguous_hour() {
        // Athens 2013-10-27: 04:00 +03:00 -> 03:00 +02:00.
        // Verified against the Python original: the 03:00 and 03:30 readings
        // occur twice, first at +03:00 then at +02:00.
        let got = fires("*/30 * * * *", "Europe/Athens", wall(2013, 10, 27, 2, 0), 7);
        let want = [
            ("2013-10-27 02:30:00", 3 * 3600),
            ("2013-10-27 03:00:00", 3 * 3600),
            ("2013-10-27 03:30:00", 3 * 3600),
            ("2013-10-27 03:00:00", 2 * 3600),
            ("2013-10-27 03:30:00", 2 * 3600),
            ("2013-10-27 04:00:00", 2 * 3600),
            ("2013-10-27 04:30:00", 2 * 3600),
        ];
        for (i, (w, o)) in want.iter().enumerate() {
            assert_eq!(got[i].0, *w, "index {i}");
            assert_eq!(got[i].1, *o, "offset at index {i}");
        }
    }

    #[test]
    fn spring_forward_skips_the_missing_hour() {
        // New York 2013-03-10: 02:00 EST -> 03:00 EDT, so 02:30 never happens.
        let got = fires("30 * * * *", "America/New_York", wall(2013, 3, 10, 1, 0), 3);
        assert_eq!(got[0].0, "2013-03-10 01:30:00");
        assert_eq!(got[1].0, "2013-03-10 03:30:00");
        assert_eq!(got[2].0, "2013-03-10 04:30:00");
    }

    #[test]
    fn roundtrip_through_the_epoch_is_stable() {
        let clock = TzClock::from_name("Asia/Kolkata").unwrap();
        let w = wall(2025, 6, 15, 9, 30);
        let ts = clock.from_wall(w).unwrap();
        assert_eq!(clock.to_wall(ts).unwrap(), w);
        // Kolkata is UTC+5:30 year round.
        assert_eq!(clock.resolve(w, ts, false).unwrap().offset, 5 * 3600 + 1800);
    }

    #[test]
    fn half_hour_dst_shift_is_handled() {
        // Lord Howe is the only 30-minute DST shift in the world.
        let clock = TzClock::from_name("Australia/Lord_Howe").unwrap();
        let before = clock.resolve(wall(2019, 10, 6, 1, 0), 0.0, false).unwrap();
        let after = clock.resolve(wall(2019, 10, 6, 3, 0), 0.0, false).unwrap();
        assert_eq!(before.offset, 10 * 3600 + 1800); // +10:30
        assert_eq!(after.offset, 11 * 3600); // +11:00
                                             // 02:00 does not exist that day; it is walked forward to 02:30.
        let gap = clock.resolve(wall(2019, 10, 6, 2, 0), 0.0, false).unwrap();
        assert!(!gap.exists);
        assert_eq!(gap.wall, wall(2019, 10, 6, 2, 30));
    }
}
