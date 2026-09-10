#!/usr/bin/env python3
import csv
import json
import math
import pathlib
import statistics
import sys

import matplotlib.pyplot as plt

raw_path = pathlib.Path(sys.argv[1])
out_dir = pathlib.Path(sys.argv[2])
records = [json.loads(line) for line in raw_path.read_text().splitlines() if line.strip()]

points = []
for record in records:
    bench = record.get("bench") or {}
    if bench.get("metric") != "throughput" or bench.get("operation") not in {"insert", "merge", "query"}:
        continue
    desc = record["workload"]["synthetic"]["description"]
    distribution = desc["column_spec"][0]["distribution"]
    params = record["sketch_config"]["params"]
    user_ms = bench["cpu_time_ms"]["user_ms"]["mean"]
    system_ms = bench["cpu_time_ms"]["sys_ms"]["mean"]
    cpu_ms = user_ms + system_ms
    operation = bench["operation"]
    if operation == "insert":
        measured_ns = cpu_ms * 1e6 / desc["row_num"]
    elif operation == "merge":
        measured_ns = cpu_ms * 1e6
    else:
        wall_ms = bench["wall_time_ms"]["mean"]
        measured_ns = (cpu_ms / wall_ms) * 1e9 / bench["throughput_items_per_sec"]["mean"]
    points.append({
        "distribution": distribution["kind"],
        "zipf_s": distribution.get("skewness", ""),
        "burst": bool(record["workload"]["synthetic"].get("burst")),
        "rows": params["rows"],
        "cols": params["cols"],
        "operation": operation,
        "measured_ns": measured_ns,
    })

# Calibrate one constant per operation at the smallest steady uniform point.
# The analytical model then sees only asymptotic work, not data shape.
for operation in {p["operation"] for p in points}:
    candidates = [p for p in points if p["operation"] == operation and p["distribution"] == "uniform" and not p["burst"]]
    anchor = min(candidates, key=lambda p: (p["rows"] * p["cols"], p["rows"]))
    anchor_work = anchor["rows"] if operation != "merge" else anchor["rows"] * anchor["cols"]
    coefficient = anchor["measured_ns"] / anchor_work
    for point in points:
        if point["operation"] != operation:
            continue
        work = point["rows"] if operation != "merge" else point["rows"] * point["cols"]
        point["analytical_ns"] = coefficient * work
        point["absolute_percentage_error"] = abs(point["analytical_ns"] - point["measured_ns"]) / point["measured_ns"] * 100

csv_path = out_dir / "figure1-points.csv"
with csv_path.open("w", newline="") as handle:
    writer = csv.DictWriter(handle, fieldnames=list(points[0]))
    writer.writeheader()
    writer.writerows(points)

fig, axes = plt.subplots(1, 3, figsize=(12, 4))
for axis, operation in zip(axes, ("insert", "merge", "query")):
    subset = [p for p in points if p["operation"] == operation]
    x = [p["analytical_ns"] for p in subset]
    y = [p["measured_ns"] for p in subset]
    axis.scatter(x, y, c=["#d95f02" if p["burst"] else "#1b9e77" for p in subset], alpha=.8)
    lo, hi = min(x + y), max(x + y)
    axis.plot([lo, hi], [lo, hi], "k--", linewidth=1)
    if operation == "merge":
        axis.set_xscale("log"); axis.set_yscale("log")
    axis.set_title(operation); axis.set_xlabel("analytical ns/op")
    axis.set_ylabel("measured ns/op")
fig.tight_layout()
fig.savefig(out_dir / "figure1-analytical-vs-empirical.png", dpi=180)

mape = statistics.median(p["absolute_percentage_error"] for p in points)
def ranks(values):
    order = sorted(range(len(values)), key=values.__getitem__)
    result = [0.0] * len(values)
    for rank, index in enumerate(order, 1):
        result[index] = rank
    return result

rx = ranks([p["analytical_ns"] for p in points])
ry = ranks([p["measured_ns"] for p in points])
mean_x, mean_y = statistics.mean(rx), statistics.mean(ry)
numerator = sum((x - mean_x) * (y - mean_y) for x, y in zip(rx, ry))
denominator = math.sqrt(sum((x - mean_x) ** 2 for x in rx) * sum((y - mean_y) ** 2 for y in ry))
summary = {
    "host": pathlib.Path("/etc/hostname").read_text().strip(),
    "records": len(records),
    "plotted_points": len(points),
    "median_absolute_percentage_error": mape,
    "spearman_rank_correlation": numerator / denominator,
    "commands": "scripts/run_erp_v1_figure1.sh",
}
(out_dir / "summary.json").write_text(json.dumps(summary, indent=2) + "\n")
