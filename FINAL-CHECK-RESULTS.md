# FINAL CHECK — results

Run: 2026-08-02 · macOS 15.7.7 (Darwin 24.6.0), Apple M4 · CPython 3.13.12 ·
Rust 1.97 · repo at `93b5d57` plus the two documentation fixes described in
PART 6 and PART 8 below.

Every command below was executed. No output is predicted.

## Summary

| Part | Check | Result |
|---|---|---|
| 1 | Repo structure | **PASS** (3 items to decide, listed) |
| 2.1 | Module pytest imports | **PASS** |
| 2.2 | Suite capable of failing | **PASS** |
| 2.3 | Benchmark checksums capable of differing | **PASS** |
| 3 | Original test suite integrity | **PASS** |
| 4 | Clean-clone build | **PASS** |
| 5 | Code quality claims | **PASS** |
| 6 | README completeness | **PASS** (DST decision surfaced; 2 deliberate deviations) |
| 7 | Evidence files | **PASS** |
| 8 | **Bug Catcher — issues filed** | **FAIL — not filed** |
| 9 | Commit and push | **PENDING — awaiting approval** |
| 10–12 | Video, submission, write-up | Owner actions |

**One blocker: PART 8.** Everything technical passes.

---

## PART 1 — Repo structure

54 files tracked. `Cargo.lock` is tracked. No `.DS_Store`, `__pycache__`,
`*.pyc`, `.venv` or `target/` in the index:

```
$ git ls-files | grep -iE "\.DS_Store|__pycache__|\.pyc|\.venv|target/"
(no output)

$ git ls-files | grep Cargo.lock
Cargo.lock
```

Deviations from the target tree, each deliberate:

| Item | Status |
|---|---|
| `tests/port/` for the 65 Rust tests | Not used. The checklist says "if separate" — they are not. 53 unit tests live in `#[cfg(test)]` modules beside the code they test (idiomatic Rust), 12 CLI integration tests in `core/tests/cli.rs`. Total 65, verified in PART 5. |
| `docs/notes.md` | `notes.md` is top-level. The checklist marks `docs/` optional. |
| `CLAUDE.md` | **Removed from the repo** at the owner's instruction (commit `32172d8`), kept on disk and gitignored. The checklist says it is "fine to keep"; keeping it was optional, not required. |
| `fuzz/probe.py`, `invariants.py`, `invariants2.py`, `triage.py`, `oracle.log` | Not in the target tree but justified: `make hunt` and `fuzz/UPSTREAM-BUGS.md` both reference them, so a judge can reproduce the bug hunt. |
| `bench/run_bench.py`, `scripts/demo.sh` | The benchmark harness and the demo script. Both referenced by the Makefile. |
| `audit-results.md` | **Undecided.** A prior audit write-up, not part of the target tree. See "Open decisions" at the end. |

---

## PART 2 — Falsification checks

### 2.1 — Which module does pytest import?

Command:
```bash
python -c "import croniter; print(croniter.__file__)"
python -c "import importlib; print(importlib.import_module('croniter.croniter').__file__)"
```
Output:
```
croniter.__file__          : pybridge/python/croniter/__init__.py
implementation module file : pybridge/python/croniter/croniter.cpython-313-darwin.so
is compiled extension      : True
class croniter             : <class 'builtins.croniter'>
inspect.getfile            : TypeError -> <class 'builtins.croniter'> is a built-in class
```
Result: **PASS**

