"""Differential fuzz harness: original Python croniter vs the Rust port.

Generates cron expressions and start times, runs the same probe under both
interpreters, and reports every input where the two disagree.

Both sides run the *same* probe script (fuzz/probe.py); the only difference is
which `croniter` is importable in each interpreter's environment.

    python fuzz/differential.py --seconds 90

Generation is weighted toward the places a port actually breaks:

  * `L`, `W`, `#` and wrap-around ranges (Sat-Sun, Apr-Jan) -- the syntax with
    the most special-cased code behind it;
  * start times within a few hours of a real DST transition, in both
    `zoneinfo` and `pytz` flavours, in zones that spring forward, fall back,
    and (southern hemisphere) do both in the opposite months;
  * the option flags that change semantics rather than formatting:
    `day_or`, `second_at_beginning`, `implement_cron_bug`,
    `expand_from_start_time`, `hash_id`.

Divergences are printed as they are found and summarized at the end. Nothing
is filtered or suppressed -- a divergence we already understand is still
counted and still printed.

The `R` (random) hash form is deliberately NOT generated: it is seeded from
Python's global RNG and is nondeterministic by construction, so no port can
reproduce it and comparing it would produce meaningless noise. See DECISIONS #7.
"""
import argparse
import json
import random
import subprocess
import sys
import time
from datetime import datetime, timedelta

# Syntax that historically breaks: last-day, nearest-weekday, nth-weekday,
# wrap-around ranges, odd steps, day/dow union semantics.
FRAGILE_EXPRS = [
    "0 12 l * *", "0 12 l-1 * *", "0 0 lw * *",
    "0 9 15w * *", "0 9 1w * *", "0 9 31w * *",
    "0 9 * * 5#3", "0 9 * * 5#5", "0 9 * * 1#1", "0 9 * * l3",
    "0 9 15 * 5", "0 9 29 2 *", "0 9 29 2 1",
    "*/7 * * * *", "0 */13 * * *", "*/61 * * * *",
    "0 0 * * 7", "0 0 * * 0", "0 0 * * 1-5,0",
    "0 0 * * sat-sun", "0 0 * apr-jan *", "0 0 * apr-jan/3 *",
    "5 0 * 8 *", "15 14 1 * *", "0 22 * * 1-5",
    "23 0-20/2 * * *", "0 0,12 1 */2 *", "0 4 8-14 * *",
    "@daily", "@weekly", "@monthly", "@yearly", "@hourly", "@midnight",
    "0 0 30 2 *", "0 6 30 3 *", "0 0 31 * *",
    "* * * * * */15", "0 0 1 1 * 0 2030", "1 1 1 1 1 1 2025",
    "0 0 ? * 1", "0 0 1 * ?",
    # every-minute and every-30-minutes matter most across DST folds
    "* * * * *", "*/30 * * * *", "0 * * * *", "30 2 * * *", "0 2 * * *",
]

# Hash expressions. 'h' only -- 'r' is nondeterministic (see module docstring).
HASH_EXPRS = [
    "h h * * *", "h/15 * * * *", "h(0-29) * * * *", "h(30-59)/10 * * * *",
    "h h h * *", "h h(0-2) * * * h", "@daily", "@weekly",
]
HASH_IDS = ["hello", "world", "team-kryptonite", "x", "a-longer-hash-id-value"]

MINUTES = ["*", "0", "59", "*/5", "0-30", "0,15,30,45", "7-52/9", "*/61", "30-10"]
HOURS = ["*", "0", "23", "*/3", "9-17", "22-4", "0,12", "2", "3"]
DOMS = ["*", "1", "31", "l", "l-1", "15w", "1w", "*/7", "20-3", "?"]
MONTHS = ["*", "1", "12", "2", "jan", "dec", "apr-jan", "*/4", "feb-mar"]
DOWS = ["*", "0", "6", "7", "mon", "sun-sat", "5#3", "l5", "1-5", "sat-sun", "?"]
SECONDS = ["*", "0", "30", "*/15", "1-5"]
YEARS = ["*", "2025", "2024-2030", "2030"]

# Real DST transition instants (local wall time at which the shift happens).
# Chosen to cover: spring-forward, fall-back, southern hemisphere (reversed
# months), a half-hour offset zone, and a zone that abolished DST mid-history.
DST_EVENTS = [
    ("Europe/Athens", "2013-03-31T03:00:00"),
    ("Europe/Athens", "2013-10-27T04:00:00"),
    ("America/New_York", "2013-03-10T02:00:00"),
    ("America/New_York", "2013-11-03T02:00:00"),
    ("Europe/London", "2018-03-25T01:00:00"),
    ("Europe/London", "2018-10-28T02:00:00"),
    ("Australia/Sydney", "2019-10-06T02:00:00"),
    ("Australia/Sydney", "2019-04-07T03:00:00"),
    ("America/Sao_Paulo", "2018-11-04T00:00:00"),
    ("Australia/Lord_Howe", "2019-10-06T02:00:00"),  # 30-minute DST shift
]
# Zones without DST, as controls.
PLAIN_ZONES = ["UTC", "Asia/Kolkata", "Asia/Tokyo"]
TZ_LIBS = ["zoneinfo", "pytz"]


def random_expr(rnd):
    kind = rnd.random()
    if kind < 0.40:
        return rnd.choice(FRAGILE_EXPRS), None
    if kind < 0.50:
        return rnd.choice(HASH_EXPRS), rnd.choice(HASH_IDS)
    fields = [rnd.choice(MINUTES), rnd.choice(HOURS), rnd.choice(DOMS),
              rnd.choice(MONTHS), rnd.choice(DOWS)]
    if kind > 0.88:
        fields.append(rnd.choice(SECONDS))
    if kind > 0.96:
        fields.append(rnd.choice(YEARS))
    return " ".join(fields), None


