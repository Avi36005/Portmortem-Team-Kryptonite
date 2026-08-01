//! PyO3 bridge: exposes `croniter-core` to the *unmodified* original pytest
//! suite. This crate is test-only and is not part of the shipped artifact —
//! the shipped artifact is `core`'s `croniter` binary, which links no Python.
//!
//! Rules for this file: no cron logic that belongs in `core`, and never
//! swallow an error. Every failure is re-raised as the exact Python exception
//! type the tests assert on.

use pyo3::create_exception;
use pyo3::exceptions::{PyStopIteration, PyTypeError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::{PyDict, PyTuple, PyType};

use croniter_core::api::{CronIterator, Options, RetType, WallClock};
use croniter_core::calc as core_calc;
use croniter_core::consts;

mod clock;
mod convert;
use clock::{py_dt_to_naive, ts_to_py_dt, PyTzClock};
use convert::*;

create_exception!(croniter, CroniterError, PyValueError);
create_exception!(croniter, CroniterBadTypeRangeError, PyTypeError);
create_exception!(croniter, CroniterBadCronError, CroniterError);
create_exception!(
    croniter,
    CroniterUnsupportedSyntaxError,
    CroniterBadCronError
);
create_exception!(croniter, CroniterBadDateError, CroniterError);
create_exception!(croniter, CroniterNotAlphaError, CroniterBadCronError);

/// Is `t` acceptable as a `ret_type`? croniter allows `float` or `datetime`.
fn ret_type_of(py: Python<'_>, t: &Bound<'_, PyAny>) -> PyResult<RetType> {
    let datetime_cls = py.import("datetime")?.getattr("datetime")?;
    if t.is_instance_of::<PyType>() {
        let ty = t.cast::<PyType>()?;
        if ty.is_subclass(&datetime_cls)? {
            return Ok(RetType::DateTime);
        }
        let float_cls = py.get_type::<pyo3::types::PyFloat>();
        if ty.is_subclass(&float_cls)? {
            return Ok(RetType::Float);
        }
    }
    Err(PyTypeError::new_err(
        "Invalid ret_type, only 'float' or 'datetime' is acceptable.",
    ))
}

/// Resolve `start_time` (datetime | float | int | None) into a timestamp and,
/// for datetimes, the `tzinfo` that came with it.
fn start_time_from_py<'py>(
    py: Python<'py>,
    start_time: Option<&Bound<'py, PyAny>>,
) -> PyResult<(f64, Option<Bound<'py, PyAny>>)> {
    let Some(obj) = start_time.filter(|o| !o.is_none()) else {
        let now: f64 = py.import("time")?.call_method0("time")?.extract()?;
        return Ok((now, None));
    };
    let datetime_cls = py.import("datetime")?.getattr("datetime")?;
    if obj.is_instance(&datetime_cls)? {
        let tzinfo = obj.getattr("tzinfo")?;
        let tzinfo = if tzinfo.is_none() { None } else { Some(tzinfo) };
        let wall = py_dt_to_naive(obj)?;
        let clock = PyTzClock::new(py, tzinfo.clone());
        let ts = clock.from_wall(wall).map_err(to_pyerr)?;
        return Ok((ts, tzinfo));
    }
    Ok((obj.extract::<f64>()?, None))
}

#[pyclass(name = "croniter", subclass)]
struct Croniter {
    inner: CronIterator,
    tzinfo: Option<Py<PyAny>>,
    ret_type: Py<PyAny>,
    expand_from_start_time: bool,
}

