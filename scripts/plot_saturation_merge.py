"""Accuracy after merging: error vs N, one line per shard count m, per sketch and distribution.

Usage: plot_saturation_merge.py saturation_merge_curve.csv OUT_DIR  (needs matplotlib)
Writes merge_error_vs_N__<sketch>.png per sketch and merge_ratio_at_nmax.png.
"""
import csv
import os
import sys

import matplotlib
matplotlib.use("Agg")
import matplotlib.pyplot as plt  # noqa: E402

rows = list(csv.DictReader(open(sys.argv[1])))
out = sys.argv[2]
os.makedirs(out, exist_ok=True)
INK, INK2, MUTED, SURF = "#0b0b0b", "#52514e", "#898781", "#fcfcfb"
SHARD_COLORS = {"1": "#2a78d6", "4": "#eb6834", "16": "#1baf7a", "64": "#eda100"}
plt.rcParams.update({
    "font.size": 9, "axes.edgecolor": MUTED, "axes.labelcolor": INK2,
    "xtick.color": MUTED, "ytick.color": MUTED, "axes.titlecolor": INK,
    "axes.spines.top": False, "axes.spines.right": False,
    "axes.grid": True, "grid.color": "#e8e7e3", "grid.linewidth": 0.6,
    "figure.facecolor": SURF, "axes.facecolor": SURF, "legend.frameon": False,
})
METRIC = {"cms-fastpath-vector2d": "are_top100", "countsketch-fastpath-vector2d": "are_top100",
          "cms-heap-topk-fastpath-vector2d": "precision@k (higher is better)",
          "hll": "relative error", "kll-percall": "mean rank error", "dd": "mean rank error"}
KLAB = {"1000": "1K", "100000": "100K", "10000000": "10M"}

curves = {}
for r in rows:
    key = (r["sketch"], r["config"], r["dist"], r["param"], r["cardinality"])
    curves.setdefault(key, {}).setdefault(r["shards"], []).append(
        (int(r["n"]), float(r["seed_mean_error"]), float(r["seed_se"])))

ratio = {}
for sk in dict.fromkeys(k[0] for k in curves):
    keys = sorted((k for k in curves if k[0] == sk),
                  key=lambda k: (float(k[3]), float(k[4] or 0)))
    params = sorted({k[3] for k in keys}, key=float)
    ks = sorted({k[4] for k in keys}, key=lambda v: float(v or 0))
    fig, axes = plt.subplots(len(ks), len(params), figsize=(3.4 * len(params) + 0.5, 2.6 * len(ks) + 1.1),
                             squeeze=False, sharex=True, constrained_layout=True)
    for key in keys:
        ax = axes[ks.index(key[4]), params.index(key[3])]
        base = None
        for m in sorted(curves[key], key=int):
            pts = sorted(curves[key][m])
            ns = [p[0] for p in pts]
            ys = [p[1] for p in pts]
            ses = [p[2] for p in pts]
            ax.fill_between(ns, [y - 2 * s for y, s in zip(ys, ses)], [y + 2 * s for y, s in zip(ys, ses)],
                            color=SHARD_COLORS[m], alpha=0.12, lw=0)
            ax.plot(ns, ys, color=SHARD_COLORS[m], lw=2 if m != "1" else 2.6, label=f"m={m}")
            if m == "1":
                base = ys[-1]
            elif base:
                ratio.setdefault(sk, []).append((key, int(m), ys[-1] / base))
        ax.set_xscale("log")
        if all(y > 0 for m in curves[key] for _, y, _ in curves[key][m]) and "topk" not in sk:
            ax.set_yscale("log")
        head = f"α={key[3]}" if key[2] == "pareto" else f"θ={key[3]}, K={KLAB.get(key[4], key[4])}"
        ax.set_title(head, loc="left", fontsize=9)
        ax.set_xlabel("items inserted N")
    axes[0, 0].legend(fontsize=8, labelcolor=INK2)
    for a in axes[:, 0]:
        a.set_ylabel(METRIC.get(sk, "error"))
    fig.suptitle(f"{sk} ({keys[0][1]})\nerror after merging m contiguous shards vs one sketch (m=1); "
                 "mean of 3 seeds, band = ±2 SE", x=0.01, ha="left", color=INK, fontsize=11)
    fig.savefig(os.path.join(out, f"merge_error_vs_N__{sk}.png"), dpi=110)
    plt.close(fig)

# Summary: merged / single error at the largest N, per sketch and shard count.
fig, ax = plt.subplots(figsize=(9, 4.2), constrained_layout=True)
sks = list(ratio)
for j, m in enumerate((4, 16, 64)):
    for i, sk in enumerate(sks):
        vals = [v for (_, mm, v) in ratio[sk] if mm == m]
        xs = [i + (j - 1) * 0.22] * len(vals)
        ax.plot(xs, vals, "o", ms=6, color=SHARD_COLORS[str(m)], mec=SURF, mew=1.5,
                label=f"m={m}" if i == 0 else None)
ax.axhline(1.0, color=MUTED, lw=1, ls=":")
ax.set_xticks(range(len(sks)))
ax.set_xticklabels([s.replace("-fastpath-vector2d", "") for s in sks], fontsize=9)
ax.set_ylabel("merged / single at N = max\n(precision for top-k: < 1 is worse)")
ax.set_title("Each dot is one distribution; 1.0 = merging is exact", loc="left", fontsize=10)
ax.legend(fontsize=8, labelcolor=INK2)
fig.savefig(os.path.join(out, "merge_ratio_at_nmax.png"), dpi=110)
print(out)
