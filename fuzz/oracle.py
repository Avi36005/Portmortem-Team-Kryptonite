"""Differential oracle: croniter.match() vs get_next() must agree.

Runs against the ORIGINAL Python croniter. No Rust involved -- this hunts for
croniter contradicting *itself*, which would be a real, filable upstream bug.

The invariant, in three parts:

  1. If get_next(start) returns N, then N itself must satisfy match().
  2. No minute strictly between start and N may satisfy match().
  3. Stepping back from N must not land after start.

A note on cost, because it shaped the design. The obvious implementation of (2)
walks every minute from start to N. That cannot terminate on sparse
expressions: `0 9 29 2 1` (Feb 29th AND a Monday) next fires ~20 years out,
and `croniter.match()` constructs a fresh croniter on every call. The first
version of this script ran for an hour on the seed list without finishing.

So (2) is exhaustive when the gap is small and randomly *sampled* when it is
large. Sampled intervals are reported as such and counted separately -- a
sampled interval is weaker evidence than an exhaustive one, and the summary
says so rather than blurring the two together.
"""
import argparse
import random
import traceback
from collections import Counter
from datetime import datetime, timedelta

from croniter import croniter


def check(expr, start, exhaustive_limit, samples, rnd):
    """Return (mode, finding_or_None). `mode` is 'exhaustive' or 'sampled'."""
    nxt = croniter(expr, start).get_next(datetime)

    # (1) the returned time must itself match
    if not croniter.match(expr, nxt):
        return "exhaustive", ("BAD_NEXT", expr, start, nxt)

    # (3) stepping back from the returned time must not overshoot
    prev = croniter(expr, nxt).get_prev(datetime)
    if prev > start:
        return "exhaustive", ("BAD_PREV_ROUNDTRIP", expr, start, nxt, prev)

    # (2) nothing strictly in between may match
    gap_minutes = int((nxt - start).total_seconds() // 60)
    if gap_minutes <= exhaustive_limit:
        mode = "exhaustive"
        offsets = range(1, gap_minutes)
    else:
        mode = "sampled"
        offsets = rnd.sample(range(1, gap_minutes), min(samples, gap_minutes - 1))

    for off in offsets:
        t = start + timedelta(minutes=off)
        if croniter.match(expr, t):
            return mode, ("SKIPPED_MATCH", expr, start, t, nxt)
    return mode, None


# Target the historically fragile syntax
FRAGILE = [
    "0 12 L * *", "0 12 L-1 * *", "0 0 LW * *",
    "0 9 15W * *", "0 9 1W * *", "0 9 31W * *",
    "0 9 * * 5#3", "0 9 * * 5#5", "0 9 * * 1#1",
    "0 9 15 * 5", "0 9 29 2 *", "0 9 29 2 1",
    "*/7 * * * *", "0 */13 * * *", "*/61 * * * *",
    "0 0 * * 7", "0 0 * * 0", "0 0 * * 1-5,0",
]


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--exhaustive-limit", type=int, default=1440,
                    help="walk every minute when the gap is at most this many minutes")
    ap.add_argument("--samples", type=int, default=250,
                    help="random minutes to probe when the gap is larger")
    ap.add_argument("--seed", type=int, default=20260801)
    args = ap.parse_args()
    rnd = random.Random(args.seed)

    starts = [datetime(y, m, d, h, mi)
              for y in (2024, 2025, 2026, 2027, 2028)   # incl. leap years
              for m in (1, 2, 3, 6, 11, 12)
              for d in (1, 14, 27, 28)
              for h in (0, 12, 23) for mi in (0, 30, 59)]

    kinds = Counter()
    modes = Counter()
    contradictions = []

    with open("fuzz/oracle-findings.txt", "w") as out:
        for expr in FRAGILE:
            tally = Counter()
            for s in starts:
                try:
                    mode, r = check(expr, s, args.exhaustive_limit, args.samples, rnd)
                except Exception:
                    mode, r = "n/a", ("EXCEPTION", expr, s, traceback.format_exc(limit=3))
                kind = r[0] if r else "OK"
                tally[kind] += 1
                kinds[kind] += 1
                modes[mode] += 1
                if r:
                    out.write(repr(r) + "\n")
                    if kind != "EXCEPTION":
                        contradictions.append(r)
                        print("CONTRADICTION:", r, flush=True)
            summary = "  ".join(f"{k}={v}" for k, v in sorted(tally.items()))
            print(f"{expr!r:26s} {summary}", flush=True)

    total = sum(kinds.values())
    print()
    print("=" * 72)
    print(f"pairs checked          : {total}")
    print(f"  exhaustive intervals : {modes['exhaustive']}")
    print(f"  sampled intervals    : {modes['sampled']} "
          f"({args.samples} random minutes each)")
    print(f"  rejected by croniter : {kinds['EXCEPTION']} (unsupported syntax)")
    print()
    print(f"CONTRADICTIONS FOUND   : {len(contradictions)}")
    print(f"  skipped_match        = {kinds['SKIPPED_MATCH']}")
    print(f"  bad_next             = {kinds['BAD_NEXT']}")
    print(f"  bad_prev_roundtrip   = {kinds['BAD_PREV_ROUNDTRIP']}")
    print("=" * 72)


if __name__ == "__main__":
    main()