impl Croniter {
    fn clock<'py>(&self, py: Python<'py>) -> PyTzClock<'py> {
        PyTzClock::new(py, self.tzinfo.as_ref().map(|t| t.bind(py).clone()))
    }

    /// `croniter._get_next` (croniter.py:405), including the `ret_type` gate.
    fn do_step<'py>(
        &mut self,
        py: Python<'py>,
        ret_type: Option<&Bound<'py, PyAny>>,
        start_time: Option<&Bound<'py, PyAny>>,
        is_prev: Option<bool>,
        update_current: bool,
    ) -> PyResult<Bound<'py, PyAny>> {
        let effective = match ret_type.filter(|r| !r.is_none()) {
            Some(r) => ret_type_of(py, r)?,
            None => ret_type_of(py, self.ret_type.bind(py))?,
        };

        let start_ts = match start_time.filter(|s| !s.is_none()) {
            Some(s) => {
                let (ts, tz) = start_time_from_py(py, Some(s))?;
                self.tzinfo = tz.map(|t| t.unbind());
                Some(ts)
            }
            None => None,
        };

        let clock = self.clock(py);
        let (wall, ts) = self
            .inner
            .step(&clock, is_prev, start_ts, update_current)
            .map_err(to_pyerr)?;

        match effective {
            RetType::Float => Ok(ts.into_pyobject(py)?.into_any()),
            RetType::DateTime => {
                let _ = wall;
                ts_to_py_dt(py, ts, self.tzinfo.as_ref().map(|t| t.bind(py)))
            }
        }
    }
}

