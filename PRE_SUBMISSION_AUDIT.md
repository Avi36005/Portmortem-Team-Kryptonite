# PRE-SUBMISSION AUDIT — Team Kryptonite

**Agent: execute this file top to bottom. Run every command. Report actual
output, never predicted output. Do not skip a check because it "should" pass.**

**Deadline: Mon 03 Aug 2026, 18:00 UTC (11:30 PM IST)**

Current claimed state: 228/228 · 65 Rust tests · 0 clippy warnings ·
0 unsafe · 0 pyo3 in core · 8/8 hashes · benchmark checksums identical.

This audit exists to verify those claims are real, not to re-assert them.
For each check, paste the raw command output into `audit-results.md` and mark
PASS or FAIL. If anything FAILs, stop and fix before continuing.

---

## SECTION A — CRITICAL: falsification tests

These are the highest-priority checks in this file. A test suite that cannot
fail is not evidence of anything. Run these before trusting any number.

### A1 — Prove the bridge actually routes to Rust

228/228 is what a correct port looks like. It is *also* what a broken bridge
that silently swallows errors looks like. Rule out the second.

```bash
# 1. Pick one core function that many tests depend on (e.g. in expand.rs
#    or calc.rs). Introduce a deliberate off-by-one or wrong constant.
#    Record exactly what you changed.

maturin develop -m pybridge/Cargo.toml
python3 -m pytest tests/original/ -q
```

**Expected: a significant number of FAILURES.**

- If failures appear → the bridge is real. Revert with `git checkout core/`,
  rebuild, confirm 228/228 returns. Record both numbers.
- If it still shows 228/228 → **STOP. The bridge is not executing Rust code.**
  Investigate immediately: is pytest importing the installed Python `croniter`
  from site-packages instead of the maturin-built module? Check with
  `python3 -c "import croniter; print(croniter.__file__)"` — it must point at
  the maturin artifact, not a `.py` file.

### A2 — Confirm which module pytest actually imports

```bash
python3 -c "import croniter; print(croniter.__file__); print(croniter.__doc__)"
pip list 2>/dev/null | grep -i croniter
```

**PASS:** path ends in `.so` / `.pyd` / points at the maturin build.
**FAIL:** path ends in `.py` → pytest is testing the Python original and the
228/228 number is meaningless.

### A3 — Falsify the benchmark checksums

"Benchmark checksums identical" must also be capable of differing. Change one
output value in the Rust bench path, re-run, confirm the checksums diverge,
then revert.

```bash
# after deliberate change:
make bench   # or the bench command in use
# confirm checksums now DIFFER, then:
git checkout core/
make bench   # confirm they match again
```

---

## SECTION B — Test integrity (the 40% criterion)

### B1 — Hashes verify

```bash
sha256sum -c .test-hashes.sha256
```
**PASS:** every line reports `OK`. **FAIL:** anything else → revert those
files from the original clone immediately, do not commit.

### B2 — Test files have exactly one commit in history

```bash
git log --all --oneline -- tests/original/
git log --all --stat --oneline -- tests/original/ | head -40
```
**PASS:** one commit — the initial vendoring.
**FAIL:** more than one → inspect every later commit with
`git show <sha> -- tests/original/`. Any content change must be reverted and
disclosed in DECISIONS.md.

### B3 — No uncommitted or untracked changes to tests

```bash
git status --porcelain tests/original/
git diff --stat -- tests/original/
```
**PASS:** both empty.

### B4 — Commit history is explainable

```bash
git log --oneline -25 --format='%h %an <%ae> %ad %s'
```
Confirm every author is expected and every commit corresponds to a phase or a
deliberate action. Note: phase-boundary commits by the agent are expected and
were requested. Flag anything from an unrecognized author, tool, or hook.

```bash
ls -la .git/hooks/ | grep -v sample   # any active hooks?
```

---

## SECTION C — Build and artifact (Rule 03)

### C1 — Clean-clone build works

The strongest version of this test: clone your own repo fresh into /tmp and
build there. "Works on my machine" is explicitly disallowed.

```bash
cd /tmp && rm -rf audit-clone
git clone https://github.com/Avi36005/Portmortem-Team-Kryptonite audit-clone
cd audit-clone
# run the ONE documented build command from README
make build   # or: cargo build --release -p core  /  docker compose up
```
**PASS:** completes with no manual steps not written in the README.
**FAIL:** any undocumented step → fix the README or the Makefile.

### C2 — The binary runs standalone

```bash
./target/release/croniter --help
./target/release/croniter '0 9 * * 1-5'   # or whatever the CLI surface is
ldd ./target/release/croniter | grep -i python   # MUST return nothing
```
**PASS:** runs; no Python linkage. This is the Rule 05 evidence.

### C3 — Zero unsafe in core, no pyo3 in core

```bash
grep -rn "unsafe" core/src/ | grep -v "forbid(unsafe_code)"     # expect empty
head -1 core/src/lib.rs                                          # expect #![forbid(unsafe_code)]
grep -rn "pyo3" core/ --include=*.rs --include=*.toml            # expect empty
cargo clippy --all-targets -- -D warnings                        # expect clean
```

---

