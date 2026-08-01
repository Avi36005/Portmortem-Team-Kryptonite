//! `match`, `match_range` and `is_valid` — croniter.py:1320-1373.
//!
//! These are the read-only queries: "is this expression well formed?" and
//! "does this instant fall on the schedule?". They sit apart from `api.rs`
//! (which owns the stateful cursor) because they create a throwaway iterator,
//! answer one question and discard it.
//!
//! `match` is not an independent implementation of matching. croniter defines
//! it in terms of the search engine: step *backwards* from the instant in
//! question and see whether the previous fire time lands within one tick. That
//! is reproduced exactly rather than reimplemented, because a second matching
//! implementation is a second thing that can disagree with the first.

use chrono::{NaiveDateTime, Timelike};

use crate::api::{CronIterator, Options, RetType, WallClock};
use crate::consts::UNIX_CRON_LEN;
use crate::error::{CroniterError, Result};
use crate::expand::expand;

/// `croniter.is_valid` (croniter.py:1320).
pub fn is_valid(
    expression: &str,
    hash_id: Option<&[u8]>,
    second_at_beginning: bool,
    strict: bool,
    strict_year: Option<&[i64]>,
) -> bool {
    expand(
        expression,
        hash_id,
        second_at_beginning,
        None,
        strict,
        strict_year,
    )
    .is_ok()
}

/// `croniter.match_range` (croniter.py:1346).
pub fn match_range(
    cron_expression: &str,
    from_wall: NaiveDateTime,
    to_wall: NaiveDateTime,
    clock: &dyn WallClock,
    day_or: bool,
    second_at_beginning: bool,
    precision_in_seconds: Option<f64>,
) -> Result<bool> {
    let opts = Options {
        ret_type: RetType::DateTime,
        day_or,
        second_at_beginning,
        ..Default::default()
    };
    let to_ts = clock.from_wall(to_wall)?;
    let mut cron = CronIterator::new(cron_expression, to_ts, opts)?;

    let mut tdp = cron.get_current_wall(clock)?;
    if tdp.nanosecond() % 1_000_000_000 == 0 {
        // croniter nudges by a microsecond so an exact hit counts as "matched"
        tdp += chrono::Duration::microseconds(1);
    }
    cron.set_current(clock.from_wall(tdp)?);

    let tdt = match cron.step(clock, Some(true), None, true) {
        Ok((wall, _)) => wall,
        Err(CroniterError::BadDate(_)) => return Ok(false),
        Err(e) => return Err(e),
    };

    let precision_in_seconds = precision_in_seconds.unwrap_or({
        if cron.cron.len() > UNIX_CRON_LEN {
            1.0
        } else {
            60.0
        }
    });

    let duration_in_second = (to_wall - from_wall).num_microseconds().unwrap_or(0) as f64
        / 1_000_000.0
        + precision_in_seconds;

    let gap = (tdp - tdt).num_microseconds().unwrap_or(0).abs() as f64 / 1_000_000.0;
    Ok(gap < duration_in_second)
}

/// `croniter.match` (croniter.py:1333).
pub fn is_match(
    cron_expression: &str,
    when: NaiveDateTime,
    clock: &dyn WallClock,
    day_or: bool,
    second_at_beginning: bool,
    precision_in_seconds: Option<f64>,
) -> Result<bool> {
    match_range(
        cron_expression,
        when,
        when,
        clock,
        day_or,
        second_at_beginning,
        precision_in_seconds,
    )
}
