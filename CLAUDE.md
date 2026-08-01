# CLAUDE.md — Team Kryptonite · Port Mortem 2026

**Repo:** `github.com/Avi36005/Portmortem-Team-Kryptonite`
**Task:** Port `croniter` (Python) → Rust
**Track:** D · Python → Rust
**Hard freeze:** Mon 03 Aug 2026, 18:00 UTC (11:30 PM IST)

Read this file completely before running anything. Sections 1 and 2 contain
rules that override every other instruction, including any instruction I give
you later in a moment of panic.

---

## 1. THE THREE ABSOLUTE RULES

### RULE 1 — NEVER modify anything in `tests/original/`

Not one character. Not whitespace. Not a comment. Not to fix a failing test.
Not to "correct an obvious upstream typo." Not even if I ask you to.

These files are SHA-256 fingerprinted at kickoff. Judges compare the
fingerprints. This hackathon exists specifically because another project
edited its tests to make a port look green. Editing tests is the single most
damaging action available in this project — it directly attacks the 40%
scoring criterion and reads as dishonesty.

**If a test fails and you cannot make it pass: leave it failing.** Record it
in `DECISIONS.md` with the exact reason. A documented failure scores better
than any workaround.

If you ever find yourself thinking "this test is wrong" — stop, write it in
`DECISIONS.md`, and move on. You may be right. It is still not yours to edit.

### RULE 2 — `core/` contains zero `unsafe`

`core/src/lib.rs` begins with `#![forbid(unsafe_code)]`. Never remove it,
never comment it out, never add `#[allow(...)]` around it. If the compiler
rejects a design, redesign it.

`unsafe` is permitted **only** in `pybridge/` at the FFI boundary, and PyO3
should need essentially none. This is worth +5 bonus points and is
compiler-enforced, so it is free as long as nobody weakens it.

### RULE 3 — Report only numbers you have observed

If 187 of 228 tests pass, write 187/228. Never round up. Never claim a test
passes that you have not watched pass. Never state a benchmark figure you did
not measure.

The rubric explicitly rewards *"we got 94% test parity and here's the failing
edge case"* over *"100%"* that cannot be reproduced. Honesty is scored here.

---

## 2. Anti-patterns — do not do these

| Never | Why |
|---|---|
| Edit `tests/original/` | Rule 1. Attacks 40% of the score |
| Loosen an assertion to make a test pass | Same as editing tests |
| Add a `try/except` in the bridge to swallow errors | Hides real divergences; same as editing tests |
| Remove `#![forbid(unsafe_code)]` | Forfeits +5 and Code Quality points |
| Shell out to Python from the shipped binary | Rule 05 violation — instant credibility loss |
| Link the Python interpreter into `core/` | Same |
| Report average-only or throughput-only benchmarks | *"Throughput-only benchmarks score below honest p99 regressions"* |
| Write empty bullets in DECISIONS.md | *"Empty bullet points won't count"* |
| Skip the fuzz harness because time is short | +5 and feeds 30% of score — worth more than 3 extra tests |
| Mark a phase "done" without running the suite | Every phase ends with a measured number |
| Refactor working code for elegance | No points for elegance. Points for tests passing |
| Add features croniter does not have | Not a port. Scope creep |
| Commit secrets, `.env`, or large binaries | Repo is public |

---

## 3. What we are building and why

croniter parses cron expressions (`0 9 * * 1-5` = "9am weekdays") and computes
next/previous fire times. Pure logic: text and dates in, dates out. No
network, no filesystem, no threads.

**Verified baseline (measured, not estimated):**

| Fact | Value |
|---|---|
| Source to port | `src/croniter/croniter.py` — 1,531 lines (+ `__init__.py`, 41) |
| Original test suite | 3,708 SLOC, 8 files, **228 tests** |
| **Baseline: 228/228 pass in 4.33s on clean clone** | verified |
| License | MIT |
| Python deps | `python-dateutil` → `chrono` + `chrono-tz`; `pytz` (tests only) |

**Scoring:** Functionality & Reliability 40% · Behavioral Equivalence 30% ·
Code Quality 20% · Innovation 10%.
**Bonuses:** Differential Fuzz +5 · Zero Unsafe +5 · Bug Catcher +3 (+$100) ·
Decision Log +3.

