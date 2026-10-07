#!/usr/bin/env python3
"""Turn worst-case data parameters per query into a recommended sketch config.

Inputs:
  --summary   ASAPQuery dataset-analysis results/skew_summary.csv: one row per
              (dataset, query_id, kind, weight, range) with the worst case a
              sketch sees (worst_theta_cms, worst_K, min_N, max_N,
              worst_alpha_rank, worst_alpha_memory), tail_class and target_*.
              The older schema (window_len_s, lower/upper, K_win_max,
              rows_win_min/max, no targets) is also read; see `read_summary`.
  --curves    saturation_curve.csv of the config grid (error per N, seed mean).
  --curves-big  optional larger-N curves (the 1e9 subset), used for a point
              when N exceeds the grid's largest N and the point is there.
  --saturation / --saturation-big  saturation.csv files for n_sat (the big
              one wins when it has the point) and, once the cost phase has
              run, insert/merge/query CPU and memory_bytes.
  --merge-curves  optional saturation_merge_curve.csv (branch merged-accuracy).

Candidate families: key queries -> CMS, CountSketch and the CMS-heap top-k;
value queries -> KLL and DDSketch. No query in the query sets is a
count-distinct, so HLL is never a candidate.

Worst case per (query, range), rounded to the measured grid toward the harder
side, so the estimate is conservative:
  - theta (key rows: the weight a per-key counter sees, count or value) is
    rounded DOWN to the grid (flatter is harder for CMS/CountSketch/top-k);
  - K (distinct keys per evaluation, K_win_max) is rounded UP;
  - alpha for the error is worst_alpha_rank (steepest tail, hardest for rank
    error) rounded UP; alpha for DDSketch cost is worst_alpha_memory
    (heaviest tail, widest value range) rounded DOWN.
  A worst case outside the grid is clamped to the grid edge and flagged.

The error is read at N = max_N (items in the largest evaluation), linear in
log N between checkpoints; below the first checkpoint the first error is
used, above the last the last one (flagged "extrapolated"). max_N is the right
N for a merged window because a merged CMS/CountSketch/DDSketch equals one
sketch over the union. For top-k and KLL a merge is not exact: with
--merge-curves the error is read from the merged curve at the shard count
m = range_s / step_s (nearest measured m in log scale; the old schema has no
step, so its finest window length is used), otherwise the row is flagged
"single-sketch curve optimistic".

Flags: "not saturated" when the point never saturated in the grid, "not
saturated at min_N" when min_N < n_sat, "light tail" for value rows whose
tail_class is light (alpha is not meaningful there), "target not met" when no
config reaches the target (the best config is then reported).

Recommendation: per (query, range, family), the smallest config whose
estimate meets the target (error <= target; precision@k >= target). Smallest
is by memory_bytes when every config of the family has it, else by the
nominal size (rows*cols, k, 1/alpha). Cost columns: insert_ns_per_item =
insert_cpu_secs / N_cost, where N_cost is the point's last curve N (the cost
run's --size); merge_us_per_fold = merge_cpu_secs / (MERGE_SHARDS - 1)
(folding MERGE_SHARDS shard sketches into one); query_phase_us = query_cpu_secs: the
benchmark's whole query phase (every probe: keys seen, 101 quantiles, or a
repeated estimate), not one query.
They are empty until the cost phase has filled saturation.csv. rows=5 configs
without cost (--cost-rows 3) take 5/3 of the rows=3 cost with the same cols
(flagged "cost scaled from rows=3").
"""

import argparse
import bisect
import csv
import math
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

from study_saturation import MERGE_SHARDS, load_curves, point_key  # noqa: E402

# family -> (sketch variant, target column, higher is better)
FAMILIES = {
    "cms": ("cms-fastpath-vector2d", "target_are_top100", False),
    "countsketch": ("countsketch-fastpath-vector2d", "target_are_top100", False),
    "topk": ("cms-heap-topk-fastpath-vector2d", "target_precision_at_k", True),
    "kll": ("kll-percall", "target_rank_err", False),
    "dd": ("dd", "target_relative_value_err", False),
}
KIND_FAMILIES = {"keys": ["cms", "countsketch", "topk"], "values": ["kll", "dd"]}
# Families whose merged error differs from one sketch over the union.
MERGE_INEXACT = {"topk", "kll"}
# Same defaults as ASAPQuery dataset-analysis fit_skew.py DEFAULT_TARGETS.
DEFAULT_TARGETS = {
    "target_are_top100": 0.05,
    "target_precision_at_k": 0.95,
    "target_hll_rel_err": 0.02,
    "target_rank_err": 0.01,
    # DDSketch is evaluated by relative value error; skew_summary has no
    # first-class target for it yet, so use the agreed 1% default.
    "target_relative_value_err": 0.01,
}

