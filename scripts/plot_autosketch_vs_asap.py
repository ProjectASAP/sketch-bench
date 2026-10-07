#!/usr/bin/env python3
"""Plot the AutoSketch vs. ASAP evaluation on the `traces` workloads
(ProjectASAP/ASAPQuery#777) from the JSON files of
`rqe-optimizer/examples/autosketch_vs_asap.rs`.

Writes fig1_objective.png, fig4_objective_vs_sla.png and summary.md next to
the inputs.

Usage: scripts/plot_autosketch_vs_asap.py DIR
"""

import json
import pathlib
import sys

import matplotlib

matplotlib.use("Agg")
import matplotlib.pyplot as plt  # noqa: E402

METHODS = ["asap", "perquery", "autosketch"]
LABELS = {"asap": "ASAP", "perquery": "PerQuery-CostAware", "autosketch": "AutoSketch-Adapted"}
COLORS = {"asap": "#1f77b4", "perquery": "#ff7f0e", "autosketch": "#2ca02c"}
WEIGHT_UNITS = {"cpu": "vCPU", "fargate": "$/hour"}


def sla_text(sla):
    return "no SLA" if sla == "inf" else f"{sla:g} ms"


def load(directory):
    runs = {}
    for path in sorted(pathlib.Path(directory).glob("traces-*.json")):
        data = json.loads(path.read_text())
        runs[data["workload"].split("/")[-1]] = data
    return runs


def pick(run, method, weights, sla):
    for r in run["results"]:
        if r["method"] == method and r["weights"] == weights and str(r["sla_ms"]) == str(sla):
            return r
    return None


def objective(r):
    return None if r is None or "objective" not in r else r["objective"]


def fig_objective(runs, out):
    """Objective per dataset and method, no SLA, one panel per weight setting."""
    weights = [w["name"] for w in next(iter(runs.values()))["weights"]]
    fig, axes = plt.subplots(1, len(weights), figsize=(5 * len(weights), 3.5), squeeze=False)
    for ax, wname in zip(axes[0], weights):
        names = list(runs)
        for i, method in enumerate(METHODS):
            ys = [objective(pick(runs[n], method, wname, "inf")) or 0 for n in names]
            ax.bar([x + i * 0.27 for x in range(len(names))], ys, 0.27,
                   label=LABELS[method], color=COLORS[method])
        ax.set_xticks([x + 0.27 for x in range(len(names))], names, fontsize=8)
        ax.set_yscale("log")
        ax.set_ylabel(f"objective ({WEIGHT_UNITS.get(wname, wname)})")
        ax.set_title(f"weights: {wname}", fontsize=9)
    axes[0][0].legend(fontsize=7)
    fig.tight_layout()
    fig.savefig(out / "fig1_objective.png", dpi=150)
    plt.close(fig)


def fig_objective_vs_sla(runs, out):
    """Objective vs. the latency SLA, CPU-only weights, one panel per dataset."""
    fig, axes = plt.subplots(1, len(runs), figsize=(4 * len(runs), 3.2), squeeze=False)
    for ax, (name, run) in zip(axes[0], runs.items()):
        slas = run["sla_grid_ms"]
        xs = list(range(len(slas)))
        for method in METHODS:
            points = [(x, objective(pick(run, method, "cpu", s))) for x, s in zip(xs, slas)]
            points = [(x, y) for x, y in points if y is not None]
            if points:
                ax.plot(*zip(*points), marker="o", label=LABELS[method], color=COLORS[method])
        ax.set_xticks(xs, [sla_text(s) for s in slas], fontsize=7)
        ax.set_yscale("log")
        ax.set_title(name, fontsize=9)
        ax.set_ylabel("CPU (vCPU)")
    axes[0][0].legend(fontsize=7)
    fig.tight_layout()
    fig.savefig(out / "fig4_objective_vs_sla.png", dpi=150)
    plt.close(fig)


def fmt(x, digits=3):
    return "—" if x is None else f"{x:.{digits}g}"


def copies(r):
    """Window / slide summed over the plan's deployments: each deployment
    writes an item into window / slide overlapping sketches, which is what
    AutoSketch's one window per query (window = lookback) pays at ingest."""
    deps = {c["deployment"]: c["window_secs"] / c["slide_secs"] for c in r["chosen"]}
    return sum(deps.values())


def summary(runs, out):
    lines = ["# AutoSketch vs. ASAP on the trace workloads\n"]
    for name, run in runs.items():
        lines.append(f"## {name}\n")
        lines.append(f"{run['rqes']} RQEs; dropped as unservable: "
                     f"{len(set(run['dropped_unservable']['asap']) | set(run['dropped_unservable']['autosketch']))}. "
                     f"Sanity violations: {len(run['sanity_violations'])}.\n")
        for w in run["weights"]:
            for sla in run["sla_grid_ms"]:
                rows = [pick(run, m, w["name"], sla) for m in METHODS]
                if not any(rows):
                    continue
                base = objective(rows[0])
                lines.append(f"Weights {w['name']}, {sla_text(sla)}:\n")
                lines.append("| method | objective | vs ASAP | CPU (vCPU) | GiB | deployments | "
                             "Σ window/slide | max latency (ms) | planning (s) |")
                lines.append("|---|---|---|---|---|---|---|---|---|")
                for method, r in zip(METHODS, rows):
                    if r is None or "error" in r:
                        lines.append(f"| {LABELS[method]} | {r and r.get('error')} ||||||||")
                        continue
                    ratio = r["objective"] / base if base else None
                    lines.append(
                        f"| {LABELS[method]} | {fmt(r['objective'])} | {fmt(ratio)}× | "
                        f"{fmt(r['cpu'])} | {fmt(r['gib'])} | {r['active_deployments']} | "
                        f"{fmt(copies(r))} | {fmt(r['max_latency_ms'])} | {fmt(r['planning_secs'])} |")
                lines.append("")
    (out / "summary.md").write_text("\n".join(lines))


def main():
    directory = pathlib.Path(sys.argv[1])
    runs = load(directory)
    fig_objective(runs, directory)
    fig_objective_vs_sla(runs, directory)
    summary(runs, directory)


if __name__ == "__main__":
    main()
