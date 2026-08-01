//! Error model.
//!
//! Mirrors croniter's Python exception hierarchy:
//!
//! ```text
//! CroniterError(ValueError)
//! CroniterBadTypeRangeError(TypeError)
//! CroniterBadCronError(CroniterError)
//! CroniterUnsupportedSyntaxError(CroniterBadCronError)
//! CroniterBadDateError(CroniterError)
//! CroniterNotAlphaError(CroniterBadCronError)
//! ```
//!
//! Rust has no exception subclassing, so the hierarchy is encoded as a flat
//! enum here and re-raised as the correct Python type by `pybridge`. Tests
//! assert on exception *type*, so this mapping has to stay exact.

use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum CroniterError {
    /// `CroniterBadCronError` — syntax, unknown value, or range error.
    #[error("{0}")]
    BadCron(String),

    /// `CroniterUnsupportedSyntaxError` — valid syntax, inaccurate results.
    /// Subclass of `CroniterBadCronError` in Python.
    #[error("{0}")]
    UnsupportedSyntax(String),

    /// `CroniterNotAlphaError` — invalid day or month abbreviation.
    /// Subclass of `CroniterBadCronError` in Python.
    #[error("{0}")]
    NotAlpha(String),

    /// `CroniterBadDateError` — unable to find a next/prev match.
    #[error("{0}")]
    BadDate(String),

    /// `CroniterBadTypeRangeError` — derives from `TypeError`, not `ValueError`.
    #[error("{0}")]
    BadTypeRange(String),

    /// A plain `ValueError` — **not** `CroniterError`.
    ///
    /// croniter raises a bare `ValueError` in at least one place that matters:
    /// `_get_low_from_current_date_number` (croniter.py:1317) rejects a field
    /// index above 4. Reached through `croniter(...)` that propagates as a
    /// plain `ValueError`, because `__init__` calls `_expand` directly rather
    /// than the `expand` classmethod that would rewrap it. Mapping this onto
    /// `CroniterError` instead was a real divergence, found by the fuzzer.
    #[error("{0}")]
    Value(String),

    /// A plain `TypeError`.
    #[error("{0}")]
    Type(String),

    /// An exception raised by host code the bridge called into (a `tzinfo`
    /// object, `datetime`, `pytz`). `class` is the exception's type name so
    /// the bridge can re-raise the *same* type rather than flattening an
    /// `OverflowError` from an out-of-range timestamp into a `ValueError`.
    ///
    /// `core` never constructs this — it only carries it back out.
    #[error("{msg}")]
    Foreign { class: String, msg: String },
}

pub type Result<T> = std::result::Result<T, CroniterError>;
