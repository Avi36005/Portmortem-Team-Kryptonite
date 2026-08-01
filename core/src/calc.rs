//! The next/prev search engine — port of `_calc_next` / `_calc`
//! (croniter.py:475-822) and the public iteration API.
//!
//! The structure deliberately mirrors the Python: a list of `proc_*` steps run
//! in a fixed order, each either leaving the candidate alone, pushing it
//! forward/backward and restarting the pass, or signalling that no match can
//! exist. Reproducing that control flow verbatim is what keeps the fire times
//! identical; a "cleaner" rewrite drifts on the edge cases.

use std::collections::{BTreeMap, BTreeSet};

use chrono::{Datelike, Duration, NaiveDate, NaiveDateTime, Timelike};

use crate::api::{is_successor, WallClock};
use crate::consts::*;
use crate::error::{CroniterError, Result};
use crate::expand::{Expanded, Item, Nth};
use crate::reldelta::{add, RelDelta};

/// Timezone context threaded through the search when the cursor is aware.
#[derive(Clone, Copy)]
pub struct AwareCtx<'a> {
    pub clock: &'a dyn WallClock,
    /// UTC instant of the starting point (`now`).
    pub now_utc: f64,
    /// UTC offset of the starting point, in seconds east.
    pub now_offset: i64,
}

/// What a `proc_*` step reports back, mirroring Python's
/// `(changed, d)` tuple where `changed` may be `True`, `False` or `None`.
enum Proc {
    /// `False` — this field is already satisfied.
    Unchanged,
    /// `True` — the candidate moved; restart the pass.
    Changed,
    /// `None` — no further match is possible (year field exhausted).
    Stop,
}

/// The parsed expression plus iteration state. Mirrors the Python `croniter`
/// instance; timezone handling is layered on top of this in `tz.rs`.
#[derive(Clone, Debug)]
pub struct Croniter {
    pub expanded: Vec<Vec<Item>>,
    pub nth_weekday_of_month: BTreeMap<Item, BTreeSet<Nth>>,
    pub expressions: Vec<String>,
    pub nearest_weekday: BTreeSet<i64>,

    pub day_or: bool,
    pub implement_cron_bug: bool,
    pub max_years_between_matches: i64,
    pub max_years_explicitly_set: bool,
    pub is_prev: bool,
}

impl Croniter {
    pub fn from_expanded(
        e: Expanded,
        day_or: bool,
        implement_cron_bug: bool,
        max_years_between_matches: Option<i64>,
        is_prev: bool,
    ) -> Self {
        let max_years_explicitly_set = max_years_between_matches.is_some();
        let max_years_between_matches = max_years_between_matches.unwrap_or(50).max(1);
        Croniter {
            expanded: e.expanded,
            nth_weekday_of_month: e.nth_weekday_of_month,
            expressions: e.expressions,
            nearest_weekday: e.nearest_weekday,
            day_or,
            implement_cron_bug,
            max_years_between_matches,
            max_years_explicitly_set,
            is_prev,
        }
    }

    /// Number of fields in the expression (5, 6 or 7).
    pub fn len(&self) -> usize {
        self.expanded.len()
    }

    /// Always false: a parsed expression has at least five fields.
    /// Present because clippy pairs `len` with `is_empty`.
    pub fn is_empty(&self) -> bool {
        self.expanded.is_empty()
    }