Organizer statement: *"The main thing is to make the original tests pass.
Most likely, the winner will be among the people who do that."*

---

## 4. Target repo structure

```
Portmortem-Team-Kryptonite/
├── CLAUDE.md                  this file
├── README.md                  rationale, build, baseline vs port numbers
├── DECISIONS.md               10+ divergences with rationale
├── LICENSE                    MIT (croniter is MIT — stay compatible)
├── Dockerfile                 one command → runnable artifact
├── Makefile                   make build / make test / make fuzz / make bench
├── Cargo.toml                 workspace: members = ["core", "pybridge"]
├── .port-mortem.toml          track letter, source URL, kickoff hash
├── .kickoff-commit            original repo git SHA
├── .test-hashes.sha256        sha256 of every original test file
├── core/
│   ├── Cargo.toml
│   └── src/
│       ├── lib.rs             #![forbid(unsafe_code)]  ← NEVER REMOVE
│       ├── error.rs           CroniterError enum (thiserror)
│       ├── consts.rs          field indices, RANGES, DOW/M alphas
│       ├── expand.rs          Phase 1 — field parsing/expansion
│       ├── matcher.rs         Phase 1 — match(), is_valid()
│       ├── calc.rs            Phase 2 — next/prev search engine
│       ├── hash.rs            Phase 3 — HashExpander
│       ├── range.rs           Phase 3 — croniter_range
│       └── bin/croniter.rs    CLI — the shipped artifact
├── pybridge/
│   ├── Cargo.toml             crate-type = ["cdylib"]
│   └── src/lib.rs             #[pymodule] named `croniter`
├── tests/
│   └── original/              ← COPIED VERBATIM · READ-ONLY · NEVER EDIT
├── fuzz/
│   ├── oracle.py              match-vs-get_next bug hunter (Python only)
│   ├── differential.py        Python vs Rust comparison harness
│   └── log.txt                60s+ continuous run output
├── bench/
│   ├── methodology.md         how measurements were taken
│   ├── next10k.py             Python workload
│   └── results.json           p99, RSS, startup, throughput
└── notes.md                   running log of what broke (→ write-up prize)
```

**Why two crates — this is DECISIONS.md entry #1.** `core/` is the real port
and ships as a standalone binary with zero Python dependency. `pybridge/`
exists only to run the original pytest suite and is not part of the shipped
artifact. This satisfies Rule 05 ("no source-language runtime") — state it
explicitly so no judge concludes we linked against Python.

---

## 5. PHASE 0 — Bootstrap (do this first, nothing else until it's green)

### 0.1 — Vendor the original and lock the evidence

```bash
cd Portmortem-Team-Kryptonite

# fetch the original into a scratch dir (NOT committed as a submodule)
git clone https://github.com/pallets-eco/croniter /tmp/croniter-src
cd /tmp/croniter-src && git rev-parse HEAD && cd -

# record provenance
(cd /tmp/croniter-src && git rev-parse HEAD) > .kickoff-commit

# copy the test suite verbatim
mkdir -p tests/original
cp /tmp/croniter-src/src/croniter/tests/*.py tests/original/

# fingerprint every test file — this is our honesty receipt
sha256sum tests/original/*.py > .test-hashes.sha256

# confirm the baseline on the untouched original
cd /tmp/croniter-src
pip install -e . python-dateutil pytz pytest
python3 -m pytest src/croniter/tests/ -q     # MUST show: 228 passed
cd -
```

**Do not proceed until you have seen `228 passed`.** Record the exact commit
SHA in `README.md`.

### 0.2 — Start the bug hunter (runs in background all weekend)

`fuzz/oracle.py` — needs **no Rust**, runs against the Python original:

```python
"""Differential oracle: croniter.match() vs get_next() must agree."""
import itertools, random, traceback
from datetime import datetime, timedelta
from croniter import croniter

def check(expr, start):
    """If get_next returns N, no minute strictly between start and N may
    match, and N itself must match. A violation = croniter contradicts
    itself = a real, filable bug."""
    nxt = croniter(expr, start).get_next(datetime)
    t = start + timedelta(minutes=1)
    while t < nxt:
        if croniter.match(expr, t):
            return ("SKIPPED_MATCH", expr, start, t, nxt)
        t += timedelta(minutes=1)
    if not croniter.match(expr, nxt):
        return ("BAD_NEXT", expr, start, nxt)
    return None

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
    starts = [datetime(y, m, d, h, mi)
              for y in (2024, 2025, 2026, 2027, 2028)   # incl. leap years
              for m in (1, 2, 3, 6, 11, 12)
              for d in (1, 14, 27, 28)
              for h in (0, 12, 23) for mi in (0, 30, 59)]
    found = 0
    for expr in FRAGILE:
        for s in starts:
            try:
                r = check(expr, s)
            except Exception:
                r = ("EXCEPTION", expr, s, traceback.format_exc(limit=3))
            if r:
                found += 1
                print("FINDING:", r, flush=True)
                with open("fuzz/oracle-findings.txt", "a") as f:
                    f.write(repr(r) + "\n")
    print(f"done. findings={found}")

if __name__ == "__main__":
    main()
```

```bash
mkdir -p fuzz
nohup python3 fuzz/oracle.py > fuzz/oracle.log 2>&1 &
```

Any finding → minimize to the smallest failing expression → file at
`github.com/pallets-eco/croniter` **during the event** → +3 points, $100
prize, and a DECISIONS.md entry.

### 0.3 — Scaffold the workspace

```bash
cargo new --lib core
cargo new --lib pybridge
```

Root `Cargo.toml`:
```toml
[workspace]
members = ["core", "pybridge"]
resolver = "2"
```

`core/src/lib.rs` — **first line, non-negotiable**:
```rust
#![forbid(unsafe_code)]
```

`core/Cargo.toml` deps: `chrono`, `chrono-tz`, `regex`, `thiserror`.
`pybridge/Cargo.toml`: `pyo3 = { version = "0.22", features = ["extension-module"] }`,
`crate-type = ["cdylib"]`, `[package] name = "croniter"` so the built module
is importable as `croniter`.

### 0.4 — Prove the harness before the port exists

Make `pybridge` export a **stub** `croniter` class with the right method
names that returns wrong answers. Then:

```bash
maturin develop -m pybridge/Cargo.toml
python3 -m pytest tests/original/ -q
```

**Expected: nearly all 228 tests FAIL.** That is success — it proves the
bridge wires the original tests to Rust. Build the proof rig before the port.

Commit here. Phase 0 done.

---

## 6. PHASES 1–4 — Implementation order

Sequenced by **tests unlocked per line written**. Do not reorder without a
reason written into `notes.md`.

### Phase 1 — Field expansion + `match` (~600 lines)

Port: constants (field indices, `RANGES`, `DOW_ALPHAS`, `M_ALPHAS`,
`CRON_FIELDS`, `VALID_LEN_EXPRESSION`), `value_alias`, `_expand` (:944),
`expand` (:1240), `is_valid` (:1320), `match` (:1333), and the exception
hierarchy.

Exceptions must map to matching Python types through PyO3:
```
CroniterError(ValueError)
CroniterBadTypeRangeError(TypeError)
CroniterBadCronError(CroniterError)
CroniterUnsupportedSyntaxError(CroniterBadCronError)
CroniterBadDateError(CroniterError)
CroniterNotAlphaError(CroniterBadCronError)
```
Tests check exception *types*, so getting the hierarchy wrong fails tests that
have nothing to do with parsing.

Unlocks: all syntax-validation and error-type tests.

### Phase 2 — Search engine (~470 lines)

Port `_calc_next` / `_calc` (:475–824), `_get_next_nearest_diff` (:825),
`_get_prev_nearest_diff` (:849), `_get_nth_weekday_of_month` (:885, the `#`
syntax), `_get_nearest_weekday` (:896, the `W` syntax) → then the public
`get_next`, `get_prev`, `get_current`, `set_current`, `all_next`, `all_prev`,
`iter`, `__iter__`.

Correctness first. Optimize only if `test_croniter_speed.py` fails.

Hardest phase. If it overruns past Sunday midday, **jump to Phase 3** and
come back.

### Phase 3 — HashExpander + croniter_range (**70 tests**)

`HashExpander` (:1455) is self-contained: `binascii.crc32` masked to 32 bits,
modulo arithmetic, a regex. **No date logic.** `test_croniter_hash.py` alone
is **54 tests = 24% of the whole suite** for very little code. Cheapest large
score gain available — go here early if Phase 2 drags.

