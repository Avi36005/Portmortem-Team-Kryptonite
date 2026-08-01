"""Benchmark harness: original Python croniter vs the Rust port.

Self-contained (stdlib only) so it needs nothing installed beyond the two
virtualenvs and the release binary.

Reports p50/p95/p99 as well as mean, because a mean alone hides the tail --
and peak RSS, measured with /usr/bin/time, because memory is the other half of
the story. Every number written to results.json was measured by this script on
the machine named in the output.
"""
import json
import os
import platform
import re
import statistics
import subprocess
import sys
import time

REPO = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
ITERATIONS = 10_000


def percentile(values, pct):
    """Nearest-rank percentile on the sorted sample."""
    if not values:
        return None
    ordered = sorted(values)
    k = max(0, min(len(ordered) - 1, int(round(pct / 100.0 * len(ordered) + 0.5)) - 1))
    return ordered[k]


def time_runs(cmd, runs, warmup):
    """Wall-clock time each run of `cmd`. Returns (times, stdout_of_last)."""
    for _ in range(warmup):
        subprocess.run(cmd, cwd=REPO, capture_output=True, check=True)
    times, out = [], ""
    for _ in range(runs):
        t0 = time.perf_counter()
        proc = subprocess.run(cmd, cwd=REPO, capture_output=True, check=True)
        times.append(time.perf_counter() - t0)
        out = proc.stdout.decode().strip()
    return times, out


def peak_rss_bytes(cmd):
    """Peak RSS via /usr/bin/time -l (macOS) or -v (GNU)."""
    flag = "-l" if platform.system() == "Darwin" else "-v"
    proc = subprocess.run(
        ["/usr/bin/time", flag] + cmd, cwd=REPO, capture_output=True
    )
    err = proc.stderr.decode()
    m = re.search(r"(\d+)\s+maximum resident set size", err)
    if m:
        return int(m.group(1))  # macOS reports bytes
    m = re.search(r"Maximum resident set size \(kbytes\):\s*(\d+)", err)
    if m:
        return int(m.group(1)) * 1024
    return None


def summarize(name, cmd, runs, warmup, iterations=None):
    times, out = time_runs(cmd, runs, warmup)
    rss = peak_rss_bytes(cmd)
    row = {
        "name": name,
        "command": " ".join(cmd),
        "runs": runs,
        "warmup": warmup,
        "mean_s": statistics.fmean(times),
        "stddev_s": statistics.stdev(times) if len(times) > 1 else 0.0,
        "min_s": min(times),
        "p50_s": percentile(times, 50),
        "p95_s": percentile(times, 95),
        "p99_s": percentile(times, 99),
        "max_s": max(times),
        "peak_rss_bytes": rss,
        "stdout": out,
    }
    if iterations:
        row["iterations"] = iterations
        row["throughput_per_s"] = iterations / statistics.fmean(times)
    return row


