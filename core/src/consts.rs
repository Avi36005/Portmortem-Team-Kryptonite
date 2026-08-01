//! Field indices, ranges and alpha tables — transcribed from croniter.py.

/// Field indices (croniter.py:88-94).
pub const MINUTE_FIELD: usize = 0;
pub const HOUR_FIELD: usize = 1;
pub const DAY_FIELD: usize = 2;
pub const MONTH_FIELD: usize = 3;
pub const DOW_FIELD: usize = 4;
pub const SECOND_FIELD: usize = 5;
pub const YEAR_FIELD: usize = 6;

pub const UNIX_CRON_LEN: usize = 5;
pub const SECOND_CRON_LEN: usize = 6;
pub const YEAR_CRON_LEN: usize = 7;

/// `VALID_LEN_EXPRESSION` — the accepted column counts.
pub const VALID_LEN_EXPRESSION: [usize; 3] = [UNIX_CRON_LEN, SECOND_CRON_LEN, YEAR_CRON_LEN];

/// `croniter.RANGES` (croniter.py:269) — inclusive (low, high) per field index.
pub const RANGES: [(i64, i64); 7] = [
    (0, 59),      // minute
    (0, 23),      // hour
    (1, 31),      // day of month
    (1, 12),      // month
    (0, 6),       // day of week
    (0, 59),      // second
    (1970, 2099), // year
];

/// `croniter.LEN_MEANS_ALL` (croniter.py:287) — cardinality that collapses to `*`.
pub const LEN_MEANS_ALL: [usize; 7] = [60, 24, 31, 12, 7, 60, 130];

pub const MONTHS_IN_YEAR: i64 = 12;

/// `DAYS` (croniter.py:111) — days per month, non-leap February.
pub const DAYS: [i64; 12] = [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];

/// `M_ALPHAS` (croniter.py:64) — month abbreviations.
pub const M_ALPHAS: [(&str, i64); 12] = [
    ("jan", 1),
    ("feb", 2),
    ("mar", 3),
    ("apr", 4),
    ("may", 5),
    ("jun", 6),
    ("jul", 7),
    ("aug", 8),
    ("sep", 9),
    ("oct", 10),
    ("nov", 11),
    ("dec", 12),
];

/// `DOW_ALPHAS` (croniter.py:78) — day-of-week abbreviations.
pub const DOW_ALPHAS: [(&str, i64); 7] = [
    ("sun", 0),
    ("mon", 1),
    ("tue", 2),
    ("wed", 3),
    ("thu", 4),
    ("fri", 5),
    ("sat", 6),
];

/// `_is_leap` (croniter.py:149).
pub fn is_leap(year: i64) -> bool {
    year % 400 == 0 || (year % 4 == 0 && year % 100 != 0)
}

/// `_last_day_of_month` (croniter.py:153).
pub fn last_day_of_month(year: i64, month: i64) -> i64 {
    let mut last_day = DAYS[(month - 1) as usize];
    if month == 2 && is_leap(year) {
        last_day += 1;
    }
    last_day
}

/// Which field indices are in play for an expression of `n` columns
/// (`CRON_FIELDS`, croniter.py:125).
pub fn cron_fields(n: usize) -> &'static [usize] {
    match n {
        UNIX_CRON_LEN => &[MINUTE_FIELD, HOUR_FIELD, DAY_FIELD, MONTH_FIELD, DOW_FIELD],
        SECOND_CRON_LEN => &[
            MINUTE_FIELD,
            HOUR_FIELD,
            DAY_FIELD,
            MONTH_FIELD,
            DOW_FIELD,
            SECOND_FIELD,
        ],
        _ => &[
            MINUTE_FIELD,
            HOUR_FIELD,
            DAY_FIELD,
            MONTH_FIELD,
            DOW_FIELD,
            SECOND_FIELD,
            YEAR_FIELD,
        ],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn leap_years_match_python() {
        assert!(is_leap(2000));
        assert!(!is_leap(1900));
        assert!(is_leap(2024));
        assert!(!is_leap(2025));
    }

    #[test]
    fn february_length_honours_leap() {
        assert_eq!(last_day_of_month(2024, 2), 29);
        assert_eq!(last_day_of_month(2025, 2), 28);
        assert_eq!(last_day_of_month(2025, 1), 31);
        assert_eq!(last_day_of_month(2025, 4), 30);
    }
}
