# PRE-SUBMISSION AUDIT — results

Run: 2026-08-01 · macOS 15.7.7 (Darwin 24.6.0), Apple M4 · CPython 3.13.12
Working tree at `main` = `b6dffb8` + uncommitted changes. Nothing was committed
or pushed during this audit.

## Verdict summary

| Check | Result |
|---|---|
| A1 Bridge falsification | **PASS** |
| A2 Module pytest imports | **PASS** (via `.so`, see note) |
| A3 Benchmark checksum falsification | **PASS** |
| B1 Test hashes verify | **PASS** |
| B2 One commit on tests/original/ | **PASS** (see note) |
| B3 No uncommitted test changes | **PASS** |
| B4 History explainable | **PASS** |
| **C1 Clean-clone build** | ~~FAIL~~ → **PASS** (fixed during audit) |
| C2 Binary standalone, no Python | **PASS** |
| C3 Zero unsafe / no pyo3 / clippy | **PASS** |
| D Deliverables present | **PASS locally / absent on GitHub** |
| D1 README claims | **PASS** |
| D2 DECISIONS.md quality | **PASS with one gap** |
| D3 Fuzz evidence | **PASS** |
| D4 Benchmark metrics | **PASS** |
| **E Bug Catcher — issues filed** | **FAIL — not filed** |
| F Repo hygiene | **PASS with one item** |
| G Final submission steps | **NOT DONE — awaiting your go-ahead** |

**C1 was a blocker and is now fixed** — the 17 unpushed commits were pushed
during this audit and the published repo now builds. **E remains open:** the two
upstream bugs are written up but have never been filed as GitHub issues.

---

## SECTION A — falsification tests

### A1 — Bridge falsification
Command:
```
# baseline
maturin develop -m pybridge/Cargo.toml && pytest tests/original/ -q
# sabotage: core/src/consts.rs:22  RANGES hour  (0, 23) -> (0, 22)
maturin develop -m pybridge/Cargo.toml && pytest tests/original/ -q
# revert, rebuild, re-run
```
Output:
```
baseline:   228 passed in 1.88s
sabotaged:  32 failed, 196 passed in 2.10s
  e.g. CroniterHashExpanderExpandHoursTest::test_expand_hours_with_full_range
       croniter.CroniterBadCronError: [* H(0-23) * * *] is not acceptable, out of ...
reverted:   228 passed in 1.78s
```
Result: **PASS**
Notes: A one-character change to a Rust constant moved 32 tests from pass to
fail, so the suite is genuinely executing Rust and is capable of failing.
Revert verified byte-identical: all 12 files in `core/src` matched their
pre-sabotage SHA-256. `git checkout core/` was deliberately **not** used — it
would have destroyed uncommitted work in `lib.rs`, `bin/croniter.rs` and the
untracked `tz.rs`. Files were restored by re-editing and checked by hash.

### A2 — Which module pytest imports
Command:
```
python -c "import croniter; print(croniter.__file__)"
python -c "import croniter.croniter as m; print(m.__file__)"
```
Output:
```
croniter.__file__          -> pybridge/python/croniter/__init__.py
croniter.croniter.__file__ -> pybridge/python/croniter/croniter.cpython-313-darwin.so
class croniter             -> <class 'builtins.croniter'>   (extension type)
inspect.getfile(croniter)  -> TypeError: is a built-in class
pip list                   -> croniter 0.1.0 <repo>/pybridge   (editable, ours)
```
Result: **PASS**
Notes: The audit's literal criterion ("path ends in `.so`") reads FAIL because
the top-level name resolves to a `.py`. It is a **pure re-export shim** — I read
the whole file; it contains zero logic, only `from .croniter import (...)`. The
implementation module is the maturin-built `.so`, and the `croniter` class is a
builtin extension type with no Python source. No upstream pure-Python croniter
is installed in `.venv`. Substantively PASS. Worth keeping the shim explanation
in the README so a judge doesn't hit the same false alarm.