    /// `croniter._calc_next` (croniter.py:475).
    ///
    /// Handles the day-of-month / day-of-week union: when both fields are
    /// restricted, cron semantics are OR, so each side is evaluated with the
    /// other blanked and the nearer result wins.
    pub fn calc_next(
        &self,
        current: NaiveDateTime,
        is_prev: bool,
        aware: Option<AwareCtx<'_>>,
    ) -> Result<(NaiveDateTime, f64)> {
        let mut expanded = self.expanded.clone();
        let nth_weekday_of_month = self.nth_weekday_of_month.clone();

        let day_restricted = expanded[DAY_FIELD].first() != Some(&Item::Star);
        let dow_restricted = expanded[DOW_FIELD].first() != Some(&Item::Star);

        if day_restricted && dow_restricted && self.day_or {
            // The vixie/ISC cron bug: when either raw field starts with '*',
            // DOM and DOW intersect (AND) instead of uniting (OR).
            let cron_bug = self.implement_cron_bug
                && (self.expressions[DAY_FIELD].starts_with('*')
                    || self.expressions[DOW_FIELD].starts_with('*'));

            if !cron_bug {
                // Under OR semantics an unsatisfiable side contributes no dates
                // rather than ruling out the expression -- but only while each
                // side is a clean operand. '#' and 'W' are carried outside the
                // DAY/DOW fields, so blanking a field does not remove them and
                // each side stays an intersection; a failure there must
                // propagate. (This is what test_issue_k33 pins down.)
                let clean_split =
                    nth_weekday_of_month.is_empty() && self.nearest_weekday.is_empty();

                let bak = expanded[DOW_FIELD].clone();
                expanded[DOW_FIELD] = vec![Item::Star];
                let t1 = match self.calc(current, &expanded, &nth_weekday_of_month, is_prev, aware)
                {
                    Ok(v) => Some(v),
                    Err(CroniterError::BadDate(m)) => {
                        if !clean_split {
                            return Err(CroniterError::BadDate(m));
                        }
                        None
                    }
                    Err(e) => return Err(e),
                };
                expanded[DOW_FIELD] = bak;
                expanded[DAY_FIELD] = vec![Item::Star];

                let t2 = match self.calc(current, &expanded, &nth_weekday_of_month, is_prev, aware)
                {
                    Ok(v) => Some(v),
                    Err(CroniterError::BadDate(m)) => {
                        if !clean_split {
                            return Err(CroniterError::BadDate(m));
                        }
                        None
                    }
                    Err(e) => return Err(e),
                };

                return match (t1, t2) {
                    (None, None) => Err(CroniterError::BadDate(
                        if is_prev {
                            "failed to find prev date"
                        } else {
                            "failed to find next date"
                        }
                        .to_string(),
                    )),
                    (None, Some(t2)) => Ok(t2),
                    (Some(t1), None) => Ok(t1),
                    (Some(t1), Some(t2)) => Ok(if is_successor(t1.1, t2.1, is_prev) {
                        t2
                    } else {
                        t1
                    }),
                };
            }
        }

        self.calc(current, &expanded, &nth_weekday_of_month, is_prev, aware)
    }

