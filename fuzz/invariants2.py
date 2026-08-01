"""Deeper invariant hunt: check croniter's answers against the EXPRESSION,
not just against its own other APIs.

`invariants.py` cross-checks croniter's APIs against each other, which found
two bugs. This goes one level lower: it decodes the expanded field sets and
asserts that every returned fire time's wall-clock fields actually satisfy
them. That does not depend on any other croniter API being right, so it can
catch a bug that is consistent across all of them.

Invariants here:

  F1  FIELDS        every fire time's (minute, hour, day, month, dow, second,
                    year) satisfies the expanded expression. Naive only, where
                    it is unconditionally sound.
  F2  FIELDS_TZ     the same check on timezone-aware runs, reported SEPARATELY
                    because croniter legitimately shifts a fire time forward
                    when the local time does not exist (spring-forward). These
                    are candidates for review, not confirmed bugs.
  G1  ALL_NEXT      the all_next()/all_prev() generators agree with repeated
                    get_next()/get_prev().
  G2  RET_TYPE      get_next(float) equals datetime_to_timestamp(get_next(dt)).
  G3  SECOND_POS    "S M H D Mo W" with second_at_beginning=True equals
                    "M H D Mo W S" with second_at_beginning=False.
  G4  DAY_OR        day_or=False (intersection) fire times are a subset of
                    day_or=True (union) fire times.
  G5  REVERSE_RANGE croniter_range(a,b) reversed == croniter_range(b,a).
  G6  IDEMPOTENT    expand() is deterministic across repeated calls.

Usage:
    python fuzz/invariants2.py --seconds 120
"""
import argparse
import calendar
import random
import sys
import time
from collections import Counter
from datetime import datetime, timedelta, timezone

from croniter import croniter, croniter_range, datetime_to_timestamp

# ---------------------------------------------------------------- generators

# Expressions with no L / W / # -- their fire times are checkable field by
# field with no special cases. Kept separate on purpose: a field checker that
# guesses at L/W/# semantics would report its own bugs as croniter's.
PLAIN_EXPRS = [
    "* * * * *", "*/2 * * * *", "*/7 * * * *", "*/13 * * * *", "*/61 * * * *",
    "0 * * * *", "0 0 * * *", "30 2 * * *", "0 9 * * 1-5", "0 22 * * 1-5",
    "0 9 15 * 5", "0 9 29 2 *", "0 0 31 * *", "0 6 30 3 *",
    "0 0 * * 0", "0 0 * * 7", "0 0 * * 1-5,0", "0 0 * * sat-sun",
    "0 0 * apr-jan *", "0 0 * apr-jan/3 *", "23 0-20/2 * * *",
    "0 0,12 1 */2 *", "0 4 8-14 * *", "5 0 * 8 *", "15 14 1 * *",
    "7-52/9 3-20 1,15 jan-jun *", "0 0 1 jan *", "*/30 * * * *",
    "@daily", "@weekly", "@monthly", "@yearly", "@hourly", "@midnight",
    "* * * * * */15", "0 0 * * * 30", "0 * * * * 0,30",
]

# Exercised by the non-field invariants, which handle L/W/# fine.
ALL_EXPRS = PLAIN_EXPRS + [
    "0 12 l * *", "0 9 15w * *", "0 9 1w * *", "0 9 31w * *",
    "0 9 * * 5#3", "0 9 * * 5#5", "0 9 * * 1#1", "0 9 * * l5",
    "0 0 ? * 1", "0 0 1 * ?",
]

DST_EVENTS = [
    ("Europe/Athens", "2013-03-31T03:00:00"), ("Europe/Athens", "2013-10-27T04:00:00"),
    ("America/New_York", "2013-03-10T02:00:00"), ("America/New_York", "2013-11-03T02:00:00"),
    ("Europe/London", "2018-03-25T01:00:00"), ("Europe/London", "2018-10-28T02:00:00"),
    ("Australia/Sydney", "2019-10-06T02:00:00"), ("Australia/Sydney", "2019-04-07T03:00:00"),
    ("America/Santiago", "2019-09-08T04:00:00"), ("America/Sao_Paulo", "2018-11-04T00:00:00"),
    # The interesting ones: sub-hour DST shifts.
    ("Australia/Lord_Howe", "2019-10-06T02:00:00"),   # +30 min
    ("Australia/Lord_Howe", "2019-04-07T02:00:00"),   # -30 min
    ("Pacific/Chatham", "2019-09-29T02:45:00"),       # 45-min offset zone
    ("Iran", "2020-03-21T00:00:00"),                  # historical, unusual date
]
PLAIN_ZONES = ["UTC", "Asia/Kolkata", "Asia/Kathmandu"]   # :45 offset


def make_tz(name, lib):
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
    return dt.replace(tzinfo=timezone.utc) if dt.tzinfo is None else dt.astimezone(timezone.utc)


