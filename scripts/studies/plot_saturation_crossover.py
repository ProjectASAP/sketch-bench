"""Exact computation vs sketch: memory and CPU as N grows, with the crossover N*.

Usage: plot_saturation_crossover.py COST_DIR OUT.png  (needs matplotlib)
COST_DIR holds exact_cost.jsonl and saturation_cost.jsonl from `--phase cost`.
"""
import json
import os
import sys

import matplotlib
matplotlib.use("Agg")
import matplotlib.pyplot as plt  # noqa: E402

cost_dir, out = sys.argv[1], sys.argv[2]
INK, INK2, MUTED, SURF = "#0b0b0b", "#52514e", "#898781", "#fcfcfb"
EXACT = {1000: "#2a78d6", 100000: "#eb6834", 10000000: "#1baf7a", None: "#2a78d6"}
SKETCH_GRAYS = ["#c3c2b7", "#898781", "#52514e", "#0b0b0b"]
plt.rcParams.update({
    "font.size": 9, "axes.edgecolor": MUTED, "axes.labelcolor": INK2,
    "xtick.color": MUTED, "ytick.color": MUTED, "axes.titlecolor": INK,
    "axes.spines.top": False, "axes.spines.right": False,
    "axes.grid": True, "grid.color": "#e8e7e3", "grid.linewidth": 0.6,
    "figure.facecolor": SURF, "axes.facecolor": SURF, "legend.frameon": False,
})


def dist_of(rec):
    d = rec["workload"]["synthetic"]["description"]["column_spec"][0]["distribution"]
    n = rec["workload"]["synthetic"]["description"]["row_num"]
    if d["kind"] == "zipf":
        return ("zipf", d["skewness"], d["population_size"]), n
    return ("pareto", d.get("alpha"), None), n


def cpu(rec, op):
    c = rec.get(f"{op}_cpu_time_ms")
    return (c["user_ms"]["mean"] + c["sys_ms"]["mean"]) / 1000.0 if c else 0.0


def one_query(rec):
    """CPU of one query (the query phase repeats it over its probes)."""
    w, t = rec.get("query_wall_time_ms"), rec.get("query_throughput_items_per_sec")
    ops = t["mean"] * w["mean"] / 1000.0 if (w and t) else 0
    return cpu(rec, "query") / ops if ops else 0.0


# Exact: records come in pairs (insert+query run, then prepare run).
exact = {}
lines = [json.loads(x) for x in open(os.path.join(cost_dir, "exact_cost.jsonl"))]
for timed, prepared in zip(lines[0::2], lines[1::2]):
    dist, n = dist_of(timed)
    exact.setdefault((timed["sketch"], dist), []).append(
        (n, prepared["memory_bytes"], cpu(timed, "insert") + one_query(timed)
         + cpu(prepared, "prepare")))

# Sketch: one cost run at N_final per point; insert CPU scales with N.
sketch = {}
for line in open(os.path.join(cost_dir, "saturation_cost.jsonl")):
    r = json.loads(line)
    dist, n = dist_of(r)
    cfg = " ".join(f"{k}={v}" for k, v in sorted(r["sketch_config"]["params"].items()))
    sketch[(r["sketch"], cfg, dist)] = (n, r.get("memory_bytes"), cpu(r, "insert"), one_query(r))

PANELS = [
    ("CMS (frequency)", "cms", "cms-fastpath-vector2d", ("zipf", 1.0)),
    ("CountSketch (frequency)", "cms", "countsketch-fastpath-vector2d", ("zipf", 1.0)),
    ("HLL (cardinality)", "hll", "hll", ("zipf", 1.0)),
    ("KLL (quantile)", "kll-cdf", "kll-percall", ("pareto", 1.5)),
    ("DDSketch (quantile)", "kll-cdf", "dd", ("pareto", 1.5)),
    ("CMS-heap top-k", "cms", "cms-heap-topk-fastpath-vector2d", ("zipf", 1.0)),
]
fig, axes = plt.subplots(2, len(PANELS), figsize=(4.6 * len(PANELS), 8.2), constrained_layout=True)
for col, (title, exact_kind, sk, (kind, param)) in enumerate(PANELS):
    for row, what in enumerate(("memory (bytes)", "CPU (s): ingest N items + answer one query")):
        ax = axes[row, col]
        for (ek, dist), pts in sorted(exact.items(), key=lambda kv: str(kv[0])):
            if ek != exact_kind or dist[0] != kind or dist[1] != param:
                continue
            pts.sort()
            ns = [p[0] for p in pts]
            ys = [p[1] if row == 0 else p[2] for p in pts]
            lab = "exact" + ("" if dist[2] is None else f", K={dist[2]:.0e}".replace("e+0", "e"))
            ax.plot(ns, ys, "-o", color=EXACT.get(dist[2], "#2a78d6"), lw=2, ms=4, label=lab)
        cfgs = sorted({c for (s, c, d) in sketch if s == sk and d[0] == kind and d[1] == param},
                      key=lambda c: [int(x) if x.isdigit() else x for x in
                                     c.replace("=", " ").split()])
        for i, c in enumerate(cfgs):
            ks = [d for (s, cc, d) in sketch if s == sk and cc == c and d[0] == kind and d[1] == param]
            n_final, mem, ins, qry = sketch[(sk, c, ks[-1])]
            xs = [1e3, n_final]
            ys = [mem, mem] if row == 0 else [ins * x / n_final + qry for x in xs]
            ax.plot(xs, ys, "--", color=SKETCH_GRAYS[i % 4], lw=1.6, label=f"sketch {c}")
        ax.set_xscale("log")
        ax.set_yscale("log")
        ax.set_xlabel("items N")
        if col == 0:
            ax.set_ylabel(what)
        if row == 0:
            ax.set_title(f"{title}\n{kind} {param}", loc="left", fontsize=10)
            ax.legend(fontsize=7, labelcolor=INK2)
fig.suptitle("Exact computation (polars, solid) vs sketch (dashed): the gap is the saving; "
             "N* = first N where exact ≥ 10× / 100× sketch. Serial on an idle machine.",
             x=0.01, ha="left", color=INK, fontsize=11)
fig.savefig(out, dpi=110)
print(out)