    /// `croniter._calc` (croniter.py:539). Searches in wall-clock time, then
    /// pins the result back onto the timeline if a timezone is in play.
    pub fn calc(
        &self,
        now: NaiveDateTime,
        expanded: &[Vec<Item>],
        nth_weekday_of_month: &BTreeMap<Item, BTreeSet<Nth>>,
        is_prev: bool,
        aware: Option<AwareCtx<'_>>,
    ) -> Result<(NaiveDateTime, f64)> {
        let has_seconds = expanded.len() > UNIX_CRON_LEN;

        let offset = if is_prev {
            RelDelta::new().microseconds(-1)
        } else if has_seconds {
            RelDelta::new().seconds(1)
        } else {
            RelDelta::new().minutes(1)
        };

        let mut t = add(now, &offset).ok_or_else(|| self.bad_date(is_prev))?;
        t = if has_seconds {
            t.with_nanosecond(0).ok_or_else(|| self.bad_date(is_prev))?
        } else {
            t.with_second(0)
                .and_then(|x| x.with_nanosecond(0))
                .ok_or_else(|| self.bad_date(is_prev))?
        };

        // `month`/`year` track the candidate as of the start of each pass and
        // are what the day-of-month procs consult -- not `t.month()`/`t.year()`.
        let mut month = i64::from(t.month());
        let mut year = i64::from(t.year());
        let current_year = year;

        // Expand the nth-weekday '*' key once, as Python does in-place.
        let mut nth_map = nth_weekday_of_month.clone();
        if let Some(star_set) = nth_map.remove(&Item::Star) {
            for i in 0..7 {
                nth_map
                    .entry(Item::Num(i))
                    .or_default()
                    .extend(star_set.iter().copied());
            }
        }

        let use_nearest_weekday = !self.nearest_weekday.is_empty();
        let use_nth = !nth_map.is_empty();

        while (year - current_year).abs() <= self.max_years_between_matches {
            let mut restart = false;
            let mut stop = false;

            for step in 0..7 {
                let r = match step {
                    0 => self.proc_year(&mut t, expanded, is_prev)?,
                    1 => self.proc_month(&mut t, expanded, is_prev)?,
                    2 => {
                        if use_nearest_weekday {
                            self.proc_nearest_weekday(&mut t, year, month, is_prev)?
                        } else {
                            self.proc_day_of_month(&mut t, expanded, year, month, is_prev)?
                        }
                    }
                    3 => {
                        if use_nth {
                            self.proc_day_of_week_nth(&mut t, &nth_map, year, month, is_prev)?
                        } else {
                            self.proc_day_of_week(&mut t, expanded, is_prev)?
                        }
                    }
                    4 => self.proc_hour(&mut t, expanded, is_prev)?,
                    5 => self.proc_minute(&mut t, expanded, is_prev)?,
                    _ => self.proc_second(&mut t, expanded, has_seconds, is_prev)?,
                };
                match r {
                    Proc::Stop => {
                        stop = true;
                        break;
                    }
                    Proc::Changed => {
                        month = i64::from(t.month());
                        year = i64::from(t.year());
                        restart = true;
                        break;
                    }
                    Proc::Unchanged => {}
                }
            }

            if stop {
                break;
            }
            if restart {
                continue;
            }

            let wall = t.with_nanosecond(0).ok_or_else(|| self.bad_date(is_prev))?;
            let Some(ctx) = aware else {
                return Ok((wall, naive_to_timestamp(wall)));
            };
            return self.pin_to_timeline(now, wall, expanded, nth_weekday_of_month, is_prev, ctx);
        }

        Err(self.bad_date(is_prev))
    }

    /// The DST tail of `_calc` (croniter.py:783-818).
    ///
    /// Two corrections happen here, in this order:
    ///
    /// 1. If the wall-clock answer was skipped by a spring-forward, walking it
    ///    to the next existing instant is only right when that instant is
    ///    still in the direction of travel — otherwise croniter keeps
    ///    searching for a cron time that genuinely exists.
    /// 2. If the answer landed on a different UTC offset than the start, the
    ///    schedule may also have a match at the *other* offset. Both are
    ///    computed and the nearer one in the direction of travel wins.
    fn pin_to_timeline(
        &self,
        now: NaiveDateTime,
        wall: NaiveDateTime,
        expanded: &[Vec<Item>],
        nth_weekday_of_month: &BTreeMap<Item, BTreeSet<Nth>>,
        is_prev: bool,
        ctx: AwareCtx<'_>,
    ) -> Result<(NaiveDateTime, f64)> {
        let mut unaware = wall;
        let mut r = ctx.clock.resolve(unaware, ctx.now_utc, is_prev)?;

        if !r.exists
            && (!is_successor(r.utc, ctx.now_utc, is_prev)
                || expanded[HOUR_FIELD].contains(&Item::Star))
        {
            while !r.exists {
                unaware = self
                    .calc(unaware, expanded, nth_weekday_of_month, is_prev, None)?
                    .0;
                r = ctx.clock.resolve(unaware, ctx.now_utc, is_prev)?;
            }
        }

        let offset_delta = r.offset - ctx.now_offset;
        if offset_delta == 0 {
            return Ok((r.wall, r.utc));
        }

        // There was a DST change: check the schedule at the other offset too.
        let alt_start = now + Duration::seconds(offset_delta);
        let alt_wall = self
            .calc(alt_start, expanded, nth_weekday_of_month, is_prev, None)?
            .0;
        let alt = ctx.clock.resolve(alt_wall, ctx.now_utc, is_prev)?;

        if !is_successor(alt.utc, ctx.now_utc, is_prev) {
            // The alternative is behind us; not an alternative at all.
            return Ok((r.wall, r.utc));
        }
        if is_successor(r.utc, alt.utc, is_prev) {
            return Ok((alt.wall, alt.utc));
        }
        Ok((r.wall, r.utc))
    }