Note: the top-level name resolves to a `.py`, which reads as FAIL against the
literal criterion. It is a pure re-export shim containing no logic (read in full;
it is a transcription of upstream's own `__init__.py`). The *implementation*
module `croniter.croniter` is the maturin-built `.so`, and the `croniter` class
is a builtin extension type with no Python source. No pure-Python croniter is
installed in `.venv`. **State this in the demo video before someone misreads the
first line.**

### 2.2 — Prove the suite is capable of failing

Change made: `core/src/expand.rs:396`, wrap-around range length
`hi_r - lo_r + 1` → `hi_r - lo_r + 2`.

Output:
```
baseline:   228 passed in 2.28s
sabotaged:  2 failed, 226 passed in 2.01s
    FAILED test_croniter.py::CroniterTest::test_mth_ranges_from
    FAILED test_croniter.py::CroniterTest::test_sunday_ranges_from
reverted:   228 passed in 1.81s
```
Result: **PASS**

Notes: `expand.rs` restored byte-identical (`diff -q` against a pre-change copy),
`git status --porcelain core/` empty afterwards. `git checkout core/` was
deliberately **not** used — the file was restored from an explicit backup and
verified, which is safer when the tree has uncommitted work.

A second, independent falsification was run earlier on the same tree
(`core/src/consts.rs` `RANGES` hour `23`→`22`) and produced **32 failed, 196
passed**. Two different single-token changes in two different files both move the
suite off 228. The bridge executes Rust.

### 2.3 — Benchmark checksums capable of differing

Change made: `core/src/bin/croniter.rs`, `sink += ts` → `sink += ts * 1.000001`.

Output:
```
baseline    python: checksum=26611225207500   rust: checksum=26611225207500   MATCH
sabotaged   python: checksum=26611225207500   rust: checksum=26611251818725   DIFFER
reverted    python: checksum=26611225207500   rust: checksum=26611225207500   MATCH
```
Result: **PASS**

Notes: `bench/results.json` confirmed byte-identical afterwards; the committed
artifact was not rewritten. `git status --porcelain core/ bench/` empty.

---

## PART 3 — Test suite integrity

```
$ sha256sum -c .test-hashes.sha256
tests/original/__init__.py: OK
tests/original/base.py: OK
tests/original/test_croniter_dst_repetition.py: OK
tests/original/test_croniter_hash.py: OK
tests/original/test_croniter_random.py: OK
tests/original/test_croniter_range.py: OK
tests/original/test_croniter_speed.py: OK
tests/original/test_croniter.py: OK

$ git status --porcelain tests/original/     (empty)
$ git diff --stat -- tests/original/         (empty)

$ git log --all --oneline -- tests/original/
ab08e81 test(bootstrap): vendor original croniter test suite and compute sha256 fingerprints
c2d5092 Phase 0: bootstrap — vendored tests, fingerprints, workspace, stub bridge
```
Result: **PASS**

`--all` reports two commits. Resolved:

- `git log main -- tests/original/` returns **exactly 1**.
- `c2d5092` is reachable only from the local branch `backup-5-commits`, not from
  `main` and not from `origin/main`.
- Both commits produce the **identical tree**
  `8048ceb2a372e1c0af1f92b85af463135cfde0c9`; `git diff` between them over
  `tests/original/` is empty.

No content ever changed. The published branch has one vendoring commit.

---

## PART 4 — Clean-clone build (judge simulation)

```
$ git clone https://github.com/Avi36005/Portmortem-Team-Kryptonite judge-sim
$ cd judge-sim && make build
93b5d57 docs: tighten README and DECISIONS, unify the baseline figure
   Compiling croniter-core v0.1.0
    Finished `release` profile [optimized] target(s) in 5.95s
built target/release/croniter

$ ./target/release/croniter --help
croniter — cron expression parsing and next/prev fire times
USAGE: croniter next|prev|range|match|validate|bench ...

$ otool -L ./target/release/croniter
  /System/Library/Frameworks/CoreFoundation.framework/.../CoreFoundation
  /usr/lib/libSystem.B.dylib
python refs: 0
```
Result: **PASS** — builds from a fresh clone of the public repo with the single
documented command, no undocumented steps. `ldd` is Linux-only; `otool -L` is the
macOS equivalent. Two system libraries, no interpreter. Rule 05 evidence.

---

## PART 5 — Code quality claims

```
$ head -1 core/src/lib.rs
#![forbid(unsafe_code)]

$ grep -rn "unsafe" core/src/ | grep -v forbid          -> 0 matches
$ grep -rn "pyo3" core/ --include='*.rs' --include='*.toml' -> 0 matches
$ cargo clippy --all-targets -- -D warnings             -> exit 0
$ cargo test --workspace                                -> 53 passed + 12 passed = 65
$ pytest tests/original/ -q                             -> 228 passed
```
Result: **PASS** — all five claims confirmed.

Note: the checklist's `--include=*.rs` needs quoting (`--include='*.rs'`) under
zsh, or it errors with `no matches found` and looks like a failure. Quote it in
the demo.

---

## PART 6 — README

All 14 required items present. Position measured as a percentage through the
document:

| Item | Line | Position |
|---|---:|---:|
| Source repo URL | 3 | 0% |
| Track D · Python → Rust | 13 | 2% |
| Rule 05 / no Python dependency | 41 | 7% |
| **DST decision** | **56** | **9%** |
| Original baseline 228/228 + commit | 51 | 9% |
| Port 228/228 | 52 | 9% |
| Unsafe count, compiler-enforced | 56 | 10% |
| In-scope / out-of-scope | 98 | 18% |
| One-command build | 116 | 21% |
| Hash verification for a judge | 201 | 37% |
| Per-phase table | 291 | 53% |
| Upstream bugs | 369 | 67% |
| Eligibility argument | 521 | 95% |
| Full kickoff SHA | 538 | 99% |

Result: **PASS**

**Fix applied.** The checklist requires the DST decision to be "surfaced high up,
not buried at DECISIONS #19". It was at 51%, inside Architecture. Added a
*"The central design decision"* subsection to the Overview stating that the
bridge deliberately does not route through `chrono-tz`, and why. Now at **9%**.
The short kickoff SHA appears at 9%; only the full 40-character form is at 99%.

**Two deliberate deviations from this checklist, both because the checklist is
wrong on the facts:**

1. The checklist expects `Original baseline: 228/228 (verified, ~4.33s)`. On this
   hardware the same suite at the same commit runs in **1.54s**. Rule 3 requires
   reporting only observed numbers, so the README says 1.54s and DECISIONS #8
   explains the difference. The 4.33s figure is not reproduced and is not claimed.

2. The checklist's eligibility wording says the `cron` crate lacks
   "second/year fields". **That is false** — zslayton/cron uses a 7-field
   `sec min hour dom month dow year` form and has both. Verified against its
   README and `src/parsing.rs`. The claim was removed and the correction stated
   explicitly in the README. The eligibility argument now rests on `L`, `W`, `#`,
   hash (`H`) and `croniter_range`, which the parser genuinely does not support.
   The README also now names [`croner`](https://crates.io/crates/croner), a
   closer competitor the checklist does not mention — it is a port of the
   *JavaScript* croner, not of Python croniter, and lacks `H`, `croniter_range`
   and croniter's `day_or` / `implement_cron_bug` semantics.

---

## PART 7 — Evidence files

```
$ head fuzz/log.txt
differential fuzz: original(python) vs port(rust)
seed=1785574786 budget=120.0s batch=250

$ tail fuzz/log.txt
elapsed_seconds=120.1   batches=642   inputs_compared=160500   divergences=0
$ wc -l fuzz/log.txt -> 664

$ git ls-files fuzz/
UPSTREAM-BUGS.md differential.py invariants.py invariants2.py
log.txt oracle.log oracle.py probe.py triage.py

bench/results.json — every benchmark carries p99, peak RSS and throughput:
  python_original_next10k    p99=0.28128s  rss=15,319,040  thru=37,868.7/s
  rust_port_next10k          p99=0.01079s  rss=4,734,976   thru=959,085.2/s
  python_original_startup    p99=0.03532s  rss=15,024,128
  rust_port_startup          p99=0.00233s  rss=2,834,432
  equivalence: python==rust==26611225207500 over 9,996 iterations, match=True

$ grep -c "^## " DECISIONS.md -> 20   (19 numbered entries + failing-tests section)
$ empty bullets in DECISIONS.md / README.md -> none
```
Result: **PASS** — 120.1s continuous run (>60s required), input count visible,
0 divergences, `differential.py` committed and re-runnable.
`bench/methodology.md` states hardware, 25 runs, 3 warmups, the six-expression
workload and five confounders. DECISIONS entry #1 is the two-crate split with the
Rule 05 argument.

Note: the fuzz harness compares the Python original against the Rust core *via
the bridge*, not against the standalone binary. That is a valid differential of
the ported logic — say so in the video so it is not mistaken for Python vs Python.

---

## PART 8 — Bug Catcher

```
$ grep -rn "pallets-eco/croniter/issues" README.md DECISIONS.md fuzz/UPSTREAM-BUGS.md
(no matches)
```
Result: **FAIL — neither bug has been filed.**

Both findings are real. Both reproductions were independently re-run against the
original Python **and** against the port, and behave exactly as documented:

```
                          original Python      Rust port
BUG1 next                 2019-10-06 03:00+11  2019-10-06 03:00+11
BUG1 prev                 2019-10-06 02:30+11  2019-10-06 02:30+11
BUG1 match(02:30)         True                 True
BUG1 prev overshoots?     True                 True
BUG2A range len           1  (true answer 6)   1
BUG2B last result         UTC 01:00  <- before the requested lower bound UTC 01:15
```

`fuzz/UPSTREAM-BUGS.md` is a complete submission: minimal reproductions, root
causes, scope, suggested fixes, and a good-faith prior-art search with its
limitations stated.

**Fix applied:** the checklist requires identifying the more consequential
finding. That was missing; **Bug 2 is now explicitly named** in
`fuzz/UPSTREAM-BUGS.md`. Rationale recorded there: Bug 1 needs
`Australia/Lord_Howe`, the only 30-minute DST shift on Earth, so its blast radius
is narrow; Bug 2 needs only two range bounds on different UTC offsets — ordinary
in any DST zone — and fails silently in both directions.

**Still outstanding: filing the two issues at `github.com/pallets-eco/croniter`.**
This requires the owner's GitHub account and is a public action, so it was not
done automatically. It must happen before the freeze to qualify. Once filed, the
URLs go into `README.md` and `DECISIONS.md` #16/#18.

---

## PART 9 — Commit, push, verify

**Pending.** Nothing was committed during this run. The working tree currently
holds two documentation fixes made above:

```
 M README.md                (DST decision surfaced in Overview)
 M fuzz/UPSTREAM-BUGS.md    (Bug 2 named as more consequential)
?? FINAL-CHECK-RESULTS.md   (this file)
```

Post-run verification on the current tree:

```
$ sha256sum -c .test-hashes.sha256   -> all 8 OK
$ pytest tests/original/ -q          -> 228 passed
$ git status --porcelain core/       -> empty (both sabotages fully reverted)
```

Repo is public; `LICENSE` present and MIT (upstream's, retained verbatim).

---

## Open decisions for the owner

1. **File the two upstream issues.** The only blocking item. +3 and $100, expires
   at freeze.
2. **`audit-results.md`** — tracked and public, not part of the target structure.
   Either justify it, move it to `docs/`, or untrack it as was done with
   `CLAUDE.md` and `PRE_SUBMISSION_AUDIT.md`. Its internal cross-reference to the
   eligibility section ("l.546–550") is stale after the README rewrite.
3. **This file** — decide whether `FINAL-CHECK-RESULTS.md` ships or stays local.