#[pymethods]
impl Croniter {
    #[new]
    #[pyo3(signature = (
        expr_format,
        start_time=None,
        ret_type=None,
        day_or=true,
        max_years_between_matches=None,
        is_prev=false,
        hash_id=None,
        implement_cron_bug=false,
        second_at_beginning=false,
        expand_from_start_time=false,
    ))]
    #[allow(clippy::too_many_arguments)]
    fn new(
        py: Python<'_>,
        expr_format: &str,
        start_time: Option<&Bound<'_, PyAny>>,
        ret_type: Option<&Bound<'_, PyAny>>,
        day_or: bool,
        max_years_between_matches: Option<i64>,
        is_prev: bool,
        hash_id: Option<&Bound<'_, PyAny>>,
        implement_cron_bug: bool,
        second_at_beginning: bool,
        expand_from_start_time: bool,
    ) -> PyResult<Self> {
        let hash_id = hash_id_from_py(hash_id)?;
        let (start_ts, tzinfo) = start_time_from_py(py, start_time)?;

        let ret_type_obj: Py<PyAny> = match ret_type.filter(|r| !r.is_none()) {
            Some(r) => r.clone().unbind(),
            None => py.get_type::<pyo3::types::PyFloat>().into_any().unbind(),
        };
        let rt = ret_type_of(py, ret_type_obj.bind(py))?;

        let opts = Options {
            ret_type: rt,
            day_or,
            max_years_between_matches,
            is_prev,
            hash_id,
            implement_cron_bug,
            second_at_beginning,
            expand_from_start_time,
        };
        let inner = CronIterator::new(expr_format, start_ts, opts).map_err(to_pyerr)?;
        Ok(Croniter {
            inner,
            tzinfo: tzinfo.map(|t| t.unbind()),
            ret_type: ret_type_obj,
            expand_from_start_time,
        })
    }

    #[pyo3(signature = (ret_type=None, start_time=None, update_current=true))]
    fn get_next<'py>(
        &mut self,
        py: Python<'py>,
        ret_type: Option<&Bound<'py, PyAny>>,
        start_time: Option<&Bound<'py, PyAny>>,
        update_current: bool,
    ) -> PyResult<Bound<'py, PyAny>> {
        if start_time.is_some_and(|s| !s.is_none()) && self.expand_from_start_time {
            return Err(PyValueError::new_err(
                "start_time is not supported when using expand_from_start_time = True.",
            ));
        }
        self.do_step(py, ret_type, start_time, Some(false), update_current)
    }

    #[pyo3(signature = (ret_type=None, start_time=None, update_current=true))]
    fn get_prev<'py>(
        &mut self,
        py: Python<'py>,
        ret_type: Option<&Bound<'py, PyAny>>,
        start_time: Option<&Bound<'py, PyAny>>,
        update_current: bool,
    ) -> PyResult<Bound<'py, PyAny>> {
        self.do_step(py, ret_type, start_time, Some(true), update_current)
    }

    #[pyo3(signature = (ret_type=None))]
    fn get_current<'py>(
        &self,
        py: Python<'py>,
        ret_type: Option<&Bound<'py, PyAny>>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let effective = match ret_type.filter(|r| !r.is_none()) {
            Some(r) => ret_type_of(py, r)?,
            None => ret_type_of(py, self.ret_type.bind(py))?,
        };
        match effective {
            RetType::Float => Ok(self.inner.cur.into_pyobject(py)?.into_any()),
            RetType::DateTime => {
                ts_to_py_dt(py, self.inner.cur, self.tzinfo.as_ref().map(|t| t.bind(py)))
            }
        }
    }

    #[pyo3(signature = (start_time, force=true))]
    fn set_current(
        &mut self,
        py: Python<'_>,
        start_time: Option<&Bound<'_, PyAny>>,
        force: bool,
    ) -> PyResult<f64> {
        if force && start_time.is_some_and(|s| !s.is_none()) {
            let (ts, tz) = start_time_from_py(py, start_time)?;
            if let Some(obj) = start_time {
                let datetime_cls = py.import("datetime")?.getattr("datetime")?;
                if obj.is_instance(&datetime_cls)? {
                    self.tzinfo = tz.map(|t| t.unbind());
                }
            }
            self.inner.set_current(ts);
        }
        Ok(self.inner.cur)
    }

    #[pyo3(signature = (ret_type=None, start_time=None, update_current=None))]
    fn all_next(
        slf: Py<Self>,
        py: Python<'_>,
        ret_type: Option<Py<PyAny>>,
        start_time: Option<Py<PyAny>>,
        update_current: Option<bool>,
    ) -> PyResult<CroniterStream> {
        let _ = py;
        Ok(CroniterStream {
            parent: slf,
            is_prev: false,
            ret_type,
            start_time,
            update_current: update_current.unwrap_or(true),
        })
    }

    #[pyo3(signature = (ret_type=None, start_time=None, update_current=None))]
    fn all_prev(
        slf: Py<Self>,
        py: Python<'_>,
        ret_type: Option<Py<PyAny>>,
        start_time: Option<Py<PyAny>>,
        update_current: Option<bool>,
    ) -> PyResult<CroniterStream> {
        let _ = py;
        Ok(CroniterStream {
            parent: slf,
            is_prev: true,
            ret_type,
            start_time,
            update_current: update_current.unwrap_or(true),
        })
    }

    /// `croniter.iter` returns the *bound generator function*, not its result.
    #[pyo3(signature = (*_args, **_kwargs))]
    fn iter<'py>(
        slf: &Bound<'py, Self>,
        _args: &Bound<'py, PyAny>,
        _kwargs: Option<&Bound<'py, PyAny>>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let is_prev = slf.borrow().inner.is_prev;
        slf.getattr(if is_prev { "all_prev" } else { "all_next" })
    }

    fn __iter__(slf: Py<Self>) -> Py<Self> {
        slf
    }

    fn __next__<'py>(&mut self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        self.do_step(py, None, None, None, true)
    }

    #[pyo3(name = "next", signature = (ret_type=None, start_time=None, update_current=true))]
    fn py_next<'py>(
        &mut self,
        py: Python<'py>,
        ret_type: Option<&Bound<'py, PyAny>>,
        start_time: Option<&Bound<'py, PyAny>>,
        update_current: bool,
    ) -> PyResult<Bound<'py, PyAny>> {
        self.do_step(py, ret_type, start_time, None, update_current)
    }

    // ---- attributes the tests read -------------------------------------

    #[getter]
    fn expanded<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        Ok(expanded_to_py(py, &self.inner.cron.expanded)?.into_any())
    }

    #[getter]
    fn nth_weekday_of_month<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        Ok(nth_map_to_py(py, &self.inner.cron.nth_weekday_of_month)?.into_any())
    }

    #[getter]
    fn expressions(&self) -> Vec<String> {
        self.inner.cron.expressions.clone()
    }

    #[getter]
    fn cur(&self) -> f64 {
        self.inner.cur
    }

    #[getter]
    fn start_time(&self) -> f64 {
        self.inner.start_time
    }

    #[getter]
    fn dst_start_time(&self) -> f64 {
        self.inner.dst_start_time
    }

    #[getter]
    fn tzinfo(&self, py: Python<'_>) -> Option<Py<PyAny>> {
        self.tzinfo.as_ref().map(|t| t.clone_ref(py))
    }

    #[getter]
    fn fields(&self) -> Vec<usize> {
        consts::cron_fields(self.inner.cron.len()).to_vec()
    }

    // ---- class/static methods -------------------------------------------

    /// `croniter.expand` (croniter.py:1240).
    #[classmethod]
    #[pyo3(signature = (expr_format, hash_id=None, second_at_beginning=false, from_timestamp=None, strict=false, strict_year=None))]
    fn expand<'py>(
        cls: &Bound<'py, PyType>,
        expr_format: &str,
        hash_id: Option<&Bound<'py, PyAny>>,
        second_at_beginning: bool,
        from_timestamp: Option<f64>,
        strict: bool,
        strict_year: Option<&Bound<'py, PyAny>>,
    ) -> PyResult<(Bound<'py, PyAny>, Bound<'py, PyAny>)> {
        let py = cls.py();
        let hash_id = hash_id_from_py(hash_id)?;
        let strict_year = strict_year_from_py(strict_year)?;
        let result = croniter_core::expand(
            expr_format,
            hash_id.as_deref(),
            second_at_beginning,
            from_timestamp,
            strict,
            strict_year.as_deref(),
        )
        .map_err(to_pyerr)?;
        Ok((
            expanded_to_py(py, &result.expanded)?.into_any(),
            nth_map_to_py(py, &result.nth_weekday_of_month)?.into_any(),
        ))
    }

    /// `croniter.is_valid` (croniter.py:1320).
    #[classmethod]
    #[pyo3(signature = (expression, hash_id=None, encoding="UTF-8", second_at_beginning=false, strict=false, strict_year=None))]
    fn is_valid(
        cls: &Bound<'_, PyType>,
        expression: &str,
        hash_id: Option<&Bound<'_, PyAny>>,
        encoding: &str,
        second_at_beginning: bool,
        strict: bool,
        strict_year: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<bool> {
        let _ = (cls, encoding);
        let hash_id = hash_id_from_py(hash_id)?;
        let strict_year = strict_year_from_py(strict_year)?;
        Ok(croniter_core::is_valid(
            expression,
            hash_id.as_deref(),
            second_at_beginning,
            strict,
            strict_year.as_deref(),
        ))
    }

    /// `croniter.match` (croniter.py:1333).
    #[classmethod]
    #[pyo3(name = "match", signature = (cron_expression, testdate, day_or=true, second_at_beginning=false, precision_in_seconds=None))]
    fn py_match(
        cls: &Bound<'_, PyType>,
        cron_expression: &str,
        testdate: &Bound<'_, PyAny>,
        day_or: bool,
        second_at_beginning: bool,
        precision_in_seconds: Option<f64>,
    ) -> PyResult<bool> {
        Self::match_range(
            cls,
            cron_expression,
            testdate,
            testdate,
            day_or,
            second_at_beginning,
            precision_in_seconds,
        )
    }

    /// `croniter.match_range` (croniter.py:1346).
    #[classmethod]
    #[pyo3(signature = (cron_expression, from_datetime, to_datetime, day_or=true, second_at_beginning=false, precision_in_seconds=None))]
    fn match_range(
        cls: &Bound<'_, PyType>,
        cron_expression: &str,
        from_datetime: &Bound<'_, PyAny>,
        to_datetime: &Bound<'_, PyAny>,
        day_or: bool,
        second_at_beginning: bool,
        precision_in_seconds: Option<f64>,
    ) -> PyResult<bool> {
        let py = cls.py();
        let tzinfo = to_datetime.getattr("tzinfo").ok().filter(|t| !t.is_none());
        let clock = PyTzClock::new(py, tzinfo);
        let from_wall = py_dt_to_naive(from_datetime)?;
        let to_wall = py_dt_to_naive(to_datetime)?;
        croniter_core::matcher::match_range(
            cron_expression,
            from_wall,
            to_wall,
            &clock,
            day_or,
            second_at_beginning,
            precision_in_seconds,
        )
        .map_err(to_pyerr)
    }

    /// `croniter._get_nth_weekday_of_month` (croniter.py:885).
    ///
    /// Returns a *tuple*, not a list — croniter does `return tuple(...)` and
    /// `test_nth_wday_simple` compares against tuple literals.
    #[staticmethod]
    fn _get_nth_weekday_of_month<'py>(
        py: Python<'py>,
        year: i64,
        month: i64,
        day_of_week: i64,
    ) -> PyResult<Bound<'py, PyTuple>> {
        PyTuple::new(
            py,
            core_calc::nth_weekday_of_month(year, month, day_of_week),
        )
    }

    /// Tests reach in and lower this to force the "no match found" path
    /// (`test_explicit_year_forward`), so it has to be writable.
    #[getter]
    fn _max_years_between_matches(&self) -> i64 {
        self.inner.cron.max_years_between_matches
    }

    // PyO3 derives the attribute name from the method name, and the attribute
    // croniter exposes starts with an underscore. Hence the non-snake-case name.
    #[setter]
    #[allow(non_snake_case)]
    fn set__max_years_between_matches(&mut self, value: i64) {
        self.inner.cron.max_years_between_matches = value.max(1);
    }

    #[getter]
    fn _max_years_btw_matches_explicitly_set(&self) -> bool {
        self.inner.cron.max_years_explicitly_set
    }

    #[setter]
    #[allow(non_snake_case)]
    fn set__max_years_btw_matches_explicitly_set(&mut self, value: bool) {
        self.inner.cron.max_years_explicitly_set = value;
    }

    #[staticmethod]
    fn datetime_to_timestamp(d: &Bound<'_, PyAny>) -> PyResult<f64> {
        py_datetime_to_timestamp(d)
    }

    #[staticmethod]
    fn _datetime_to_timestamp(d: &Bound<'_, PyAny>) -> PyResult<f64> {
        py_datetime_to_timestamp(d)
    }

    #[pyo3(signature = (timestamp, tzinfo=None))]
    fn timestamp_to_datetime<'py>(
        &self,
        py: Python<'py>,
        timestamp: f64,
        tzinfo: Option<&Bound<'py, PyAny>>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let tz = match tzinfo {
            Some(t) => Some(t.clone()),
            None => self.tzinfo.as_ref().map(|t| t.bind(py).clone()),
        };
        ts_to_py_dt(py, timestamp, tz.as_ref())
    }
}

