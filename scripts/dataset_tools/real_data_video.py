#!/usr/bin/env python3
"""Convert one Puffer day into the video experience use case's five metrics, and write its workload.

Puffer has no city, ISP, CDN or device: the channel stands in for the city and the experiment (an ABR
and congestion-control pair) for the ISP, so a subpopulation is a channel under one scheme.
"""

import argparse
import csv
import glob
import gzip
import json
import os
import re

import data_root
import video_spec

REPO = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
DAY = "2024-01-01T11_2024-01-02T11"
SPEC = os.path.join(REPO, "aqpbm-usecase", "workloads", "uc3_video_real.json")
WINDOW_START_MS = 1704132000000
MINUTE = 60000
CHUNK_SECONDS = 2.002
CDN, DEVICE = "puffer", "all"
METRICS = {"bitrate": {"floor": True}, "buffering": {"non_negative": True}, "play_time": {},
           "startup_time": {"non_negative": True}, "device_id": {"floor": True}}


def raw(root, table):
    whole = os.path.join(root, "puffer", DAY, f"{table}_{DAY}.csv")
    if os.path.exists(whole):
        return [whole]
    parts = glob.glob(os.path.join(root, "puffer", DAY, f"{table}_{DAY}_part*.csv"))
    if not parts:
        data_root.existing(whole, root)
    return sorted(parts, key=lambda p: int(re.search(r"_part(\d+)\.csv$", p).group(1)))


def rows_in(root, table, start_ns, end_ns, keep=None):
    for path in raw(root, table):
        yield from rows_of(path, start_ns, end_ns, keep)


def rows_of(path, start_ns, end_ns, keep):
    with open(path, newline="") as f:
        for line in f:
            if keep is not None and keep not in line:
                continue
            fields = line.rstrip("\n").split(",")
            if not fields[0].isdigit():
                continue
            time_ns = int(fields[0])
            if start_ns <= time_ns < end_ns:
                yield time_ns, fields


def labels(fields):
    return [fields[4], f"expt-{fields[3]}", CDN, DEVICE]


def rung_order(formats):
    def key(name):
        resolution, quality = name.split("-")
        width, height = resolution.split("x")
        return int(width) * int(height), -int(quality)
    return {name: i for i, name in enumerate(sorted(formats, key=key))}


def convert(root, start_ns, end_ns):
    out = {name: [] for name in METRICS}
    sent = [(t, labels(f), f[6]) for t, f in rows_in(root, "video_sent", start_ns, end_ns)]
    rungs = rung_order({fmt for _, _, fmt in sent})
    out["bitrate"] = [(t, group, rungs[fmt]) for t, group, fmt in sent]
    previous, devices = {}, {}
    for t, fields in rows_in(root, "video_acked", start_ns, end_ns):
        group = labels(fields)
        stream = (fields[1], fields[2])
        cumulative = float(fields[7])
        out["buffering"].append((t, group, max(0.0, cumulative - previous.get(stream, cumulative))))
        previous[stream] = cumulative
        out["play_time"].append((t, group, CHUNK_SECONDS))
        out["device_id"].append((t, group, devices.setdefault(fields[1], len(devices))))
    out["startup_time"] = [(t, labels(f), float(f[7])) for t, f in rows_in(root, "client_buffer", start_ns,
                                                                           end_ns, ",startup,")]
    return out, rungs, len(devices)


def write(name, rows, directory):
    path = os.path.join(directory, f"{name}.csv.gz")
    with gzip.open(path, "wt", newline="", compresslevel=1) as f:
        writer = csv.writer(f)
        writer.writerow(["time", "city", "isp", "cdn", "device", name])
        for t, group, value in sorted(rows, key=lambda r: r[0]):
            writer.writerow([t, *group, value])
    return path


def spec(converted, directory, minutes, out_root):
    metrics = []
    for name, flags in METRICS.items():
        rows = converted[name]
        groups = len({tuple(g) for _, g, _ in rows})
        metrics.append({
            "name": name, "sample_interval_ms": max(1, round(minutes * MINUTE * groups / max(1, len(rows)))),
            "planned_series_count": groups, **flags,
            "import": {"time_scale": 1, "origin_ns": WINDOW_START_MS * 10**6, "table": {
                "source": "custom", "dataset": os.path.relpath(os.path.join(directory, f"{name}.csv.gz"), out_root),
                "mode": "grouped", "group_columns": ["city", "isp", "cdn", "device"], "value_column": name,
                "timestamp_column": "time", "timestamp_unit": "nanoseconds"}}})
    return {"use_case": "video_experience", "start_ms": WINDOW_START_MS, "duration_ms": minutes * MINUTE,
            "seed": 31, "metrics": metrics, "queries": video_spec.queries()}


def main(argv=None):
    parser = argparse.ArgumentParser(description="Convert a busy window of one Puffer day into the video "
                                                 "experience metrics and write the real-data workload")
    parser.add_argument("--minutes", type=int, default=120)
    parser.add_argument("--data-root", default=data_root.default(),
                        help=f"where the Puffer day is; default ${data_root.ENV}, else datasets/ in the repository")
    parser.add_argument("--out-root", help="where converted/uc3 is written; default --data-root")
    parser.add_argument("--spec", default=SPEC)
    args = parser.parse_args(argv)
    out_root = args.out_root or args.data_root
    out_dir = os.path.join(out_root, "converted", "uc3")
    os.makedirs(out_dir, exist_ok=True)
    start_ns = WINDOW_START_MS * 1_000_000
    end_ns = start_ns + args.minutes * MINUTE * 1_000_000
    converted, rungs, devices = convert(args.data_root, start_ns, end_ns)
    for name, rows in converted.items():
        path = write(name, rows, out_dir)
        groups = len({tuple(g) for _, g, _ in rows})
        print(f"{name}: {len(rows)} rows in {groups} subpopulations -> {path}")
    print(f"bitrate rungs: {rungs}; sessions: {devices}")
    with open(args.spec, "w") as f:
        json.dump(spec(converted, out_dir, args.minutes, out_root), f, indent=1)
        f.write("\n")


if __name__ == "__main__":
    main()
