#!/usr/bin/env python3
"""Export the accuracy/cost table for the AutoSketch vs ASAPQuery evaluation.

Plan: ProjectASAP/ASAPQuery#777 (docs/evaluation/autosketch-vs-planner.md,
section 6 "Benchmark input"). One RQE per (dataset, query_id, kind, range) of
ASAPQuery's dataset-analysis skew_summary.csv, worst case over the whole
dataset, rounded to the measured grid exactly as recommend_config.py does.
Every config of every candidate family is listed (no pick), with two lookups:

  autosketch_error  the single-sketch curve at N = max_N. AutoSketch keeps
                    one sketch per query window and never merges, so it has
                    no saturation requirement.
  asap_error_by_m   the error after merging m = 1, 4, 16, 64 shards, at
                    N = max_N. CMS, CountSketch, HLL and DDSketch merge
                    exactly, so every m reads the single-sketch curve. KLL and
                    top-k read the merged curve at m (sketch-bench #131) when
                    one is given, else the single-sketch curve, flagged.
                    ASAP merges smaller-window sketches, so it may only use a
                    value when `saturated` (max_N >= n_sat).

Costs are recommend_config.py's (memory_bytes, insert_ns_per_item,
merge_us_per_fold, query_phase_us). There is no burst variant: the worst case
over the whole dataset replaces AutoSketch's synthetic burst injection.

BOOM rows have no timestamps, window or N in skew_summary.csv; boom_query()
derives them from the 20 equal chunks per series and records the assumptions.
"""

import argparse
import csv
import hashlib
import json
import math
import os
import re
import subprocess
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

from recommend_config import (  # noqa: E402
    DEFAULT_TARGETS, FAMILIES, KIND_FAMILIES, MERGE_INEXACT, cost_of, error_at, grid_point,
    load_merge_curves, load_saturation, nominal_size, num, read_summary)
from study_saturation import load_curves  # noqa: E402

SHARD_COUNTS = [1, 4, 16, 64]
CAPABILITY = {"cms": "freq", "countsketch": "freq", "topk": "topk",
              "kll": "quantile", "dd": "quantile"}


def params_of(config):
    """'rows=3 cols=1024' -> {"rows": 3, "cols": 1024}."""
    out = {}
    for part in config.split():
        k, v = part.split("=")
        out[k] = int(v) if v.isdigit() else float(v)
    return out


def saturation_of(srow, n):
    """(n_sat or None, saturated): saturated when n >= n_sat."""
    if not srow or srow["n_sat"] == "not_saturated":
        return None, False
    n_sat = num(srow["n_sat"])
    return n_sat, n_sat is not None and n >= n_sat


def config_row(q, family, key, cost_key, curves, curves_big, sat, sat_big, merge):
    """One config's lookups for query-range q, or None without a curve."""
    curve = curves.get(key)
    if not curve:
        return None
    n = q["max_N"]
    if n > curve[-1][0] and key in curves_big:
        curve = curves_big[key]
    single, single_flag = error_at(curve, n)
    by_m, m_flags = {}, {}
    for m in SHARD_COUNTS:
        if m == 1 or family not in MERGE_INEXACT:
            by_m[str(m)], m_flags[str(m)] = single, single_flag
            continue
        merged = merge.get(key, {}).get(m)
        if merged:
            by_m[str(m)], m_flags[str(m)] = error_at(merged, n)
        else:
            by_m[str(m)] = single
            m_flags[str(m)] = "; ".join(
                x for x in ["no merge curve; single-sketch curve optimistic", single_flag] if x)
    srow = sat_big.get(key) or sat.get(key)
    n_sat, saturated = saturation_of(srow, n)
    cost = cost_of(sat, cost_key, curves[key][-1][0])
    memory, insert_ns, merge_us, query_us = (None if x == "" else x for x in cost[:4])
    cost_flag = cost[4]
    return {
        "config": key[1], "params": params_of(key[1]),
        "memory_bytes": memory, "insert_ns_per_item": insert_ns,
        "merge_us_per_fold": merge_us, "query_phase_us": query_us,
        "cost_flag": cost_flag,
        "n_sat": n_sat, "not_saturated_in_grid": bool(srow) and srow["n_sat"] == "not_saturated",
        "saturated": saturated,
        "autosketch_error": single, "autosketch_flag": single_flag,
        "asap_error_by_m": by_m, "asap_flag_by_m": m_flags,
    }


BOOM_CHUNKS = 20
STEP_UNIT_SECS = {"S": 1, "T": 60, "H": 3600, "D": 86400}


def boom_query(r):
    """A read_summary-shaped query for one BOOM series row.

    One evaluation is one of the series' 20 equal chunks over all variates:
    N = rows_total / 20, lookback = interval = the chunk's duration, from the
    series' sampling step in its name (ds-2187-H: hourly). A series without an
    alpha fit (too few rows, or every variate light) uses the hardest grid
    alpha for the error and the easiest for memory, flagged by grid_point.
    """
    match = re.search(r"-(\d*)([SDHT])\]$", r["query_id"])
    step = int(match.group(1) or 1) * STEP_UNIT_SECS[match.group(2)]
    rows, variates = num(r["rows_total"]), num(r["K_total"])
    n = rows / BOOM_CHUNKS
    span = round(rows / variates / BOOM_CHUNKS * step)
    alpha_rank, alpha_memory = num(r["worst_alpha_rank"]), num(r["worst_alpha_memory"])
    assumptions = [f"N = rows_total / {BOOM_CHUNKS} chunks",
                   f"lookback = interval = one chunk ({rows / variates / BOOM_CHUNKS:g} "
                   f"steps of {step} s)"]
    if alpha_rank is None:
        alpha_rank, alpha_memory = math.inf, 0.0
        assumptions.append("no alpha fit: hardest grid alpha for error, easiest for memory")
    q = {"dataset": r["dataset"], "query_id": r["query_id"], "kind": r["kind"],
         "range": f"chunk_of_{BOOM_CHUNKS}", "range_s": span, "step_s": span,
         "theta": None, "K": None, "min_N": n, "max_N": n,
         "alpha_rank": alpha_rank, "alpha_memory": alpha_memory,
         "tail_class": r.get("tail_class", ""), "assumptions": assumptions}
    for t, default in DEFAULT_TARGETS.items():
        q[t] = default if num(r.get(t)) is None else num(r.get(t))
    return q


