//! The public, stateful surface: `get_next` / `get_prev` / `get_current` /
//! `set_current` / `match` / `match_range` (croniter.py:345-473, 1332-1373).
//!
//! croniter carries its cursor as a float UNIX timestamp and converts to and
//! from wall-clock time on every step. That round-trip is preserved here
//! because it is observable: `get_current(float)` returns the timestamp, and
//! sub-second precision is lost or kept in exactly the same places.

use chrono::NaiveDateTime;

use crate::calc::{naive_to_timestamp, timestamp_to_naive, AwareCtx, Croniter};
use crate::error::{CroniterError, Result};
use crate::expand::expand;

/// Converts between the float timestamp croniter stores and the wall-clock
/// reading the cron fields are matched against.
///
/// This is the *only* seam where a timezone database is consulted. The search
/// engine itself never sees a timezone — it works purely on wall-clock time,
/// exactly as `_calc` does on `now.replace(tzinfo=None)`.
///
/// `core` ships `FixedClock` (naive and fixed-offset), which is all the CLI
/// needs. `pybridge` implements this trait against the *actual Python tzinfo
/// object* the test passed in, so DST behaviour is decided by the same
/// `zoneinfo`/`pytz`/`dateutil` database the test asserts against rather than
/// by a second, subtly different copy of the rules.
// `from_wall` takes `&self` on purpose: it is a conversion *performed by* a
// clock, not a constructor. clippy's `from_*` convention does not apply.
#[allow(clippy::wrong_self_convention)]
pub trait WallClock {
    fn to_wall(&self, ts: f64) -> Result<NaiveDateTime>;
    fn from_wall(&self, wall: NaiveDateTime) -> Result<f64>;
    /// True when a timezone is attached at all (croniter's `if now.tzinfo`).
    fn is_aware(&self) -> bool {
        false
    }

    /// `_add_tzinfo` (croniter.py:179) — pin a wall-clock reading onto the
    /// timeline across a DST transition.
    ///
    /// Two things can go wrong at a transition and croniter treats them
    /// differently:
    ///
    /// * the reading is **ambiguous** (it happens twice): pick whichever side
    ///   is nearer `prev_utc` while still being a successor in the direction
    ///   of travel;
    /// * the reading **does not exist** (it was skipped): step forward a
    ///   minute at a time until it does, and report `exists: false` so the
    ///   caller can decide whether jumping forward was the right answer at all.
    fn resolve(&self, wall: NaiveDateTime, prev_utc: f64, is_prev: bool) -> Result<Resolved> {
        let _ = (prev_utc, is_prev);
        let utc = self.from_wall(wall)?;
        Ok(Resolved {
            wall,
            utc,
            offset: (naive_to_timestamp(wall) - utc).round() as i64,
            exists: true,
        })
    }
}

/// Outcome of pinning a wall-clock reading onto the timeline.
#[derive(Clone, Copy, Debug)]
pub struct Resolved {
    /// The reading actually used — may be later than requested when the
    /// original did not exist.
    pub wall: NaiveDateTime,
    /// The UTC instant it corresponds to.
    pub utc: f64,
    /// UTC offset in seconds east.
    pub offset: i64,
    /// False when the requested reading was skipped by a DST jump.
    pub exists: bool,
}

/// `_is_successor` (croniter.py:161) — ordering in the direction of travel.
pub fn is_successor(a_utc: f64, b_utc: f64, is_prev: bool) -> bool {
    if is_prev {
        a_utc < b_utc
    } else {
        a_utc > b_utc
    }
}

/// How the cursor's wall-clock time relates to UTC, for the no-Python case.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TzSpec {
    Naive,
    Offset(i32),
}

impl TzSpec {
    fn offset_seconds(&self) -> i64 {
        match self {
            TzSpec::Naive => 0,
            TzSpec::Offset(s) => i64::from(*s),
        }
    }
}

/// `WallClock` for naive and fixed-offset time — no DST transitions.
#[derive(Clone, Copy, Debug)]
pub struct FixedClock(pub TzSpec);

impl WallClock for FixedClock {
    fn to_wall(&self, ts: f64) -> Result<NaiveDateTime> {
        let utc = timestamp_to_naive(ts)
            .ok_or_else(|| CroniterError::Value("timestamp out of range".into()))?;
        Ok(utc + chrono::Duration::seconds(self.0.offset_seconds()))
    }
    fn from_wall(&self, wall: NaiveDateTime) -> Result<f64> {
        Ok(naive_to_timestamp(
            wall - chrono::Duration::seconds(self.0.offset_seconds()),
        ))
    }
    fn is_aware(&self) -> bool {
        self.0 != TzSpec::Naive
    }
}

