#![forbid(unsafe_code)]
//! croniter-core — a Rust port of the `croniter` cron-expression library.
//!
//! This crate is the real port and the shipped artifact. It has no Python
//! dependency of any kind: no interpreter is linked, no subprocess is spawned.
//! The `pybridge` crate (PyO3) exists solely to run the original Python test
//! suite against this code and is not part of what ships.
//!
//! The `#![forbid(unsafe_code)]` above is compiler-enforced. Do not remove it.

pub mod api;
pub mod calc;
pub mod consts;
pub mod error;
pub mod expand;
pub mod hash;
pub mod matcher;
pub mod range;
pub mod reldelta;
pub mod tz;

pub use api::{CronIterator, Options, RetType, TzSpec};
pub use calc::Croniter;
pub use error::{CroniterError, Result};
pub use expand::{expand, Expanded, Item, Nth};
pub use matcher::{is_match, is_valid, match_range};
pub use tz::TzClock;