/// The object returned by `all_next` / `all_prev`.
///
/// croniter implements these as generators that stop silently on
/// `CroniterBadDateError` **only** when `max_years_between_matches` was passed
/// explicitly, and re-raise otherwise. That asymmetry is load-bearing for
/// `croniter_range`, so it is reproduced exactly.
#[pyclass]
struct CroniterStream {
    parent: Py<Croniter>,
    is_prev: bool,
    ret_type: Option<Py<PyAny>>,
    start_time: Option<Py<PyAny>>,
    update_current: bool,
}

#[pymethods]
impl CroniterStream {
    fn __iter__(slf: PyRef<'_, Self>) -> PyRef<'_, Self> {
        slf
    }

    fn __next__<'py>(mut slf: PyRefMut<'py, Self>, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let is_prev = slf.is_prev;
        let update_current = slf.update_current;
        let ret_type = slf.ret_type.as_ref().map(|r| r.bind(py).clone());
        let start_time = slf.start_time.take();
        let parent = slf.parent.clone_ref(py);

        let mut borrowed = parent.borrow_mut(py);
        borrowed.inner.is_prev = is_prev;
        let start_bound = start_time.as_ref().map(|s| s.bind(py).clone());
        let explicitly_set = borrowed.inner.max_years_explicitly_set();

        match borrowed.do_step(
            py,
            ret_type.as_ref(),
            start_bound.as_ref(),
            Some(is_prev),
            update_current,
        ) {
            Ok(v) => Ok(v),
            Err(e) if e.is_instance_of::<CroniterBadDateError>(py) => {
                if explicitly_set {
                    Err(PyStopIteration::new_err(()))
                } else {
                    Err(e)
                }
            }
            Err(e) => Err(e),
        }
    }
}

