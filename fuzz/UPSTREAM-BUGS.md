# Upstream bugs found in croniter — Bug Catcher submission

Team Kryptonite · Track D · croniter (Python) → Rust

Two genuine defects in the **original Python croniter**, found while porting.
Tested at commit `3c6ce9bcc5cc7f89116a58f43aaea67e760bff50` (v6.2.5.dev0) on
CPython 3.13.12.

Neither is a disagreement about what a cron expression *ought* to mean. In both
cases croniter answers the same question two different ways depending on which
API you ask, so one of the two answers is wrong under any reading of the
semantics. Both are reproducible in three lines.

| # | Defect | Impact |
|---|---|---|
| 1 | `get_next` skips a fire time when a DST shift is not a whole hour | a scheduled job silently does not run |
| 2 | `croniter_range`'s stop test ignores the UTC offset | silently returns too few results, or results outside the requested interval |

---

# Bug 1 — `get_next` skips a fire time when a DST shift is not a whole hour

### Repro steps

```python
import zoneinfo
from datetime import datetime
from croniter import croniter

tz = zoneinfo.ZoneInfo("Australia/Lord_Howe")
start = datetime(2019, 10, 6, 1, 43, tzinfo=tz)

nxt = croniter("0 * * * *", start).get_next(datetime)
prv = croniter("0 * * * *", nxt).get_prev(datetime)

print(nxt)   # 2019-10-06 03:00:00+11:00   (UTC 16:00)
print(prv)   # 2019-10-06 02:30:00+11:00   (UTC 15:30)

print(croniter.match("0 * * * *", datetime(2019, 10, 6, 2, 30, tzinfo=tz)))
# True
```

### What the original does wrong

Three APIs give two different answers about whether `02:30+11:00` is on an
**hourly** schedule:

| API | Says 02:30+11:00 is on the schedule? |
|---|---|
| `get_next` | **No** — jumps from 01:43 straight to 03:00 |
| `get_prev` | **Yes** — steps back from 03:00 and lands on it |
| `match` | **Yes** |

At least one is wrong. Two independent things are provably broken:

1. **`get_prev` overshoots.** Stepping back from `nxt` lands at 02:30, which is
   *after* the `start` that produced `nxt`. A backward step from a fire time
   must never land after the start that generated it, so `get_next` skipped a
   fire time that croniter itself considers valid.

2. **`match` contradicts the expression.** `match("0 * * * *", …02:30…)`
   returns `True` for a **minute-0** schedule at **minute 30**. No reading of
   `0 * * * *` makes minute 30 a fire time.

### Root cause

`Australia/Lord_Howe` is the only zone in the world with a **30-minute** DST
shift: on the spring transition 02:00 jumps to 02:30. The 02:00 fire time
therefore does not exist, and `_add_tzinfo` (croniter.py:179) resolves a
non-existent local time by walking forward one minute at a time until one
exists — arriving at 02:30.

Walking forward is deliberate and documented. The defect is that the forward
walk **manufactures a fire time at a minute the expression never names**, and
`get_prev`/`match` then treat it as real while `get_next` does not.

In ordinary 1-hour zones the same walk lands on 03:00, which *is* a fire time
for an hourly schedule, so the inconsistency is invisible. The 30-minute shift
is what exposes it.

### Scope

- Reproduces on **every** Lord Howe spring transition, 2018–2022 (verified).
- Reproduces with `0 * * * *`, `@hourly`, `0 0-23 * * *`, `*/61 * * * *`.
- Does **not** reproduce with `0,30 * * * *` — there 02:30 is a legitimate fire
  time, so the two paths coincidentally agree.
- Not reproducible in 1-hour-shift zones (New York, London, Sydney, Athens all
  verified clean).

### Suggested fix direction

The forward and backward paths must agree. Either:

- the forward search should also accept the shifted resolution, so `get_next`
  returns 02:30 as the realisation of the skipped 02:00; **or**
- `_add_tzinfo`'s forward walk should not yield a time whose fields the
  expression does not name, in which case `get_prev` and `match` are the ones
  to correct.

Today they disagree, and that is the bug regardless of which is chosen.

### How our port handles it

**We reproduce it exactly** — same skipped fire time, same `get_prev` result,
same `match` answer. A port's job is to behave like the thing it ports,
including where that is wrong.

This is a deliberate decision, recorded as DECISIONS.md #18. "Fixing" it in
the port would mean the original test suite still passes, our differential
fuzzer starts reporting divergences against upstream, and the port quietly
stops being a port. If upstream fixes this, the fix should be ported across,
not anticipated.

---

# Bug 2 — `croniter_range`'s stop test ignores the UTC offset

One defective comparison, two observable failure modes: it returns **too few**
results in one direction and results **outside the requested interval** in the
other.

### Root cause (shared by both symptoms)

`croniter_range` decides when to stop with (croniter.py:1430-1441):

```python
if start < stop:                    # forward
    def cont(v): return v < stop
else:                               # reverse
    def cont(v): return v > stop
```

CPython specifies:

> If both comparands are aware and have the same `tzinfo` attribute, the common
> `tzinfo` attribute is ignored and the base datetimes are compared.

Inside one timezone every datetime shares the same `tzinfo` object, so across a
DST transition this comparison silently uses **wall-clock order** instead of
real elapsed time. Wall clock and real time disagree exactly across a
transition, which is where both symptoms live.

---

## Symptom A — silently returns too few results (fall-back)

### Repro steps

```python
import zoneinfo
from datetime import datetime
from croniter import croniter_range

tz = zoneinfo.ZoneInfo("Australia/Sydney")
start = datetime(2019, 4, 7, 2, 22, tzinfo=tz)            # +11:00, before the fold
stop  = datetime(2019, 4, 7, 2, 26, tzinfo=tz, fold=1)    # +10:00, after the fold

print(len(list(croniter_range(start, stop, "*/13 * * * *"))))   # 1
```

### What the original does wrong

The interval contains **six** fire times. `croniter_range` returns **one**.

```
2019-04-07 02:26:00+11:00   UTC 15:26      <- the only one returned
2019-04-07 02:39:00+11:00   UTC 15:39
2019-04-07 02:52:00+11:00   UTC 15:52
2019-04-07 02:00:00+10:00   UTC 16:00
2019-04-07 02:13:00+10:00   UTC 16:13
2019-04-07 02:26:00+10:00   UTC 16:26
```

Stepping the underlying `croniter` over the same window produces all six, with
or without `max_years_between_matches`. Only the range's termination test is
wrong.

This is **silent data loss** — no exception, no warning. The generator simply
ends and the caller sees a prefix of the correct answer.

The comparison that fails:

```python
v    = datetime(2019, 4, 7, 2, 39, tzinfo=tz)          # UTC 15:39
stop = datetime(2019, 4, 7, 2, 26, tzinfo=tz, fold=1)  # UTC 16:26

v < stop                                    # False  <- wrong, ignores fold
v.astimezone(utc) < stop.astimezone(utc)    # True   <- correct
```

`fold` is part of neither the base datetime nor the ignored `tzinfo`, so it is
discarded entirely.

---

## Symptom B — returns fire times outside the requested interval (spring-forward)

### Repro steps

```python
import zoneinfo
from datetime import datetime, timezone
from croniter import croniter_range

tz = zoneinfo.ZoneInfo("Europe/London")
a = datetime(2018, 3, 25, 1, 15, tzinfo=tz)   # 01:15 GMT = UTC 01:15
b = datetime(2018, 3, 25, 5, 0,  tzinfo=tz)   # 05:00 BST = UTC 04:00

for d in croniter_range(b, a, "0 * * * *"):
    print(d, d.astimezone(timezone.utc))
```

### What the original does wrong

```
2018-03-25 05:00:00+01:00   UTC 04:00
2018-03-25 04:00:00+01:00   UTC 03:00
2018-03-25 03:00:00+01:00   UTC 02:00
2018-03-25 02:00:00+01:00   UTC 01:00    <- BEFORE the requested lower bound
```

The caller asked for fire times between UTC 01:15 and UTC 04:00. The last
result is at **UTC 01:00**, fifteen minutes before the interval starts.

Returning a value outside the requested range is unambiguously wrong — there is
no semantics under which `croniter_range(a, b)` should yield something outside
`[a, b]`.

The failing comparison:

```python
v = datetime(2018, 3, 25, 2, 0, tzinfo=tz)    # UTC 01:00
a = datetime(2018, 3, 25, 1, 15, tzinfo=tz)   # UTC 01:15

v > a                                   # True   <- wall clock 02:00 > 01:15
v.astimezone(utc) > a.astimezone(utc)   # False  <- real order
```

Note the two bounds here have *different UTC offsets* (GMT vs BST) and are both
unambiguous — `fold` is not involved at all. Symptom A is the `fold` case;
Symptom B shows the same comparison is wrong for **any** offset change within
the zone, which is the more general statement.

### Suggested fix

Compare instants, not wall-clock readings:

```python
if start < stop:
    def cont(v):
        return v.astimezone(timezone.utc) < stop.astimezone(timezone.utc)
else:
    def cont(v):
        return v.astimezone(timezone.utc) > stop.astimezone(timezone.utc)
```

or normalise both bounds to UTC once, up front, and compare those.

### How our port handles it

**We reproduce both symptoms exactly** — 1 result for Symptom A, the same
out-of-range value for Symptom B. Same reasoning as Bug 1: see DECISIONS.md
#18.

---

# Prior art check — are these already reported?

Searched `pallets-eco/croniter`'s issue tracker on 2026-08-01. **Neither bug
appears to be reported.**

croniter has a long history of DST bugs, so this mattered. What is different
about these two:

| search | result |
|---|---|
| `Lord_Howe`, "half hour", "30 minute" | **no matches anywhere** in issues or PRs |
| `croniter_range` | only **#60** (last-Thursday pattern, closed not-planned) and **#20** (infinite loop, fixed 2022). Neither is about truncation or out-of-range results |
| `get_prev` | **#203**, **#85**, **#83**, **#62**, **#57**, **#10** — all closed; leap-year, state-mutation and API-parameter issues. None about a DST overshoot |
| `DST` | **#191, #151, #149, #138, #91, #70, #64, #56** — **all closed** |

