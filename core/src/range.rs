//! `croniter_range` (croniter.py:1376) — every fire time between two bounds.
//!
//! The bound arithmetic is shared: `setup()` computes the ±1µs nudge, the
//! direction and the `max_years_between_matches` span that the generator
//! depends on. `core` uses it for the native iterator below; `pybridge` uses
//! the same function so the Python-facing generator cannot drift from it.

use chrono::{Datelike, NaiveDateTime};

use crate::api::{CronIterator, Options, RetType, WallClock};
use crate::error::Result;

/// Direction and bounds adjustment for a range.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RangeSetup {
    /// `start < stop` — iterate with `get_next`; otherwise `get_prev`.
    pub forward: bool,
    /// Microseconds to add to `start` (0 when `exclude_ends`).
    pub start_nudge_us: i64,
    /// Microseconds to add to `stop` (0 when `exclude_ends`).
    pub stop_nudge_us: i64,
    /// `max_years_between_matches` for the underlying iterator.
    pub year_span: i64,
}

/// croniter widens the interval by a microsecond at each end unless
/// `exclude_ends`, so a fire time exactly on a bound is included.
pub fn setup(start: NaiveDateTime, stop: NaiveDateTime, exclude_ends: bool) -> RangeSetup {
    let forward = start < stop;
    let (start_nudge_us, stop_nudge_us) = if exclude_ends {
        (0, 0)
    } else if forward {
        (-1, 1)
    } else {
        (1, -1)
    };
    let year_span = (i64::from(stop.year()) - i64::from(start.year())).abs() + 1;
    RangeSetup {
        forward,
        start_nudge_us,
        stop_nudge_us,
        year_span,
    }
}

/// Native (no-Python) range iterator, used by the CLI and library consumers.
pub struct CroniterRange {
    iter: CronIterator,
    stop: NaiveDateTime,
    forward: bool,
    done: bool,
}

impl CroniterRange {
    pub fn new(
        expr_format: &str,
        start: NaiveDateTime,
        stop: NaiveDateTime,
        clock: &dyn WallClock,
        day_or: bool,
        exclude_ends: bool,
        second_at_beginning: bool,
    ) -> Result<Self> {
        let s = setup(start, stop, exclude_ends);
        let start = start + chrono::Duration::microseconds(s.start_nudge_us);
        let stop = stop + chrono::Duration::microseconds(s.stop_nudge_us);

        let opts = Options {
            ret_type: RetType::DateTime,
            day_or,
            max_years_between_matches: Some(s.year_span),
            second_at_beginning,
            ..Default::default()
        };
        let iter = CronIterator::new(expr_format, clock.from_wall(start)?, opts)?;
        Ok(CroniterRange {
            iter,
            stop,
            forward: s.forward,
            done: false,
        })
    }

    /// Next fire time, or `None` once the range is exhausted.
    pub fn next(&mut self, clock: &dyn WallClock) -> Option<NaiveDateTime> {
        if self.done {
            return None;
        }
        let step = if self.forward {
            self.iter.get_next(clock)
        } else {
            self.iter.get_prev(clock)
        };
        match step {
            Ok((wall, _)) => {
                let keep = if self.forward {
                    wall < self.stop
                } else {
                    wall > self.stop
                };
                if keep {
                    Some(wall)
                } else {
                    self.done = true;
                    None
                }
            }
            // CroniterBadDateError ends the range quietly -- this is why
            // croniter_range always passes max_years_between_matches.
            Err(_) => {
                self.done = true;
                None
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::{FixedClock, TzSpec};
    use chrono::NaiveDate;

    const CLOCK: FixedClock = FixedClock(TzSpec::Naive);

    fn wall(y: i32, m: u32, d: u32, h: u32, mi: u32) -> NaiveDateTime {
        NaiveDate::from_ymd_opt(y, m, d)
            .unwrap()
            .and_hms_opt(h, mi, 0)
            .unwrap()
    }

    #[test]
    fn inclusive_ends_by_default() {
        let mut r = CroniterRange::new(
            "0 0 * * *",
            wall(2025, 6, 1, 0, 0),
            wall(2025, 6, 4, 0, 0),
            &CLOCK,
            true,
            false,
            false,
        )
        .unwrap();
        let mut got = Vec::new();
        while let Some(d) = r.next(&CLOCK) {
            got.push(d);
        }
        assert_eq!(
            got,
            vec![
                wall(2025, 6, 1, 0, 0),
                wall(2025, 6, 2, 0, 0),
                wall(2025, 6, 3, 0, 0),
                wall(2025, 6, 4, 0, 0),
            ]
        );
    }

    #[test]
    fn exclude_ends_drops_both_bounds() {
        let mut r = CroniterRange::new(
            "0 0 * * *",
            wall(2025, 6, 1, 0, 0),
            wall(2025, 6, 4, 0, 0),
            &CLOCK,
            true,
            true,
            false,
        )
        .unwrap();
        let mut got = Vec::new();
        while let Some(d) = r.next(&CLOCK) {
            got.push(d);
        }
        assert_eq!(got, vec![wall(2025, 6, 2, 0, 0), wall(2025, 6, 3, 0, 0)]);
    }

    #[test]
    fn reverse_order_when_start_is_after_stop() {
        let mut r = CroniterRange::new(
            "0 0 * * *",
            wall(2025, 6, 4, 0, 0),
            wall(2025, 6, 1, 0, 0),
            &CLOCK,
            true,
            false,
            false,
        )
        .unwrap();
        let mut got = Vec::new();
        while let Some(d) = r.next(&CLOCK) {
            got.push(d);
        }
        assert_eq!(
            got,
            vec![
                wall(2025, 6, 4, 0, 0),
                wall(2025, 6, 3, 0, 0),
                wall(2025, 6, 2, 0, 0),
                wall(2025, 6, 1, 0, 0),
            ]
        );
    }

    #[test]
    fn setup_computes_year_span_and_direction() {
        let s = setup(wall(2024, 1, 1, 0, 0), wall(2026, 1, 1, 0, 0), false);
        assert!(s.forward);
        assert_eq!(s.year_span, 3);
        assert_eq!((s.start_nudge_us, s.stop_nudge_us), (-1, 1));

        let s = setup(wall(2026, 1, 1, 0, 0), wall(2024, 1, 1, 0, 0), true);
        assert!(!s.forward);
        assert_eq!((s.start_nudge_us, s.stop_nudge_us), (0, 0));
    }
}
