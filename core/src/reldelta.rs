//! A faithful re-implementation of the slice of `dateutil.relativedelta` that
//! croniter uses.
//!
//! This is not a general date-delta type and is not meant to be one. croniter
//! writes things like
//!
//! ```python
//! d += relativedelta(days=diff_day, hour=0, minute=0, second=0)
//! ```
//!
//! where **plural** names are relative offsets and **singular** names are
//! absolute replacements, applied in a specific order. `dateutil` does:
//!
//! 1. `year   = (self.year or other.year) + self.years`
//! 2. `month  = self.month or other.month`, then `+= self.months` with a single
//!    wrap into the neighbouring year
//! 3. `day    = min(days_in(year, month), self.day or other.day)`  ← clamps
//! 4. `replace(year, month, day, [hour], [minute], [second], [microsecond])`
//! 5. `+ timedelta(days, hours, minutes, seconds, microseconds)`
//!
//! Step 3 is the one that matters and the one that is easy to get wrong:
//! `Mar 31 + relativedelta(months=-1)` is **Feb 28**, not an error and not
//! Mar 3. Step 5 running *after* the absolute replacements is the other: in
//! `relativedelta(hours=diff, minute=0, second=0)` the minute and second are
//! zeroed first and the hours added afterwards.

use chrono::{Datelike, Duration, NaiveDate, NaiveDateTime, Timelike};

use crate::consts::last_day_of_month;

#[derive(Clone, Copy, Debug, Default)]
pub struct RelDelta {
    // relative (plural)
    pub years: i64,
    pub months: i64,
    pub days: i64,
    pub hours: i64,
    pub minutes: i64,
    pub seconds: i64,
    pub microseconds: i64,
    // absolute (singular)
    pub month: Option<u32>,
    pub day: Option<u32>,
    pub hour: Option<u32>,
    pub minute: Option<u32>,
    pub second: Option<u32>,
    pub microsecond: Option<u32>,
}

impl RelDelta {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn years(mut self, v: i64) -> Self {
        self.years = v;
        self
    }
    pub fn months(mut self, v: i64) -> Self {
        self.months = v;
        self
    }
    pub fn days(mut self, v: i64) -> Self {
        self.days = v;
        self
    }
    pub fn hours(mut self, v: i64) -> Self {
        self.hours = v;
        self
    }
    pub fn minutes(mut self, v: i64) -> Self {
        self.minutes = v;
        self
    }
    pub fn seconds(mut self, v: i64) -> Self {
        self.seconds = v;
        self
    }
    pub fn microseconds(mut self, v: i64) -> Self {
        self.microseconds = v;
        self
    }
    pub fn month_at(mut self, v: u32) -> Self {
        self.month = Some(v);
        self
    }
    pub fn day_at(mut self, v: u32) -> Self {
        self.day = Some(v);
        self
    }
    pub fn hour_at(mut self, v: u32) -> Self {
        self.hour = Some(v);
        self
    }
    pub fn minute_at(mut self, v: u32) -> Self {
        self.minute = Some(v);
        self
    }
    pub fn second_at(mut self, v: u32) -> Self {
        self.second = Some(v);
        self
    }
}

/// `datetime + relativedelta`, following `dateutil`'s `__add__` exactly.
/// Returns `None` if the result falls outside the representable range.
pub fn add(dt: NaiveDateTime, rd: &RelDelta) -> Option<NaiveDateTime> {
    let mut year = i64::from(dt.year()) + rd.years;
    let mut month = i64::from(rd.month.unwrap_or_else(|| dt.month()));

    if rd.months != 0 {
        month += rd.months;
        if month > 12 {
            year += 1;
            month -= 12;
        } else if month < 1 {
            year -= 1;
            month += 12;
        }
    }

    // dateutil clamps the day to the length of the target month.
    let wanted_day = i64::from(rd.day.unwrap_or_else(|| dt.day()));
    let day = wanted_day.min(last_day_of_month(year, month));

    let date = NaiveDate::from_ymd_opt(i32::try_from(year).ok()?, month as u32, day as u32)?;
    let replaced = date.and_hms_micro_opt(
        rd.hour.unwrap_or_else(|| dt.hour()),
        rd.minute.unwrap_or_else(|| dt.minute()),
        rd.second.unwrap_or_else(|| dt.second()),
        rd.microsecond
            .unwrap_or_else(|| dt.and_utc().timestamp_subsec_micros()),
    )?;

    replaced
        .checked_add_signed(Duration::days(rd.days))?
        .checked_add_signed(Duration::hours(rd.hours))?
        .checked_add_signed(Duration::minutes(rd.minutes))?
        .checked_add_signed(Duration::seconds(rd.seconds))?
        .checked_add_signed(Duration::microseconds(rd.microseconds))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;

    fn dt(y: i32, m: u32, d: u32, h: u32, mi: u32, s: u32) -> NaiveDateTime {
        NaiveDate::from_ymd_opt(y, m, d)
            .unwrap()
            .and_hms_opt(h, mi, s)
            .unwrap()
    }

    #[test]
    fn month_subtraction_clamps_the_day() {
        // dateutil: datetime(2021,3,31) + relativedelta(months=-1) -> 2021-02-28
        let r = add(dt(2021, 3, 31, 0, 0, 0), &RelDelta::new().months(-1)).unwrap();
        assert_eq!(r, dt(2021, 2, 28, 0, 0, 0));
    }

    #[test]
    fn month_addition_wraps_the_year() {
        let r = add(dt(2025, 12, 15, 1, 2, 3), &RelDelta::new().months(1)).unwrap();
        assert_eq!(r, dt(2026, 1, 15, 1, 2, 3));
        let r = add(dt(2025, 1, 15, 1, 2, 3), &RelDelta::new().months(-1)).unwrap();
        assert_eq!(r, dt(2024, 12, 15, 1, 2, 3));
    }

    #[test]
    fn absolute_fields_apply_before_relative_ones() {
        // relativedelta(hours=2, minute=0, second=0): zero m/s, then add hours.
        let r = add(
            dt(2025, 6, 1, 10, 45, 30),
            &RelDelta::new().hours(2).minute_at(0).second_at(0),
        )
        .unwrap();
        assert_eq!(r, dt(2025, 6, 1, 12, 0, 0));
    }

    #[test]
    fn leap_day_clamp() {
        // 2024-02-29 + relativedelta(years=1) -> 2025-02-28
        let r = add(dt(2024, 2, 29, 0, 0, 0), &RelDelta::new().years(1)).unwrap();
        assert_eq!(r, dt(2025, 2, 28, 0, 0, 0));
    }

    #[test]
    fn day_absolute_is_clamped_to_month_length() {
        // relativedelta(day=31) in a 30-day month gives the 30th.
        let r = add(dt(2025, 4, 10, 0, 0, 0), &RelDelta::new().day_at(31)).unwrap();
        assert_eq!(r, dt(2025, 4, 30, 0, 0, 0));
    }

    #[test]
    fn microsecond_step_back() {
        let base = dt(2025, 6, 1, 0, 0, 0);
        let r = add(base, &RelDelta::new().microseconds(-1)).unwrap();
        assert_eq!(
            r,
            dt(2025, 5, 31, 23, 59, 59) + Duration::microseconds(999_999)
        );
    }
}