    fn bad_date(&self, is_prev: bool) -> CroniterError {
        CroniterError::BadDate(
            if is_prev {
                "failed to find prev date"
            } else {
                "failed to find next date"
            }
            .to_string(),
        )
    }

    fn nearest_diff(
        &self,
        is_prev: bool,
        x: i64,
        to_check: &[Item],
        range_val: Option<i64>,
    ) -> Option<i64> {
        if is_prev {
            prev_nearest_diff(x, to_check, range_val)
        } else {
            next_nearest_diff(x, to_check, range_val)
        }
    }

    fn proc_year(
        &self,
        t: &mut NaiveDateTime,
        expanded: &[Vec<Item>],
        is_prev: bool,
    ) -> Result<Proc> {
        if expanded.len() != YEAR_CRON_LEN {
            return Ok(Proc::Unchanged);
        }
        if expanded[YEAR_FIELD].contains(&Item::Star) {
            return Ok(Proc::Unchanged);
        }
        // range_val None => no wraparound for the year field.
        let Some(diff_year) =
            self.nearest_diff(is_prev, i64::from(t.year()), &expanded[YEAR_FIELD], None)
        else {
            return Ok(Proc::Stop);
        };
        if diff_year == 0 {
            return Ok(Proc::Unchanged);
        }
        let rd = if is_prev {
            RelDelta::new()
                .years(diff_year)
                .month_at(12)
                .day_at(31)
                .hour_at(23)
                .minute_at(59)
                .second_at(59)
        } else {
            RelDelta::new()
                .years(diff_year)
                .month_at(1)
                .day_at(1)
                .hour_at(0)
                .minute_at(0)
                .second_at(0)
        };
        *t = add(*t, &rd).ok_or_else(|| self.bad_date(is_prev))?;
        Ok(Proc::Changed)
    }

    fn proc_month(
        &self,
        t: &mut NaiveDateTime,
        expanded: &[Vec<Item>],
        is_prev: bool,
    ) -> Result<Proc> {
        if expanded[MONTH_FIELD].contains(&Item::Star) {
            return Ok(Proc::Unchanged);
        }
        let diff_month = self.nearest_diff(
            is_prev,
            i64::from(t.month()),
            &expanded[MONTH_FIELD],
            Some(MONTHS_IN_YEAR),
        );
        let Some(diff_month) = diff_month else {
            return Ok(Proc::Unchanged);
        };
        if diff_month == 0 {
            return Ok(Proc::Unchanged);
        }
        if is_prev {
            *t = add(*t, &RelDelta::new().months(diff_month))
                .ok_or_else(|| self.bad_date(is_prev))?;
            let reset_day = last_day_of_month(i64::from(t.year()), i64::from(t.month()));
            *t = add(
                *t,
                &RelDelta::new()
                    .day_at(reset_day as u32)
                    .hour_at(23)
                    .minute_at(59)
                    .second_at(59),
            )
            .ok_or_else(|| self.bad_date(is_prev))?;
        } else {
            *t = add(
                *t,
                &RelDelta::new()
                    .months(diff_month)
                    .day_at(1)
                    .hour_at(0)
                    .minute_at(0)
                    .second_at(0),
            )
            .ok_or_else(|| self.bad_date(is_prev))?;
        }
        Ok(Proc::Changed)
    }