### A3 — Benchmark checksum falsification
Command:
```
# baseline, then sabotage core/src/bin/croniter.rs:237  sink += ts  ->  sink += ts + 1.0
cargo build --release -p croniter-core
.venv-baseline/bin/python bench/next10k.py 10000 ; ./target/release/croniter bench -n 10000
```
Output:
```
baseline    python: checksum=26611225207500   rust: checksum=26611225207500   (match)
sabotaged   python: checksum=26611225207500   rust: checksum=26611225217496   (differ by 9996)
reverted    python: checksum=26611225207500   rust: checksum=26611225207500   (match)
```
Result: **PASS**
Notes: Delta is exactly 9,996 — one per iteration — so the checksum tracks every
fire time and cannot mask a wrong schedule. `bench/results.json` was backed up
and confirmed byte-identical afterwards; the full `make bench` was not re-run
because it rewrites that committed artifact, and the direct two-binary
comparison is precisely what the equivalence claim rests on.

---

## SECTION B — test integrity

### B1 — Hashes verify
Command: `shasum -a 256 -c .test-hashes.sha256` (macOS equivalent of `sha256sum -c`)
Output: all 8 files `OK`, exit 0.
Result: **PASS**

### B2 — Test files have exactly one commit
Command: `git log --all --oneline -- tests/original/`
Output:
```
c97d972 test(bootstrap): vendor original croniter test suite and compute sha256 fingerprints
c2d5092 Phase 0: bootstrap — vendored tests, fingerprints, workspace, stub bridge
```
Result: **PASS**
Notes: Two commits appear only because `--all` sweeps an abandoned local branch.
Resolved:
- `git log main -- tests/original/` → **exactly 1 commit** (`c97d972`).
- `c2d5092` is reachable only from the local branch `backup-5-commits`, not from
  `main` and not from `origin/main`.
- Both commits produce the **identical tree**:
  `8048ceb2a372e1c0af1f92b85af463135cfde0c9`, and
  `git diff c2d5092 c97d972 -- tests/original/` is empty.
No content ever changed. The shipped branch has one vendoring commit, as claimed.
The README's stated command (`git log --oneline -- tests/original/`, no `--all`)
returns 1 and is accurate as written.

### B3 — No uncommitted or untracked changes to tests
Command: `git status --porcelain tests/original/` and `git diff --stat -- tests/original/`
Output: both empty.
Result: **PASS**

### B4 — History explainable
Command: `git log --oneline -25 --format='%h %an <%ae> %ad %s'` · `ls -la .git/hooks/ | grep -v sample`
Output:
```
31 commits on main. Authors:
  11  2005sahildeshmukh <149414701+...@users.noreply.github.com>
  10  Hardik182005      <148485624+...@users.noreply.github.com>
  10  Avi36005          <177668299+...@users.noreply.github.com>
.git/hooks/ : no active hooks (samples only)
```
Result: **PASS**
Notes: Three authors, all GitHub noreply addresses for Team Kryptonite members.
No AI/tool authorship, no hooks. Commit messages map to phases.

---

## SECTION C — build and artifact

### C1 — Clean-clone build
Command:
```
git clone https://github.com/Avi36005/Portmortem-Team-Kryptonite audit-clone
cd audit-clone && make build
```
Output:
```
cargo build --release -p croniter-core
error: can't find bin `croniter` at path `.../audit-clone/core/src/bin/croniter.rs`
 --> .../audit-clone/core/Cargo.toml
error: could not compile due to 1 previous target resolution error
make: *** [build] Error 101
```
Result: ~~**FAIL — BLOCKER**~~ → **PASS** (fixed and re-verified, see below)

**Original finding.** The published repository did not build. `origin/main` was
16 commits behind local `main`; the clone contained 32 of 54 tracked files.
Missing from GitHub at the time of the audit:

| Missing | Kind |
|---|---|
| `core/src/bin/croniter.rs` | **the shipped binary** — build fails on this |
| `core/src/hash.rs`, `core/src/range.rs` | core source (HashExpander, croniter_range) |
| `pybridge/src/clock.rs` | bridge source |
| `README.md`, `DECISIONS.md`, `notes.md` | scored deliverables |
| `Dockerfile` | Rule 03 artifact |
| `fuzz/log.txt`, `fuzz/differential.py`, `fuzz/UPSTREAM-BUGS.md`, `fuzz/oracle.py`, `fuzz/invariants.py`, `fuzz/probe.py` | +5 fuzz bonus evidence |
| `bench/methodology.md`, `bench/results.json`, `bench/next10k.py`, `bench/run_bench.py` | benchmark evidence |
| `scripts/demo.sh`, `core/tests/cli.rs`, `tests/original/` docs | demo + Rust CLI tests |

A judge cloning the repo would have got a non-building project with no README,
no DECISIONS.md, and none of the bonus evidence — near zero against "repo builds
with one command".

**Resolution.** The junk deletions were committed as `0274644`
(`chore: remove committed junk files`) and all 17 commits pushed to
`origin/main`. Before pushing, a clean clone of the exact commit was built to
confirm it was self-contained — in particular that HEAD's `core/src/lib.rs` does
**not** reference the untracked `core/src/tz.rs`, so nothing was published in a
half-wired state.

Re-verified against the live GitHub repo after the push:
```
git clone https://github.com/Avi36005/Portmortem-Team-Kryptonite audit-clone2
cd audit-clone2 && make build

0274644 chore: remove committed junk files
files: 54
    Finished `release` profile [optimized] target(s) in 5.36s
built target/release/croniter

$ ./target/release/croniter next '0 12 L * *' -n 2 --start 2024-02-01T00:00:00
2024-02-29T12:00:00      <- leap-year last-day, correct
2024-03-31T12:00:00
```
Also confirmed in the clone: `shasum -a 256 -c .test-hashes.sha256` all OK, no
`.DS_Store` or findings dumps tracked, and `README.md`, `DECISIONS.md`,
`Dockerfile`, `fuzz/log.txt`, `bench/results.json`, `core/src/bin/croniter.rs`
all present. **C1 now PASSES.**

### C2 — Binary runs standalone
Command: `./target/release/croniter --help` · `croniter next '0 9 * * 1-5'` · `otool -L`
Output:
```
croniter — cron expression parsing and next/prev fire times
USAGE: croniter next|prev|range|match|validate|bench ...

$ croniter next '0 9 * * 1-5'
2026-08-03T09:00:00
2026-08-04T09:00:00
2026-08-05T09:00:00 ...

$ otool -L ./target/release/croniter
  /System/Library/Frameworks/CoreFoundation.framework/.../CoreFoundation
  /usr/lib/libSystem.B.dylib

python linkage matches: 0
undefined Py_/PyObject symbols: 0
```
Result: **PASS**
Notes: `ldd` is Linux-only; `otool -L` is the macOS equivalent. Two system
libraries, no interpreter, no Python symbols. This is solid Rule 05 evidence.

### C3 — Zero unsafe, no pyo3, clippy clean
Command: as written in the audit (with globs quoted for zsh)
Output:
```
grep -rn "unsafe" core/src/ | grep -v "forbid(unsafe_code)" | wc -l   -> 0
head -1 core/src/lib.rs                                               -> #![forbid(unsafe_code)]
grep -rn "pyo3" core/ --include='*.rs' --include='*.toml'              -> 0
cargo tree -p croniter-core --edges normal | grep -ci pyo3             -> 0
cargo clippy --all-targets -- -D warnings                              -> exit 0, 0 warnings
```
Result: **PASS**
Notes: `core/Cargo.toml` deps are exactly `chrono`, `chrono-tz`, `regex`,
`thiserror`. The audit's unquoted `--include=*.rs` errors under zsh
(`no matches found`) — quote it in the demo script or it will look like a failure
on camera.

---

## SECTION D — deliverables

All present locally with real content. **All except `Makefile`/`LICENSE` are
absent from GitHub** — see C1.

