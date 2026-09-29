"""CPU per operation and memory vs distribution parameter, from serial cost JSONL.

Usage: plot_cost.py out.png cost.jsonl [override.jsonl ...]
Later files override earlier records for the same (sketch, distribution).
"""
import json
import sys

import matplotlib
matplotlib.use("Agg")
import matplotlib.pyplot as plt  # noqa: E402

INK, INK2, MUTED, SURF = "#0b0b0b", "#52514e", "#898781", "#fcfcfb"
CAT = {1000: "#2a78d6", 100000: "#eb6834", 10000000: "#1baf7a"}
KLAB = {1000: "K=1K", 100000: "K=100K", 10000000: "K=10M"}
SKETCHES = [
    ("cms-fastpath-vector2d", "CMS (rows=3 cols=1024)"),
    ("countsketch-fastpath-vector2d", "CountSketch (rows=3 cols=1024)"),
    ("cms-heap-topk-fastpath-vector2d", "CMS-heap top-k (rows=3 cols=1024)"),
    ("hll", "HLL (lg_k=12)"),
    ("kll-percall", "KLL (k=200)"),
    ("dd", "DDSketch (alpha=0.01)"),
]
plt.rcParams.update({
    "font.size": 9, "axes.edgecolor": MUTED, "axes.labelcolor": INK2,
    "xtick.color": MUTED, "ytick.color": MUTED, "axes.titlecolor": INK,
    "axes.spines.top": False, "axes.spines.right": False,
    "axes.grid": True, "grid.color": "#e8e7e3", "grid.linewidth": 0.6,
    "figure.facecolor": SURF, "axes.facecolor": SURF, "legend.frameon": False,
})


def per_op(rec, op, rate_key):
    """CPU seconds per operation: phase CPU / (ops per second * phase wall seconds)."""
    cpu, wall, rate = rec.get(f"{op}_cpu_time_ms"), rec.get(f"{op}_wall_time_ms"), rec.get(rate_key)
    if not (cpu and wall and rate):
        return None
    ops = rate["mean"] * wall["mean"] / 1000.0
    return (cpu["user_ms"]["mean"] + cpu["sys_ms"]["mean"]) / 1000.0 / ops


recs = {}
for path in sys.argv[2:]:
    for line in open(path):
        r = json.loads(line)
        d = r["workload"]["synthetic"]["description"]["column_spec"][0]["distribution"]
        if d["kind"] == "zipf":
            key = (r["sketch"], d["skewness"], d["population_size"])
        else:
            key = (r["sketch"], d.get("alpha"), None)
        recs[key] = r

ROWS = [
    ("insert", "insert_throughput_items_per_sec", 1e9, "insert CPU (ns / item)"),
    ("merge", "merge_folds_per_sec", 1e6, "merge CPU (µs / fold)"),
    ("query", "query_throughput_items_per_sec", 1e6, "query CPU (µs / query)"),
    ("memory", None, 1.0, "memory_bytes (sketch's own formula)"),
]
fig, axes = plt.subplots(4, 6, figsize=(24, 13), constrained_layout=True)
for col, (sk, title) in enumerate(SKETCHES):
    pts = {k: r for k, r in recs.items() if k[0] == sk}
    pareto = all(k[2] is None for k in pts)
    groups = {None: sorted(pts)} if pareto else {
        K: sorted(k for k in pts if k[2] == K) for K in CAT}
    for row, (op, rate, scale, ylab) in enumerate(ROWS):
        ax = axes[row, col]
        for K, keys in groups.items():
            xs, ys = [], []
            for k in keys:
                r = pts[k]
                v = r.get("memory_bytes") if op == "memory" else per_op(r, op, rate)
                if v is not None:
                    xs.append(k[1])
                    ys.append(v * scale)
            ax.plot(xs, ys, "-o", color=CAT.get(K, "#2a78d6"), lw=2, ms=7, mec=SURF, mew=2,
                    label=KLAB.get(K, "Pareto"))
        ax.set_ylim(bottom=0)
        ax.set_xlabel("Pareto α" if pareto else "Zipf θ")
        if row == 0:
            ax.set_title(title, loc="left", fontsize=11)
            if not pareto:
                ax.legend(fontsize=8, labelcolor=INK2)
        if col == 0:
            ax.set_ylabel(ylab)
fig.suptitle("Serial cost at N = 10⁷ (3 runs + 1 warmup, 16 merge shards, seed 1). "
             "Per-op CPU = phase user+sys CPU / operations in the phase.",
             x=0.01, ha="left", color=INK, fontsize=12)
fig.savefig(sys.argv[1], dpi=110)
print(sys.argv[1])