/// Lazy generator returned by `croniter_range`.
///
/// The bound arithmetic comes from `croniter_core::range::setup`, the same
/// function the native Rust range uses, so the two cannot drift. The stepping
/// is driven through the *Python* iterator object because `croniter_range`
/// accepts a `_croniter` parameter that may be an arbitrary subclass — see
/// `test_croniter_range_derived_class`.
#[pyclass]
struct CroniterRangeIter {
    ic: Py<PyAny>,
    stop: Py<PyAny>,
    forward: bool,
    ret_float: bool,
    done: bool,
}

#[pymethods]
impl CroniterRangeIter {
    fn __iter__(slf: PyRef<'_, Self>) -> PyRef<'_, Self> {
        slf
    }

    fn __next__<'py>(mut slf: PyRefMut<'py, Self>, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        if slf.done {
            return Err(PyStopIteration::new_err(()));
        }
        let ic = slf.ic.bind(py).clone();
        let method = if slf.forward { "get_next" } else { "get_prev" };

        let dt = match ic.call_method0(method) {
            Ok(v) => v,
            Err(e) if e.is_instance_of::<CroniterBadDateError>(py) => {
                // No match within the year span: the generator ends quietly.
                slf.done = true;
                return Err(PyStopIteration::new_err(()));
            }
            Err(e) => return Err(e),
        };

        // Compare with Python's own datetime ordering so aware/naive and
        // cross-offset comparisons behave exactly as they do upstream.
        let stop = slf.stop.bind(py);
        let keep: bool = if slf.forward {
            dt.lt(stop)?
        } else {
            dt.gt(stop)?
        };
        if !keep {
            slf.done = true;
            return Err(PyStopIteration::new_err(()));
        }

        if slf.ret_float {
            let float_cls = py.get_type::<pyo3::types::PyFloat>();
            return ic.call_method1("get_current", (float_cls,));
        }
        Ok(dt)
    }
}

