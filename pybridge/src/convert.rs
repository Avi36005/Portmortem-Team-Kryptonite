//! Conversions between `croniter-core` types and Python objects.
//!
//! This module deliberately contains no croniter logic. It re-raises Rust
//! errors as the *exact* Python exception classes the original test suite
//! asserts on, and it never swallows an error.

use pyo3::prelude::*;
use pyo3::types::{PyBytes, PyDict, PyList, PySet, PyString};

use croniter_core::error::CroniterError;
use croniter_core::expand::{Item, Nth};

use crate::{
    CroniterBadCronError, CroniterBadDateError, CroniterBadTypeRangeError, CroniterNotAlphaError,
    CroniterUnsupportedSyntaxError,
};

/// Map a Rust `CroniterError` onto the matching Python exception type.
/// The hierarchy has to be exact — tests assert on type, not message.
pub fn to_pyerr(err: CroniterError) -> PyErr {
    match err {
        CroniterError::BadCron(m) => CroniterBadCronError::new_err(m),
        CroniterError::UnsupportedSyntax(m) => CroniterUnsupportedSyntaxError::new_err(m),
        CroniterError::NotAlpha(m) => CroniterNotAlphaError::new_err(m),
        CroniterError::BadDate(m) => CroniterBadDateError::new_err(m),
        CroniterError::BadTypeRange(m) => CroniterBadTypeRangeError::new_err(m),
        // A bare ValueError, not croniter's CroniterError subclass.
        CroniterError::Value(m) => pyo3::exceptions::PyValueError::new_err(m),
        CroniterError::Type(m) => pyo3::exceptions::PyTypeError::new_err(m),
        // Re-raise host exceptions as their original type where we can name it.
        CroniterError::Foreign { class, msg } => Python::attach(|py| {
            py.import("builtins")
                .and_then(|b| b.getattr(class.as_str()))
                .and_then(|cls| cls.call1((msg.clone(),)))
                .map(|exc| PyErr::from_value(exc))
                .unwrap_or_else(|_| pyo3::exceptions::PyValueError::new_err(msg))
        }),
    }
}

/// `Item` -> `int | "*" | "l"`, matching croniter's mixed-type lists.
pub fn item_to_py<'py>(py: Python<'py>, item: Item) -> PyResult<Bound<'py, PyAny>> {
    Ok(match item {
        Item::Star => PyString::new(py, "*").into_any(),
        Item::Last => PyString::new(py, "l").into_any(),
        Item::Num(n) => n.into_pyobject(py)?.into_any(),
    })
}

/// `Nth` -> `int | "l"`.
pub fn nth_to_py<'py>(py: Python<'py>, nth: Nth) -> PyResult<Bound<'py, PyAny>> {
    Ok(match nth {
        Nth::Last => PyString::new(py, "l").into_any(),
        Nth::Num(n) => n.into_pyobject(py)?.into_any(),
    })
}

pub fn expanded_to_py<'py>(
    py: Python<'py>,
    expanded: &[Vec<Item>],
) -> PyResult<Bound<'py, PyList>> {
    let outer = PyList::empty(py);
    for field in expanded {
        let inner = PyList::empty(py);
        for item in field {
            inner.append(item_to_py(py, *item)?)?;
        }
        outer.append(inner)?;
    }
    Ok(outer)
}

pub fn nth_map_to_py<'py>(
    py: Python<'py>,
    map: &std::collections::BTreeMap<Item, std::collections::BTreeSet<Nth>>,
) -> PyResult<Bound<'py, PyDict>> {
    let dict = PyDict::new(py);
    for (key, values) in map {
        let set = PySet::empty(py)?;
        for v in values {
            set.add(nth_to_py(py, *v)?)?;
        }
        dict.set_item(item_to_py(py, *key)?, set)?;
    }
    Ok(dict)
}

/// Accept `bytes | str | None` for `hash_id`, exactly as croniter does, and
/// raise `TypeError` for anything else.
pub fn hash_id_from_py(obj: Option<&Bound<'_, PyAny>>) -> PyResult<Option<Vec<u8>>> {
    let Some(obj) = obj else { return Ok(None) };
    if obj.is_none() {
        return Ok(None);
    }
    if let Ok(b) = obj.cast::<PyBytes>() {
        return Ok(Some(b.as_bytes().to_vec()));
    }
    if let Ok(s) = obj.cast::<PyString>() {
        return Ok(Some(s.to_cow()?.as_bytes().to_vec()));
    }
    Err(pyo3::exceptions::PyTypeError::new_err(
        "hash_id must be bytes or UTF-8 string",
    ))
}

/// `strict_year` accepts `int | Iterable[int] | None`.
pub fn strict_year_from_py(obj: Option<&Bound<'_, PyAny>>) -> PyResult<Option<Vec<i64>>> {
    let Some(obj) = obj else { return Ok(None) };
    if obj.is_none() {
        return Ok(None);
    }
    if let Ok(v) = obj.extract::<i64>() {
        return Ok(Some(vec![v]));
    }
    Ok(Some(obj.extract::<Vec<i64>>()?))
}

/// `datetime_to_timestamp` (croniter.py:142) for an arbitrary Python datetime.
///
///     if d.tzinfo is not None:
///         d = d.replace(tzinfo=None) - d.utcoffset()
///     return (d - datetime(1970, 1, 1)).total_seconds()
pub fn py_datetime_to_timestamp(d: &Bound<'_, PyAny>) -> PyResult<f64> {
    let wall = crate::clock::py_dt_to_naive(d)?;
    let base = croniter_core::calc::naive_to_timestamp(wall);
    let tzinfo = d.getattr("tzinfo")?;
    if tzinfo.is_none() {
        return Ok(base);
    }
    let offset = d.call_method0("utcoffset")?;
    if offset.is_none() {
        return Ok(base);
    }
    let offset_secs: f64 = offset.call_method0("total_seconds")?.extract()?;
    Ok(base - offset_secs)
}