## SECTION D — Deliverables present

Check each file exists AND has real content (not a placeholder).

```bash
ls -la README.md DECISIONS.md LICENSE Dockerfile Makefile \
      .port-mortem.toml .kickoff-commit .test-hashes.sha256 \
      fuzz/log.txt bench/methodology.md bench/results.json 2>&1
wc -l README.md DECISIONS.md
grep -c "^##\|^[0-9]\+\." DECISIONS.md    # count entries — need 10+
```

### D1 — README contains the required claims

Verify by reading, not grepping:

- [ ] `Original baseline: 228/228 at commit <sha>` — the actual SHA from `.kickoff-commit`
- [ ] `Port: 228/228` — the measured number
- [ ] Unsafe count in core, stated as compiler-enforced
- [ ] Eligibility argument (no direct Rust port exists; `cron` crate is
      independent and lacks L / W / # / hash expressions / croniter_range)
- [ ] One-command build instructions
- [ ] How a judge verifies the test hashes
- [ ] Per-file pass-rate table
- [ ] In-scope / out-of-scope section
- [ ] Track letter (D) and source repo URL

### D2 — DECISIONS.md quality

- [ ] 10+ entries minimum (currently claims 19+)
- [ ] Every entry has a real rationale, not an empty bullet
- [ ] Entry #1 is the two-crate split + Rule 05 compliance argument
- [ ] The DST / WallClock decision is present and explains *why* the bridge
      deliberately does not route through chrono-tz
- [ ] Both upstream bugs documented with links to the filed issues
- [ ] Out-of-scope items listed with reasons

### D3 — Fuzz evidence

```bash
head -20 fuzz/log.txt
wc -l fuzz/log.txt
grep -ci "diverg" fuzz/log.txt
```
- [ ] Log shows a 60+ second continuous run
- [ ] Input count is visible
- [ ] Divergences (if any) are reported honestly, not filtered out
- [ ] `fuzz/differential.py` is committed so a judge can re-run it

### D4 — Benchmarks report the right metrics

```bash
cat bench/results.json
cat bench/methodology.md
```
- [ ] **p99** present (not just mean/average)
- [ ] **RSS** present
- [ ] **startup time** present
- [ ] throughput present
- [ ] methodology states hardware, iteration count, warmup, workload, confounders

---

## SECTION E — Bug Catcher (+3 and $100)

```bash
cat DECISIONS.md | grep -i -A5 "upstream\|bug\|issue"
```

- [ ] Both bugs filed at `github.com/pallets-eco/croniter` **during** the event
- [ ] Issue URLs recorded in DECISIONS.md and README
- [ ] Each repro minimized to the smallest failing expression
- [ ] Which bug is the more consequential one is identified (the $100 goes to
      the most consequential finding)

**If not yet filed: file them now.** The bonus requires filing during the
hackathon window. This is the single highest-value remaining action.

---

## SECTION F — Repo hygiene

```bash
git ls-files | grep -iE "\.DS_Store|\.env|__pycache__|\.pyc|target/|\.venv" 
du -sh .git
cat .gitignore
git ls-files | wc -l
```
- [ ] No junk files tracked
- [ ] No secrets or `.env`
- [ ] `LICENSE` present and MIT (croniter is MIT — must stay compatible)
- [ ] Repo is public
- [ ] `fuzz/log.txt` IS committed (it is required evidence)
- [ ] Generated findings dumps are NOT committed

---

## SECTION G — Final submission steps

- [ ] **Commit and push everything.** Uncommitted work is unbacked work.
- [ ] Re-run `sha256sum -c .test-hashes.sha256` after the final commit
- [ ] Re-run the full suite after the final commit; record the number
- [ ] **Record the 5-minute demo video** showing, on screen:
      1. `sha256sum -c .test-hashes.sha256` passing
      2. `python3 -m pytest tests/original/` reaching 228/228 live
      3. `python3 -c "import croniter; print(croniter.__file__)"` proving it's Rust
      4. the unsafe count command returning 0
      5. the one-command build from a clean clone
- [ ] Submit via the organizers' submission form (shared on the final day)
- [ ] Include: repo URL, track D, team details, source repo URL

---

## SECTION H — After the deadline (Aug 10, separate $300)

- [ ] Publish the write-up from `notes.md` + "the three bugs that cost us the
      most" section of the README
- [ ] Post on X / LinkedIn / Dev.to, tag Hackathon Raptors
- [ ] Cover: what was picked and why, what broke, how behavioral equivalence
      was proven, the DST decision and why the bridge deliberately avoids
      chrono-tz, the two upstream bugs, and what you'd do differently
- [ ] Judged on insight, not follower count. Deadline Aug 10, 18:00 UTC.

---

## Reporting format

Write `audit-results.md` with one block per check:

```
### A1 — Bridge falsification
Command: <exact command run>
Output:  <raw output, trimmed only for length>
Result:  PASS / FAIL
Notes:   <what was changed and reverted, if applicable>
```

**Report failures loudly. Do not fix a failure by weakening the check.**
If a check cannot pass, record why in DECISIONS.md and state it honestly in
the README. A documented shortfall scores better than a hidden one.
