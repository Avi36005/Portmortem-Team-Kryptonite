"""Invariant hunter: look for the ORIGINAL Python croniter contradicting itself.

No Rust involved. Every check below is a property croniter must satisfy no
matter what the correct answer is, so a violation is a bug in croniter, not a
disagreement about semantics.

  I1  MONOTONIC      repeated get_next strictly increases; get_prev strictly
                     decreases. Compared as UTC instants, so a DST fold is not
                     mistaken for a violation.
  I2  ROUNDTRIP      get_prev from a fire time must not overshoot the start
                     that produced it.
  I3  MATCH_AGREES   every fire time returned by get_next must satisfy match(),
                     and (sampled) minutes strictly between two consecutive
                     fire times must not.
  I4  RANGE_AGREES   croniter_range(start, stop) must equal iterating get_next
                     from start until the value passes stop.
  5   EQUIVALENCE    expressions that are definitionally the same schedule must
                     produce the same fire times (0 vs 7 vs sun, @daily vs
                     0 0 * * *, and so on).

Usage:
    python fuzz/invariants.py --seconds 120
"""
import argparse
import itertools
import random
import sys
import time
import traceback
from collections import Counter
from datetime import datetime, timedelta, timezone

from croniter import croniter, croniter_range

# ---------------------------------------------------------------- generators

EXPRS = [
    "* * * * *", "*/2 * * * *", "*/7 * * * *", "*/13 * * * *", "*/61 * * * *",
    "0 * * * *", "0 0 * * *", "30 2 * * *", "0 9 * * 1-5", "0 22 * * 1-5",
    "0 12 l * *", "0 9 15w * *", "0 9 1w * *", "0 9 31w * *",
    "0 9 * * 5#3", "0 9 * * 5#5", "0 9 * * 1#1", "0 9 * * l5",
    "0 9 15 * 5", "0 9 29 2 *", "0 0 31 * *", "0 6 30 3 *",
    "0 0 * * 0", "0 0 * * 7", "0 0 * * 1-5,0", "0 0 * * sat-sun",
    "0 0 * apr-jan *", "0 0 * apr-jan/3 *", "23 0-20/2 * * *",
    "0 0,12 1 */2 *", "0 4 8-14 * *", "5 0 * 8 *", "15 14 1 * *",
    "* * * * * */15", "0 0 * * * 30", "0 0 1 1 * 0 2030",
    "@daily", "@weekly", "@monthly", "@yearly", "@hourly", "@midnight",
    "0 0 30 2 *", "0 0 ? * 1", "0 0 1 * ?",
]

DST_EVENTS = [
    ("Europe/Athens", "2013-03-31T03:00:00"), ("Europe/Athens", "2013-10-27T04:00:00"),
    ("America/New_York", "2013-03-10T02:00:00"), ("America/New_York", "2013-11-03T02:00:00"),
    ("Europe/London", "2018-03-25T01:00:00"), ("Europe/London", "2018-10-28T02:00:00"),
    ("Australia/Sydney", "2019-10-06T02:00:00"), ("Australia/Sydney", "2019-04-07T03:00:00"),
    ("America/Santiago", "2019-09-08T04:00:00"), ("Australia/Lord_Howe", "2019-10-06T02:00:00"),
    ("America/Sao_Paulo", "2018-11-04T00:00:00"), ("Pacific/Chatham", "2019-09-29T02:45:00"),
]
PLAIN_ZONES = ["UTC", "Asia/Kolkata"]

# Schedules that are the same schedule written differently. Any difference in
# fire times is croniter disagreeing with itself.
EQUIVALENCE_CLASSES = [
    ("0 0 * * 0", "0 0 * * 7", "0 0 * * sun"),
    ("0 0 * * 1", "0 0 * * mon"),
    ("0 0 * * 1-5", "0 0 * * mon-fri"),
    ("0 0 1 1 *", "@yearly", "@annually"),
    ("0 0 1 * *", "@monthly"),
    ("0 0 * * 0", "@weekly"),
    ("0 0 * * *", "@daily"),
    ("0 * * * *", "@hourly"),
    ("0 0 1 jan *", "0 0 1 1 *"),
    ("0 0 1 * *", "0 0 1 * ?"),
    ("*/30 * * * *", "0,30 * * * *"),
    ("*/2 * * * *", "0-58/2 * * * *"),
    ("0 0-23 * * *", "0 * * * *"),
    ("0 9 * jan-dec *", "0 9 * * *"),
    ("0 9 * * sun-sat", "0 9 * * *"),
]


