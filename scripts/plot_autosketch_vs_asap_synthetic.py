#!/usr/bin/env python3
"""Plot the synthetic AutoSketch vs. ASAP evaluation (ProjectASAP/ASAPQuery#777,
section 7) from the JSON files of `autosketch_vs_asap synthetic`.

Writes, next to the inputs:
  fig_objective_vs_latency.png   default workload: objective vs. achieved max
                                 latency over the SLA grid, one panel per
                                 weights; only points over one RQE set are
                                 joined (dashed: the subset tight SLAs keep)
  fig_objective_by_dimension.png objective with no SLA, each grid dimension
                                 varied alone (templates, shared replicas)
  fig_planning_time.png          planning time vs. RQEs (shared replicas);
                                 AutoSketch also with its benchmark time
  summary.md                     every workload, method, weights and SLA

Usage: scripts/plot_autosketch_vs_asap_synthetic.py DIR
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
DEFAULT = ("dashboard", 1, "p95")


def point(data):
    """(templates, shared, target) of a result's workload name."""
    m = re.search(r"templates=(\w+)/shared=(\d+)/t(\w+)", data["workload"])
    return m.group(1), int(m.group(2)), m.group(3)


def load(directory):
    return {point(d): d for d in (json.loads(p.read_text())
                                  for p in sorted(pathlib.Path(directory).glob("synthetic-*.json")))}


def pick(data, method, weights, sla):
    for r in data["results"]:
        if r["method"] == method and r["weights"] == weights and str(r["sla_ms"]) == str(sla):
            return r if "objective" in r else None
    return None


def weight_names(runs):
    return [w["name"] for w in next(iter(runs.values()))["weights"]]


def fig_objective_vs_latency(runs, out):
    data = runs.get(DEFAULT)
    if data is None:
        return
    names = weight_names(runs)
    fig, axes = plt.subplots(1, len(names), figsize=(5 * len(names), 3.6), squeeze=False)
    for ax, w in zip(axes[0], names):
        # Tight SLAs leave fewer RQEs, so only points over one RQE set are
        # joined: solid for the full set, dashed for the subsets.
        full = data["rqes"]
        for m in METHODS:
            by_n = {}
            for s in data["sla_grid_ms"]:
                if r := pick(data, m, w, s):
                    by_n.setdefault(r["rqes"], []).append((r["max_latency_ms"], r["objective"]))
            for n, pts in sorted(by_n.items(), reverse=True):
                ax.plot(*zip(*sorted(pts)), marker="o", color=COLORS[m],
                        linestyle="-" if n == full else "--",
                        label=LABELS[m] if n == full else None)
        ax.set_xscale("log")
        ax.set_yscale("log")
        ax.set_xlabel("max estimated latency (ms)")
        ax.set_ylabel(f"objective ({UNITS.get(w, w)})")
        ax.set_title(f"dashboard, p95, weights {w}; dashed: subset meeting tight SLAs",
                     fontsize=8)
    axes[0][0].legend(fontsize=7)
    fig.tight_layout()
    fig.savefig(out / "fig_objective_vs_latency.png", dpi=150)
    plt.close(fig)


def fig_objective_by_dimension(runs, out):
    dims = [("templates", 0, ["dashboard", "all"]), ("shared", 1, [1, 8, 64])]
    fig, axes = plt.subplots(1, len(dims), figsize=(4.5 * len(dims), 3.4), squeeze=False)
    for ax, (name, index, values) in zip(axes[0], dims):
        xs = list(range(len(values)))
        for i, m in enumerate(METHODS):
            ys = []
            for v in values:
                key = list(DEFAULT)
                key[index] = v
                data = runs.get(tuple(key))
                r = data and pick(data, m, "cpu", "inf")
                ys.append(r["objective"] if r else 0)
            ax.bar([x + i * 0.27 for x in xs], ys, 0.27, label=LABELS[m], color=COLORS[m])
        ax.set_xticks([x + 0.27 for x in xs], [str(v) for v in values])
        ax.set_yscale("log")
        ax.set_title(name, fontsize=9)
        ax.set_ylabel("CPU (vCPU), no SLA")
    axes[0][0].legend(fontsize=7)
    fig.tight_layout()
    fig.savefig(out / "fig_objective_by_dimension.png", dpi=150)
    plt.close(fig)