/// What `ret_type` the caller asked for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RetType {
    Float,
    DateTime,
}

#[derive(Clone, Debug)]
pub struct CronIterator {
    pub cron: Croniter,
    pub ret_type: RetType,
    pub start_time: f64,
    pub dst_start_time: f64,
    pub cur: f64,
    pub is_prev: bool,
    pub second_at_beginning: bool,
    pub expand_from_start_time: bool,
}

/// Everything `croniter.__init__` accepts (croniter.py:289).
#[derive(Clone, Debug)]
pub struct Options {
    pub ret_type: RetType,
    pub day_or: bool,
    pub max_years_between_matches: Option<i64>,
    pub is_prev: bool,
    pub hash_id: Option<Vec<u8>>,
    pub implement_cron_bug: bool,
    pub second_at_beginning: bool,
    pub expand_from_start_time: bool,
}

impl Default for Options {
    fn default() -> Self {
        Options {
            ret_type: RetType::Float,
            day_or: true,
            max_years_between_matches: None,
            is_prev: false,
            hash_id: None,
            implement_cron_bug: false,
            second_at_beginning: false,
            expand_from_start_time: false,
        }
    }
}

impl CronIterator {
    /// `croniter.__init__` (croniter.py:289).
    pub fn new(expr_format: &str, start_ts: f64, opts: Options) -> Result<Self> {
        let from_timestamp = if opts.expand_from_start_time {
            Some(start_ts)
        } else {
            None
        };
        let e = expand(
            expr_format,
            opts.hash_id.as_deref(),
            opts.second_at_beginning,
            from_timestamp,
            false,
            None,
        )?;
        let cron = Croniter::from_expanded(
            e,
            opts.day_or,
            opts.implement_cron_bug,
            opts.max_years_between_matches,
            opts.is_prev,
        );
        Ok(CronIterator {
            cron,
            ret_type: opts.ret_type,
            start_time: start_ts,
            dst_start_time: start_ts,
            cur: start_ts,
            is_prev: opts.is_prev,
            second_at_beginning: opts.second_at_beginning,
            expand_from_start_time: opts.expand_from_start_time,
        })
    }

    /// `croniter.set_current` (croniter.py:365) with `force=True`.
    pub fn set_current(&mut self, ts: f64) {
        self.start_time = ts;
        self.dst_start_time = ts;
        self.cur = ts;
    }

    pub fn get_current_wall(&self, clock: &dyn WallClock) -> Result<NaiveDateTime> {
        clock.to_wall(self.cur)
    }

    /// `croniter._get_next` (croniter.py:405). Returns the wall-clock match and
    /// its timestamp; the caller picks which to hand back per `ret_type`.
    pub fn step(
        &mut self,
        clock: &dyn WallClock,
        is_prev: Option<bool>,
        start_time: Option<f64>,
        update_current: bool,
    ) -> Result<(NaiveDateTime, f64)> {
        if let Some(ts) = start_time {
            self.set_current(ts);
        }
        let is_prev = is_prev.unwrap_or(self.is_prev);
        self.is_prev = is_prev;

        let current = clock.to_wall(self.cur)?;
        let aware = if clock.is_aware() {
            // The offset in force at `now` -- croniter's `now.utcoffset()`.
            let now_offset = (naive_to_timestamp(current) - self.cur).round() as i64;
            Some(AwareCtx {
                clock,
                now_utc: self.cur,
                now_offset,
            })
        } else {
            None
        };

        let (result, timestamp) = self.cron.calc_next(current, is_prev, aware)?;
        if update_current {
            self.cur = timestamp;
        }
        Ok((result, timestamp))
    }

    pub fn get_next(&mut self, clock: &dyn WallClock) -> Result<(NaiveDateTime, f64)> {
        self.step(clock, Some(false), None, true)
    }

    pub fn get_prev(&mut self, clock: &dyn WallClock) -> Result<(NaiveDateTime, f64)> {
        self.step(clock, Some(true), None, true)
    }

