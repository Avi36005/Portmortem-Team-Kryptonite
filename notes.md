# notes.md — running log

What broke, what it cost, and the measured number at the end of each phase.
Every number here was watched, not estimated.

---

## Phase 0 — bootstrap

**Baseline on the untouched original: `228 passed in 1.54s`**
at croniter commit `3c6ce9bcc5cc7f89116a58f43aaea67e760bff50`,
CPython 3.13.12, Apple Silicon.

Toolchain was not present on this machine — no `cargo`, no `rustup`. Installed
rustup (1.97.1) before anything else; that ran in the background while the
baseline was being established, which cost nothing.

Two virtualenvs, not one, and this mattered more than expected:

- `.venv-baseline/` — the **original Python croniter**, for the baseline number,
  the bug-hunter oracle, and the "original" side of differential fuzzing.
- `.venv/` — the **Rust port** installed as `croniter`.

They cannot be the same environment: both install a package called `croniter`.
Keeping both alive is what makes differential testing a single command later.

`.venv` disappeared once between creation and use — recreated it and verified
before continuing rather than guessing at the cause.

**Phase 0.4 proof rig: `222 failed, 6 passed`** with a deliberately wrong stub
bridge. That is the desired result: it proves the unmodified original suite is
importing and exercising Rust, before any real logic exists. The 6 "passes"
were accidental — a stub returning `False` from `is_valid` happens to satisfy
tests that assert invalidity.

Two things the test suite needs that a bare `.so` cannot give:
`from croniter.croniter import VALID_LEN_EXPRESSION` (a submodule) and
`from croniter.tests import base` (a subpackage). Solved by making the compiled
extension *be* `croniter.croniter` inside a real package. See DECISIONS #4.

---

## Phase 1 — field expansion, `is_valid`, error hierarchy

**Suite: `38/228` (was 6/228).**
Core unit tests: `22/22`.
Differential expander check vs the Python original: **308 compared, 0 divergences.**

The subtle part was not the parsing — it was the *sort*. croniter sorts each
expanded field with `key=lambda i: f"{i:02}" if isinstance(i, int) else i`,
which is a string sort over a list that mixes ints with `"*"` and `"l"`. The
real ordering is `"*" < digits < "l"`. `_get_next_nearest_diff` iterates that
list in order and returns the first match, so a naive numeric sort would have
produced wrong fire times with no compile error and no obvious failing test —
it would just be quietly wrong for `l` expressions. Encoded as the variant
order of `enum Item { Star, Num(i64), Last }` with derived `Ord`, and commented
so nobody "tidies" it later.

Second subtlety: `value_alias`. The 0→1 and 7→0 remaps are *suppressed* for
particular (field, column-count) pairs. A consequence that looks like a bug but
is faithful: in a 7-field expression, day-of-week `7` is **not** normalized to
`0`, so it fails the range check and the expression is rejected. Ported as-is.

`match` is listed under Phase 1 in the plan but genuinely depends on
`get_prev` — `match_range` constructs a croniter and steps backwards. It is
therefore unlocked by Phase 2, not Phase 1. Noted rather than reordered.

Rust-vs-Python friction, all trivial: `do` is a reserved word (→ `hash_do`);
`downcast` became `cast` in PyO3 0.29; one borrow-checker rejection where the
Python code reassigns the loop variable it is still matching against, fixed by
copying the capture groups out before the reassignment.

---

## Phase 2 — search engine

**Suite: `200/228` (was 38/228).** Core unit tests: `44/44`.

The hardest phase, as expected, and the cost was concentrated in two places
that had nothing to do with cron.

**`relativedelta` is not a timedelta.** croniter writes
`d += relativedelta(days=diff, hour=0, minute=0, second=0)` — plural names are
relative offsets, singular names are absolute replacements, and `dateutil`
applies them in a specific order: compute year, compute month, **clamp the day
to the length of the target month**, replace the absolute fields, *then* add
the relative ones. `Mar 31 + relativedelta(months=-1)` is `Feb 28`. Getting the
clamp or the ordering wrong produces dates that are off by a day or two only in
some months, which is exactly the kind of bug that survives a casual test pass.
Ported as its own module with its own tests before touching the search loop.