OUTPUT_COLUMNS = [
    "dataset", "query_id", "range", "family", "config", "est_error", "target",
    "meets_target", "n_sat", "flags", "memory_bytes", "insert_ns_per_item",
    "merge_us_per_fold", "query_phase_us", "grid_param", "grid_K", "N", "shards",
]


def num(value):
    """float(value), or None for an empty / non-numeric cell."""
    try:
        x = float(value)
    except (TypeError, ValueError):
        return None
    return None if math.isnan(x) else x


def read_summary(path):
    """One dict per (dataset, query_id, range, kind) worst case.

    New schema: key rows without worst_theta_cms (the weight a counter sketch
    does not see) are dropped. Old schema: range = window_len_s, theta =
    `lower` of the weight a counter sees (value for `sum by`, else count),
    K = K_win_max, N = rows_win_min/max, alpha_rank = upper, alpha_memory =
    lower, step = the query's finest window length, default targets.
    Rows without N or without their worst parameter (BOOM) are dropped.
    """
    with open(path, newline="") as f:
        rows = list(csv.DictReader(f))
    new = rows and "worst_theta_cms" in rows[0]
    finest = {}
    if not new:
        for r in rows:
            key = (r["dataset"], r["query_id"])
            s = num(r["window_len_s"])
            if s is not None:
                finest[key] = min(finest.get(key, s), s)
    out = []
    for r in rows:
        kind = r["kind"]
        if new:
            q = {"range": r["range"], "range_s": num(r["range_s"]), "step_s": num(r["step_s"]),
                 "theta": num(r["worst_theta_cms"]), "K": num(r["worst_K"]),
                 "min_N": num(r["min_N"]), "max_N": num(r["max_N"]),
                 "alpha_rank": num(r["worst_alpha_rank"]),
                 "alpha_memory": num(r["worst_alpha_memory"])}
            for t, default in DEFAULT_TARGETS.items():
                q[t] = default if num(r.get(t)) is None else num(r.get(t))
        else:
            counter_weight = "value" if r["promql"].startswith("sum by") else "count"
            if kind == "keys" and r["weight"] != counter_weight:
                continue
            q = {"range": r["window_len_s"], "range_s": num(r["window_len_s"]),
                 "step_s": finest.get((r["dataset"], r["query_id"])),
                 "theta": num(r["lower"]), "K": num(r["K_win_max"]),
                 "min_N": num(r["rows_win_min"]), "max_N": num(r["rows_win_max"]),
                 "alpha_rank": num(r["upper"]), "alpha_memory": num(r["lower"]),
                 **DEFAULT_TARGETS}
        q.update(dataset=r["dataset"], query_id=r["query_id"], kind=kind,
                 tail_class=r.get("tail_class", ""))
        needed = ["theta", "K"] if kind == "keys" else ["alpha_rank"]
        if q["max_N"] is None or any(q[k] is None for k in needed):
            continue
        out.append(q)
    return out


def round_down(x, grid):
    """(largest grid value <= x, flag); clamps to the smallest value."""
    grid = sorted(grid)
    i = bisect.bisect_right(grid, x + 1e-12) - 1
    if i < 0:
        return grid[0], f"{x:g} below grid, used {grid[0]:g}"
    return grid[i], ""


def round_up(x, grid):
    """(smallest grid value >= x, flag); clamps to the largest value."""
    grid = sorted(grid)
    i = bisect.bisect_left(grid, x - 1e-12)
    if i == len(grid):
        return grid[-1], f"{x:g} beyond grid, used {grid[-1]:g}"
    return grid[i], ""