/// `croniter_range` (croniter.py:1376).
#[pyfunction]
#[pyo3(signature = (start, stop, expr_format, ret_type=None, day_or=true, exclude_ends=false, _croniter=None, second_at_beginning=false, expand_from_start_time=false))]
#[allow(clippy::too_many_arguments)]
fn croniter_range<'py>(
    py: Python<'py>,
    start: &Bound<'py, PyAny>,
    stop: &Bound<'py, PyAny>,
    expr_format: &str,
    ret_type: Option<&Bound<'py, PyAny>>,
    day_or: bool,
    exclude_ends: bool,
    _croniter: Option<&Bound<'py, PyAny>>,
    second_at_beginning: bool,
    expand_from_start_time: bool,
) -> PyResult<CroniterRangeIter> {
    let datetime_mod = py.import("datetime")?;
    let datetime_cls = datetime_mod.getattr("datetime")?;
    let timedelta_cls = datetime_mod.getattr("timedelta")?;
    let float_cls = py.get_type::<pyo3::types::PyFloat>();

    // start and stop must be the same kind of object.
    let start_ty = start.get_type();
    let stop_ty = stop.get_type();
    let same_type =
        start_ty.is(&stop_ty) || start.is_instance(&stop_ty)? || stop.is_instance(&start_ty)?;
    if !same_type {
        return Err(CroniterBadTypeRangeError::new_err(format!(
            "The start and stop must be same type.  {} != {}",
            start_ty.str()?,
            stop_ty.str()?
        )));
    }

    // Numeric bounds are read as UTC and yield floats by default.
    let numeric = !start.is_instance(&datetime_cls)?
        && (start.extract::<f64>().is_ok() && !start.is_instance_of::<pyo3::types::PyBool>());
    let (mut start_dt, mut stop_dt, auto_float) = if numeric {
        let utc = datetime_mod.getattr("timezone")?.getattr("utc")?;
        let kwargs = PyDict::new(py);
        kwargs.set_item("tzinfo", py.None())?;
        let conv = |o: &Bound<'py, PyAny>| -> PyResult<Bound<'py, PyAny>> {
            datetime_cls
                .call_method1("fromtimestamp", (o.extract::<f64>()?, &utc))?
                .call_method("replace", (), Some(&kwargs))
        };
        (conv(start)?, conv(stop)?, true)
    } else {
        (start.clone(), stop.clone(), false)
    };

    let ret_float = match ret_type.filter(|r| !r.is_none()) {
        Some(r) => r.is(&float_cls),
        None => auto_float,
    };

    let s = croniter_core::range::setup(
        py_dt_to_naive(&start_dt)?,
        py_dt_to_naive(&stop_dt)?,
        exclude_ends,
    );
    if s.start_nudge_us != 0 {
        let kwargs = PyDict::new(py);
        kwargs.set_item("microseconds", s.start_nudge_us)?;
        start_dt = start_dt.add(timedelta_cls.call((), Some(&kwargs))?)?;
    }
    if s.stop_nudge_us != 0 {
        let kwargs = PyDict::new(py);
        kwargs.set_item("microseconds", s.stop_nudge_us)?;
        stop_dt = stop_dt.add(timedelta_cls.call((), Some(&kwargs))?)?;
    }

    let cls: Bound<'py, PyAny> = match _croniter.filter(|c| !c.is_none()) {
        Some(c) => c.clone(),
        None => py.get_type::<Croniter>().into_any(),
    };
    let kwargs = PyDict::new(py);
    kwargs.set_item("ret_type", &datetime_cls)?;
    kwargs.set_item("day_or", day_or)?;
    kwargs.set_item("max_years_between_matches", s.year_span)?;
    kwargs.set_item("second_at_beginning", second_at_beginning)?;
    kwargs.set_item("expand_from_start_time", expand_from_start_time)?;
    let ic = cls.call((expr_format, &start_dt), Some(&kwargs))?;

    Ok(CroniterRangeIter {
        ic: ic.unbind(),
        stop: stop_dt.unbind(),
        forward: s.forward,
        ret_float,
        done: false,
    })
}

