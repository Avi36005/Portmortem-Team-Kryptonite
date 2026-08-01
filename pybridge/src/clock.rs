//! `WallClock` implemented against a real Python `tzinfo` object.
//!
//! croniter's timezone behaviour is defined by whatever `tzinfo` the caller
//! passed — `zoneinfo`, `pytz` or `dateutil.tz` — and those three do not agree
//! with each other on ambiguous local times. Rather than ship a fourth opinion,
//! the bridge asks the very object the test supplied.
//!
//! The cron *search* stays in Rust; this only converts between the float
//! timestamp and the wall-clock reading, which is exactly what
//! `timestamp_to_datetime` / `datetime_to_timestamp` do in croniter.py.

use chrono::{Datelike, NaiveDate, NaiveDateTime, Timelike};
use pyo3::prelude::*;
use pyo3::types::PyDict;

use croniter_core::api::{is_successor, Resolved, WallClock};
use croniter_core::calc::{naive_to_timestamp, timestamp_to_naive};
use croniter_core::error::{CroniterError, Result as CronResult};

pub struct PyTzClock<'py> {
    py: Python<'py>,
    tzinfo: Option<Bound<'py, PyAny>>,
}

impl<'py> PyTzClock<'py> {
    pub fn new(py: Python<'py>, tzinfo: Option<Bound<'py, PyAny>>) -> Self {
        let tzinfo = tzinfo.filter(|t| !t.is_none());
        PyTzClock { py, tzinfo }
    }
}

/// Carry a Python exception back through `core` without losing its type.
///
/// These come from `tzinfo` objects, `datetime` and `pytz` — an out-of-range
/// timestamp raises `OverflowError`, not `ValueError`, and the original test
/// suite distinguishes them. Recording the class name lets `to_pyerr` re-raise
/// the same type on the way out.
fn as_cron_err(e: PyErr) -> CroniterError {
    let (class, msg) = Python::attach(|py| {
        (
            e.get_type(py)
                .name()
                .map(|n| n.to_string())
                .unwrap_or_else(|_| "ValueError".into()),
            e.value(py)
                .str()
                .map(|s| s.to_string())
                .unwrap_or_else(|_| e.to_string()),
        )
    });
    CroniterError::Foreign { class, msg }
}

/// Pull `(y, m, d, H, M, S, us)` off a Python datetime into a NaiveDateTime.
pub fn py_dt_to_naive(dt: &Bound<'_, PyAny>) -> PyResult<NaiveDateTime> {
    let y: i32 = dt.getattr("year")?.extract()?;
    let mo: u32 = dt.getattr("month")?.extract()?;
    let d: u32 = dt.getattr("day")?.extract()?;
    let h: u32 = dt.getattr("hour")?.extract()?;
    let mi: u32 = dt.getattr("minute")?.extract()?;
    let s: u32 = dt.getattr("second")?.extract()?;
    let us: u32 = dt.getattr("microsecond")?.extract()?;
    NaiveDate::from_ymd_opt(y, mo, d)
        .and_then(|date| date.and_hms_micro_opt(h, mi, s, us))
        .ok_or_else(|| pyo3::exceptions::PyValueError::new_err("datetime out of range"))
}

/// Build a Python `datetime` from a wall-clock reading, attaching `tzinfo`
/// the way croniter does (`replace(tzinfo=UTC).astimezone(tz)`, or pytz's
/// `localize` when the object provides it).
pub fn naive_to_py_dt<'py>(
    py: Python<'py>,
    wall: NaiveDateTime,
    tzinfo: Option<&Bound<'py, PyAny>>,
) -> PyResult<Bound<'py, PyAny>> {
    let datetime_mod = py.import("datetime")?;
    let naive = datetime_mod.getattr("datetime")?.call1((
        wall.year(),
        wall.month(),
        wall.day(),
        wall.hour(),
        wall.minute(),
        wall.second(),
        wall.and_utc().timestamp_subsec_micros(),
    ))?;

    let Some(tz) = tzinfo.filter(|t| !t.is_none()) else {
        return Ok(naive);
    };

    // pytz zones must be attached with localize(); everything else uses replace().
    if let Ok(localize) = tz.getattr("localize") {
        if !localize.is_none() {
            return localize.call1((naive,));
        }
    }
    let kwargs = PyDict::new(py);
    kwargs.set_item("tzinfo", tz)?;
    naive.call_method("replace", (), Some(&kwargs))
}

