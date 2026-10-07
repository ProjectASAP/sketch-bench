#!/usr/bin/env python3
"""Wall time of the sketch-bench runs behind the evaluation tables: ASAP's
one-time profiling cost (ProjectASAP/ASAPQuery#777 section 7).

Usage: scripts/profiling_time.py OUT.json NAME=FILE.jsonl [NAME=FILE.jsonl ...]

Each FILE is a raw approxbench record stream (saturation_accuracy.jsonl,
saturation_cost.jsonl). A run's wall time is the span between its first and
last record timestamp (any `timestamp` or `*_timestamp` field), which
undercounts by the duration of its last measurement and overcounts by any
pause between a stopped run and its resumption. A file without timestamps is
reported as missing rather than estimated.
"""

import json
import re
import sys
from datetime import datetime

STAMP = re.compile(r"\d{4}-\d\d-\d\dT\d\d:\d\d:\d\d")


def stamps(obj, out):
    if isinstance(obj, dict):
        for key, value in obj.items():
            if key == "timestamp" or key.endswith("_timestamp"):
                if isinstance(value, str) and STAMP.match(value):
                    out.append(datetime.fromisoformat(value[:19]))
            else:
                stamps(value, out)
    elif isinstance(obj, list):
        for value in obj:
            stamps(value, out)


def span(path):
    found = []
    records = 0
    with open(path) as f:
        for line in f:
            if line.strip():
                records += 1
                stamps(json.loads(line), found)
    if not found:
        return {"file": path, "records": records, "wall_secs": None,
                "note": "no timestamps: wall time missing"}
    return {"file": path, "records": records, "first": min(found).isoformat() + "Z",
            "last": max(found).isoformat() + "Z",
            "wall_secs": (max(found) - min(found)).total_seconds()}


def main():
    out = sys.argv[1]
    runs = {}
    for arg in sys.argv[2:]:
        name, path = arg.split("=", 1)
        runs[name] = span(path)
    known = [r["wall_secs"] for r in runs.values() if r["wall_secs"] is not None]
    result = {
        "method": "first-to-last record timestamp per run (see scripts/profiling_time.py)",
        "runs": runs,
        "total_wall_secs": sum(known),
        "missing": [n for n, r in runs.items() if r["wall_secs"] is None],
    }
    with open(out, "w") as f:
        json.dump(result, f, indent=1)
        f.write("\n")
    for name, r in runs.items():
        print(f"{name}: {r['wall_secs']} s over {r['records']} records", file=sys.stderr)
    print(f"total {result['total_wall_secs'] / 3600:.2f} h; missing {result['missing']}",
          file=sys.stderr)


if __name__ == "__main__":
    main()