**The suite hung instead of failing.** After wiring the bridge, `pytest` never
finished. Bisecting by file, then by test, found `test_match` — and the cause
was `proc_second` reading direction from the struct field `self.is_prev` rather
than the current call's `is_prev`. `match_range` builds an iterator with the
default `is_prev=false` and steps it *backwards*, so on 6-field expressions the
second field searched forward while everything else searched backward. The
candidate oscillated forever.

An infinite loop was the lucky outcome. The same bug on a different code path
would have returned a plausible-looking wrong time.

Time cost: roughly half the phase went to those two, and almost none to the
`proc_*` pipeline itself — transcribing that structure verbatim from Python,
including the `(changed, d)` tuple protocol and the `month`/`year` variables
that track the candidate separately from `d`, was mechanical and worked first
time. That is an argument for resisting the urge to restructure: the parts I
translated literally were the parts that gave no trouble.

---

## Phase 3 — croniter_range

**Suite: `215/228`.** `test_croniter_range.py` went 1/16 → 16/16.

Mostly plumbing, with one design constraint: `croniter_range` takes a
`_croniter` parameter for injecting a subclass, so the stepping loop has to
construct an arbitrary *Python* object and cannot live in `core`. Split the
difference — the bound arithmetic (±1µs nudge, direction, year span) is in
`core::range::setup()` and is called by both the native Rust iterator and the
bridge's generator, so only the loop is duplicated and never the semantics.

Two small real bugs fell out here, both caught by reading the failure rather
than guessing: `_get_nth_weekday_of_month` was returning a list where the test
compares against a tuple, and tests assign `_max_years_between_matches`
directly on the instance, which a `#[pyclass]` rejects unless you give it a
setter.

---

## Phase 4 — timezones and DST

**Suite: `228/228`.** Full parity.

CLAUDE.md predicted this was "the expected place to lose points" and that
`chrono-tz` and `dateutil` would legitimately disagree on ambiguous local
times. They would have — so the port doesn't ask `chrono-tz`. The `WallClock`
trait lets `pybridge` answer timezone questions using **the caller's own
`tzinfo` object**, which means `zoneinfo`, `pytz` and `dateutil` each behave as
their own tests expect, while all the cron logic stays in Rust. That single
decision is why this phase cost 11 tests instead of losing them.

The last four failures were the most educational bug of the whole port, and it
was invisible in the timestamps. `test_timezone_winter_time` walks
`*/30 * * * *` through Athens' fall-back, where 03:00–04:00 happens twice. Side
by side, `ct.cur` matched the original **exactly** at every step — the port was
finding the right instants. It just printed the repeated hour as `03:00+03:00`
instead of `03:00+02:00`.

The bridge was rebuilding datetimes from wall-clock fields and attaching the
zone with `replace(tzinfo=tz)`. `replace` has no way to know which side of a
fold you meant, so it always gives `fold=0` and the pre-transition offset:
right instant, wrong offset. croniter never does this — it renders via
`fromtimestamp(ts, utc).astimezone(tz)`, starting from an unambiguous instant
so `astimezone` can set `fold` itself.

The generalisable lesson: **during a DST fold a wall-clock reading is not a
sufficient representation of a point in time.** Any API that round-trips
through one loses the fold silently. Once `calc` returned `(wall, instant)` and
the bridge rendered from the instant, all four passed at once.

---

## After 228/228 — hardening, and what the fuzzer found

Reaching full parity is not the same as being right, and the gap between those
two showed up immediately.

The first differential harness only generated **naive** datetimes. That is a
poor place to stop, because the riskiest code in the port — the DST fold
handling from Phase 4 — was the one path with no differential coverage at all.
Extending generation to timezone-aware start times within ±6h of real DST
transitions (Athens, New York, London, Sydney, São Paulo, and Lord Howe for its
30-minute shift), in both `zoneinfo` and `pytz` flavours, plus the semantic
flags and `croniter_range`, produced **221 divergences out of 164,500** on the
first run.