/// `croniter.timestamp_to_datetime` (croniter.py:387) — render an instant.
///
/// This must go through the timestamp, not through the wall-clock reading:
/// `astimezone` is what sets `fold` on the second pass of an ambiguous hour.
/// Rebuilding from wall fields and `replace(tzinfo=...)` silently yields
/// `fold=0` and therefore the pre-transition offset, which is the same instant
/// printed with the wrong UTC offset.
pub fn ts_to_py_dt<'py>(
    py: Python<'py>,
    ts: f64,
    tzinfo: Option<&Bound<'py, PyAny>>,
) -> PyResult<Bound<'py, PyAny>> {
    let datetime_mod = py.import("datetime")?;
    let utc = datetime_mod.getattr("timezone")?.getattr("utc")?;
    let aware_utc = datetime_mod
        .getattr("datetime")?
        .call_method1("fromtimestamp", (ts, &utc))?;

    let Some(tz) = tzinfo.filter(|t| !t.is_none()) else {
        let kwargs = PyDict::new(py);
        kwargs.set_item("tzinfo", py.None())?;
        return aware_utc.call_method("replace", (), Some(&kwargs));
    };
    aware_utc.call_method1("astimezone", (tz,))
}

impl WallClock for PyTzClock<'_> {
    fn to_wall(&self, ts: f64) -> CronResult<NaiveDateTime> {
        let Some(tz) = &self.tzinfo else {
            return timestamp_to_naive(ts)
                .ok_or_else(|| CroniterError::Value("timestamp out of range".into()));
        };
        // datetime.fromtimestamp(ts, tz=utc).astimezone(tzinfo), then drop tzinfo
        let dt = (|| -> PyResult<NaiveDateTime> {
            let datetime_mod = self.py.import("datetime")?;
            let utc = datetime_mod.getattr("timezone")?.getattr("utc")?;
            let aware = datetime_mod
                .getattr("datetime")?
                .call_method1("fromtimestamp", (ts, utc))?;
            let local = aware.call_method1("astimezone", (tz,))?;
            py_dt_to_naive(&local)
        })()
        .map_err(as_cron_err)?;
        Ok(dt)
    }

    fn from_wall(&self, wall: NaiveDateTime) -> CronResult<f64> {
        let Some(_) = &self.tzinfo else {
            return Ok(naive_to_timestamp(wall));
        };
        // datetime_to_timestamp: strip tzinfo, subtract utcoffset, seconds since epoch
        (|| -> PyResult<f64> {
            let aware = naive_to_py_dt(self.py, wall, self.tzinfo.as_ref())?;
            let offset = aware.call_method0("utcoffset")?;
            let offset_secs: f64 = if offset.is_none() {
                0.0
            } else {
                offset.call_method0("total_seconds")?.extract()?
            };
            Ok(naive_to_timestamp(wall) - offset_secs)
        })()
        .map_err(as_cron_err)
    }

    fn is_aware(&self) -> bool {
        self.tzinfo.is_some()
    }

    /// `_add_tzinfo` (croniter.py:179), transcribed against the caller's own
    /// tzinfo object.
    ///
    /// pytz and everything else need different treatment, exactly as upstream:
    /// pytz signals trouble by raising `NonExistentTimeError` /
    /// `AmbiguousTimeError` from `localize(..., is_dst=None)`, while
    /// `zoneinfo`/`dateutil` express the same two cases through `fold` and
    /// `dateutil.tz.datetime_exists`.
    fn resolve(&self, wall: NaiveDateTime, prev_utc: f64, is_prev: bool) -> CronResult<Resolved> {
        let Some(tz) = &self.tzinfo else {
            return Ok(Resolved {
                wall,
                utc: naive_to_timestamp(wall),
                offset: 0,
                exists: true,
            });
        };
        self.resolve_inner(tz, wall, prev_utc, is_prev)
            .map_err(as_cron_err)
    }
}

