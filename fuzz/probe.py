"""Probe: run croniter operations and emit a normalized JSON result per input.

Run under two different interpreters -- one with the original Python croniter
installed, one with the Rust port installed -- and diff the output streams.
The probe never hides a failure: exceptions are *recorded* (type name) so that
error behaviour is compared too, not skipped.

Datetimes are normalized to (naive reading, UTC instant, UTC offset). All three
must agree. That triple is deliberate: during a DST fold the UTC instant can be
correct while the offset is wrong, which is exactly the bug this port hit in
Phase 4 -- comparing instants alone would have missed it.

Usage:  echo '<json-task>' | python fuzz/probe.py
Each stdin line is a JSON object; each stdout line is a JSON object.
"""
import json
import sys
from datetime import datetime, timezone

from croniter import croniter, croniter_range


def _tz(name, lib):
    """Build a tzinfo the way a user of that library would."""
    if not name:
        return None
    if lib == "pytz":
        import pytz

        return pytz.timezone(name)
    import zoneinfo

    return zoneinfo.ZoneInfo(name)


def _localize(naive, tzobj, lib):
    if tzobj is None:
        return naive
    if lib == "pytz":
        return tzobj.localize(naive)
    return naive.replace(tzinfo=tzobj)


def _norm(obj):
    """Normalize values so Python and Rust results are comparable."""
    if isinstance(obj, bool) or obj is None:
        return obj
    if isinstance(obj, int):
        return obj
    if isinstance(obj, float):
        return round(obj, 6)
    if isinstance(obj, str):
        return obj
    if isinstance(obj, datetime):
        naive = obj.replace(tzinfo=None).isoformat()
        if obj.tzinfo is not None:
            return [
                "dt",
                naive,
                obj.astimezone(timezone.utc).isoformat(),
                str(obj.utcoffset()),
            ]
        return ["dt", naive, None, None]
    if isinstance(obj, (list, tuple)):
        return [_norm(x) for x in obj]
    if isinstance(obj, set):
        return sorted(str(_norm(x)) for x in obj)
    if isinstance(obj, dict):
        return {str(_norm(k)): _norm(v) for k, v in sorted(obj.items(), key=lambda kv: str(kv[0]))}
    return repr(obj)


def _run(fn):
    """Return ['ok', value] or ['exc', ExceptionTypeName]."""
    try:
        return ["ok", _norm(fn())]
    except Exception as exc:  # noqa: BLE001 - recording, not swallowing
        return ["exc", type(exc).__name__]


def handle(task):
    op = task["op"]
    expr = task["expr"]
    hash_id = task.get("hash_id")
    hash_id = hash_id.encode() if hash_id else None
    day_or = task.get("day_or", True)
    sab = task.get("second_at_beginning", False)

    if op == "expand":
        return _run(lambda: croniter.expand(expr, hash_id=hash_id, second_at_beginning=sab))

    if op == "is_valid":
        return _run(lambda: croniter.is_valid(expr, hash_id=hash_id, second_at_beginning=sab))

    if op in ("next", "prev"):
        tzobj = _tz(task.get("tz"), task.get("tzlib"))
        start = _localize(datetime.fromisoformat(task["start"]), tzobj, task.get("tzlib"))
        n = task.get("n", 5)

        def go():
            it = croniter(
                expr,
                start,
                ret_type=datetime,
                day_or=day_or,
                second_at_beginning=sab,
                hash_id=hash_id,
                implement_cron_bug=task.get("implement_cron_bug", False),
                expand_from_start_time=task.get("expand_from_start_time", False),
            )
            step = it.get_next if op == "next" else it.get_prev
            return [step() for _ in range(n)]

        return _run(go)

    if op == "match":
        tzobj = _tz(task.get("tz"), task.get("tzlib"))
        when = _localize(datetime.fromisoformat(task["when"]), tzobj, task.get("tzlib"))
        return _run(lambda: croniter.match(expr, when, day_or=day_or, second_at_beginning=sab))

    if op == "range":
        tzobj = _tz(task.get("tz"), task.get("tzlib"))
        lib = task.get("tzlib")
        start = _localize(datetime.fromisoformat(task["start"]), tzobj, lib)
        stop = _localize(datetime.fromisoformat(task["stop"]), tzobj, lib)
        limit = task.get("limit", 40)

        def go():
            out = []
            for i, dt in enumerate(
                croniter_range(
                    start, stop, expr, day_or=day_or,
                    exclude_ends=task.get("exclude_ends", False),
                    second_at_beginning=sab,
                )
            ):
                if i >= limit:
                    break
                out.append(dt)
            return out

        return _run(go)

    raise SystemExit(f"unknown op {op!r}")


def main():
    for line in sys.stdin:
        line = line.strip()
        if not line:
            continue
        task = json.loads(line)
        try:
            result = handle(task)
        except SystemExit:
            raise
        except Exception as exc:  # harness-level failure, still reported
            result = ["harness-error", f"{type(exc).__name__}: {exc}"]
        sys.stdout.write(json.dumps({"task": task, "result": result}) + "\n")
    sys.stdout.flush()


if __name__ == "__main__":
    main()