    fn proc_day_of_month(
        &self,
        t: &mut NaiveDateTime,
        expanded: &[Vec<Item>],
        year: i64,
        month: i64,
        is_prev: bool,
    ) -> Result<Proc> {
        if expanded[DAY_FIELD].contains(&Item::Star) {
            return Ok(Proc::Unchanged);
        }
        let days = last_day_of_month(year, month);
        if expanded[DAY_FIELD].contains(&Item::Last) && days == i64::from(t.day()) {
            return Ok(Proc::Unchanged);
        }

        let diff_day = if is_prev {
            let prev_month = (month - 2).rem_euclid(MONTHS_IN_YEAR) + 1;
            let prev_year = if month == 1 { year - 1 } else { year };
            let days_in_prev_month = last_day_of_month(prev_year, prev_month);
            self.nearest_diff(
                true,
                i64::from(t.day()),
                &expanded[DAY_FIELD],
                Some(days_in_prev_month),
            )
        } else {
            self.nearest_diff(false, i64::from(t.day()), &expanded[DAY_FIELD], Some(days))
        };

        let Some(diff_day) = diff_day else {
            return Ok(Proc::Unchanged);
        };
        if diff_day == 0 {
            return Ok(Proc::Unchanged);
        }
        *t = add(*t, &shift_day(diff_day, is_prev)).ok_or_else(|| self.bad_date(is_prev))?;
        Ok(Proc::Changed)
    }

    fn proc_day_of_week(
        &self,
        t: &mut NaiveDateTime,
        expanded: &[Vec<Item>],
        is_prev: bool,
    ) -> Result<Proc> {
        if expanded[DOW_FIELD].contains(&Item::Star) {
            return Ok(Proc::Unchanged);
        }
        let dow = i64::from(t.weekday().num_days_from_sunday());
        let Some(diff) = self.nearest_diff(is_prev, dow, &expanded[DOW_FIELD], Some(7)) else {
            return Ok(Proc::Unchanged);
        };
        if diff == 0 {
            return Ok(Proc::Unchanged);
        }
        *t = add(*t, &shift_day(diff, is_prev)).ok_or_else(|| self.bad_date(is_prev))?;
        Ok(Proc::Changed)
    }

    fn proc_day_of_week_nth(
        &self,
        t: &mut NaiveDateTime,
        nth_map: &BTreeMap<Item, BTreeSet<Nth>>,
        year: i64,
        month: i64,
        is_prev: bool,
    ) -> Result<Proc> {
        let mut candidates: Vec<i64> = Vec::new();
        for (wday, nths) in nth_map {
            let Some(wday) = wday.as_num() else { continue };
            let c = nth_weekday_of_month(i64::from(t.year()), i64::from(t.month()), wday);
            for n in nths {
                let candidate = match n {
                    Nth::Last => match c.last() {
                        Some(v) => *v,
                        None => continue,
                    },
                    Nth::Num(n) => {
                        if (c.len() as i64) < *n {
                            continue;
                        }
                        c[(*n - 1) as usize]
                    }
                };
                let day = i64::from(t.day());
                if (is_prev && candidate <= day) || (!is_prev && day <= candidate) {
                    candidates.push(candidate);
                }
            }
        }

        self.finish_day_candidates(t, candidates, year, month, is_prev)
    }

    fn proc_nearest_weekday(
        &self,
        t: &mut NaiveDateTime,
        year: i64,
        month: i64,
        is_prev: bool,
    ) -> Result<Proc> {
        let mut candidates: Vec<i64> = Vec::new();
        for w_day in &self.nearest_weekday {
            let candidate = nearest_weekday(i64::from(t.year()), i64::from(t.month()), *w_day);
            let day = i64::from(t.day());
            if (is_prev && candidate <= day) || (!is_prev && day <= candidate) {
                candidates.push(candidate);
            }
        }
        self.finish_day_candidates(t, candidates, year, month, is_prev)
    }

