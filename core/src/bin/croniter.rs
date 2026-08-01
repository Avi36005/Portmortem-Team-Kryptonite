//! croniter CLI — the shipped artifact.
//!
//! Links no Python, spawns no subprocess, reads no interpreter. This binary is
//! the deliverable; the PyO3 bridge used to run the original test suite is a
//! separate crate that is not part of it.

use std::process::ExitCode;

use chrono::{NaiveDateTime, Utc};
use croniter_core::api::{CronIterator, FixedClock, Options, RetType, TzSpec, WallClock};
use croniter_core::calc::naive_to_timestamp;
use croniter_core::matcher::is_match;
use croniter_core::range::CroniterRange;
use croniter_core::tz::TzClock;

const NAIVE: FixedClock = FixedClock(TzSpec::Naive);

/// Pick the clock for this invocation. `--tz` selects a real IANA zone, with
/// full DST resolution; without it, times are naive wall-clock.
fn clock_for(tz: &Option<String>) -> Result<Box<dyn WallClock>, String> {
    match tz {
        None => Ok(Box::new(NAIVE)),
        Some(name) => TzClock::from_name(name)
            .map(|c| Box::new(c) as Box<dyn WallClock>)
            .map_err(|e| e.to_string()),
    }
}

const USAGE: &str = "\
croniter — cron expression parsing and next/prev fire times

USAGE:
    croniter next <expr> [-n COUNT] [--start ISO8601] [--tz ZONE]
    croniter prev <expr> [-n COUNT] [--start ISO8601] [--tz ZONE]
    croniter range <expr> --start ISO8601 --stop ISO8601 [--tz ZONE]
    croniter match <expr> <ISO8601> [--tz ZONE]
    croniter validate <expr>
    croniter bench [-n COUNT]

EXAMPLES:
    croniter next '0 9 * * 1-5' -n 5
    croniter next '0 12 L * *' --start 2024-02-01T00:00:00
    croniter match '*/15 * * * *' 2025-06-01T00:15:00
    croniter next '*/30 * * * *' --tz Europe/Athens --start 2013-10-27T02:00:00

Times are ISO 8601 wall-clock. Without --tz they are naive; with --tz they are
local times in that IANA zone, and DST transitions are resolved.
";

