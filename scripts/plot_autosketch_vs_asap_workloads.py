#!/usr/bin/env python3
"""Cost vs. total estimated query latency for every workload of the
AutoSketch vs. ASAP evaluation (ProjectASAP/ASAPQuery#777, section 7): one
column per workload (synthetic and traces), one row per weight setting, one
line per method across the SLAs every RQE can meet (tighter ones exclude
RQEs, so their points cover fewer queries). Total latency is the sum of the
RQEs' estimated latencies. Each point is labeled with its cost.

Writes fig_cost_vs_total_latency.png into OUT.

Usage: scripts/plot_autosketch_vs_asap_workloads.py SYNTHETIC_DIR TRACES_DIR OUT
"""

import json
import pathlib
import re
import sys

import matplotlib

matplotlib.use("Agg")
import matplotlib.pyplot as plt  # noqa: E402

METHODS = ["asap", "perquery", "autosketch"]
LABELS = {"asap": "ASAP", "perquery": "PerQuery-CostAware", "autosketch": "AutoSketch-Adapted"}
COLORS = {"asap": "#1f77b4", "perquery": "#ff7f0e", "autosketch": "#2ca02c"}
UNITS = {"cpu": "vCPU", "fargate": "$/hour"}
# Synthetic workloads in the order of the grid, then the traces.
SYNTHETIC = [
    ("synthetic-templatesdashboard-shared1-tp95.json", "dashboard"),
    ("synthetic-templatesall-shared1-tp95.json", "mixed"),
    ("synthetic-templatesdashboard-shared8-tp95.json", "dashboard, r = 8"),
    ("synthetic-templatesdashboard-shared1-metrics8-tp95.json", "dashboard, m = 8"),
    ("synthetic-templatesdashboard-shared1-metrics16-tp95.json", "dashboard, m = 16"),
]


def workloads(synthetic_dir, traces_dir):
    out = []
    for name, title in SYNTHETIC:
        path = pathlib.Path(synthetic_dir) / name
        if path.exists():
            out.append((title, json.loads(path.read_text())))
    for path in sorted(pathlib.Path(traces_dir).glob("traces-*.json")):
        out.append((re.sub(r"^traces-", "", path.stem), json.loads(path.read_text())))
    return out


def total_latency_ms(r):
    return sum(c["latency_ms"] for c in r["chosen"])


def main():
    synthetic_dir, traces_dir, out = sys.argv[1:4]
    runs = workloads(synthetic_dir, traces_dir)
    weights = [w["name"] for w in runs[0][1]["weights"]]
    fig, axes = plt.subplots(len(weights), len(runs), figsize=(3.3 * len(runs), 3.0 * len(weights)),
                             squeeze=False)
    for col, (title, data) in enumerate(runs):
        full = data["rqes"]
        for row, w in enumerate(weights):
            ax = axes[row][col]
            for m in METHODS:
                pts = [(total_latency_ms(r), r["objective"]) for r in data["results"]
                       if r["method"] == m and r["weights"] == w and "objective" in r
                       and r["rqes"] == full]
                ax.plot(*zip(*sorted(pts)), marker="o", markersize=3, color=COLORS[m],
                        label=LABELS[m])
                for x, y in pts:
                    ax.annotate(f"{y:.3g}", (x, y), textcoords="offset points",
                                xytext=(2, 2), fontsize=5, color=COLORS[m])
            ax.set_xscale("log")
            ax.set_yscale("log")
            ax.tick_params(labelsize=6)
            if row == 0:
                ax.set_title(f"{title} ({full} RQEs)", fontsize=8)
            if row == len(weights) - 1:
                ax.set_xlabel("total estimated latency (ms)", fontsize=7)
            if col == 0:
                ax.set_ylabel(f"cost ({UNITS.get(w, w)})", fontsize=7)
    handles, labels = axes[0][0].get_legend_handles_labels()
    fig.legend(handles, labels, loc="upper center", ncol=3, fontsize=8)
    fig.tight_layout(rect=(0, 0, 1, 0.94))
    fig.savefig(pathlib.Path(out) / "fig_cost_vs_total_latency.png", dpi=150)
    plt.close(fig)


if __name__ == "__main__":
    main()