    /// Shared tail of `proc_day_of_week_nth` and `proc_nearest_weekday`.
    fn finish_day_candidates(
        &self,
        t: &mut NaiveDateTime,
        mut candidates: Vec<i64>,
        year: i64,
        month: i64,
        is_prev: bool,
    ) -> Result<Proc> {
        if candidates.is_empty() {
            // No candidate this month: step to the edge of the adjacent month.
            let rd = if is_prev {
                RelDelta::new()
                    .days(-i64::from(t.day()))
                    .hour_at(23)
                    .minute_at(59)
                    .second_at(59)
            } else {
                let days = last_day_of_month(year, month);
                RelDelta::new()
                    .days(days - i64::from(t.day()) + 1)
                    .hour_at(0)
                    .minute_at(0)
                    .second_at(0)
            };
            *t = add(*t, &rd).ok_or_else(|| self.bad_date(is_prev))?;
            return Ok(Proc::Changed);
        }

        candidates.sort_unstable();
        let chosen = if is_prev {
            *candidates.last().expect("non-empty")
        } else {
            candidates[0]
        };
        let diff_day = chosen - i64::from(t.day());
        if diff_day == 0 {
            return Ok(Proc::Unchanged);
        }
        *t = add(*t, &shift_day(diff_day, is_prev)).ok_or_else(|| self.bad_date(is_prev))?;
        Ok(Proc::Changed)
    }

    fn proc_hour(
        &self,
        t: &mut NaiveDateTime,
        expanded: &[Vec<Item>],
        is_prev: bool,
    ) -> Result<Proc> {
        if expanded[HOUR_FIELD].contains(&Item::Star) {
            return Ok(Proc::Unchanged);
        }
        let Some(diff) = self.nearest_diff(
            is_prev,
            i64::from(t.hour()),
            &expanded[HOUR_FIELD],
            Some(24),
        ) else {
            return Ok(Proc::Unchanged);
        };
        if diff == 0 {
            return Ok(Proc::Unchanged);
        }
        let rd = if is_prev {
            RelDelta::new().hours(diff).minute_at(59).second_at(59)
        } else {
            RelDelta::new().hours(diff).minute_at(0).second_at(0)
        };
        *t = add(*t, &rd).ok_or_else(|| self.bad_date(is_prev))?;
        Ok(Proc::Changed)
    }

    fn proc_minute(
        &self,
        t: &mut NaiveDateTime,
        expanded: &[Vec<Item>],
        is_prev: bool,
    ) -> Result<Proc> {
        if expanded[MINUTE_FIELD].contains(&Item::Star) {
            return Ok(Proc::Unchanged);
        }
        let Some(diff) = self.nearest_diff(
            is_prev,
            i64::from(t.minute()),
            &expanded[MINUTE_FIELD],
            Some(60),
        ) else {
            return Ok(Proc::Unchanged);
        };
        if diff == 0 {
            return Ok(Proc::Unchanged);
        }
        let rd = if is_prev {
            RelDelta::new().minutes(diff).second_at(59)
        } else {
            RelDelta::new().minutes(diff).second_at(0)
        };
        *t = add(*t, &rd).ok_or_else(|| self.bad_date(is_prev))?;
        Ok(Proc::Changed)
    }

    fn proc_second(
        &self,
        t: &mut NaiveDateTime,
        expanded: &[Vec<Item>],
        has_seconds: bool,
        is_prev: bool,
    ) -> Result<Proc> {
        if !has_seconds {
            *t = add(*t, &RelDelta::new().second_at(0)).ok_or_else(|| self.bad_date(false))?;
            return Ok(Proc::Unchanged);
        }
        if expanded[SECOND_FIELD].contains(&Item::Star) {
            return Ok(Proc::Unchanged);
        }
        let Some(diff) = self.nearest_diff(
            is_prev,
            i64::from(t.second()),
            &expanded[SECOND_FIELD],
            Some(60),
        ) else {
            return Ok(Proc::Unchanged);
        };
        if diff == 0 {
            return Ok(Proc::Unchanged);
        }
        *t = add(*t, &RelDelta::new().seconds(diff)).ok_or_else(|| self.bad_date(is_prev))?;
        Ok(Proc::Changed)
    }
}