Then `croniter_range` (16 tests).

Match `binascii.crc32` exactly: `crc32(data) & 0xFFFFFFFF`, then
`((crc >> idx) % (range_end - range_begin + 1)) + range_begin`.

### Phase 4 — Timezones and DST (~27 tests)

~23 of the 148 tests in `test_croniter.py` touch `zoneinfo` / `dateutil.tz` /
`pytz`, plus `test_croniter_dst_repetition.py` (4 tests).

`chrono-tz` and `dateutil` may legitimately disagree on ambiguous local times
during DST transitions. **Do not hack around a disagreement.** Document it
precisely in DECISIONS.md and leave the test failing. This is the expected
place to lose points, and losing them honestly is fine.

---

## 7. Test suite map — know what each file is worth

| File | Tests | Phase |
|---|---|---|
| `test_croniter.py` | **148** | 1–2 |
| `test_croniter_hash.py` | **54** | 3 — cheapest big win |
| `test_croniter_range.py` | 16 | 3 |
| `test_croniter_dst_repetition.py` | 4 | 4 |
| `test_croniter_random.py` | 4 | 4 |
| `test_croniter_speed.py` | 2 | perf |
| **Total** | **228** | baseline 228/228 |

**End every phase by running the suite and recording the number in
`notes.md`.** Format: `Phase 2 complete — 171/228 (was 96/228)`.

---

## 8. FINAL STRETCH — freeze features Monday morning

These are worth more than three extra passing tests. Do not skip them.

### 8.1 Differential fuzz harness (+5)

No template is provided by the organizers — write your own and document it.
`fuzz/differential.py`: generate expression + start-time pairs, run both the
Python original and the Rust binary, compare outputs, log every input tried.
Run **60+ continuous seconds**, commit `fuzz/log.txt` including the input
count and any divergences found. If divergences exist, report them honestly —
a documented divergence beats a hidden one.

### 8.2 Benchmarks

```bash
hyperfine --warmup 3 'python3 bench/next10k.py' './target/release/croniter bench'
/usr/bin/time -v ./target/release/croniter bench   # RSS
```

Report **p99, RSS, startup, throughput** — not just averages. Write
`bench/methodology.md`: hardware, iteration count, warmup, what the workload
is, confounders. croniter is pure-Python date math, so a large speedup is
expected and honest — but measure it, don't assume it.

### 8.3 DECISIONS.md — 10+ real entries

Write these as they happen, not reconstructed at the end. Seeds:

1. **Two-crate split** — `core` (`forbid(unsafe_code)`, ships) vs `pybridge`
   (PyO3, test-only). Rule 05 compliance argument.
2. **`chrono`/`chrono-tz` replacing `python-dateutil`** — mapping, DST differences.
3. **Error model** — Python exception hierarchy → Rust enum + PyO3 re-raise.
4. **Float epoch timestamps** vs typed `DateTime`; precision boundaries.
5. **`ret_type` polymorphism** — Python returns `float` *or* `datetime` by
   argument; how Rust models that.
6. **Unbounded Python ints** → chosen integer widths, `max_years_between_matches`.
7. **Regex engine** — Python `re` vs Rust `regex`; anything hand-rewritten.
8. **`day_or` / `implement_cron_bug`** — the OR/AND day-field ambiguity.
9. **`binascii.crc32` semantics** — masking and signedness in HashExpander.
10. **Any upstream bug found** — croniter's behavior, correct behavior, which
    we implemented, which test fails as a result.
11. **Non-core files unported** (CI, build scripts) — per organizer ruling that
    only the core language needs porting.
12. **Every failing test** — one entry per unresolved failure with the reason.

### 8.4 README.md must state

```
Original baseline: 228/228 passing at commit <sha> (verified, 4.33s)
Port:              N/228 passing
Unsafe blocks in core/: 0 (compiler-enforced via #![forbid(unsafe_code)])
Eligibility: no direct Rust port of croniter exists. The `cron` crate
(zslayton) is an independent implementation lacking L, W, #, hash
expressions, croniter_range, and second/year fields.
```

Plus: build instructions (one command), migration rationale, how to verify the
test hashes, and a per-file pass-rate table.

