#!/usr/bin/env python3
"""The paper's §6.3 figures, from the committed AutoSketch-vs-ASAP results.

Writes into OUT (PDF and PNG each):
  sec63_cost_bars     each method's cost at AutoSketch-Adapted's batch latency
                      (CPU only, log scale), per workload (panels) at base,
                      8 users and 16 metrics; a baseline's bar also names how
                      many times ASAP's cost it is
  sec63_frontier      cost vs. batch latency (ASAP and PerQuery swept over
                      latency bounds; AutoSketch one point), CPU and Fargate
  sec63_rollups       the roll-up ablation: ASAP's cheapest plan with and
                      without roll-ups (CPU only, log scale)

Every cost is absolute (vCPU or $/hour).

Usage: scripts/plot_sec63_paper.py RESULTS_DIR OUT
  RESULTS_DIR: rqe-optimizer/results/autosketch-vs-asap-hydra
"""

import json
import os
import sys

import matplotlib
import matplotlib.ticker

matplotlib.use("Agg")
import matplotlib.pyplot as plt  # noqa: E402

plt.rcParams.update({"font.size": 8, "pdf.fonttype": 42})

WORKLOADS = [("classic", "Aggregations"), ("multigroup", "Roll-ups"),
             ("all", "Aggregations + roll-ups")]
VARIANTS = [("shared1", "base"), ("shared8", "8 users"), ("shared1-metrics16", "16 metrics")]
METHODS = [("autosketch", "AutoSketch-Adapted", "C2"), ("perquery", "PerQuery", "C1"),
           ("asap", "ASAP", "C0")]


def load(results, table):
    with open(os.path.join(results, f"synthetic-templates{table}-tp95.json")) as f:
        return json.load(f)


def plans(data, method, weights):
    return [r for r in data["results"]
            if r["weights"] == weights and r["method"] == method and "objective" in r]


def cheapest(data, method, weights="cpu", max_latency_ms=None):
    return min((r for r in plans(data, method, weights)
                if max_latency_ms is None or r["latency_ms"] <= max_latency_ms + 1e-6),
               key=lambda r: r["objective"])["objective"]


def save(fig, out, name):
    fig.savefig(os.path.join(out, f"{name}.pdf"))
    fig.savefig(os.path.join(out, f"{name}.png"), dpi=150)
    plt.close(fig)


def cost_bars(results, out):
    fig, axes = plt.subplots(1, len(WORKLOADS), figsize=(7, 2.4))
    width = 0.27
    for ax, (templates, title) in zip(axes, WORKLOADS):
        labels = []
        for i, (variant, name) in enumerate(VARIANTS):
            data = load(results, f"{templates}-{variant}")
            limit = data["frontier"]["autosketch_latency_ms"]
            cost = {m: cheapest(data, m, max_latency_ms=limit) for m, _, _ in METHODS}
            for k, (method, label, color) in enumerate(METHODS):
                x = i + (k - 1) * width
                ax.bar(x, cost[method], width, color=color, label=label if i == 0 else None)
                text = f"{cost[method]:,.0f}" if cost[method] >= 100 else f"{cost[method]:.2g}"
                if method != "asap":
                    ratio = cost[method] / cost["asap"]
                    text += f"\n{ratio:,.0f}×" if ratio >= 10 else f"\n{ratio:.1f}×"
                ax.text(x, cost[method] * 1.15, text, ha="center", va="bottom", fontsize=5.5)
            labels.append(f"{name}\n({data['rqes']} RQEs)")
        ax.set_yscale("log")
        ax.set_xticks(range(len(VARIANTS)))
        ax.set_xticklabels(labels, fontsize=6.5)
        ax.set_title(title, fontsize=8)
        ax.set_ylim(top=ax.get_ylim()[1] * 30)
        ax.grid(axis="y", alpha=0.3, lw=0.3)
    axes[0].set_ylabel("cost (vCPU, log)")
    handles, names = axes[0].get_legend_handles_labels()
    fig.legend(handles, names, loc="upper center", ncol=len(METHODS), fontsize=7, frameon=False)
    fig.tight_layout(rect=(0, 0, 1, 0.92))
    save(fig, out, "sec63_cost_bars")