The decisive evidence is that the closed DST issues are genuinely fixed at our
kickoff commit, while our two still reproduce there. Verified by running their
original reproductions against `3c6ce9bc`:

| issue | original complaint | at our commit |
|---|---|---|
| **#151** | `get_next` returns a past time after a fall-back (`America/Los_Angeles`, `45 1 * * *`) | **fixed** — returns UTC 09:45, not 08:45 |
| **#191** | weekly schedule jumps to 04:10 across a fall-back (`Europe/Prague`, `10 3 * * 0`) | **fixed** — steps correctly week to week |
| **#147** | `croniter_range` wrong across a fall-back (`Europe/Berlin`, `0 1 * * *`) | **correct** — Berlin falls back Oct 25, so Oct 26 at +01:00 is right; the report was mistaken |

So the maintainers have been actively fixing this area, and every previously
reported DST defect we could find is resolved. Ours survive.

Why these two were missed by earlier reports is also explicable:

- **Bug 1 needs a sub-hour DST shift.** Every prior DST report uses a
  whole-hour zone (Berlin, Prague, Los Angeles, London). In those zones the
  forward-walk lands on a time that *is* a fire time for an hourly schedule, so
  the inconsistency is invisible. `Australia/Lord_Howe` is the only zone on
  Earth with a 30-minute shift, and nothing in the tracker mentions it.
- **Bug 2 needs the two range bounds to sit on different UTC offsets.** The
  usual `croniter_range` call passes bounds days apart on the same offset, where
  the comparison is accidentally correct.

**Caveat, stated plainly.** GitHub's issue search is fuzzy and matches mostly
titles; a duplicate could be buried in a comment thread, a closed PR discussion,
or a downstream tracker (Airflow and Sentry both carry croniter DST issues).
This is a good-faith search, not a proof of novelty.

---

# How these were found

Three harnesses, each stronger than the last. The progression matters, because
the first one found nothing and *looked* like evidence of correctness.

### `fuzz/oracle.py` — 0 findings

Checked one property: `match` must agree with `get_next`. Ran 19,440 pairs,
found nothing.

It found nothing because it was too weak in two specific ways: it only used
**naive** start times, so it never touched the timezone code at all, and it
never called `get_prev`, so it could not see the two APIs disagree. Neither bug
was reachable. **An invariant that holds trivially is not evidence.**

### `fuzz/invariants.py` — found both bugs

Five properties instead of one, with roughly half the start times within four
hours of a real DST transition, in both `zoneinfo` and `pytz` flavours:

| | invariant |
|---|---|
| I1 | repeated `get_next` strictly increases as UTC instants; `get_prev` strictly decreases |
| I2 | `get_prev` from a fire time must not overshoot the start that produced it |
| I3 | every fire time satisfies `match`; sampled minutes between consecutive fire times do not |
| I4 | `croniter_range(a, b)` equals every fire time `t` with `a <= t <= b` |
| 5 | definitionally identical expressions (`0` vs `7` vs `sun`, `@daily` vs `0 0 * * *`) produce identical schedules |

Bug 1 is an I2 violation. Bug 2 Symptom A is an I4 violation.
~23,800 cases → 15 findings, all instances of these two bugs.

### `fuzz/invariants2.py` — found Symptom B, confirmed nothing else

Checks fire times against the **decoded expression fields** rather than against
another croniter API, so a bug consistent across all APIs would still be
caught. Plus generator/stepping agreement, `ret_type` agreement,
`second_at_beginning` equivalence, `day_or` subset, reverse-range symmetry, and
`expand` determinism.

~23,900 cases → 927 raw findings, triaged by `fuzz/triage.py`:

```
F2_FIELDS_TZ      750  ->  750 explained by croniter's documented
                           skip-forward, 0 unexplained
G5_REVERSE_RANGE  177  ->  all timezone-related; Symptom B above
                           (0 naive failures)
```

**The 750 field mismatches are all legitimate.** When a local time does not
exist, croniter deliberately walks forward, and the result correctly has
different fields. Reporting those as bugs would have been noise, so
`fuzz/triage.py` filters them by asking whether the time croniter *should* have
returned actually existed. Zero survived. That negative result is stated here
rather than buried, because it is the part that makes the two positives
credible.

### Reproduce the hunt

```bash
make setup                                    # first time only
.venv-baseline/bin/python fuzz/invariants.py  --seconds 120
.venv-baseline/bin/python fuzz/invariants2.py --seconds 120
.venv-baseline/bin/python fuzz/triage.py
```

### One correction worth recording

The I4 check initially reported **1,408** findings that were entirely our own
error: `croniter_range` is inclusive of both ends, so a `start` that is itself
a fire time is legitimately returned by the range and legitimately missed by a
`get_next` reference that begins *at* `start`. The corrected check starts the
reference a microsecond earlier.

Both bugs above survived that correction. Nothing else did.