fn parse_dt(s: &str) -> Result<NaiveDateTime, String> {
    NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%S")
        .or_else(|_| NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S"))
        .or_else(|_| {
            chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d")
                .map(|d| d.and_hms_opt(0, 0, 0).expect("midnight is valid"))
        })
        .map_err(|_| format!("cannot parse datetime {s:?} (expected ISO 8601)"))
}

struct Args {
    count: usize,
    start: Option<String>,
    stop: Option<String>,
    tz: Option<String>,
}

fn parse_flags(rest: &[String]) -> Result<Args, String> {
    let mut args = Args {
        count: 10,
        start: None,
        stop: None,
        tz: None,
    };
    let mut i = 0;
    while i < rest.len() {
        match rest[i].as_str() {
            "-n" | "--count" => {
                i += 1;
                args.count = rest
                    .get(i)
                    .ok_or("-n needs a value")?
                    .parse()
                    .map_err(|_| "-n needs a number".to_string())?;
            }
            "--start" => {
                i += 1;
                args.start = Some(rest.get(i).ok_or("--start needs a value")?.clone());
            }
            "--stop" => {
                i += 1;
                args.stop = Some(rest.get(i).ok_or("--stop needs a value")?.clone());
            }
            "--tz" => {
                i += 1;
                args.tz = Some(rest.get(i).ok_or("--tz needs a value")?.clone());
            }
            other => return Err(format!("unknown option {other:?}")),
        }
        i += 1;
    }
    Ok(args)
}

fn start_or_now(args: &Args) -> Result<NaiveDateTime, String> {
    match &args.start {
        Some(s) => parse_dt(s),
        None => Ok(Utc::now().naive_utc()),
    }
}

fn run() -> Result<(), String> {
    let argv: Vec<String> = std::env::args().collect();
    if argv.len() < 2 {
        // Usage goes to stderr on the error path -- stdout is for fire times,
        // so `croniter next ... | while read` is not polluted by diagnostics.
        // An explicit `--help` still prints to stdout, below.
        eprint!("{USAGE}");
        return Err("no command given".into());
    }

    match argv[1].as_str() {
        "bench" => {
            let args = parse_flags(&argv[2..])?;
            let n = if args.start.is_none() && args.count == 10 {
                10_000
            } else {
                args.count
            };
            bench(n)
        }
        "next" | "prev" => {
            let expr = argv.get(2).ok_or("missing expression")?;
            let args = parse_flags(&argv[3..])?;
            let start = start_or_now(&args)?;
            let is_prev = argv[1] == "prev";
            let clock = clock_for(&args.tz)?;
            let mut it = CronIterator::new(
                expr,
                clock.from_wall(start).map_err(|e| e.to_string())?,
                Options {
                    ret_type: RetType::DateTime,
                    is_prev,
                    ..Default::default()
                },
            )
            .map_err(|e| e.to_string())?;
            for _ in 0..args.count {
                let (wall, _) = it
                    .step(clock.as_ref(), Some(is_prev), None, true)
                    .map_err(|e| e.to_string())?;
                println!("{}", wall.format("%Y-%m-%dT%H:%M:%S"));
            }
            Ok(())
        }
        "range" => {
            let expr = argv.get(2).ok_or("missing expression")?;
            let args = parse_flags(&argv[3..])?;
            let start = parse_dt(args.start.as_deref().ok_or("range needs --start")?)?;
            let stop = parse_dt(args.stop.as_deref().ok_or("range needs --stop")?)?;
            let clock = clock_for(&args.tz)?;
            let mut r = CroniterRange::new(expr, start, stop, clock.as_ref(), true, false, false)
                .map_err(|e| e.to_string())?;
            while let Some(wall) = r.next(clock.as_ref()) {
                println!("{}", wall.format("%Y-%m-%dT%H:%M:%S"));
            }
            Ok(())
        }
        "match" => {
            let expr = argv.get(2).ok_or("missing expression")?;
            let when = parse_dt(argv.get(3).ok_or("missing datetime")?)?;
            let args = parse_flags(&argv[4..])?;
            let clock = clock_for(&args.tz)?;
            let hit = is_match(expr, when, clock.as_ref(), true, false, None)
                .map_err(|e| e.to_string())?;
            println!("{hit}");
            if hit {
                Ok(())
            } else {
                Err(String::new())
            }
        }
        "validate" => {
            let expr = argv.get(2).ok_or("missing expression")?;
            match croniter_core::expand(expr, None, false, None, false, None) {
                Ok(_) => {
                    println!("valid");
                    Ok(())
                }
                Err(e) => Err(format!("invalid: {e}")),
            }
        }
        "-h" | "--help" | "help" => {
            print!("{USAGE}");
            Ok(())
        }
        "--version" | "-V" => {
            println!("croniter {}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
        other => Err(format!("unknown command {other:?}\n\n{USAGE}")),
    }
}

/// The benchmark workload, mirrored exactly by `bench/next10k.py`.
///
/// Six expressions covering plain steps, ranges, last-day, nth-weekday and
/// nearest-weekday, iterated `n / 6` times each so the mix is not dominated by
/// the cheapest path.
fn bench(n: usize) -> Result<(), String> {
    const EXPRS: [&str; 6] = [
        "*/5 * * * *",
        "0 9 * * 1-5",
        "0 12 l * *",
        "0 9 * * 5#3",
        "0 9 15w * *",
        "*/13 3-20 1,15 jan-jun *",
    ];
    let start = chrono::NaiveDate::from_ymd_opt(2019, 1, 14)
        .expect("valid")
        .and_hms_opt(0, 0, 0)
        .expect("valid");

    let per = n / EXPRS.len();
    let mut sink = 0f64;
    for expr in EXPRS {
        let mut it = CronIterator::new(
            expr,
            naive_to_timestamp(start),
            Options {
                ret_type: RetType::DateTime,
                ..Default::default()
            },
        )
        .map_err(|e| e.to_string())?;
        for _ in 0..per {
            let (_, ts) = it.get_next(&NAIVE).map_err(|e| e.to_string())?;
            sink += ts;
        }
    }
    // Printed so the work cannot be optimised away.
    println!("iterations={} checksum={:.0}", per * EXPRS.len(), sink);
    Ok(())
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(msg) => {
            if !msg.is_empty() {
                eprintln!("croniter: {msg}");
            }
            ExitCode::FAILURE
        }
    }
}