    pub fn max_years_explicitly_set(&self) -> bool {
        self.cron.max_years_explicitly_set
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::matcher::is_match;
    use chrono::NaiveDate;

    fn wall(y: i32, m: u32, d: u32, h: u32, mi: u32, s: u32) -> NaiveDateTime {
        NaiveDate::from_ymd_opt(y, m, d)
            .unwrap()
            .and_hms_opt(h, mi, s)
            .unwrap()
    }

    const CLOCK: FixedClock = FixedClock(TzSpec::Naive);

    fn it(expr: &str, start: NaiveDateTime) -> CronIterator {
        CronIterator::new(expr, naive_to_timestamp(start), Options::default()).unwrap()
    }

    #[test]
    fn every_minute() {
        let mut c = it("* * * * *", wall(2010, 1, 25, 4, 46, 0));
        assert_eq!(c.get_next(&CLOCK).unwrap().0, wall(2010, 1, 25, 4, 47, 0));
        assert_eq!(c.get_next(&CLOCK).unwrap().0, wall(2010, 1, 25, 4, 48, 0));
    }

    #[test]
    fn weekday_nine_am() {
        // 2025-06-07 is a Saturday; next weekday 9am is Monday the 9th.
        let mut c = it("0 9 * * 1-5", wall(2025, 6, 7, 12, 0, 0));
        assert_eq!(c.get_next(&CLOCK).unwrap().0, wall(2025, 6, 9, 9, 0, 0));
    }

    #[test]
    fn last_day_of_month() {
        let mut c = it("0 12 l * *", wall(2024, 2, 1, 0, 0, 0));
        assert_eq!(c.get_next(&CLOCK).unwrap().0, wall(2024, 2, 29, 12, 0, 0));
    }

    #[test]
    fn third_friday() {
        let mut c = it("0 9 * * 5#3", wall(2025, 6, 1, 0, 0, 0));
        assert_eq!(c.get_next(&CLOCK).unwrap().0, wall(2025, 6, 20, 9, 0, 0));
    }

    #[test]
    fn nearest_weekday_to_the_15th() {
        // 2025-03-15 is a Saturday -> fires Friday the 14th.
        let mut c = it("0 9 15w * *", wall(2025, 3, 1, 0, 0, 0));
        assert_eq!(c.get_next(&CLOCK).unwrap().0, wall(2025, 3, 14, 9, 0, 0));
    }

    #[test]
    fn get_prev_walks_backwards() {
        let mut c = it("0 0 * * *", wall(2025, 6, 15, 12, 0, 0));
        assert_eq!(c.get_prev(&CLOCK).unwrap().0, wall(2025, 6, 15, 0, 0, 0));
        assert_eq!(c.get_prev(&CLOCK).unwrap().0, wall(2025, 6, 14, 0, 0, 0));
    }

    #[test]
    fn day_and_dow_are_a_union() {
        // "the 15th OR any Friday". Verified against the Python original:
        // from 2025-08-01 00:00 the next four are Aug 8, 15, 22, 29 --
        // the 15th is also a Friday that month, so the union is just Fridays.
        let mut c = it("0 0 15 * 5", wall(2025, 8, 1, 0, 0, 0));
        assert_eq!(c.get_next(&CLOCK).unwrap().0, wall(2025, 8, 8, 0, 0, 0));
        assert_eq!(c.get_next(&CLOCK).unwrap().0, wall(2025, 8, 15, 0, 0, 0));
        assert_eq!(c.get_next(&CLOCK).unwrap().0, wall(2025, 8, 22, 0, 0, 0));
    }

    #[test]
    fn seconds_field() {
        let mut c = it("* * * * * */15", wall(2025, 6, 1, 0, 0, 0));
        assert_eq!(c.get_next(&CLOCK).unwrap().0, wall(2025, 6, 1, 0, 0, 15));
    }

    #[test]
    fn match_is_true_on_a_fire_time() {
        assert!(is_match(
            "0 9 * * *",
            wall(2025, 6, 2, 9, 0, 0),
            &CLOCK,
            true,
            false,
            None
        )
        .unwrap());
        assert!(!is_match(
            "0 9 * * *",
            wall(2025, 6, 2, 9, 1, 0),
            &CLOCK,
            true,
            false,
            None
        )
        .unwrap());
    }

    #[test]
    fn impossible_date_reports_bad_date() {
        // Feb 30th never happens.
        let mut c = CronIterator::new(
            "0 0 30 2 *",
            naive_to_timestamp(wall(2025, 1, 1, 0, 0, 0)),
            Options {
                max_years_between_matches: Some(2),
                ..Default::default()
            },
        )
        .unwrap();
        assert!(matches!(c.get_next(&CLOCK), Err(CroniterError::BadDate(_))));
    }
}
