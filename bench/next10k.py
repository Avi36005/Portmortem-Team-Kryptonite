"""Python side of the benchmark: the exact workload `croniter bench` runs.

Six expressions covering plain steps, ranges, last-day, nth-weekday and
nearest-weekday, iterated n/6 times each so the mix is not dominated by the
cheapest code path. Run against the ORIGINAL Python croniter.
"""
import sys
from datetime import datetime

from croniter import croniter

EXPRS = [
    "*/5 * * * *",
    "0 9 * * 1-5",
    "0 12 l * *",
    "0 9 * * 5#3",
    "0 9 15w * *",
    "*/13 3-20 1,15 jan-jun *",
]

START = datetime(2019, 1, 14, 0, 0, 0)


def main():
    n = int(sys.argv[1]) if len(sys.argv) > 1 else 10_000
    per = n // len(EXPRS)
    sink = 0.0
    for expr in EXPRS:
        it = croniter(expr, START, ret_type=datetime)
        for _ in range(per):
            it.get_next(datetime)
            sink += it.get_current(float)
    # Printed so the work cannot be optimised away.
    print(f"iterations={per * len(EXPRS)} checksum={sink:.0f}")


if __name__ == "__main__":
    main()