```
README.md            567 lines   DECISIONS.md   410 lines
bench/methodology.md 5,308 B     bench/results.json 3,059 B
fuzz/log.txt         664 lines   Dockerfile     832 B
DECISIONS.md headings: 20  (19 numbered entries + failing-tests section)
```

Additional verification run beyond the audit script:

| Claim | Measured |
|---|---|
| Port 228/228 | **228 passed in 1.78s** — matches README exactly |
| Baseline 228/228 | **228 passed in 1.48s** (README says 1.54s — run-to-run variance, both honest) |
| Baseline provenance | `/tmp/croniter-src` HEAD = `3c6ce9bcc5cc...` = `.kickoff-commit`, clean tree |
| 65 Rust tests | **53 unit + 12 CLI = 65 passed, 0 failed** |
| `make verify` | fingerprints OK · unsafe 0 · pyo3 0 |

### D1 — README claims
Result: **PASS** — every checklist item present and accurate.
Baseline+SHA (l.67), port number (l.68), unsafe as compiler-enforced (l.72),
eligibility incl. the `cron`/zslayton argument (l.546–550), one-command build
(l.137), hash-verification instructions (l.220–226), per-file pass table
(l.82–90), in/out-of-scope (l.99–128), track D + source URL (l.5, l.17).
Notes: The Docker path is explicitly flagged "Not verified by us" (l.190–193).
I re-checked — the Docker daemon is still down on this machine, so that
disclosure remains accurate and should stay.

### D2 — DECISIONS.md quality
Result: **PASS with one gap**
19 substantive numbered entries, no empty bullets. #1 is the two-crate split
with the Rule 05 argument. The DST/`TzClock` decision is #19, with #11 and #12
covering the timezone seam. Out-of-scope items listed with reasons. The
"Failing tests" section honestly states there are none and explains two
near-misses that were fixed at the cause.
**Gap:** the checklist requires "both upstream bugs documented **with links to
the filed issues**". The bugs are documented superbly (#16, #18 and the
1,100-line `fuzz/UPSTREAM-BUGS.md`) but **no issue URL exists anywhere** — see
Section E.

### D3 — Fuzz evidence
Command: `head -20 fuzz/log.txt` · `wc -l` · `grep -ci diverg`
Output:
```
seed=1785574786 budget=120.0s batch=250
...
elapsed_seconds=120.1
batches=642
inputs_compared=160500
divergences=0
coverage by (operation, timezone-aware?):  prev/next/expand/is_valid/match/range,
                                           tz and naive, 8,970–22,378 each
664 lines
```
Result: **PASS** — 120.1s (>60s required), input count visible, per-operation
coverage broken out, `fuzz/differential.py` committed and re-runnable.
Notes: The "port" side runs the Rust extension through the bridge rather than
the standalone binary. That is a legitimate differential of the Rust core, but
say so out loud in the video so it isn't mistaken for Python-vs-Python.

### D4 — Benchmark metrics
Result: **PASS** — p99, RSS, startup and throughput all present for all four
benchmarks. `methodology.md` states hardware, 25 runs, 3 warmups, the six-expression
workload, and five confounders — including that p99 over 25 samples is "worst
observed run" and that an earlier 328ms reading was contaminated by a background
fuzz job. This is the strongest artifact in the repo.

---

## SECTION E — Bug Catcher

Command: `grep -n "github.com/pallets-eco/croniter/issues" DECISIONS.md README.md fuzz/UPSTREAM-BUGS.md`
Output: **no matches.** The only issue link in the repo is `cpython#101069`.

Result: **FAIL — the bugs have not been filed**

Notes: I independently re-ran both repros. Both reproduce exactly as documented,
on the original **and** on the port:

```
                          original Python      Rust port
BUG1 next                 2019-10-06 03:00+11  2019-10-06 03:00+11
BUG1 prev                 2019-10-06 02:30+11  2019-10-06 02:30+11
BUG1 match(02:30)         True                 True
BUG1 prev overshoots?     True                 True
BUG2A range len           1  (true answer 6)   1
BUG2B last result         UTC 01:00  <- before requested lower bound UTC 01:15
```

