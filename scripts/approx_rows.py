#!/usr/bin/env python3

import json
import os
import re
import shutil

SCHEMA_VERSION = 1
KINDS = ("query_result", "low_level", "app_state", "fidelity_event", "fidelity_summary")
FIELDS = ("schema_version", "time", "kind", "run_id", "labels", "metric", "value", "unit")
INDEX = "index.json"


def row(time, kind, run_id, labels, metric, value, unit):
    r = {
        "schema_version": SCHEMA_VERSION,
        "time": time,
        "kind": kind,
        "run_id": run_id,
        "labels": dict(labels),
        "metric": metric,
        "value": value,
        "unit": unit,
    }
    check(r)
    return r


def check(r):
    if set(r) != set(FIELDS):
        raise ValueError(f"row fields {sorted(r)} are not {sorted(FIELDS)}")
    if r["schema_version"] != SCHEMA_VERSION:
        raise ValueError(f"schema_version {r['schema_version']!r} is not {SCHEMA_VERSION}")
    if r["kind"] not in KINDS:
        raise ValueError(f"kind {r['kind']!r} is not one of {KINDS}")
    if not isinstance(r["time"], int) or isinstance(r["time"], bool):
        raise ValueError(f"time {r['time']!r} is not integer milliseconds")
    for field in ("run_id", "metric", "unit"):
        if not isinstance(r[field], str) or not r[field]:
            raise ValueError(f"{field} {r[field]!r} is not a non-empty string")
    if not isinstance(r["labels"], dict) or not all(
        isinstance(k, str) and isinstance(v, str) for k, v in r["labels"].items()
    ):
        raise ValueError(f"labels {r['labels']!r} are not string to string")
    if isinstance(r["value"], bool) or not isinstance(r["value"], (int, float, str)):
        raise ValueError(f"value {r['value']!r} is not a number or a string")


def run_dir(out_dir, run_id):
    return os.path.join(out_dir, "runs", run_id)


def write_run(out_dir, run_id, use_case, rows, details=None, split=None):
    if not rows:
        raise ValueError(f"run {run_id} has no rows")
    split = split or {}
    files = {kind: {kind: []} for kind in KINDS}
    for r in rows:
        check(r)
        if r["run_id"] != run_id:
            raise ValueError(f"row of run {r['run_id']} written into run {run_id}")
        name = r["kind"]
        entity = r["labels"].get(split.get(name))
        if entity is not None:
            if not re.fullmatch(r"[\w-][\w.-]*", entity):
                raise ValueError(f"{split[name]} {entity!r} cannot name a file")
            name = os.path.join(r["kind"], entity)
        files[r["kind"]].setdefault(name, []).append(r)
    for kind, by_name in files.items():
        folded = {}
        for name in by_name:
            other = folded.setdefault(name.casefold(), name)
            if other != name:
                raise ValueError(f"{split[kind]} {os.path.basename(other)!r} and {os.path.basename(name)!r} "
                                 "name the same file on a case-insensitive filesystem")
    path = run_dir(out_dir, run_id)
    os.makedirs(path, exist_ok=True)
    for kind, by_name in files.items():
        shutil.rmtree(os.path.join(path, kind), ignore_errors=True)
        if len(by_name) > 1:
            os.makedirs(os.path.join(path, kind))
        for name, file_rows in by_name.items():
            file_rows.sort(key=lambda r: r["time"])
            with open(os.path.join(path, f"{name}.json"), "w") as f:
                json.dump(file_rows, f)
    times = [r["time"] for r in rows]
    entry = {"run_id": run_id, "use_case": use_case, "start": min(times), "end": max(times)}
    if details is not None:
        entry["details"] = details
    runs = [e for e in read_index(out_dir) if e["run_id"] != run_id] + [entry]
    runs.sort(key=lambda e: e["run_id"])
    with open(os.path.join(out_dir, "runs", INDEX), "w") as f:
        json.dump(runs, f, indent=1)


def read_index(out_dir):
    path = os.path.join(out_dir, "runs", INDEX)
    if not os.path.exists(path):
        return []
    with open(path) as f:
        return json.load(f)


def read_run(out_dir, run_id, kind):
    if kind not in KINDS:
        raise ValueError(f"kind {kind!r} is not one of {KINDS}")
    path = run_dir(out_dir, run_id)
    names = [f"{kind}.json"]
    if os.path.isdir(os.path.join(path, kind)):
        names += sorted(os.path.join(kind, name) for name in os.listdir(os.path.join(path, kind))
                        if name.endswith(".json"))
    rows = []
    for name in names:
        with open(os.path.join(path, name)) as f:
            rows += json.load(f)
    for r in rows:
        check(r)
    return sorted(rows, key=lambda r: r["time"])
