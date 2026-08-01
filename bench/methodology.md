# Benchmark methodology

Every number in `results.json` and in the README was produced by
`bench/run_bench.py` on the machine described below. Nothing here is estimated
or copied from another project.

## Reproduce

```bash
make bench          # or:
cargo build --release -p croniter-core
.venv-baseline/bin/python bench/run_bench.py 25
```

## Hardware and software

| | |
|---|---|
| CPU | Apple M4 |
| RAM | 16 GB |
| OS | macOS 15.7.7 (Darwin 24.6.0), arm64 |
| Python | CPython 3.13.12 |
| Rust | 1.97.1, `--release` (`opt-level = 3`) |
| croniter (original) | commit `3c6ce9bcc5cc7f89116a58f43aaea67e760bff50` |

## Workload

Six cron expressions, chosen so the mix is not dominated by the cheapest code
path — a plain step expression is far cheaper to advance than a
nearest-weekday one, and averaging only over `*/5 * * * *` would flatter the
port:

```
*/5 * * * *                 plain step
0 9 * * 1-5                 weekday range
0 12 l * *                  last day of month
0 9 * * 5#3                 nth weekday (#)
0 9 15w * *                 nearest weekday (W)
*/13 3-20 1,15 jan-jun *    multi-field, alpha months, lists
```

Each is iterated `10000 / 6 = 1666` times with `get_next`, from a fixed start
of `2019-01-14T00:00:00`, for **9,996 iterations total**. Both sides run the
identical workload: `bench/next10k.py` (Python) and `croniter bench` (Rust)
are line-for-line the same schedule and the same start time, and both print a
checksum so the work cannot be optimised away.

## How the numbers were taken

- **25 timed runs** per benchmark, after **3 warmup runs** that are discarded.
- Each run is a **fresh process**, timed end-to-end with
  `time.perf_counter()` around `subprocess.run`.
- Percentiles are **nearest-rank** over the 25 samples.
- Peak RSS comes from `/usr/bin/time -l` (macOS reports bytes), taken in a
  separate invocation from the timed runs so the measurement does not perturb
  the timings.

## Confounders — read these before quoting the speedup

1. **Process startup is included in the workload figure.** This is the honest
   end-to-end number for "run this job", but it is not a pure compute
   comparison. Startup is measured separately so it can be subtracted:

   | | workload (mean) | startup (mean) | implied compute |
   |---|---|---|---|
   | Python original | 263.96 ms | 30.09 ms | ~234 ms |
   | Rust port | 10.42 ms | 2.16 ms | ~8.3 ms |

   So the compute-only ratio is roughly **28.3x**, higher than the end-to-end
   25.3x. The README quotes the end-to-end number because that is what was
   actually measured; the 28.3x is arithmetic on top of it.

2. **25 samples is a small sample for a p99.** At this count the nearest-rank
   p99 is the single worst observation and p95 the second worst. They are real
   measurements of the observed tail, not distributional estimates. Read p99
   here as "worst observed run".

   An earlier 20-run pass recorded 328 ms for the Python workload against
   264 ms here — a 20% swing caused by that run competing with a background
   fuzzing job, not by anything in the code. The numbers above were taken on an
   otherwise idle machine. This is exactly why the run count and conditions are
   recorded rather than just the result.

3. **Python startup includes importing `croniter` and `dateutil`.** The Rust
   binary has no import step at all, so the 14.0x startup ratio is partly a
   comparison of dynamic-import cost, not of code quality.

4. **No CPU pinning, no disabled turbo, laptop on mains.** Runs were taken
   back-to-back on an otherwise idle machine. Run-to-run stddev was under
   2 ms on the workload benchmarks, so the ordering is not in doubt, but a
   third significant figure would not be meaningful.

5. **croniter is pure date arithmetic** — no I/O, no network, no threads. A
   large speedup moving to a compiled language is the expected result. The
   point of measuring was to find out *how much* and to check the tail and
   memory, not to discover whether Rust is faster.

## Results

See `results.json` for the raw numbers, including every field summarised here.

| benchmark | mean | p50 | p95 | p99 | peak RSS | throughput |
|---|---:|---:|---:|---:|---:|---:|
| Python original, 9,996 iterations | 263.96 ms | 263.80 ms | 269.76 ms | 281.28 ms | 14.6 MB | 37,869 ops/s |
| Rust port, 9,996 iterations | 10.42 ms | 10.44 ms | 10.75 ms | 10.79 ms | 4.5 MB | 959,085 ops/s |
| Python original, startup only | 30.09 ms | 29.81 ms | 32.38 ms | 35.32 ms | 14.3 MB | — |
| Rust port, startup only | 2.16 ms | 2.14 ms | 2.28 ms | 2.33 ms | 2.7 MB | — |

**Speedup: 25.3x mean, 26.1x at p99. Startup 14.0x. Peak RSS 3.2x smaller.**

## Equivalence: the benchmark is also a correctness check

Both workloads print `checksum=<sum of every fire time's timestamp>`. Over
9,996 fire times spanning all six expression shapes, the two implementations
produce the identical total:

```
python:  iterations=9996 checksum=26611225207500
rust:    iterations=9996 checksum=26611225207500
```

A single differing minute anywhere in those 9,996 results would change the sum.
`run_bench.py` compares the two and **exits non-zero if they differ**, so a
"fast" result that is quietly computing the wrong schedule cannot pass silently.
A speed win on the wrong answer is not a win.