fn shift_day(diff_day: i64, is_prev: bool) -> RelDelta {
    if is_prev {
        RelDelta::new()
            .days(diff_day)
            .hour_at(23)
            .minute_at(59)
            .second_at(59)
    } else {
        RelDelta::new()
            .days(diff_day)
            .hour_at(0)
            .minute_at(0)
            .second_at(0)
    }
}

/// `croniter._get_next_nearest_diff` (croniter.py:825).
pub fn next_nearest_diff(x: i64, to_check: &[Item], range_val: Option<i64>) -> Option<i64> {
    for item in to_check {
        let d = match (item, range_val) {
            (Item::Last, Some(rv)) => rv,
            (Item::Last, None) => continue,
            (Item::Num(n), Some(rv)) => {
                if *n > rv {
                    continue;
                }
                *n
            }
            (Item::Num(n), None) => *n,
            (Item::Star, _) => continue,
        };
        if d >= x {
            return Some(d - x);
        }
    }
    // No wraparound possible for the year field.
    let range_val = range_val?;
    let first = to_check.first().and_then(Item::as_num)?;
    Some(first - x + range_val)
}

/// `croniter._get_prev_nearest_diff` (croniter.py:849).
pub fn prev_nearest_diff(x: i64, to_check: &[Item], range_val: Option<i64>) -> Option<i64> {
    let candidates: Vec<Item> = to_check.iter().rev().copied().collect();
    for item in &candidates {
        if let Item::Num(d) = item {
            if *d <= x {
                return Some(d - x);
            }
        }
    }
    if candidates.contains(&Item::Last) {
        return Some(-x);
    }
    let range_val = range_val?;

    let mut candidate = candidates.first().and_then(Item::as_num)?;
    for c in &candidates {
        // Deliberately `<=`, not `<`: with `<` every 31st day-of-month, 12th
        // month, 59th second and 23rd hour would be rejected.
        if let Item::Num(v) = c {
            if *v <= range_val {
                candidate = *v;
                break;
            }
        }
    }
    if candidate > range_val {
        return Some(-range_val);
    }
    Some(candidate - x - range_val)
}

/// `croniter._get_nth_weekday_of_month` (croniter.py:885).
///
/// Python builds this with `calendar.Calendar(w).monthdayscalendar(...)` and
/// takes the first column; that is exactly "every day of this month whose
/// weekday is `day_of_week`", in ascending order.
pub fn nth_weekday_of_month(year: i64, month: i64, day_of_week: i64) -> Vec<i64> {
    let last = last_day_of_month(year, month);
    (1..=last)
        .filter(|d| {
            NaiveDate::from_ymd_opt(year as i32, month as u32, *d as u32)
                .map(|dt| i64::from(dt.weekday().num_days_from_sunday()) == day_of_week)
                .unwrap_or(false)
        })
        .collect()
}

/// `croniter._get_nearest_weekday` (croniter.py:896) — the `W` syntax.
pub fn nearest_weekday(year: i64, month: i64, day: i64) -> i64 {
    let last_day = last_day_of_month(year, month);
    let day = day.min(last_day);
    let weekday = NaiveDate::from_ymd_opt(year as i32, month as u32, day as u32)
        .map(|d| d.weekday().num_days_from_monday())
        .unwrap_or(0); // 0=Mon .. 6=Sun

    if weekday < 5 {
        return day; // Mon-Fri
    }
    if weekday == 5 {
        // Saturday -> Friday, unless that leaves the month
        return if day > 1 { day - 1 } else { day + 2 };
    }
    // Sunday -> Monday, unless that leaves the month
    if day < last_day {
        day + 1
    } else {
        day - 2
    }
}

