<div align="center">

# ⏱️ croniter → Rust

**A complete, behaviour-preserving port of [croniter](https://github.com/pallets-eco/croniter) from Python to Rust.**

[![Tests](https://img.shields.io/badge/original_test_suite-228%2F228_passing-2ea44f?style=for-the-badge)](#-results)
[![Unsafe](https://img.shields.io/badge/unsafe_in_core-0-2ea44f?style=for-the-badge)](#-verify-our-claims-yourself)
[![Fuzz](https://img.shields.io/badge/differential_fuzz-160%2C500_inputs_·_0_divergences-2ea44f?style=for-the-badge)](#-differential-fuzzing)
[![Bugs](https://img.shields.io/badge/upstream_bugs_found-2-orange?style=for-the-badge)](#-upstream-bugs-found)

[![Rust](https://img.shields.io/badge/Rust-1.97-000000?style=flat-square&logo=rust)](https://www.rust-lang.org/)
[![Python](https://img.shields.io/badge/CPython-3.13-3776AB?style=flat-square&logo=python&logoColor=white)](https://www.python.org/)
[![PyO3](https://img.shields.io/badge/PyO3-0.29-cc7722?style=flat-square)](https://pyo3.rs/)
[![License](https://img.shields.io/badge/License-MIT-blue?style=flat-square)](LICENSE)

**Team Kryptonite** · Port Mortem 2026 · Track D — Python → Rust

</div>

---

## 📋 Table of contents

| | |
|---|---|
| [What we built](#-what-we-built) | the deliverable, in one paragraph |
| [Results](#-results) | the honest numbers |
| [Scope](#-scope-whats-in-whats-out) | what is ported and what is not |
| [Quick start](#-quick-start) | build and run in one command |
| [Verify our claims](#-verify-our-claims-yourself) | don't take our word for any of it |
| [Why the architecture looks like this](#-why-the-architecture-looks-like-this) | the two-crate split, the one seam |
| [How we did it](#-how-we-did-it) | phase by phase, with the number at each step |
| [Differential fuzzing](#-differential-fuzzing) | 160,500 inputs, 0 divergences |
| [Upstream bugs found](#-upstream-bugs-found) | two real defects in croniter |
| [Performance](#-performance) | 25x, with the confounders stated |
| [Repository layout](#-repository-layout) | where everything lives |
| [Eligibility](#-eligibility) | why this is a valid entry |

---

## 🎯 What we built

croniter parses cron expressions (`0 9 * * 1-5` = "9am on weekdays") and computes
next/previous fire times. This is a **complete port to Rust** — every feature,
including the awkward ones: `L` (last day of month), `W` (nearest weekday), `#`
(nth weekday), hash/`H` expressions, `?`, second and year fields, `croniter_range`,
and full DST transition handling.

The deliverable is a **standalone native Rust package** with **zero Python
dependency**. It builds a binary that links no interpreter and spawns no
subprocess. A separate, test-only PyO3 bridge lets the *unmodified original
Python test suite* run against the Rust code — that bridge is glue, not part of
what ships.

---

## 📊 Results

<div align="center">

### 228 / 228 — the complete original test suite, unmodified

</div>

```
Original baseline:  228/228 passing in 1.54s   (untouched Python, commit 3c6ce9bc)
Rust port:          228/228 passing in 1.78s   (same suite, via PyO3 bridge)

Rust tests:         65 passing  (53 unit + 12 CLI integration)
clippy:             0 warnings  (--workspace --all-targets)
unsafe in core/:    0           (compiler-enforced, #![forbid(unsafe_code)])
pyo3 in core/:      0           (verifiable via `cargo tree`)

Differential fuzz:  160,500 inputs vs the Python original — 0 divergences
Benchmark:          25.3x faster (mean), 26.1x at p99, 3.2x smaller peak RSS
Equivalence:        both benchmarks produce an identical checksum over 9,996 fire times
```

### Per-file breakdown

| Test file | Tests | Passing | |
|---|---:|---:|:--|
| `test_croniter.py` | 148 | **148** | ✅ |
| `test_croniter_hash.py` | 54 | **54** | ✅ |
| `test_croniter_range.py` | 16 | **16** | ✅ |
| `test_croniter_dst_repetition.py` | 4 | **4** | ✅ |
| `test_croniter_random.py` | 4 | **4** | ✅ |
| `test_croniter_speed.py` | 2 | **2** | ✅ |
| **Total** | **228** | **228** | 🎉 |

> **On honesty.** Every number here was measured on this machine and watched. The
> project brief quotes a 4.33s baseline; we measure **1.54s** — different hardware.
> We report what we observed. Where we could not verify something (the Docker
> path), we say so explicitly rather than claim it.

---

## 🎯 Scope: what's in, what's out

Per the organizers' ruling that scope must be stated explicitly:

### ✅ In scope — fully ported, all covered by the original tests

| Area | Status |
|---|---|
| Field expansion: ranges, steps, lists, wrap-around (`Sat-Sun`, `Apr-Jan`) | ✅ complete |
| Alphabetic names (`mon`, `jan`), `?`, `@daily`/`@weekly`/`@yearly`/… aliases | ✅ complete |
| `L` (last day), `W` (nearest weekday), `#` / `lN` (nth weekday) | ✅ complete |
| 5-, 6- (second) and 7-field (year) expressions, `second_at_beginning` | ✅ complete |
| `get_next`, `get_prev`, `get_current`, `set_current`, `all_next`, `all_prev`, `iter` | ✅ complete |
| `match`, `match_range`, `is_valid`, `expand` | ✅ complete |
| `croniter_range`, incl. the `_croniter` subclass-injection parameter | ✅ complete |
| `HashExpander` — `H`/`R`, with `binascii.crc32` semantics | ✅ complete |
| Full exception hierarchy, mapped to the exact Python types | ✅ complete |
| DST: ambiguous and non-existent local times, in the standalone crate | ✅ complete |
| `day_or`, `implement_cron_bug`, `expand_from_start_time`, `max_years_between_matches` | ✅ complete |

### ⚪ Out of scope — and why

| Not ported | Reason |
|---|---|
| Upstream CI config, `pyproject.toml`, packaging metadata | Not core language behaviour; per organizer ruling only the library itself needs porting |
| `OVERFLOW32B_MODE` 32-bit degraded path | A CPython 32-bit workaround ([cpython#101069](https://github.com/python/cpython/issues/101069)); Rust has no equivalent limit. Exposed as a flag so the tests that toggle it pass |
| Deterministic reproduction of `R` (random) hash expressions | Impossible by construction — seeded from Python's global RNG. Range and validity are matched; the exact value cannot be. See [DECISIONS #7](DECISIONS.md) |

**No test is skipped, xfailed, loosened, or worked around.** All 228 pass on the
byte-for-byte original files.

---

## 🚀 Quick start

### Build the shipped artifact — one command

```bash
cargo build --release -p croniter-core     # -> target/release/croniter
```

### Use it

```bash
$ croniter next '0 9 * * 1-5' -n 3 --start 2025-06-07T12:00:00
2025-06-09T09:00:00
2025-06-10T09:00:00
2025-06-11T09:00:00

$ croniter next '0 12 L * *' -n 2 --start 2024-02-01T00:00:00   # last day, leap year
2024-02-29T12:00:00
2024-03-31T12:00:00

$ croniter next '0 9 * * 5#3' -n 2 --start 2025-06-01T00:00:00  # third Friday
2025-06-20T09:00:00
2025-07-18T09:00:00

# Real IANA timezones with full DST resolution — no Python involved
$ croniter next '*/30 * * * *' --tz Europe/Athens --start 2013-10-27T02:00:00 -n 4
2013-10-27T02:30:00
2013-10-27T03:00:00     # +03:00
2013-10-27T03:30:00
2013-10-27T03:00:00     # +02:00 — the repeated hour, resolved correctly
```

Commands: `next` · `prev` · `range` · `match` · `validate` · `bench`. Run
`croniter --help`.

### Run the original Python test suite against the Rust port

```bash
make setup     # creates both virtualenvs (first run only, ~1 min)
make test      # builds the PyO3 bridge, runs tests/original/
```

> `make setup` builds **two** virtualenvs, and it must: `.venv-baseline/` holds
> the original Python croniter (the reference side), `.venv/` holds the Rust port
> installed under the same name. Both install a package called `croniter`, so
> they cannot share an environment.

### Docker

```bash
docker build -t croniter-rs .
docker run --rm croniter-rs next '0 9 * * 1-5' -n 5
```

The runtime stage is `debian:bookworm-slim` with the binary copied in and **no
Python installed** — the sharpest demonstration that the artifact carries no
source-language runtime.

> ⚠️ **Not verified by us.** The Docker daemon was unavailable on our build
> machine, so this image was never actually built. The `cargo build` path above
> *was* run and is where every number in this README comes from. We don't claim
> results we didn't observe.

### All available commands

```
make build    Build the shipped artifact (target/release/croniter)
make test     Build the PyO3 bridge and run the ORIGINAL test suite
make unit     Run the Rust unit + CLI integration tests
make verify   Check tests/original/ fingerprints AND that core/ has no unsafe
make fuzz     120s differential fuzz: Python original vs Rust port
make bench    Benchmark both implementations (p50/p95/p99 + RSS)
make hunt     Hunt for upstream croniter bugs (invariant harnesses)
make demo     Run every claim live (for the demo video)
make setup    Create both virtualenvs from scratch
make all      verify + build + unit + test
```

---

## 🔍 Verify our claims yourself

```bash
make verify
```

Checks three things you should not have to take on trust:

**1. The tests are the originals, untouched.**
```bash
sha256sum -c .test-hashes.sha256      # fingerprints taken at kickoff
git log --oneline -- tests/original/  # exactly one commit: the vendoring
```
The hashes were recorded before a line of Rust existed. `tests/original/` is
byte-for-byte upstream — 3,708 SLOC across 8 files.

**2. Zero `unsafe` in the shipped crate.**
```bash
grep -rn "unsafe" core/src/ | grep -v "forbid(unsafe_code)" | wc -l   # 0
head -1 core/src/lib.rs                                               # #![forbid(unsafe_code)]
```
Belt and braces — the attribute already makes `unsafe` a compile error.

**3. No Python in the shipped crate.**
```bash
cargo tree -p croniter-core --edges normal | grep -ci pyo3            # 0
```
`croniter-core` depends on `chrono`, `chrono-tz`, `regex`, `thiserror`. Nothing
else. It cannot link Python even by accident.

---

## 🏛 Why the architecture looks like this

### Two crates, and why that isn't a loophole

```
core/       ← THE PORT. Ships. #![forbid(unsafe_code)]. No Python, at all.
pybridge/   ← TEST-ONLY glue. PyO3. Not part of the artifact.
```

The organizers ruled that a test-only FFI adapter is fine — *"keep the actual
porting logic in your Rust code, the FFI layer should only bridge the tests, not
do the work."* That is exactly this split, and it is **measurable**:

| | lines | contains |
|---|---:|---|
| `core/` | ~3,200 | all parsing, expansion, matching, date arithmetic, DST |
| `pybridge/` | ~1,250 | argument marshalling, type conversion, error re-raising |

`pybridge` contains **zero** occurrences of `calc_next`, `proc_*`,
`nearest_diff`, `value_alias`, `crc32` or `step_search` — the search engine and
parser exist only in `core`. The only Python files shipped are a 48-line
re-export shim (a transcription of upstream's own `__init__.py`, which is itself
pure re-export) and an 11-line `unittest.TestCase` subclass copied verbatim from
the original.

Because the two are separate build targets, "no source-language runtime" is
**checkable** rather than merely asserted.

### The one interesting seam

Timezone data is consulted in exactly **one** place: the `WallClock` trait. The
search engine never sees a timezone — it works on wall-clock time, exactly as
croniter's `_calc` does on `now.replace(tzinfo=None)`.

```
┌────────────────────────────────────────────┐
│  core: search engine (no timezones at all) │
└────────────────────┬───────────────────────┘
                     │ WallClock trait
        ┌────────────┼─────────────┐
        ▼            ▼             ▼
   FixedClock     TzClock      PyTzClock
   naive/offset   chrono-tz    caller's tzinfo
   (core)         (core)       (pybridge only)
```

- **`FixedClock`** and **`TzClock`** live in `core`, so the standalone binary
  handles real IANA zones and DST with no Python.
- **`PyTzClock`** lives in `pybridge` and asks *the caller's own `tzinfo`
  object*. This matters: `zoneinfo`, `pytz` and `dateutil` genuinely disagree on
  ambiguous local times, and the test suite has separate tests pinning each.
  Shipping a fourth opinion would guarantee disagreement with at least one.

See [DECISIONS.md](DECISIONS.md) #11 and #19.

---

## 🛠 How we did it

Ported module by module, running the full suite after each phase and recording
the real number. Never "should pass now" — always a measured count.

| Phase | What landed | Suite |
|---|---|---:|
| **0** — Bootstrap | Vendored + fingerprinted tests, verified the baseline, built a deliberately-wrong stub bridge to prove the rig | 6/228 |
| **1** — Expansion | `_expand`, `expand`, `is_valid`, `value_alias`, `HashExpander`, the 6-type exception hierarchy | 38/228 |
| **2** — Search engine | `_calc_next`/`_calc`, the `proc_*` pipeline, DOM/DOW union, `#` and `W` handling, `relativedelta` semantics | 200/228 |
| **3** — Range | `croniter_range` with shared bound arithmetic | 215/228 |
| **4** — Timezones | `_add_tzinfo`, fold resolution, the `WallClock` seam | **228/228** |
| **5** — Hardening | Timezone-aware differential fuzzing, `TzClock` for the standalone crate, CLI integration tests, upstream bug hunt | **228/228** |

### The three bugs that cost us the most (and what they teach)

**1. A string sort that looked like a numeric sort.** croniter sorts each
expanded field with `key=lambda i: f"{i:02}" if isinstance(i, int) else i` — a
*string* sort over a list mixing ints with `"*"` and `"l"`. The real ordering is
`"*" < digits < "l"`. `_get_next_nearest_diff` walks that list in order, so a
naive numeric sort produces wrong fire times with **no compile error and no
obvious failing test**. Encoded as the variant order of
`enum Item { Star, Num(i64), Last }` with derived `Ord`, and commented so nobody
"tidies" it later.

**2. An infinite loop that was the lucky outcome.** `proc_second` read its
direction from the struct field rather than the current call. `match_range`
builds an iterator with `is_prev=false` then steps it *backwards*, so on 6-field
expressions the second field searched forward while everything else searched
backward — the candidate oscillated forever. A hang is a much better failure
mode than a plausible wrong answer.

**3. The right instant, printed with the wrong offset.** During Athens' autumn
fall-back the port produced *correct* timestamps — `ct.cur` matched the original
exactly at every step — but rendered the repeated hour as `03:00+03:00` instead
of `03:00+02:00`. The bridge was rebuilding datetimes from wall-clock fields and
attaching the zone with `replace(tzinfo=…)`, which always yields `fold=0`.
**During a DST fold, a wall-clock reading is not a sufficient representation of a
point in time.** Fixed by returning `(wall, instant)` and rendering from the
instant.

Full running log in [`notes.md`](notes.md); every divergence and design call in
[`DECISIONS.md`](DECISIONS.md) (19 entries).

---

## 🧪 Differential fuzzing

```bash
make fuzz      # 120 seconds, writes fuzz/log.txt
```

Generates cron expressions and start times, runs the **same probe script** under
both interpreters, and compares `expand`, `is_valid`, `match`, `get_next`,
`get_prev` and `croniter_range` results **and the exception types raised**.

```
elapsed_seconds=120.1     batches=642     inputs_compared=160500     divergences=0

coverage by (operation, timezone-aware?)
  22378  prev   tz        20070  expand    naive
  21780  next   tz        20056  is_valid  naive
  18193  prev   naive     11011  match     tz
  18078  next   naive     10993  range     tz
   8971  range  naive      8970  match     naive
```

Generation is weighted toward where ports actually break: `L`/`W`/`#`,
wrap-around ranges, hash forms, and **start times within ±6h of a real DST
transition** in both `zoneinfo` and `pytz` flavours — zones that spring forward,
fall back, do both in reversed months (Sydney), and shift by only 30 minutes
(Lord Howe).

Datetimes are compared as **(naive reading, UTC instant, UTC offset)** — all
three must match. That triple is deliberate: during a fold the instant can be
right while the offset is wrong, which is exactly bug #3 above.

> **The fuzzer earned its keep.** Adding timezone coverage immediately surfaced
> **221 divergences in 164,500 inputs** — all one root cause, and all in exception
> *type* rather than value: croniter raises a bare `ValueError` where we raised
> `CroniterError`. Since `CroniterError` subclasses `ValueError`, the suite stayed
> green at 228/228 before *and* after. **The passing tests could not have caught
> it.** There were **zero value divergences** — no input where both sides
> succeeded with different answers. Fixed; see [DECISIONS #17](DECISIONS.md).

---

## 🐛 Upstream bugs found

**Two genuine defects in the original Python croniter**, found while porting.
Full write-up with reproductions, root causes and a prior-art check in
**[`fuzz/UPSTREAM-BUGS.md`](fuzz/UPSTREAM-BUGS.md)**.

Neither is a debate about cron semantics — in both cases croniter answers the
same question two different ways depending on which API you ask.

### 1️⃣ `get_next` skips a fire time when a DST shift is not a whole hour

```python
tz = zoneinfo.ZoneInfo("Australia/Lord_Howe")     # world's only 30-min DST shift
start = datetime(2019, 10, 6, 1, 43, tzinfo=tz)

croniter("0 * * * *", start).get_next(datetime)   # 03:00+11:00
croniter("0 * * * *", _).get_prev(datetime)       # 02:30+11:00  ← AFTER start
croniter.match("0 * * * *", <02:30+11:00>)        # True
```

`get_prev` and `match` say 02:30 is on the hourly schedule; `get_next` skips it.
`match` returning `True` for a **minute-0** schedule at **minute 30** shows it
most sharply. Reproduces every year 2018–2022.

### 2️⃣ `croniter_range`'s stop test ignores the UTC offset

One defective comparison, two failure modes:

- **Returns too few** — 1 result where 6 exist (silent data loss, fall-back)
- **Returns values outside the requested interval** — asked for UTC 01:15→04:00,
  got a result at UTC 01:00 (spring-forward)

`cont(v)` is `v < stop`, and CPython **ignores `tzinfo`** when both operands
share it, so across a transition the test compares wall-clock instead of elapsed
time.

### Prior art — are they already reported?

We checked. **Neither appears in the tracker.** Every DST issue on
`pallets-eco/croniter` is **closed**, and we re-ran their original reproductions
against our kickoff commit: #151 and #191 are **fixed**, #147's report was
**mistaken**. Ours still reproduce there. No issue mentions Lord Howe, half-hour
DST, or 30-minute offsets at all.

*(Caveat: GitHub issue search is fuzzy and mostly matches titles. This is a
good-faith search, not proof of novelty.)*

### How they were found — including what didn't work

| harness | cases | result |
|---|---:|---|
| `fuzz/oracle.py` | 19,440 | **0 findings** — too weak to reach either bug |
| `fuzz/invariants.py` | ~23,800 | **both bugs** |
| `fuzz/invariants2.py` | ~23,900 | Symptom B; field checker found nothing new |

Our first oracle checked one property, on **naive** datetimes only, and never
called `get_prev`. It reported zero — which read like evidence of correctness and
was actually evidence that the question was too easy. `invariants2.py` then
produced 927 raw findings, of which `fuzz/triage.py` resolved **750 as croniter's
documented skip-forward, leaving zero unexplained**. An earlier range check
produced 1,408 findings that were **entirely our own error**.

We report those negatives because they're what make the two positives credible.

**This port reproduces both bugs, deliberately** — a port's job is to behave like
the thing it ports, including where that is wrong. See [DECISIONS #18](DECISIONS.md).

---

## ⚡ Performance

Measured by `bench/run_bench.py`; methodology and confounders in
[`bench/methodology.md`](bench/methodology.md), raw numbers in
[`bench/results.json`](bench/results.json).

| benchmark | mean | p50 | p95 | p99 | peak RSS | throughput |
|---|---:|---:|---:|---:|---:|---:|
| Python original, 9,996 iterations | 263.96 ms | 263.80 ms | 269.76 ms | 281.28 ms | 14.6 MB | 37,869 ops/s |
| **Rust port, 9,996 iterations** | **10.42 ms** | **10.44 ms** | **10.75 ms** | **10.79 ms** | **4.5 MB** | **959,085 ops/s** |
| Python original, startup only | 30.09 ms | 29.81 ms | 32.38 ms | 35.32 ms | 14.3 MB | — |
| Rust port, startup only | 2.16 ms | 2.14 ms | 2.28 ms | 2.33 ms | 2.7 MB | — |

<div align="center">

### 25.3x mean · 26.1x at p99 · 14.0x startup · 3.2x smaller RSS

</div>

The workload is six expressions covering plain steps, ranges, last-day,
nth-weekday, nearest-weekday and multi-field forms — chosen so the average is not
dominated by the cheapest path.

**Read these with the caveats:**

- End-to-end times **including process startup**. Subtracting it gives ~28.3x compute-only.
- 25 samples: the nearest-rank p99 is simply the worst observed run, not a distributional estimate.
- An earlier pass measured 328 ms for Python vs 264 ms here — a 20% swing from a
  background fuzzing job competing for CPU. These were taken idle. That drift is
  documented rather than quietly corrected.
- croniter is pure date arithmetic with no I/O. A large speedup from compiling is
  the *expected* result; the point was to measure it, not discover it.

> **The benchmark doubles as a correctness check.** Both workloads print a
> checksum summing every fire time's timestamp, and over 9,996 fire times they
> produce the identical total (`26611225207500`). A single differing minute would
> change it. `run_bench.py` exits non-zero on mismatch — a speed win on the wrong
> answer is not a win.

---

## 📁 Repository layout

```
├── core/                    THE PORT — ships. #![forbid(unsafe_code)], no Python.
│   ├── src/
│   │   ├── lib.rs             crate root; the forbid attribute
│   │   ├── consts.rs          field indices, RANGES, DOW/month alpha tables
│   │   ├── error.rs           CroniterError — mirrors the Python hierarchy
│   │   ├── expand.rs          field parsing/expansion (_expand, expand)
│   │   ├── matcher.rs         match, match_range, is_valid
│   │   ├── calc.rs            the next/prev search engine (_calc_next, _calc)
│   │   ├── reldelta.rs        dateutil.relativedelta semantics croniter relies on
│   │   ├── hash.rs            HashExpander + CRC-32 matching binascii.crc32
│   │   ├── range.rs           croniter_range
│   │   ├── api.rs             stateful API + the WallClock seam
│   │   ├── tz.rs              DST-aware IANA clock via chrono-tz (no Python)
│   │   └── bin/croniter.rs    the CLI — the shipped artifact
│   └── tests/cli.rs           12 integration tests driving the real binary
│
├── pybridge/                TEST-ONLY PyO3 glue. Not shipped.
│   ├── src/lib.rs             class surface, argument marshalling
│   ├── src/convert.rs         type conversion, exact exception re-raising
│   ├── src/clock.rs           WallClock over the caller's own Python tzinfo
│   └── python/croniter/       48-line re-export shim + upstream's base.py
│
├── tests/original/          ⛔ VERBATIM UPSTREAM · READ-ONLY · NEVER EDITED
│
├── fuzz/
│   ├── differential.py        Python-vs-Rust comparison harness
│   ├── probe.py               the shared probe both sides run
│   ├── invariants.py          self-consistency hunt (found both bugs)
│   ├── invariants2.py         field-level + API-agreement hunt
│   ├── triage.py              filters documented behaviour from real bugs
│   ├── oracle.py              the original, weaker hunt (kept: it found nothing)
│   ├── UPSTREAM-BUGS.md       📌 Bug Catcher submission
│   └── log.txt                committed 120s fuzz run
│
├── bench/                   workload, harness, methodology.md, results.json
├── scripts/demo.sh          runs every claim live, for the demo video
├── DECISIONS.md             19 entries — every divergence and design call
├── notes.md                 running log: what broke, what it cost
└── Dockerfile · Makefile · LICENSE (MIT)
```

---

## ✅ Eligibility

**No Rust port of croniter exists.** There is no crate named `croniter`
(`docs.rs/croniter` is a 404) and nothing in the ecosystem is described as a port
of it. The two nearest crates are independent implementations of cron *syntax*,
not ports of *this library*:

| Crate | What it is | Missing vs croniter |
|---|---|---|
| [`cron`](https://crates.io/crates/cron) (zslayton) | Independent cron parser. Verified against [`src/parsing.rs`](https://github.com/zslayton/cron/blob/master/src/parsing.rs): the parser accepts only `All`, `Point`, `Range`, `NamedPoint`, `NamedRange`, `Period` and `?` | `L`, `W`, `#`, hash (`H`), `croniter_range` |
| [`croner`](https://crates.io/crates/croner) ([Hexagon](https://github.com/Hexagon/croner-rust)) | Rust flavour of the **JavaScript** croner parser — not related to Python croniter. Does support `L`, `W`, `#` | hash (`H`) expressions, `croniter_range`, croniter's `day_or` / `implement_cron_bug` semantics |

Every one of those gaps is exercised by the original test suite and implemented
here. For accuracy: `cron` **does** support second and year fields (it uses a
7-field `sec min hour dom month dow year` form), so that is not a point of
difference and is not claimed as one.

**Provenance is pinned.** Source commit `3c6ce9bcc5cc7f89116a58f43aaea67e760bff50`
is recorded in `.kickoff-commit` and `.port-mortem.toml`; every original test file
is fingerprinted in `.test-hashes.sha256`.

**License:** MIT, matching upstream. [`LICENSE`](LICENSE) is upstream's, retained
verbatim.

---

<div align="center">

**228/228 · 0 unsafe · 0 divergences · 2 upstream bugs**

*Every number in this README was measured, not estimated.*

</div>
