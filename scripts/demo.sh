#!/usr/bin/env bash
# Demo script for the 5-minute video. Runs the claims live, in order, so the
# recording shows results rather than assertions.
#
#     bash scripts/demo.sh
#
# Pauses between sections unless DEMO_NOPAUSE=1 is set.

set -uo pipefail
cd "$(dirname "$0")/.."

BOLD=$'\033[1m'; DIM=$'\033[2m'; GREEN=$'\033[32m'; RED=$'\033[31m'; OFF=$'\033[0m'
SHASUM=$(command -v sha256sum >/dev/null 2>&1 && echo "sha256sum" || echo "shasum -a 256")

step() { printf "\n%s══ %s %s\n\n" "$BOLD" "$1" "$OFF"; }
run()  { printf "%s\$ %s%s\n" "$DIM" "$*" "$OFF"; "$@"; }
pause() { [ "${DEMO_NOPAUSE:-0}" = "1" ] || { printf "\n%s[enter to continue]%s" "$DIM" "$OFF"; read -r _; }; }

step "1/7  The tests are the originals, untouched"
run $SHASUM -c .test-hashes.sha256
echo
echo "Commits that have ever touched tests/original/ (expect: 1, the vendoring):"
run git log --oneline -- tests/original/
pause

step "2/7  The original suite, running against Rust"
run .venv/bin/python -m pytest tests/original/ -q
pause

step "3/7  The baseline it is being compared against"
echo "The untouched Python original at the kickoff commit:"
( cd /tmp/croniter-src 2>/dev/null && run "$OLDPWD/.venv-baseline/bin/python" -m pytest src/croniter/tests/ -q ) \
  || echo "(/tmp/croniter-src not present — clone it to re-verify the baseline)"
pause

step "4/7  Zero unsafe, and zero Python in the shipped crate"
echo -n "unsafe blocks in core/ (excluding the forbid attribute): "
grep -rn "unsafe" core/src/ | grep -v "forbid(unsafe_code)" | wc -l | tr -d ' '
echo -n "occurrences of pyo3 in croniter-core's dependency tree: "
cargo tree -p croniter-core --edges normal 2>/dev/null | grep -ci pyo3 || echo 0
echo
echo "The attribute itself, first line of core/src/lib.rs:"
head -1 core/src/lib.rs
pause

step "5/7  The shipped artifact — no interpreter involved"
run cargo build --release -p croniter-core
echo
run ./target/release/croniter next '0 9 * * 1-5' -n 3 --start 2025-06-07T12:00:00
echo "Last day of February in a leap year:"
run ./target/release/croniter next '0 12 l * *' -n 2 --start 2024-02-01T00:00:00
echo "Third Friday of the month:"
run ./target/release/croniter next '0 9 * * 5#3' -n 2 --start 2025-06-01T00:00:00
pause

step "6/7  Evidence: differential fuzz and benchmarks"
echo "Last committed differential fuzz run:"
grep -E "^elapsed_seconds|^inputs_compared|^divergences" fuzz/log.txt
echo
sed -n '/coverage by/,$p' fuzz/log.txt | head -13
echo
echo "Benchmarks (see bench/methodology.md for confounders):"
python3 - <<'PY'
import json
r = json.load(open("bench/results.json"))
for b in r["benchmarks"]:
    rss = b["peak_rss_bytes"]
    print(f"  {b['name']:32s} mean={b['mean_s']*1000:8.2f}ms  "
          f"p99={b['p99_s']*1000:8.2f}ms  rss={rss/1024/1024:6.1f}MB")
c = r["comparison"]
print(f"\n  speedup mean={c['workload_speedup_mean']:.1f}x  "
      f"p99={c['workload_speedup_p99']:.1f}x  rss={c['rss_ratio']:.1f}x smaller")
PY

pause

step "7/7  Two bugs found in the ORIGINAL croniter"
echo "Bug 1 — get_next skips a fire time (Australia/Lord_Howe, 30-min DST shift):"
.venv-baseline/bin/python -c "
import zoneinfo
from datetime import datetime
from croniter import croniter
tz = zoneinfo.ZoneInfo('Australia/Lord_Howe')
s = datetime(2019,10,6,1,43,tzinfo=tz)
n = croniter('0 * * * *', s).get_next(datetime)
p = croniter('0 * * * *', n).get_prev(datetime)
print('   start      ', s)
print('   get_next   ', n)
print('   get_prev   ', p, ' <- AFTER start: a fire time was skipped')
print('   match(02:30)', croniter.match('0 * * * *', datetime(2019,10,6,2,30,tzinfo=tz)),
      ' <- True for a MINUTE-0 schedule at minute 30')
"
echo
echo "Bug 2 — croniter_range returns values outside the requested interval:"
.venv-baseline/bin/python -c "
import zoneinfo
from datetime import datetime, timezone
from croniter import croniter_range
tz = zoneinfo.ZoneInfo('Europe/London')
a = datetime(2018,3,25,1,15,tzinfo=tz); b = datetime(2018,3,25,5,0,tzinfo=tz)
print('   asked for UTC', a.astimezone(timezone.utc).strftime('%H:%M'), '->',
      b.astimezone(timezone.utc).strftime('%H:%M'))
for d in croniter_range(b, a, '0 * * * *'):
    u = d.astimezone(timezone.utc)
    bad = not (a.astimezone(timezone.utc) <= u <= b.astimezone(timezone.utc))
    print('   ', d, ' UTC', u.strftime('%H:%M'), '  <- OUTSIDE' if bad else '')
"
echo
echo "Our port reproduces both, deliberately (DECISIONS.md #18)."
pause

printf "\n%s══ Summary %s\n" "$BOLD" "$OFF"
printf "  Baseline : 228/228 (untouched Python original)\n"
printf "  Port     : %s228/228%s (same unmodified suite, via PyO3 bridge)\n" "$GREEN" "$OFF"
printf "  Say the real number out loud. It is 228/228, and the tests are unedited.\n\n"
