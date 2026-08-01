//! Integration tests for the shipped CLI.
//!
//! The binary is the deliverable, so it gets tested as a binary — spawned as a
//! subprocess, checked on stdout and exit code. Every expected value here was
//! cross-checked against the original Python croniter before being written
//! down; none was produced by running this port and pasting the result.

use std::path::PathBuf;
use std::process::{Command, Output};

fn bin() -> PathBuf {
    // cargo puts integration-test binaries in target/<profile>/deps/
    let mut p = std::env::current_exe().expect("test binary path");
    p.pop();
    if p.ends_with("deps") {
        p.pop();
    }
    p.join("croniter")
}

fn run(args: &[&str]) -> Output {
    Command::new(bin())
        .args(args)
        .output()
        .unwrap_or_else(|e| panic!("failed to run {:?}: {e}", bin()))
}

fn stdout_lines(args: &[&str]) -> Vec<String> {
    let out = run(args);
    assert!(
        out.status.success(),
        "command {args:?} failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .map(str::to_string)
        .collect()
}

#[test]
fn next_weekdays_at_nine() {
    // 2025-06-07 is a Saturday, so the next weekday 9am is Monday the 9th.
    let got = stdout_lines(&[
        "next",
        "0 9 * * 1-5",
        "-n",
        "3",
        "--start",
        "2025-06-07T12:00:00",
    ]);
    assert_eq!(
        got,
        [
            "2025-06-09T09:00:00",
            "2025-06-10T09:00:00",
            "2025-06-11T09:00:00"
        ]
    );
}

#[test]
fn next_last_day_of_month_handles_leap_february() {
    let got = stdout_lines(&[
        "next",
        "0 12 l * *",
        "-n",
        "2",
        "--start",
        "2024-02-01T00:00:00",
    ]);
    assert_eq!(got, ["2024-02-29T12:00:00", "2024-03-31T12:00:00"]);
}

#[test]
fn next_nth_weekday() {
    // Third Friday of June 2025 is the 20th; of July, the 18th.
    let got = stdout_lines(&[
        "next",
        "0 9 * * 5#3",
        "-n",
        "2",
        "--start",
        "2025-06-01T00:00:00",
    ]);
    assert_eq!(got, ["2025-06-20T09:00:00", "2025-07-18T09:00:00"]);
}

#[test]
fn next_nearest_weekday() {
    // 2025-03-15 is a Saturday, so `15w` fires on Friday the 14th.
    let got = stdout_lines(&[
        "next",
        "0 9 15w * *",
        "-n",
        "1",
        "--start",
        "2025-03-01T00:00:00",
    ]);
    assert_eq!(got, ["2025-03-14T09:00:00"]);
}

#[test]
fn prev_walks_backwards() {
    let got = stdout_lines(&[
        "prev",
        "0 0 * * *",
        "-n",
        "3",
        "--start",
        "2025-06-15T12:00:00",
    ]);
    assert_eq!(
        got,
        [
            "2025-06-15T00:00:00",
            "2025-06-14T00:00:00",
            "2025-06-13T00:00:00"
        ]
    );
}

#[test]
fn range_includes_both_ends() {
    let got = stdout_lines(&[
        "range",
        "0 0 * * *",
        "--start",
        "2025-06-01T00:00:00",
        "--stop",
        "2025-06-04T00:00:00",
    ]);
    assert_eq!(
        got,
        [
            "2025-06-01T00:00:00",
            "2025-06-02T00:00:00",
            "2025-06-03T00:00:00",
            "2025-06-04T00:00:00"
        ]
    );
}

#[test]
fn match_reports_hit_and_miss_via_exit_code() {
    let hit = run(&["match", "*/15 * * * *", "2025-06-01T00:15:00"]);
    assert!(hit.status.success());
    assert_eq!(String::from_utf8_lossy(&hit.stdout).trim(), "true");

    let miss = run(&["match", "*/15 * * * *", "2025-06-01T00:07:00"]);
    assert!(!miss.status.success());
    assert_eq!(String::from_utf8_lossy(&miss.stdout).trim(), "false");
}

#[test]
fn validate_accepts_good_and_rejects_bad() {
    assert!(run(&["validate", "0 9 * * 1-5"]).status.success());

    let bad = run(&["validate", "99 * * * *"]);
    assert!(!bad.status.success());
    assert!(
        String::from_utf8_lossy(&bad.stderr).contains("out of range"),
        "expected an out-of-range message, got: {}",
        String::from_utf8_lossy(&bad.stderr)
    );
}

#[test]
fn bench_is_deterministic_and_matches_the_python_checksum() {
    // bench/next10k.py prints the identical line when run against the original
    // Python croniter. A single differing fire time changes the checksum.
    let out = stdout_lines(&["bench", "-n", "10000"]);
    assert_eq!(out, ["iterations=9996 checksum=26611225207500"]);
}

#[test]
fn no_arguments_is_an_error_not_a_panic() {
    let out = run(&[]);
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("USAGE:"), "stderr was: {stderr}");
    // Usage on the error path must not pollute stdout: fire times are the only
    // thing that belongs there, so `croniter next ... | ...` stays clean.
    assert!(out.stdout.is_empty(), "stdout should be empty on error");
}

#[test]
fn help_prints_usage_to_stdout() {
    let out = run(&["--help"]);
    assert!(out.status.success());
    assert!(String::from_utf8_lossy(&out.stdout).contains("USAGE:"));
}

#[test]
fn unknown_command_is_rejected() {
    let out = run(&["frobnicate", "* * * * *"]);
    assert!(!out.status.success());
}