#[pyfunction]
fn datetime_to_timestamp(d: &Bound<'_, PyAny>) -> PyResult<f64> {
    py_datetime_to_timestamp(d)
}

#[pymodule]
fn croniter(m: &Bound<'_, PyModule>) -> PyResult<()> {
    let py = m.py();

    m.add("MINUTE_FIELD", consts::MINUTE_FIELD)?;
    m.add("HOUR_FIELD", consts::HOUR_FIELD)?;
    m.add("DAY_FIELD", consts::DAY_FIELD)?;
    m.add("MONTH_FIELD", consts::MONTH_FIELD)?;
    m.add("DOW_FIELD", consts::DOW_FIELD)?;
    m.add("SECOND_FIELD", consts::SECOND_FIELD)?;
    m.add("YEAR_FIELD", consts::YEAR_FIELD)?;
    m.add("UNIX_CRON_LEN", consts::UNIX_CRON_LEN)?;
    m.add("SECOND_CRON_LEN", consts::SECOND_CRON_LEN)?;
    m.add("YEAR_CRON_LEN", consts::YEAR_CRON_LEN)?;

    let valid = pyo3::types::PySet::empty(py)?;
    for n in consts::VALID_LEN_EXPRESSION {
        valid.add(n)?;
    }
    m.add("VALID_LEN_EXPRESSION", valid)?;

    m.add("OVERFLOW32B_MODE", false)?;
    m.add(
        "UTC_DT",
        py.import("datetime")?.getattr("timezone")?.getattr("utc")?,
    )?;

    let ranges = PyTuple::new(py, consts::RANGES.iter().map(|(a, b)| (*a, *b)))?;
    let croniter_type = py.get_type::<Croniter>();
    croniter_type.setattr("RANGES", ranges)?;
    croniter_type.setattr("MONTHS_IN_YEAR", consts::MONTHS_IN_YEAR)?;
    croniter_type.setattr("LEN_MEANS_ALL", consts::LEN_MEANS_ALL.to_vec())?;

    m.add("CroniterError", py.get_type::<CroniterError>())?;
    m.add(
        "CroniterBadTypeRangeError",
        py.get_type::<CroniterBadTypeRangeError>(),
    )?;
    m.add(
        "CroniterBadCronError",
        py.get_type::<CroniterBadCronError>(),
    )?;
    m.add(
        "CroniterUnsupportedSyntaxError",
        py.get_type::<CroniterUnsupportedSyntaxError>(),
    )?;
    m.add(
        "CroniterBadDateError",
        py.get_type::<CroniterBadDateError>(),
    )?;
    m.add(
        "CroniterNotAlphaError",
        py.get_type::<CroniterNotAlphaError>(),
    )?;

    m.add_class::<Croniter>()?;
    m.add_class::<CroniterStream>()?;
    m.add_class::<CroniterRangeIter>()?;
    m.add_function(wrap_pyfunction!(croniter_range, m)?)?;
    m.add_function(wrap_pyfunction!(datetime_to_timestamp, m)?)?;

    let _ = PyDict::new(py);
    Ok(())
}