### 8.5 Demo video (5 min)

Show the original test suite running live against the Rust port. Show
`sha256sum -c .test-hashes.sha256` passing. Show the unsafe count. State the
honest number out loud.

---

## 9. Command reference

```bash
# Build the shipped artifact
cargo build --release -p core

# Build + install the test bridge
maturin develop -m pybridge/Cargo.toml

# Run the ORIGINAL suite against the Rust port
python3 -m pytest tests/original/ -q

# Per-file breakdown
for f in tests/original/test_*.py; do echo -n "$f: "; python3 -m pytest $f -q 2>&1 | tail -1; done

# VERIFY WE NEVER TOUCHED THE TESTS  (run before every commit)
sha256sum -c .test-hashes.sha256

# Confirm zero unsafe in core
grep -rn "unsafe" core/src/ | grep -v "forbid(unsafe_code)" | wc -l    # must be 0

# Bug hunter (background, no Rust needed)
nohup python3 fuzz/oracle.py > fuzz/oracle.log 2>&1 &

# Differential fuzz (final stretch)
python3 fuzz/differential.py --seconds 90 | tee fuzz/log.txt

# Benchmarks
hyperfine --warmup 3 'python3 bench/next10k.py' './target/release/croniter bench'
```

### Makefile targets to provide

```make
build:  cargo build --release -p core
test:   maturin develop -m pybridge/Cargo.toml && python3 -m pytest tests/original/ -q
verify: sha256sum -c .test-hashes.sha256
fuzz:   python3 fuzz/differential.py --seconds 90 | tee fuzz/log.txt
bench:  hyperfine --warmup 3 'python3 bench/next10k.py' './target/release/croniter bench'
```

**Rule 03 requires one-command build.** `docker compose up` or
`cargo build --release` must produce a working artifact with no extra steps.
If a judge has to read CI to figure out how to build, that rule is failed.

---

## 10. Working agreement with the agent

- **Port module by module, test-first.** Never attempt a whole-file
  translation in one shot. That produces exactly the unreviewable output this
  hackathon was built to catch.
- **After every module: run the suite, report the real number.** Never say
  "should pass now" — run it.
- **Append to `DECISIONS.md` at the moment of each divergence.** Not later.
- **Append to `notes.md` whatever broke and how long it took.** That file
  becomes the $300 write-up submission (closes Aug 10, judged on insight not
  audience) for about an hour of work.
- **Before every commit run `sha256sum -c .test-hashes.sha256`.** If it fails,
  something touched the tests — revert immediately, do not commit.
- **If blocked >30 min on one test, move on.** Note it, come back later.
  Never solve a blocker by weakening a test.
- **Ask before adding a dependency.** Small, well-known crates only.
- **Do not push croniter's name to public channels.** No requirement to
  announce the repo choice.

---

## 11. Deliverables checklist

- [ ] Public repo, MIT license (croniter is MIT — stay compatible)
- [ ] Builds with ONE command
- [ ] `tests/original/` present, unmodified, `sha256sum -c` passes
- [ ] Original suite runs against the port via PyO3 bridge
- [ ] `fuzz/log.txt` — 60s+ continuous differential run
- [ ] `DECISIONS.md` — 10+ substantive entries
- [ ] `bench/methodology.md` + `results.json` with p99 and RSS
- [ ] `README.md` with baseline vs port numbers and eligibility argument
- [ ] `.port-mortem.toml`, `.kickoff-commit`, `.test-hashes.sha256`
- [ ] 5-minute demo video
- [ ] Upstream issue filed if the oracle found a bug
- [ ] `notes.md` → write-up published by Aug 10 18:00 UTC ($300 side quest)

---

## 12. If time runs out

Priority order when the clock is short:

1. **Repo builds with one command.** A non-building submission scores near zero.
2. **Whatever tests pass, pass honestly**, with the real number in README.
3. **DECISIONS.md** (+3, and feeds 20% Code Quality) — cheap, high value.
4. **Fuzz log** (+5, feeds 30% Behavioral Equivalence) — even 60 seconds counts.
5. **Benchmarks** with p99 — even a single honest workload.
6. **Demo video.**

A 70% port with all five proof artifacts beats a 95% port with none of them.
Do not trade proof artifacts for extra passing tests in the last six hours.