def make_tz(name, lib):
    if name is None:
        return None
    if lib == "pytz":
        import pytz
        return pytz.timezone(name)
    import zoneinfo
    return zoneinfo.ZoneInfo(name)


def localize(naive, tz, lib):
    if tz is None:
        return naive
    if lib == "pytz":
        return tz.localize(naive)
    return naive.replace(tzinfo=tz)


def as_utc(dt):
    if dt.tzinfo is None:
        return dt.replace(tzinfo=timezone.utc)
    return dt.astimezone(timezone.utc)


def pick_start(rnd):
    """Return (start_datetime, label). Half the time, near a DST transition."""
    r = rnd.random()
    if r < 0.45:
        zone, when = rnd.choice(DST_EVENTS)
        lib = rnd.choice(["zoneinfo", "pytz"])
        tz = make_tz(zone, lib)
        base = datetime.fromisoformat(when) + timedelta(minutes=rnd.randint(-240, 240))
        try:
            return localize(base, tz, lib), f"{zone}/{lib}"
        except Exception:
            return base, "naive"
    if r < 0.6:
        zone = rnd.choice(PLAIN_ZONES)
        lib = rnd.choice(["zoneinfo", "pytz"])
        return localize(datetime(rnd.choice([2024, 2025, 2026]), rnd.randint(1, 12),
                                 rnd.randint(1, 28), rnd.randint(0, 23), rnd.randint(0, 59)),
                        make_tz(zone, lib), lib), f"{zone}/{lib}"
    return datetime(rnd.choice([2024, 2025, 2026, 2027, 2028]), rnd.randint(1, 12),
                    rnd.randint(1, 28), rnd.randint(0, 23), rnd.randint(0, 59)), "naive"


# ------------------------------------------------------------------ checks

