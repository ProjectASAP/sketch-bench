#!/usr/bin/env python3
"""Summarize the saturation runs at the synthetic workload's label
cardinalities into the tables in
docs/figures/saturation/synthetic_cardinalities/README.md.

Usage: summarize_synthetic_curves.py GRID_DIR MERGE_DIR
  GRID_DIR/saturation.csv            single-sketch N_sat per point
  MERGE_DIR/saturation_merge_curve.csv  top-k precision per shard count
Prints Markdown: N_sat per (sketch, config) x (theta, K), counts of points
that never saturate, and the top-k merged/single precision ratio at the
largest N of the merge run.
"""

import csv
import math
import sys
from collections import defaultdict


def nsat_table(path):
    rows = list(csv.DictReader(open(path)))
    thetas = sorted({float(r["param"]) for r in rows})
    ks = sorted({int(r["cardinality"]) for r in rows})
    cells = defaultdict(dict)
    for r in rows:
        cells[(r["sketch"], r["config"])][(float(r["param"]), int(r["cardinality"]))] = r["n_sat"]
    head = ["sketch", "config"] + [f"θ={t:g} K={k:.0e}" for t in thetas for k in ks]
    print("| " + " | ".join(head) + " |")
    print("|" + "---|" * len(head))
    for (sketch, config), c in sorted(cells.items()):
        vals = []
        for t in thetas:
            for k in ks:
                v = c.get((t, k), "")
                vals.append("never" if v == "not_saturated" else (f"{float(v):.0e}" if v else ""))
        print(f"| {sketch} | {config} | " + " | ".join(vals) + " |")
    never = defaultdict(int)
    total = defaultdict(int)
    for r in rows:
        key = (r["sketch"], int(r["cardinality"]))
        total[key] += 1
        never[key] += r["n_sat"] == "not_saturated"
    print("\nPoints that never saturate by the largest N, per sketch and K:\n")
    print("| sketch | K | never saturated / points |")
    print("|---|---|---|")
    for key in sorted(total):
        print(f"| {key[0]} | {key[1]:.0e} | {never[key]} / {total[key]} |")


def merge_table(path):
    rows = list(csv.DictReader(open(path)))
    nmax = max(int(r["n"]) for r in rows)
    single = {}
    merged = defaultdict(dict)
    for r in rows:
        if int(r["n"]) != nmax:
            continue
        key = (r["config"], float(r["param"]), int(r["cardinality"]))
        m = int(r["shards"])
        if m == 1:
            single[key] = float(r["seed_mean_error"])
        else:
            merged[key][m] = float(r["seed_mean_error"])
    print(f"\nTop-k precision@k merged / single at N = {nmax:.0e} (1.00 = no loss):\n")
    shards = sorted({m for v in merged.values() for m in v})
    print("| config | θ | K | " + " | ".join(f"m={m}" for m in shards) + " |")
    print("|---|---|---|" + "---|" * len(shards))
    worst = []
    for key in sorted(merged):
        s = single.get(key)
        ratios = [merged[key].get(m, float("nan")) / s if s else float("nan") for m in shards]
        if not any(math.isnan(r) for r in ratios):  # single-sketch precision 0: no ratio
            worst.append((min(ratios), key))
        print(f"| {key[0]} | {key[1]:g} | {key[2]:.0e} | " + " | ".join(f"{x:.2f}" for x in ratios) + " |")
    worst.sort()
    undefined = sum(1 for key in merged if not single.get(key))
    print(f"\n{undefined} points have single-sketch precision 0 (no ratio, shown as nan).")
    print("\nLargest losses:", ", ".join(f"{k[0]} θ={k[1]:g} K={k[2]:.0e}: {r:.2f}" for r, k in worst[:5]))


if __name__ == "__main__":
    nsat_table(f"{sys.argv[1]}/saturation.csv")
    merge_table(f"{sys.argv[2]}/saturation_merge_curve.csv")