def pick_start(rnd, force_naive=False):
    if force_naive or rnd.random() < 0.4:
        return (datetime(rnd.choice([2024, 2025, 2026, 2027, 2028]), rnd.randint(1, 12),
                         rnd.randint(1, 28), rnd.randint(0, 23), rnd.randint(0, 59)), "naive")
    if rnd.random() < 0.75:
        zone, when = rnd.choice(DST_EVENTS)
        lib = rnd.choice(["zoneinfo", "pytz"])
        base = datetime.fromisoformat(when) + timedelta(minutes=rnd.randint(-240, 240))
        try:
            return localize(base, make_tz(zone, lib), lib), f"{zone}/{lib}"
        except Exception:
            return base, "naive"
    zone, lib = rnd.choice(PLAIN_ZONES), rnd.choice(["zoneinfo", "pytz"])
    base = datetime(rnd.choice([2024, 2025, 2026]), rnd.randint(1, 12),
                    rnd.randint(1, 28), rnd.randint(0, 23), rnd.randint(0, 59))
    return localize(base, make_tz(zone, lib), lib), f"{zone}/{lib}"


# ------------------------------------------------------------ field decoding

def field_ok(value, allowed):
    """Does `value` satisfy one expanded field list?"""
    return allowed == ["*"] or "*" in allowed or value in allowed


def check_fields(expr, fire, day_or=True):
    """Return None if the fire time satisfies the expression, else a reason.

    Only called for expressions without L / W / #, so day-of-month and
    day-of-week are the only subtlety: cron ORs them when both are restricted
    and ANDs otherwise.
    """
    expanded, nth = croniter.expand(expr)
    if nth:
        return None                      # not our business here
    n = len(expanded)

    if not field_ok(fire.minute, expanded[0]):
        return f"minute {fire.minute} not in {expanded[0]}"
    if not field_ok(fire.hour, expanded[1]):
        return f"hour {fire.hour} not in {expanded[1]}"
    if not field_ok(fire.month, expanded[3]):
        return f"month {fire.month} not in {expanded[3]}"

    dom, dow = expanded[2], expanded[4]
    if "l" in dom:
        return None                      # last-day syntax, skip
    dom_star = dom == ["*"] or "*" in dom
    dow_star = dow == ["*"] or "*" in dow
    fire_dow = fire.isoweekday() % 7     # cron: Sunday == 0
    dom_hit, dow_hit = fire.day in dom, fire_dow in dow

    if dom_star and dow_star:
        pass
    elif dom_star:
        if not dow_hit:
            return f"dow {fire_dow} not in {dow}"
    elif dow_star:
        if not dom_hit:
            return f"day {fire.day} not in {dom}"
    else:
        # both restricted: OR under day_or=True, AND under day_or=False
        if day_or and not (dom_hit or dow_hit):
            return f"day {fire.day} not in {dom} AND dow {fire_dow} not in {dow} (OR)"
        if not day_or and not (dom_hit and dow_hit):
            return f"day/dow {fire.day}/{fire_dow} fails intersection {dom}/{dow}"

    if n > 5 and not field_ok(fire.second, expanded[5]):
        return f"second {fire.second} not in {expanded[5]}"
    if n > 6 and not field_ok(fire.year, expanded[6]):
        return f"year {fire.year} not in {expanded[6]}"
    return None


# ------------------------------------------------------------------- checks

def c_fields(expr, start, label, rnd, n=6):
    """F1 / F2 — fire times must satisfy the expression's fields."""
    out = []
    tag = "F1_FIELDS" if start.tzinfo is None else "F2_FIELDS_TZ"
    for meth in ("get_next", "get_prev"):
        it = croniter(expr, start, ret_type=datetime)
        for _ in range(n):
            fire = getattr(it, meth)(datetime)
            why = check_fields(expr, fire)
            if why:
                out.append((tag, expr, str(start), label, meth, str(fire), why))
                break
    return out


def c_all_next(expr, start, label, rnd, n=5):
    """G1 — the generators must agree with repeated stepping."""
    out = []
    a = [croniter(expr, start, ret_type=datetime).get_next(datetime)]
    it = croniter(expr, start, ret_type=datetime)
    a = [it.get_next(datetime) for _ in range(n)]
    gen = croniter(expr, start, ret_type=datetime).all_next(datetime)
    b = [next(gen) for _ in range(n)]
    if a != b:
        out.append(("G1_ALL_NEXT", expr, str(start), label,
                    f"step={[str(x) for x in a]}", f"gen={[str(x) for x in b]}"))
    it = croniter(expr, start, ret_type=datetime)
    a = [it.get_prev(datetime) for _ in range(n)]
    gen = croniter(expr, start, ret_type=datetime).all_prev(datetime)
    b = [next(gen) for _ in range(n)]
    if a != b:
        out.append(("G1_ALL_PREV", expr, str(start), label,
                    f"step={[str(x) for x in a]}", f"gen={[str(x) for x in b]}"))
    return out