def error_at(curve, n):
    """(error at n, flag) from a sorted [(n, error, se)] curve, linear in log n."""
    ns = [c[0] for c in curve]
    if n <= ns[0]:
        return curve[0][1], ("N below grid" if n < ns[0] else "")
    if n >= ns[-1]:
        return curve[-1][1], (f"extrapolated beyond N={ns[-1]:g}" if n > ns[-1] else "")
    i = bisect.bisect_right(ns, n)
    (n0, e0, _), (n1, e1, _) = curve[i - 1], curve[i]
    w = (math.log(n) - math.log(n0)) / (math.log(n1) - math.log(n0))
    return e0 + w * (e1 - e0), ""


def nominal_size(config):
    """Size order when memory_bytes is missing: rows*cols, k, 1/alpha."""
    kv = dict(part.split("=") for part in config.split())
    if "cols" in kv:
        return int(kv["rows"]) * int(kv["cols"])
    if "k" in kv:
        return int(kv["k"])
    return 1.0 / float(kv["alpha"])


def load_saturation(path):
    """{point key: saturation.csv row}; {} when path is None or missing."""
    if not path or not os.path.exists(path):
        return {}
    with open(path, newline="") as f:
        return {point_key(r["sketch"], r["config"], r["dist"], r["param"], r["cardinality"]): r
                for r in csv.DictReader(f)}


def load_merge_curves(path):
    """{point key: {shards: sorted curve}} from a saturation_merge_curve.csv."""
    curves = {}
    if not path:
        return curves
    with open(path, newline="") as f:
        for r in csv.DictReader(f):
            key = point_key(r["sketch"], r["config"], r["dist"], r["param"], r["cardinality"])
            curves.setdefault(key, {}).setdefault(int(r["shards"]), []).append(
                (int(r["n"]), float(r["seed_mean_error"]), float(r["seed_se"])))
    return {k: {m: sorted(c) for m, c in per_m.items()} for k, per_m in curves.items()}


def cost_of(sat, key, n_cost):
    """(memory_bytes, insert_ns_per_item, merge_us_per_fold, query_phase_us, flag),
    '' where the cost phase has not filled saturation.csv; a rows=5 config
    without cost takes 5/3 of the rows=3 one with the same cols."""
    row, scale, flag = sat.get(key), 1.0, ""
    sketch, config = key[0], key[1]
    if (row is None or row["memory_bytes"] == "") and config.startswith("rows=5 "):
        row3 = sat.get((sketch, config.replace("rows=5 ", "rows=3 "), *key[2:]))
        if row3 is not None and row3["memory_bytes"] != "":
            row, scale, flag = row3, 5.0 / 3.0, "cost scaled from rows=3"
    if row is None or row["memory_bytes"] == "":
        return "", "", "", "", ""

    def scaled(col, factor):
        x = num(row[col])
        return "" if x is None else x * scale * factor

    return (scaled("memory_bytes", 1.0), scaled("insert_cpu_secs", 1e9 / n_cost),
            scaled("merge_cpu_secs", 1e6 / (MERGE_SHARDS - 1)),
            scaled("query_cpu_secs", 1e6), flag)


def grid_point(q, points):
    """(param, cost_param, card, flags): q's worst case rounded to the grid of
    one sketch's measured points, toward the harder side."""
    common = []
    if q["kind"] == "keys":
        param, f1 = round_down(q["theta"], {k[3] for k in points})
        card, f2 = round_up(q["K"], {int(k[4]) for k in points})
        cost_param, card = param, str(card)
        common += [f"theta {f1}" if f1 else "", f"K {f2}" if f2 else ""]
    else:
        param, f1 = round_up(q["alpha_rank"], {k[3] for k in points})
        cost_param = q["alpha_memory"] if q["alpha_memory"] is not None else q["alpha_rank"]
        cost_param, _ = round_down(cost_param, {k[3] for k in points})
        card = ""
        common += [f"alpha {f1}" if f1 else ""]
        if q["tail_class"] == "light":
            common.append("light tail")
    return param, cost_param, card, common