All 221 had one cause, and it was an exception *type*, not a value:
`_get_low_from_current_date_number` raises a bare `ValueError`, and the port
raised `CroniterError`. Since `CroniterError` subclasses `ValueError`, every
`except ValueError` still caught it — which is exactly why 228/228 stayed green
before and after the fix.

**The passing test suite could not have found this.** That is the whole
argument for the fuzz harness: it is not a nicer way to run the same checks,
it checks something the tests structurally do not. Worth more than three more
passing tests, as the plan said.

Also worth recording: **zero value divergences**. In 164,500 comparisons there
was no input where both implementations succeeded and disagreed about a fire
time. The date arithmetic was right; only an error label was wrong.

The same audit found a latent sibling bug — the bridge flattened *any* Python
exception from a `tzinfo` callback into `ValueError`, so an `OverflowError`
from an out-of-range timestamp would have surfaced as the wrong type. Fixed
with a `Foreign { class, msg }` variant that preserves the original type.

One more free proof turned up while tidying. Both benchmark workloads print a
checksum summing every fire time's timestamp — and they are **identical**
(`26611225207500` over 9,996 fire times spanning all six expression shapes).
That had been true all along; I simply had not looked. `run_bench.py` now
compares them and exits non-zero on a mismatch, so the benchmark doubles as a
correctness check. A speed win on the wrong answer is not a win.

Two smaller things from this pass:

- The oracle had to be redesigned to terminate at all. `croniter.match()`
  builds a fresh croniter per call, so the exhaustive minute-walk is hopeless
  on sparse expressions. It now walks exhaustively under 24h and samples 250
  minutes above that, reporting the two counts separately rather than blending
  weaker evidence into stronger.
- The README claimed the Docker build worked. It had never been run — the
  daemon was not available on this machine. Corrected to say so explicitly
  rather than leave an unverified claim in a deliverable. An unverified claim
  in the README is exactly the failure mode Rule 3 exists to prevent, and I
  had written one without noticing.

---

## Final numbers

| | |
|---|---|
| Original baseline | 228/228 in 1.54s |
| Port | **228/228** in 2.08s |
| Rust tests | 65/65 (53 unit + 12 CLI integration) |
| Differential fuzz | 160,500 inputs / 120.1s / **0 divergences** (~66k timezone-aware) |
| Oracle self-consistency | 19,440 pairs, **0** contradictions |
| clippy | 0 warnings (`--workspace --all-targets`) |
| `unsafe` in `core/` | 0, compiler-enforced |
| `pyo3` in `core/`'s dep tree | 0 |
| Performance | 25.3x mean, 26.1x p99, 3.2x smaller RSS |
| Bench checksum equivalence | identical over 9,996 fire times |

What I would tell someone starting the same port: translate the control flow
literally, even where it looks clumsy, and spend the saved time on the two or
three places where the *host language's* semantics differ — date arithmetic,
sort keys, and how a timezone is attached to a naive time. Every bug in this
port came from one of those three, and none came from the cron logic itself.

---

## Who wrote what

**40 commits across three contributors**, as of this commit. Confirm with
`git shortlog -sne`.

- **Sahil Deshmukh** (`@2005sahildeshmukh`) — next/prev search engine, `#` and
  `W` stepping, `CronIterator` state machine, CLI binary, differential fuzz
  harness, benchmark workload, Dockerfile — 11 commits
- **Hardik Hinduja** (`@Hardik182005`) — field tokenizer and expander, the
  `Item` enum and its sort order, matching and validation, PyO3 bindings,
  `PyTzClock`, DST fold handling — 10 commits
- **Avinash Gehi** (`@Avi36005`) — hash expander, `croniter_range`, error
  hierarchy, upstream bug hunt and reports, QA, docs, release — 19 commits

Avinash's commits are split across two e-mail identities
(`177668299+Avi36005@users.noreply.github.com` and
`2023.avinash.gehi@ves.ac.in`), so `git shortlog` prints four rows for three
people. 11 + 10 + 19 = 40.

Verified with `make all`: 228/228 pytest, 65/65 Rust tests, zero unsafe.