def fig_planning_time(runs, out):
    fig, ax = plt.subplots(figsize=(4.5, 3.4))
    for m in METHODS:
        pts = []
        bench = {"lower": [], "paper": []}
        for r in (1, 8, 64):
            data = runs.get(("dashboard", r, "p95"))
            res = data and pick(data, m, "cpu", "inf")
            if res:
                pts.append((data["rqes"], res["planning_secs"]))
                if m == "autosketch":
                    # #777 section 7: AutoSketch's planning time includes the
                    # benchmark time of every probed configuration.
                    a = data["autosketch"]
                    bench["lower"].append((data["rqes"], res["planning_secs"]
                                           + a["benchmark_secs_lower_bound_nbench1e8"]))
                    bench["paper"].append((data["rqes"], res["planning_secs"]
                                           + a["benchmark_secs_paper_rate"]))
        if pts:
            ax.plot(*zip(*pts), marker="o", label=LABELS[m] + (" (search only)"
                    if m == "autosketch" else ""), color=COLORS[m],
                    linestyle=":" if m == "autosketch" else "-")
        for kind, style in (("lower", "-"), ("paper", "--")):
            if bench[kind]:
                ax.plot(*zip(*bench[kind]), marker="o", color=COLORS[m], linestyle=style,
                        label=f"{LABELS[m]} + benchmark ({'lower bound' if kind == 'lower' else '60 s/probe'})")
    ax.set_xscale("log")
    ax.set_yscale("log")
    ax.set_xlabel("RQEs")
    ax.set_ylabel("planning time (s)")
    ax.legend(fontsize=7)
    fig.tight_layout()
    fig.savefig(out / "fig_planning_time.png", dpi=150)
    plt.close(fig)


def fmt(x, digits=3):
    return "—" if x is None else f"{x:.{digits}g}"


def summary(runs, out):
    lines = ["# Synthetic workload: AutoSketch vs. ASAP\n"]
    for key, data in sorted(runs.items(), key=lambda kv: str(kv[0])):
        lines.append(f"## templates={key[0]}, shared={key[1]}, accuracy={key[2]}\n")
        lines.append(f"{data['rqes']} RQEs on {data['streams']} streams; sanity violations: "
                     f"{len(data['sanity_violations'])}. AutoSketch probes: "
                     f"{data['autosketch']['probes']}, benchmark time ≥ "
                     f"{fmt(data['autosketch']['benchmark_secs_lower_bound_nbench1e8'])} s.\n")
        lines.append("| weights | SLA (ms) | RQEs | method | objective | vs ASAP | CPU (vCPU) | "
                     "GiB | deployments | max latency (ms) | planning (s) |")
        lines.append("|---|---|---|---|---|---|---|---|---|---|---|")
        for w in weight_names(runs):
            for sla in data["sla_grid_ms"]:
                base = pick(data, "asap", w, sla)
                for m in METHODS:
                    r = pick(data, m, w, sla)
                    if r is None:
                        continue
                    ratio = r["objective"] / base["objective"] if base and base["objective"] else None
                    lines.append(
                        f"| {w} | {sla} | {r['rqes']} | {LABELS[m]} | {fmt(r['objective'])} | "
                        f"{fmt(ratio)}× | {fmt(r['cpu'])} | {fmt(r['gib'])} | "
                        f"{r['active_deployments']} | {fmt(r['max_latency_ms'])} | "
                        f"{fmt(r['planning_secs'])} |")
        lines.append("")
    (out / "summary.md").write_text("\n".join(lines))


def main():
    out = pathlib.Path(sys.argv[1])
    runs = load(out)
    fig_objective_vs_latency(runs, out)
    fig_objective_by_dimension(runs, out)
    fig_planning_time(runs, out)
    summary(runs, out)


if __name__ == "__main__":
    main()