def main():
    py_orig = os.path.join(REPO, ".venv-baseline", "bin", "python")
    rust_bin = os.path.join(REPO, "target", "release", "croniter")
    for path in (py_orig, rust_bin):
        if not os.path.exists(path):
            sys.exit(f"missing {path}")

    runs = int(sys.argv[1]) if len(sys.argv) > 1 else 20

    results = {
        "hardware": {
            "machine": platform.machine(),
            "processor": subprocess.run(
                ["sysctl", "-n", "machdep.cpu.brand_string"],
                capture_output=True, text=True,
            ).stdout.strip() or platform.processor(),
            "system": f"{platform.system()} {platform.release()}",
            "os_version": platform.mac_ver()[0] or platform.version(),
            "python": platform.python_version(),
        },
        "workload": {
            "description": (
                "6 cron expressions (plain step, weekday range, last-day-of-month, "
                "nth-weekday, nearest-weekday, multi-field) iterated "
                f"{ITERATIONS // 6} times each via get_next"
            ),
            "iterations_total": (ITERATIONS // 6) * 6,
        },
        "benchmarks": [],
    }

    # Full workload: 10k get_next calls, process start included.
    results["benchmarks"].append(
        summarize(
            "python_original_next10k",
            [py_orig, "bench/next10k.py", str(ITERATIONS)],
            runs, 3, iterations=(ITERATIONS // 6) * 6,
        )
    )
    results["benchmarks"].append(
        summarize(
            "rust_port_next10k",
            [rust_bin, "bench", "-n", str(ITERATIONS)],
            runs, 3, iterations=(ITERATIONS // 6) * 6,
        )
    )

    # Startup only: what it costs to exist and do nothing.
    results["benchmarks"].append(
        summarize(
            "python_original_startup",
            [py_orig, "-c", "import croniter"],
            runs, 3,
        )
    )
    results["benchmarks"].append(
        summarize("rust_port_startup", [rust_bin, "--version"], runs, 3)
    )

    by = {b["name"]: b for b in results["benchmarks"]}

    # Both workloads print `checksum=<sum of every fire time's timestamp>`.
    # If the two implementations agree on all 9,996 fire times the checksums
    # are identical; a single differing minute anywhere changes the total.
    # This makes the benchmark double as an equivalence check, so a "fast"
    # result that is quietly computing the wrong schedule cannot pass silently.
    def checksum_of(name):
        m = re.search(r"checksum=(-?\d+)", by[name]["stdout"])
        return m.group(1) if m else None

    py_sum = checksum_of("python_original_next10k")
    rs_sum = checksum_of("rust_port_next10k")
    results["equivalence"] = {
        "python_checksum": py_sum,
        "rust_checksum": rs_sum,
        "iterations": (ITERATIONS // 6) * 6,
        "match": py_sum is not None and py_sum == rs_sum,
    }

    # Derived comparisons, computed rather than asserted.
    results["comparison"] = {
        "workload_speedup_mean": by["python_original_next10k"]["mean_s"]
        / by["rust_port_next10k"]["mean_s"],
        "workload_speedup_p99": by["python_original_next10k"]["p99_s"]
        / by["rust_port_next10k"]["p99_s"],
        "startup_speedup_mean": by["python_original_startup"]["mean_s"]
        / by["rust_port_startup"]["mean_s"],
        "rss_ratio": (
            by["python_original_next10k"]["peak_rss_bytes"]
            / by["rust_port_next10k"]["peak_rss_bytes"]
            if by["rust_port_next10k"]["peak_rss_bytes"]
            else None
        ),
    }

    out_path = os.path.join(REPO, "bench", "results.json")
    with open(out_path, "w") as f:
        json.dump(results, f, indent=2)

    def fmt(row):
        rss = row["peak_rss_bytes"]
        rss_s = f"{rss / 1024 / 1024:8.1f} MB" if rss else "        n/a"
        thr = row.get("throughput_per_s")
        thr_s = f"{thr:12,.0f}" if thr else "         n/a"
        return (
            f"{row['name']:32s} {row['mean_s']*1000:9.2f} {row['p50_s']*1000:9.2f} "
            f"{row['p95_s']*1000:9.2f} {row['p99_s']*1000:9.2f} {rss_s} {thr_s}"
        )

    print(f"{'benchmark':32s} {'mean ms':>9} {'p50 ms':>9} {'p95 ms':>9} {'p99 ms':>9} "
          f"{'peak RSS':>11} {'ops/s':>12}")
    print("-" * 100)
    for row in results["benchmarks"]:
        print(fmt(row))
    print("-" * 100)
    c = results["comparison"]
    print(f"workload speedup (mean): {c['workload_speedup_mean']:.1f}x")
    print(f"workload speedup (p99):  {c['workload_speedup_p99']:.1f}x")
    print(f"startup speedup (mean):  {c['startup_speedup_mean']:.1f}x")
    if c["rss_ratio"]:
        print(f"peak RSS ratio:          {c['rss_ratio']:.1f}x smaller")

    eq = results["equivalence"]
    print()
    if eq["match"]:
        print(f"equivalence: checksums MATCH over {eq['iterations']} fire times "
              f"({eq['python_checksum']})")
    else:
        print("equivalence: CHECKSUMS DIFFER — the two implementations do not "
              "agree on the schedule")
        print(f"  python={eq['python_checksum']}  rust={eq['rust_checksum']}")

    print(f"\nwrote {out_path}")
    # A speed win on the wrong answer is not a win.
    return 0 if eq["match"] else 1


if __name__ == "__main__":
    sys.exit(main())