So the findings are real, the write-up is excellent, prior art was searched in
good faith, and the port's "we reproduce both deliberately" claim is verified.
**The only missing step is clicking submit on two GitHub issues.** The audit
calls this "the single highest-value remaining action" and I agree: +3 points
and $100 are sitting unclaimed, and the bonus requires filing *during* the
event window. Bug 2 is the more consequential of the two (silent data loss in
a public API, no timezone exotica required) — name it as the $100 candidate.

---

## SECTION F — repo hygiene

Output:
```
junk in index (staged):   none
junk in HEAD commit:      .DS_Store, core/.DS_Store, pybridge/.DS_Store
junk in history:          same three
tracked files:            54
.git size:                2.7M
LICENSE:                  MIT, upstream's (Matsumoto Taichi), retained verbatim
repo visibility:          public
secrets scan:             1 hit — CLAUDE.md:71, the prose "Commit secrets, .env,
                          or large binaries | Repo is public". Not a secret.
fuzz/log.txt committed:   yes
findings dumps committed: fuzz/invariant-findings.txt is in HEAD (now gitignored
                          and staged for deletion, not yet committed)
```
Result: **PASS** (junk removed during audit)
Notes: No secrets, no `.env`, no binaries, no `target/`, no `.venv`. `.git` is
2.7M. The four junk files were committed as deleted in `0274644` and are gone
from `origin/main` — confirmed absent from a fresh clone. Note they remain in
*history* (the commits that introduced them were never rewritten); removing them
from history would require a force-push, which is not worth the risk this close
to the freeze and is not what the rubric asks. `.gitignore` now covers
`.DS_Store` and the findings dumps, but **that change is still uncommitted.**

---

## SECTION G — final submission steps

**Partly done.** Junk deletions committed (`0274644`) and all 17 commits pushed
to `origin/main` on approval. Post-commit re-verification:

```
shasum -a 256 -c .test-hashes.sha256   -> all 8 OK
pytest tests/original/ -q              -> 228 passed in 1.41s
clean clone from GitHub + make build   -> exit 0, binary correct
```

Outstanding, in priority order:

1. **File the two upstream issues** at `github.com/pallets-eco/croniter`, then
   paste the URLs into `DECISIONS.md` #16/#18 and the README. Highest value per
   minute of work, and time-boxed to the event window. This is now the single
   biggest remaining gap.
2. **Decide on the uncommitted work** (listed below) — in particular the two
   untracked fuzz files that `make hunt` and `UPSTREAM-BUGS.md` both reference.
3. Record the demo video. Quote the exact commands from C1–C3 above, with the
   `--include` globs quoted, and mention the `__init__.py` shim before the
   `.so` (A2) so nobody misreads it.

Uncommitted work also still outstanding: `core/src/tz.rs` (untracked),
`fuzz/invariants2.py`, `fuzz/triage.py` (untracked, both referenced by the
`make hunt` target and by `UPSTREAM-BUGS.md`), plus modifications to
`DECISIONS.md`, `README.md`, `Makefile`, `core/src/lib.rs`,
`core/src/bin/croniter.rs`, `fuzz/UPSTREAM-BUGS.md`, `scripts/demo.sh`.
**`make hunt` and the UPSTREAM-BUGS reproduction instructions reference two
files that are untracked — they would not exist for a judge.**

---

## Overall

The engineering and the evidence are in very good shape. Every headline claim I
could test held up under falsification: the bridge really runs Rust, the
checksum really can diverge, the tests really are untouched, the binary really
carries no interpreter, and both upstream bugs really reproduce. The honesty
discipline is visible throughout — the unverified Docker path and the
contaminated benchmark run are both disclosed rather than hidden.

The larger of the two risks is now closed: the published repository builds and
contains every deliverable. What remains is that **two bugs worth +3 and $100
are written up but never filed** — minutes of work, and it must happen inside
the event window.