def frontier(results, out):
    lines = [("asap", "ASAP", "C0", "o"), ("perquery", "PerQuery", "C1", "s")]
    units = [("cpu", "cost (vCPU)"), ("fargate", "cost ($/hour)")]
    fig, axes = plt.subplots(2, len(WORKLOADS), figsize=(7, 4))
    for j, (templates, title) in enumerate(WORKLOADS):
        data = load(results, f"{templates}-shared1")
        for i, (weights, unit) in enumerate(units):
            ax = axes[i, j]
            for method, label, color, marker in lines:
                rs = sorted(plans(data, method, weights), key=lambda r: r["latency_ms"])
                ax.plot([r["latency_ms"] / 1e3 for r in rs], [r["objective"] for r in rs],
                        "-", color=color, marker=marker, ms=2.5, lw=1, label=label)
            auto = plans(data, "autosketch", weights)[0]
            ax.scatter([auto["latency_ms"] / 1e3], [auto["objective"]], marker="D", s=20,
                       color="C2", label="AutoSketch-Adapted", zorder=5)
            ax.set_xscale("log")
            ax.set_yscale("log")
            ax.xaxis.set_major_locator(matplotlib.ticker.LogLocator(subs=(1, 2, 5)))
            ax.xaxis.set_major_formatter(matplotlib.ticker.FormatStrFormatter("%g"))
            ax.xaxis.set_minor_formatter(matplotlib.ticker.NullFormatter())
            ax.grid(alpha=0.3, which="both", lw=0.3)
            if j == 0:
                ax.set_ylabel(unit)
            if i == 0:
                ax.set_title(f"{title} ({data['rqes']} RQEs)", fontsize=8)
            if i == len(units) - 1:
                ax.set_xlabel("batch latency (s)")
    handles, names = axes[0, 0].get_legend_handles_labels()
    fig.legend(handles, names, loc="upper center", ncol=3, fontsize=7, frameon=False)
    fig.tight_layout(rect=(0, 0, 1, 0.94))
    save(fig, out, "sec63_frontier")


def rollups(results, out):
    fig, ax = plt.subplots(figsize=(7, 2.3))
    x, ticks, labels = 0, [], []
    for templates, title in WORKLOADS:
        for variant, name in VARIANTS:
            data = load(results, f"{templates}-{variant}")
            for k, (method, label, color) in enumerate(
                    [("asap-norollup", "ASAP w/o roll-ups", "C9"), ("asap", "ASAP", "C0")]):
                cost = cheapest(data, method)
                ax.bar(x + (k - 0.5) * 0.38, cost, 0.38, color=color,
                       label=label if x == 0 else None, hatch="//" if k == 0 else None,
                       edgecolor="white", lw=0)
                ax.text(x + (k - 0.5) * 0.38, cost * 1.08, f"{cost:.3g}", ha="center",
                        fontsize=6)
            ticks.append(x)
            labels.append(f"{name}\n({data['rqes']} RQEs)")
            x += 1
        ax.text(x - 2, 250, title, ha="center", fontsize=8, weight="bold")
        x += 0.6
    ax.set_yscale("log")
    ax.set_ylim(0.1, 900)
    ax.set_xticks(ticks)
    ax.set_xticklabels(labels, fontsize=6.5)
    ax.set_ylabel("cheapest plan (vCPU)")
    ax.legend(fontsize=7, loc="lower left", bbox_to_anchor=(0, 1), frameon=False, ncol=2)
    ax.grid(axis="y", alpha=0.3, lw=0.3)
    fig.tight_layout()
    save(fig, out, "sec63_rollups")


def main():
    results, out = sys.argv[1], sys.argv[2]
    os.makedirs(out, exist_ok=True)
    cost_bars(results, out)
    frontier(results, out)
    rollups(results, out)


if __name__ == "__main__":
    main()