def recommend(q, curves, curves_big, sat, sat_big, merge):
    """One output row per candidate family of query-range q."""
    out = []
    for family in KIND_FAMILIES[q["kind"]]:
        sketch, target_col, higher_better = FAMILIES[family]
        target = q[target_col]
        points = [k for k in curves if k[0] == sketch]
        if not points:
            continue
        configs = sorted({k[1] for k in points}, key=nominal_size)
        param, cost_param, card, common = grid_point(q, points)
        n = q["max_N"]
        shards = 1
        if q["range_s"] and q["step_s"]:
            shards = max(1, round(q["range_s"] / q["step_s"]))
        candidates = []
        for config in configs:
            key = (sketch, config, points[0][2], float(param), card)
            curve = curves.get(key)
            if not curve:
                continue
            flags = list(common)
            if n > curve[-1][0] and key in curves_big:
                curve = curves_big[key]
            if family in MERGE_INEXACT and shards > 1:
                per_m = merge.get(key)
                if per_m:
                    m = min(per_m, key=lambda m: abs(math.log(m / shards)))
                    curve = per_m[m]
                    flags.append(f"merged curve at m={m}")
                elif merge:
                    flags.append("no merge curve for point; single-sketch curve optimistic")
                else:
                    flags.append("single-sketch curve optimistic")
            est, f = error_at(curve, n)
            flags.append(f)
            srow = sat_big.get(key) or sat.get(key)
            n_sat = srow["n_sat"] if srow else ""
            if n_sat == "not_saturated":
                flags.append("not saturated")
            elif num(n_sat) is not None and q["min_N"] is not None and q["min_N"] < num(n_sat):
                flags.append("not saturated at min_N")
            n_cost = curves[key][-1][0]
            cost_key = (sketch, config, key[2], float(cost_param), card)
            *cost, cflag = cost_of(sat, cost_key, n_cost)
            flags.append(cflag)
            meets = est >= target if higher_better else est <= target
            candidates.append({
                "dataset": q["dataset"], "query_id": q["query_id"], "range": q["range"],
                "family": family, "config": config, "est_error": est, "target": target,
                "meets_target": meets, "n_sat": n_sat,
                "flags": "; ".join(x for x in flags if x),
                "memory_bytes": cost[0], "insert_ns_per_item": cost[1],
                "merge_us_per_fold": cost[2], "query_phase_us": cost[3],
                "grid_param": param, "grid_K": card, "N": int(n), "shards": shards,
            })
        if not candidates:
            continue
        if all(c["memory_bytes"] != "" for c in candidates):
            candidates.sort(key=lambda c: c["memory_bytes"])
        meeting = [c for c in candidates if c["meets_target"]]
        if meeting:
            best = meeting[0]
        else:
            sign = -1 if higher_better else 1
            best = min(candidates, key=lambda c: sign * c["est_error"])
            best["flags"] = "; ".join(x for x in ["target not met", best["flags"]] if x)
        out.append(best)
    return out


def main(argv=None):
    parser = argparse.ArgumentParser(
        description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument(
        "--summary",
        default="/mydata/ASAPQuery/asap-tools/dataset-analysis/results/skew_summary.csv")
    parser.add_argument("--curves", default="out_grid_1e7/saturation_curve.csv")
    parser.add_argument("--curves-big", default="out_1e9/saturation_curve.csv")
    parser.add_argument("--saturation", default="out_grid_1e7/saturation.csv")
    parser.add_argument("--saturation-big", default="out_1e9/saturation.csv")
    parser.add_argument("--merge-curves", help="saturation_merge_curve.csv (optional)")
    parser.add_argument("--out", default="out/recommendations.csv")
    args = parser.parse_args(argv)

    curves = load_curves(args.curves)
    curves_big = load_curves(args.curves_big) if os.path.exists(args.curves_big) else {}
    sat = load_saturation(args.saturation)
    sat_big = load_saturation(args.saturation_big)
    merge = load_merge_curves(args.merge_curves)
    rows = []
    for q in read_summary(args.summary):
        rows.extend(recommend(q, curves, curves_big, sat, sat_big, merge))
    os.makedirs(os.path.dirname(args.out) or ".", exist_ok=True)
    with open(args.out, "w", newline="") as f:
        writer = csv.DictWriter(f, fieldnames=OUTPUT_COLUMNS)
        writer.writeheader()
        writer.writerows(rows)
    print(f"Done. {len(rows)} rows in {args.out}", file=sys.stderr)
    return 0


if __name__ == "__main__":
    sys.exit(main())