def check_sequence(expr, start, label, rnd, n=8, samples=60):
    """I1 + I2 + I3 over one forward run and one backward run."""
    findings = []

    fwd = []
    it = croniter(expr, start, ret_type=datetime)
    for _ in range(n):
        fwd.append(it.get_next(datetime))

    # I1 forward: strictly increasing as instants
    for a, b in itertools.pairwise(fwd):
        if not as_utc(b) > as_utc(a):
            findings.append(("I1_NOT_INCREASING", expr, str(start), label, str(a), str(b)))
            break

    # I2: stepping back from the first fire time must not overshoot start
    back = croniter(expr, fwd[0], ret_type=datetime).get_prev(datetime)
    if as_utc(back) > as_utc(start):
        findings.append(("I2_PREV_OVERSHOOT", expr, str(start), label, str(fwd[0]), str(back)))

    # I3: each fire time matches; sampled gaps do not.
    # match() is naive-only in effect, so only assert it when start is naive.
    if start.tzinfo is None:
        for f in fwd[:4]:
            if not croniter.match(expr, f):
                findings.append(("I3_FIRE_DOES_NOT_MATCH", expr, str(start), label, str(f)))
                break
        for a, b in itertools.pairwise(fwd[:4]):
            gap = int((b - a).total_seconds() // 60)
            if gap <= 1:
                continue
            probes = rnd.sample(range(1, gap), min(samples, gap - 1))
            for off in probes:
                t = a + timedelta(minutes=off)
                if croniter.match(expr, t):
                    findings.append(("I3_GAP_MATCHES", expr, str(start), label, str(a), str(t), str(b)))
                    break

    # I1 backward: strictly decreasing
    rev = []
    it = croniter(expr, start, ret_type=datetime)
    for _ in range(n):
        rev.append(it.get_prev(datetime))
    for a, b in itertools.pairwise(rev):
        if not as_utc(b) < as_utc(a):
            findings.append(("I1_NOT_DECREASING", expr, str(start), label, str(a), str(b)))
            break

    return findings


def check_range(expr, start, label, n=6):
    """I4: croniter_range(a, b) must equal every fire time t with a <= t <= b.

    croniter_range is inclusive of BOTH ends, so the reference iteration has to
    begin a microsecond before `start` -- otherwise a `start` that is itself a
    fire time is counted by range and missed by the reference, which looks like
    a bug and is not one. (It looked like one to me first: 1,408 false
    findings before this was corrected.)
    """
    it = croniter(expr, start, ret_type=datetime)
    fires = [it.get_next(datetime) for _ in range(n)]
    stop = fires[-1]

    ref_it = croniter(expr, start - timedelta(microseconds=1), ret_type=datetime)
    reference = []
    while True:
        v = ref_it.get_next(datetime)
        if as_utc(v) > as_utc(stop):
            break
        reference.append(v)

    via_range = list(croniter_range(start, stop, expr))
    if via_range != reference:
        return [("I4_RANGE_DISAGREES", expr, str(start), label,
                 f"iter={[str(x) for x in reference]}", f"range={[str(x) for x in via_range]}")]
    return []


def check_equivalence(group, start, label, n=5):
    """5: definitionally-identical expressions must fire identically."""
    seqs = {}
    for expr in group:
        it = croniter(expr, start, ret_type=datetime)
        seqs[expr] = [as_utc(it.get_next(datetime)).isoformat() for _ in range(n)]
    first = seqs[group[0]]
    for expr in group[1:]:
        if seqs[expr] != first:
            return [("EQUIVALENCE_BROKEN", group[0], expr, str(start), label,
                     f"{group[0]}={first}", f"{expr}={seqs[expr]}")]
    return []


# -------------------------------------------------------------------- main

def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--seconds", type=float, default=120.0)
    ap.add_argument("--seed", type=int, default=None)
    args = ap.parse_args()

    seed = args.seed if args.seed is not None else int(time.time())
    rnd = random.Random(seed)
    started = time.time()

    kinds = Counter()
    checks = Counter()
    findings = []
    errors = Counter()
    cases = 0

    print(f"invariant hunt against the ORIGINAL Python croniter  seed={seed} "
          f"budget={args.seconds}s")
    print("-" * 76, flush=True)

    while time.time() - started < args.seconds:
        expr = rnd.choice(EXPRS)
        start, label = pick_start(rnd)
        cases += 1

        for name, fn in (
            ("sequence", lambda: check_sequence(expr, start, label, rnd)),
            ("range", lambda: check_range(expr, start, label)),
        ):
            try:
                got = fn()
                checks[name] += 1
                for f in got:
                    kinds[f[0]] += 1
                    findings.append(f)
                    print("FINDING:", f, flush=True)
            except Exception as exc:
                errors[f"{name}:{type(exc).__name__}"] += 1

        group = rnd.choice(EQUIVALENCE_CLASSES)
        try:
            got = check_equivalence(group, start, label)
            checks["equivalence"] += 1
            for f in got:
                kinds[f[0]] += 1
                findings.append(f)
                print("FINDING:", f, flush=True)
        except Exception as exc:
            errors[f"equivalence:{type(exc).__name__}"] += 1

        if cases % 200 == 0:
            print(f"[{time.time()-started:6.1f}s] cases={cases} findings={len(findings)}",
                  flush=True)

    print("-" * 76)
    print(f"elapsed_seconds={time.time()-started:.1f}")
    print(f"cases={cases}")
    for k, v in sorted(checks.items()):
        print(f"  {k:12s} checks run: {v}")
    print(f"\nFINDINGS: {len(findings)}")
    for k, v in kinds.most_common():
        print(f"  {k:26s} {v}")
    if errors:
        print("\nexceptions raised (expected for unsupported syntax):")
        for k, v in errors.most_common(12):
            print(f"  {k:44s} {v}")

    if findings:
        with open("fuzz/invariant-findings.txt", "w") as f:
            for x in findings:
                f.write(repr(x) + "\n")
        print("\nwrote fuzz/invariant-findings.txt")
    return 1 if findings else 0


if __name__ == "__main__":
    sys.exit(main())