def c_ret_type(expr, start, label, rnd, n=5):
    """G2 — float and datetime results must denote the same instant."""
    out = []
    fi = croniter(expr, start, ret_type=float)
    di = croniter(expr, start, ret_type=datetime)
    for _ in range(n):
        f, d = fi.get_next(float), di.get_next(datetime)
        if abs(f - datetime_to_timestamp(d)) > 1e-6:
            out.append(("G2_RET_TYPE", expr, str(start), label, repr(f), str(d)))
            break
    return out


def c_second_pos(expr, start, label, rnd, n=4):
    """G3 — second_at_beginning is a pure re-spelling."""
    parts = expr.split()
    if len(parts) != 6:
        return []
    moved = " ".join([parts[5]] + parts[:5])
    a = croniter(expr, start, ret_type=datetime)
    b = croniter(moved, start, ret_type=datetime, second_at_beginning=True)
    xs = [a.get_next(datetime) for _ in range(n)]
    ys = [b.get_next(datetime) for _ in range(n)]
    if xs != ys:
        return [("G3_SECOND_POS", expr, moved, str(start), label,
                 f"{[str(x) for x in xs]}", f"{[str(y) for y in ys]}")]
    return []


def c_day_or(expr, start, label, rnd, n=4):
    """G4 — intersection results must also be union results."""
    expanded, nth = croniter.expand(expr)
    if nth or expanded[2] == ["*"] or expanded[4] == ["*"] or "l" in expanded[2]:
        return []
    inter = croniter(expr, start, ret_type=datetime, day_or=False)
    union_set = set()
    u = croniter(expr, start, ret_type=datetime, day_or=True)
    for _ in range(n * 30):
        union_set.add(as_utc(u.get_next(datetime)))
    for _ in range(n):
        v = as_utc(inter.get_next(datetime))
        if v > max(union_set):
            break
        if v not in union_set:
            return [("G4_DAY_OR", expr, str(start), label, str(v))]
    return []


def c_reverse_range(expr, start, label, rnd, n=5):
    """G5 — a reversed range must be the reverse of the forward range."""
    it = croniter(expr, start, ret_type=datetime)
    fires = [it.get_next(datetime) for _ in range(n)]
    stop = fires[-1]
    fwd = list(croniter_range(start, stop, expr))
    rev = list(croniter_range(stop, start, expr))
    if fwd != list(reversed(rev)):
        return [("G5_REVERSE_RANGE", expr, str(start), label,
                 f"fwd={[str(x) for x in fwd]}", f"rev={[str(x) for x in rev]}")]
    return []


def c_idempotent(expr, start, label, rnd):
    """G6 — expand() must be deterministic."""
    a, b = croniter.expand(expr), croniter.expand(expr)
    if a != b:
        return [("G6_EXPAND_NONDETERMINISTIC", expr, str(a), str(b))]
    return []


CHECKS = [
    ("fields", c_fields, True),          # True == needs a plain expression
    ("all_next", c_all_next, False),
    ("ret_type", c_ret_type, False),
    ("second_pos", c_second_pos, False),
    ("day_or", c_day_or, True),
    ("reverse_range", c_reverse_range, False),
    ("idempotent", c_idempotent, False),
]


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--seconds", type=float, default=120.0)
    ap.add_argument("--seed", type=int, default=None)
    args = ap.parse_args()

    seed = args.seed if args.seed is not None else int(time.time())
    rnd = random.Random(seed)
    started = time.time()

    kinds, ran, errors = Counter(), Counter(), Counter()
    findings, cases = [], 0

    print(f"deep invariant hunt vs the ORIGINAL Python croniter  seed={seed} "
          f"budget={args.seconds}s")
    print("-" * 76, flush=True)

    while time.time() - started < args.seconds:
        cases += 1
        for name, fn, needs_plain in CHECKS:
            expr = rnd.choice(PLAIN_EXPRS if needs_plain else ALL_EXPRS)
            start, label = pick_start(rnd)
            try:
                got = fn(expr, start, label, rnd)
                ran[name] += 1
                for f in got:
                    kinds[f[0]] += 1
                    findings.append(f)
                    print("FINDING:", f, flush=True)
            except Exception as exc:
                errors[f"{name}:{type(exc).__name__}"] += 1
        if cases % 300 == 0:
            print(f"[{time.time()-started:6.1f}s] cases={cases} "
                  f"findings={len(findings)}", flush=True)

    print("-" * 76)
    print(f"elapsed_seconds={time.time()-started:.1f}")
    print(f"cases={cases}")
    for k, v in sorted(ran.items()):
        print(f"  {k:14s} {v}")
    print(f"\nFINDINGS: {len(findings)}")
    for k, v in kinds.most_common():
        print(f"  {k:28s} {v}")
    if errors:
        print("\nexceptions (expected: unsupported syntax, no-match-in-range):")
        for k, v in errors.most_common(10):
            print(f"  {k:46s} {v}")
    if findings:
        with open("fuzz/invariant2-findings.txt", "w") as f:
            for x in findings:
                f.write(repr(x) + "\n")
        print("\nwrote fuzz/invariant2-findings.txt")
    return 0


if __name__ == "__main__":
    sys.exit(main())