def random_tz(rnd):
    """Return (tz_name, tzlib, anchor_datetime_or_None)."""
    r = rnd.random()
    if r < 0.45:
        return None, None, None                       # naive
    if r < 0.85:
        zone, when = rnd.choice(DST_EVENTS)           # near a transition
        return zone, rnd.choice(TZ_LIBS), datetime.fromisoformat(when)
    return rnd.choice(PLAIN_ZONES), rnd.choice(TZ_LIBS), None


def random_start(rnd, anchor):
    if anchor is not None:
        # Land within +/- 6h of the transition, on a minute boundary.
        return anchor + timedelta(minutes=rnd.randint(-360, 360))
    year = rnd.choice([2024, 2025, 2026, 2027, 2028])
    return datetime(year, rnd.randint(1, 12), rnd.randint(1, 28),
                    rnd.randint(0, 23), rnd.randint(0, 59))


def gen_tasks(rnd, n):
    tasks = []
    for _ in range(n):
        expr, hash_id = random_expr(rnd)
        tz, tzlib, anchor = random_tz(rnd)
        op = rnd.choice(
            ["expand", "is_valid", "next", "prev", "match", "range", "next", "prev"]
        )
        task = {"op": op, "expr": expr}
        if hash_id:
            task["hash_id"] = hash_id
        if rnd.random() < 0.15:
            task["day_or"] = False
        if rnd.random() < 0.08:
            task["second_at_beginning"] = True
        if rnd.random() < 0.05:
            task["implement_cron_bug"] = True
        if rnd.random() < 0.05:
            task["expand_from_start_time"] = True

        if op in ("next", "prev"):
            start = random_start(rnd, anchor)
            task.update({"start": start.isoformat(), "n": rnd.choice([3, 6, 10])})
            if tz:
                task.update({"tz": tz, "tzlib": tzlib})
        elif op == "match":
            task["when"] = random_start(rnd, anchor).isoformat()
            if tz:
                task.update({"tz": tz, "tzlib": tzlib})
        elif op == "range":
            start = random_start(rnd, anchor)
            span = rnd.choice([timedelta(hours=6), timedelta(days=1), timedelta(days=40)])
            if rnd.random() < 0.25:
                span = -span
            task.update({
                "start": start.isoformat(),
                "stop": (start + span).isoformat(),
                "limit": 40,
                "exclude_ends": rnd.random() < 0.3,
            })
            if tz:
                task.update({"tz": tz, "tzlib": tzlib})
        tasks.append(task)
    return tasks


def run_probe(python_bin, tasks, cwd):
    payload = "\n".join(json.dumps(t) for t in tasks) + "\n"
    proc = subprocess.run(
        [python_bin, "fuzz/probe.py"],
        input=payload, capture_output=True, text=True, cwd=cwd, timeout=600,
    )
    if proc.returncode != 0:
        raise RuntimeError(f"probe failed under {python_bin}:\n{proc.stderr[-3000:]}")
    return [json.loads(line) for line in proc.stdout.splitlines() if line.strip()]


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--seconds", type=float, default=90.0)
    ap.add_argument("--batch", type=int, default=300)
    ap.add_argument("--seed", type=int, default=None)
    ap.add_argument("--python-orig", default=".venv-baseline/bin/python")
    ap.add_argument("--python-port", default=".venv/bin/python")
    ap.add_argument("--cwd", default=".")
    args = ap.parse_args()

    seed = args.seed if args.seed is not None else int(time.time())
    rnd = random.Random(seed)

    started = time.time()
    total = 0
    divergences = []
    batches = 0
    coverage = {}

    print("differential fuzz: original(python) vs port(rust)")
    print(f"seed={seed} budget={args.seconds}s batch={args.batch}")
    print(f"orig={args.python_orig}")
    print(f"port={args.python_port}")
    print("-" * 72, flush=True)

    while time.time() - started < args.seconds:
        tasks = gen_tasks(rnd, args.batch)
        a = run_probe(args.python_orig, tasks, args.cwd)
        b = run_probe(args.python_port, tasks, args.cwd)
        batches += 1
        for ra, rb in zip(a, b):
            total += 1
            t = ra["task"]
            key = (t["op"], "tz" if t.get("tz") else "naive")
            coverage[key] = coverage.get(key, 0) + 1
            if ra["result"] != rb["result"]:
                divergences.append(
                    {"task": t, "original": ra["result"], "port": rb["result"]}
                )
                print("DIVERGENCE:", json.dumps(divergences[-1])[:2000], flush=True)
        print(f"[{time.time() - started:6.1f}s] inputs={total} "
              f"divergences={len(divergences)}", flush=True)

    elapsed = time.time() - started
    print("-" * 72)
    print(f"elapsed_seconds={elapsed:.1f}")
    print(f"batches={batches}")
    print(f"inputs_compared={total}")
    print(f"divergences={len(divergences)}")

    print("\ncoverage by (operation, timezone-aware?):")
    for (op, kind), count in sorted(coverage.items(), key=lambda kv: -kv[1]):
        print(f"  {count:7d}  {op:10s} {kind}")

    if divergences:
        by_expr = {}
        for d in divergences:
            k = (d["task"]["op"], d["task"]["expr"])
            by_expr[k] = by_expr.get(k, 0) + 1
        print("\ndivergences grouped by (op, expression), most frequent first:")
        for (op, expr), count in sorted(by_expr.items(), key=lambda kv: -kv[1]):
            print(f"  {count:5d}  {op:12s} {expr!r}")
    return 1 if divergences else 0


if __name__ == "__main__":
    sys.exit(main())