impl PyTzClock<'_> {
    fn utc_of(&self, aware: &Bound<'_, PyAny>) -> PyResult<(f64, i64)> {
        let naive = py_dt_to_naive(aware)?;
        let offset = aware.call_method0("utcoffset")?;
        let offset_secs: f64 = if offset.is_none() {
            0.0
        } else {
            offset.call_method0("total_seconds")?.extract()?
        };
        Ok((
            naive_to_timestamp(naive) - offset_secs,
            offset_secs.round() as i64,
        ))
    }

    fn resolved_from(&self, aware: &Bound<'_, PyAny>, exists: bool) -> PyResult<Resolved> {
        let (utc, offset) = self.utc_of(aware)?;
        Ok(Resolved {
            wall: py_dt_to_naive(aware)?,
            utc,
            offset,
            exists,
        })
    }

    fn resolve_inner(
        &self,
        tz: &Bound<'_, PyAny>,
        wall: NaiveDateTime,
        prev_utc: f64,
        is_prev: bool,
    ) -> PyResult<Resolved> {
        let py = self.py;
        let datetime_mod = py.import("datetime")?;
        let timedelta = datetime_mod.getattr("timedelta")?;
        let naive = naive_to_py_dt(py, wall, None)?;

        // ---- pytz path -------------------------------------------------
        let localize = tz.getattr("localize").ok().filter(|l| !l.is_none());
        if let Some(localize) = localize {
            let pytz = py.import("pytz")?;
            let non_existent = pytz.getattr("NonExistentTimeError")?;
            let ambiguous = pytz.getattr("AmbiguousTimeError")?;

            let kwargs = PyDict::new(py);
            kwargs.set_item("is_dst", py.None())?;

            match localize.call((&naive,), Some(&kwargs)) {
                Ok(result) => return self.resolved_from(&result, true),
                Err(e) if e.is_instance(py, &non_existent) => {
                    // Skipped local time: walk forward a minute at a time.
                    let mut probe = naive.clone();
                    loop {
                        let kw = PyDict::new(py);
                        kw.set_item("minutes", 1)?;
                        probe = probe.add(timedelta.call((), Some(&kw))?)?;
                        let kwargs = PyDict::new(py);
                        kwargs.set_item("is_dst", py.None())?;
                        if let Ok(result) = localize.call((&probe,), Some(&kwargs)) {
                            return self.resolved_from(&result, false);
                        }
                    }
                }
                Err(e) if e.is_instance(py, &ambiguous) => {
                    // Happens twice: prefer the side nearer `prev`, provided
                    // it is still a successor in the direction of travel.
                    let mk = |is_dst: bool| -> PyResult<Bound<'_, PyAny>> {
                        let kw = PyDict::new(py);
                        kw.set_item("is_dst", is_dst)?;
                        localize.call((&naive,), Some(&kw))
                    };
                    let closer = mk(!is_prev)?;
                    let farther = mk(is_prev)?;
                    let (closer_utc, _) = self.utc_of(&closer)?;
                    let (farther_utc, _) = self.utc_of(&farther)?;
                    let pick = if is_successor(closer_utc, prev_utc, is_prev) {
                        closer
                    } else {
                        let _ = farther_utc;
                        farther
                    };
                    return self.resolved_from(&pick, true);
                }
                Err(e) => return Err(e),
            }
        }

        // ---- zoneinfo / dateutil path ----------------------------------
        let dateutil_tz = py.import("dateutil.tz")?;
        let datetime_exists = dateutil_tz.getattr("datetime_exists")?;

        fn with_fold<'a>(
            py: Python<'a>,
            tz: &Bound<'a, PyAny>,
            fold: u8,
            base: &Bound<'a, PyAny>,
        ) -> PyResult<Bound<'a, PyAny>> {
            let kwargs = PyDict::new(py);
            kwargs.set_item("fold", fold)?;
            kwargs.set_item("tzinfo", tz)?;
            base.call_method("replace", (), Some(&kwargs))
        }

        let mut result = with_fold(py, tz, u8::from(is_prev), &naive)?;
        if !datetime_exists.call1((&result,))?.is_truthy()? {
            while !datetime_exists.call1((&result,))?.is_truthy()? {
                let kw = PyDict::new(py);
                kw.set_item("minutes", 1)?;
                result = result.add(timedelta.call((), Some(&kw))?)?;
            }
            return self.resolved_from(&result, false);
        }

        let farther = with_fold(py, tz, u8::from(!is_prev), &naive)?;
        let (result_utc, _) = self.utc_of(&result)?;
        let (farther_utc, _) = self.utc_of(&farther)?;
        if result_utc != farther_utc && !is_successor(result_utc, prev_utc, is_prev) {
            return self.resolved_from(&farther, true);
        }
        self.resolved_from(&result, true)
    }
}
