"""Triage raw findings: separate croniter's DOCUMENTED behaviour from real bugs.

The field checker in `invariants2.py` flags every fire time whose wall-clock
fields do not satisfy the expression. Most of those are not bugs: when a local
time does not exist (a DST spring-forward skipped it), croniter deliberately
walks forward to the next existing instant, and the result legitimately has
different fields. `_add_tzinfo`, croniter.py:179.

So a field mismatch is only interesting when the time croniter *should* have
returned actually existed. This script applies that filter, so the reported
count is a count of real problems rather than a count of alarms.
"""
import ast
import sys
from collections import Counter
from datetime import datetime, timedelta

from dateutil.tz import datetime_exists


def parse(path):
    out = []
    with open(path) as f:
        for line in f:
            line = line.strip()
            if not line:
                continue
            try:
                out.append(ast.literal_eval(line))
            except Exception:
                pass
    return out


def load_tz(label):
    """label looks like 'Australia/Lord_Howe/pytz' or 'naive'."""
    if label == "naive":
        return None, None
    *zone_parts, lib = label.split("/")
    zone = "/".join(zone_parts)
    if lib == "pytz":
        import pytz
        return pytz.timezone(zone), "pytz"
    import zoneinfo
    return zoneinfo.ZoneInfo(zone), "zoneinfo"


def explained_by_forward_shift(fire_str, label):
    """True when the mismatch is croniter's documented skip-forward.

    Look at the wall-clock times shortly before the returned one and ask
    whether any of them does NOT exist in this zone. If so, croniter was
    walking forward out of a non-existent local time and the odd fields are
    expected rather than wrong.
    """
    tz, lib = load_tz(label)
    if tz is None:
        return False
    try:
        fire = datetime.fromisoformat(fire_str)
    except ValueError:
        return False

    naive = fire.replace(tzinfo=None)
    for back in range(1, 181):          # DST gaps are at most a couple of hours
        cand = naive - timedelta(minutes=back)
        if lib == "pytz":
            try:
                tz.localize(cand, is_dst=None)
            except Exception as exc:
                if type(exc).__name__ == "NonExistentTimeError":
                    return True
        else:
            if not datetime_exists(cand.replace(tzinfo=tz)):
                return True
    return False


def main():
    path = sys.argv[1] if len(sys.argv) > 1 else "fuzz/invariant2-findings.txt"
    rows = parse(path)
    print(f"parsed {len(rows)} findings from {path}\n")

    for k, v in Counter(r[0] for r in rows).most_common():
        print(f"  {k:28s} {v}")

    print("\n--- triaging field mismatches ---")
    explained, unexplained = 0, []
    for r in rows:
        if r[0] not in ("F1_FIELDS", "F2_FIELDS_TZ"):
            continue
        _, expr, start, label, meth, fire, why = r
        if explained_by_forward_shift(fire, label):
            explained += 1
        else:
            unexplained.append(r)

    print(f"  explained by documented skip-forward : {explained}")
    print(f"  UNEXPLAINED (candidate bugs)         : {len(unexplained)}")

    if unexplained:
        sig = Counter()
        for r in unexplained:
            _, expr, start, label, meth, fire, why = r
            zone = label.rsplit("/", 1)[0] if label != "naive" else "naive"
            sig[(zone, meth, why.split()[0])] += 1
        print("\n  unexplained grouped by (zone, method, field):")
        for (zone, meth, field), c in sig.most_common(15):
            print(f"    {c:5d}  {zone:24s} {meth:9s} {field}")
        print("\n  samples:")
        for r in unexplained[:6]:
            print("   ", r)

    others = [r for r in rows if r[0] not in ("F1_FIELDS", "F2_FIELDS_TZ")]
    if others:
        print(f"\n--- other findings: {len(others)} ---")
        by_zone = Counter()
        for r in others:
            label = next((x for x in r if isinstance(x, str)
                          and ("/" in x and ":" not in x or x == "naive")), "?")
            by_zone[(r[0], label.rsplit("/", 1)[0])] += 1
        for (kind, zone), c in by_zone.most_common(12):
            print(f"    {c:5d}  {kind:22s} {zone}")


if __name__ == "__main__":
    main()