def queries(path):
    """read_summary's queries, plus one per BOOM series (read_summary drops them)."""
    out = read_summary(path)
    with open(path, newline="") as f:
        out += [boom_query(r) for r in csv.DictReader(f) if r["dataset"] == "boom"]
    return out


def rqe_entry(q, curves, curves_big, sat, sat_big, merge):
    """The table entry for one skew_summary query-range."""
    instant = not q["range_s"]
    # An instant query reads one step's samples (max_N counts one step window).
    lookback = q["step_s"] if instant else q["range_s"]
    families = []
    for family in KIND_FAMILIES[q["kind"]]:
        sketch, target_col, higher_better = FAMILIES[family]
        points = [k for k in curves if k[0] == sketch]
        if not points:
            continue
        param, cost_param, card, common = grid_point(q, points)
        rows = []
        for config in sorted({k[1] for k in points}, key=nominal_size):
            key = (sketch, config, points[0][2], float(param), card)
            cost_key = (sketch, config, key[2], float(cost_param), card)
            row = config_row(q, family, key, cost_key, curves, curves_big, sat, sat_big, merge)
            if row:
                rows.append(row)
        if not rows:
            continue
        metric = (sat.get(key) or sat_big.get(key) or {}).get("error_metric", "")
        families.append({
            "family": family, "capability": CAPABILITY[family], "sketch": sketch,
            "error_metric": metric, "higher_is_better": higher_better,
            "target": q[target_col], "grid_param": param, "grid_K": card,
            "flags": [x for x in common if x], "configs": rows,
        })
    return {
        "id": f"{q['dataset']}/{q['query_id']}/{q['kind']}/{q['range']}",
        "dataset": q["dataset"], "query_id": q["query_id"], "kind": q["kind"],
        "range": q["range"], "instant": instant,
        "lookback_secs": lookback, "interval_secs": q["step_s"],
        "min_N": q["min_N"], "max_N": q["max_N"],
        # One sketch per query: group-by values are the sketch's keys, not
        # separate instances (README, "Label sets").
        "label_set": {"groups": 1,
                      "arrival_rate_per_sec": q["max_N"] / lookback if lookback else None,
                      "keys_per_window": q["K"]},
        "assumptions": q.get("assumptions", []) + (
            ["instant: lookback = one step (max_N counts one step window)"] if instant else []),
        "families": families,
    }


def sha256(path):
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for block in iter(lambda: f.read(1 << 20), b""):
            h.update(block)
    return h.hexdigest()


def build(args):
    curves = load_curves(args.curves)
    curves_big = load_curves(args.curves_big) if args.curves_big else {}
    sat = load_saturation(args.saturation)
    sat_big = load_saturation(args.saturation_big)
    merge = {}
    for path in args.merge_curves or []:
        for key, per_m in load_merge_curves(path).items():
            merge.setdefault(key, {}).update(per_m)
    workloads = {}
    for q in queries(args.summary):
        if not q["step_s"]:
            continue
        entry = rqe_entry(q, curves, curves_big, sat, sat_big, merge)
        workloads.setdefault(q["dataset"], []).append(entry)
    ids = [r["id"] for rqes in workloads.values() for r in rqes]
    assert len(ids) == len(set(ids)), "RQE ids must be unique"
    inputs = [p for p in [args.summary, args.curves, args.curves_big, args.saturation,
                          args.saturation_big, *(args.merge_curves or [])] if p]
    return {
        "schema_version": 1,
        "plan": "ProjectASAP/ASAPQuery#777",
        "shard_counts": SHARD_COUNTS,
        "inputs": {os.path.basename(os.path.dirname(p)) + "/" + os.path.basename(p): sha256(p)
                   for p in inputs},
        "sketch_bench_revision": args.revision,
        "workloads": [{"dataset": d, "rqes": r} for d, r in sorted(workloads.items())],
    }


def main(argv=None):
    parser = argparse.ArgumentParser(
        description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--summary", required=True, help="ASAPQuery skew_summary.csv")
    parser.add_argument("--curves", required=True)
    parser.add_argument("--curves-big")
    parser.add_argument("--saturation", required=True)
    parser.add_argument("--saturation-big")
    parser.add_argument("--merge-curves", nargs="*", help="saturation_merge_curve.csv files")
    parser.add_argument("--revision", help="sketch-bench commit of the curves (default: HEAD)")
    parser.add_argument("--out", required=True)
    args = parser.parse_args(argv)
    if args.revision is None:
        args.revision = subprocess.run(["git", "rev-parse", "HEAD"], capture_output=True,
                                       text=True, check=False).stdout.strip()
    table = build(args)
    os.makedirs(os.path.dirname(args.out) or ".", exist_ok=True)
    with open(args.out, "w") as f:
        json.dump(table, f, indent=1, sort_keys=True)
        f.write("\n")
    n = sum(len(w["rqes"]) for w in table["workloads"])
    print(f"Done. {n} RQEs in {args.out}", file=sys.stderr)
    return 0


if __name__ == "__main__":
    sys.exit(main())