/// `datetime_to_timestamp` (croniter.py:142) for a naive (UTC) datetime.
pub fn naive_to_timestamp(d: NaiveDateTime) -> f64 {
    let epoch = NaiveDate::from_ymd_opt(1970, 1, 1)
        .expect("epoch is valid")
        .and_hms_opt(0, 0, 0)
        .expect("epoch is valid");
    let delta = d - epoch;
    delta.num_seconds() as f64 + f64::from(delta.subsec_nanos()) / 1_000_000_000.0
}

/// Inverse of the above: `datetime.fromtimestamp(ts, tz=utc).replace(tzinfo=None)`.
pub fn timestamp_to_naive(ts: f64) -> Option<NaiveDateTime> {
    // Python rounds a float timestamp to the nearest microsecond.
    let micros = (ts * 1_000_000.0).round();
    if !micros.is_finite() {
        return None;
    }
    let micros = micros as i64;
    let epoch = NaiveDate::from_ymd_opt(1970, 1, 1)?.and_hms_opt(0, 0, 0)?;
    epoch.checked_add_signed(Duration::microseconds(micros))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nth_weekday_lists_all_matching_days() {
        // June 2025: Sundays are 1, 8, 15, 22, 29.
        assert_eq!(nth_weekday_of_month(2025, 6, 0), vec![1, 8, 15, 22, 29]);
        // Fridays in Feb 2024 (leap): 2, 9, 16, 23.
        assert_eq!(nth_weekday_of_month(2024, 2, 5), vec![2, 9, 16, 23]);
    }

    #[test]
    fn nearest_weekday_rules() {
        // 2025-03-15 is a Saturday -> Friday the 14th.
        assert_eq!(nearest_weekday(2025, 3, 15), 14);
        // 2025-03-16 is a Sunday -> Monday the 17th.
        assert_eq!(nearest_weekday(2025, 3, 16), 17);
        // 2025-03-17 is a Monday -> itself.
        assert_eq!(nearest_weekday(2025, 3, 17), 17);
        // 2025-02-01 is a Saturday and day==1 -> Monday the 3rd.
        assert_eq!(nearest_weekday(2025, 2, 1), 3);
        // 2025-08-31 is a Sunday and is the last day -> Friday the 29th.
        assert_eq!(nearest_weekday(2025, 8, 31), 29);
    }

    #[test]
    fn timestamp_roundtrip() {
        let d = NaiveDate::from_ymd_opt(2019, 1, 14)
            .unwrap()
            .and_hms_opt(15, 21, 0)
            .unwrap();
        let ts = naive_to_timestamp(d);
        assert_eq!(ts as i64, 1_547_479_260);
        assert_eq!(timestamp_to_naive(ts).unwrap(), d);
    }

    #[test]
    fn next_nearest_diff_wraps_when_no_larger_value() {
        let to_check = vec![Item::Num(0), Item::Num(30)];
        assert_eq!(next_nearest_diff(0, &to_check, Some(60)), Some(0));
        assert_eq!(next_nearest_diff(15, &to_check, Some(60)), Some(15));
        assert_eq!(next_nearest_diff(45, &to_check, Some(60)), Some(15));
    }

    #[test]
    fn prev_nearest_diff_wraps_backwards() {
        let to_check = vec![Item::Num(0), Item::Num(30)];
        assert_eq!(prev_nearest_diff(45, &to_check, Some(60)), Some(-15));
        assert_eq!(prev_nearest_diff(0, &to_check, Some(60)), Some(0));
        assert_eq!(prev_nearest_diff(10, &to_check, Some(60)), Some(-10));
    }

    #[test]
    fn year_field_has_no_wraparound() {
        let to_check = vec![Item::Num(2030)];
        assert_eq!(next_nearest_diff(2025, &to_check, None), Some(5));
        assert_eq!(next_nearest_diff(2031, &to_check, None), None);
    }
}
