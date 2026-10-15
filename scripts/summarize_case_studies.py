#!/usr/bin/env python3
"""Each case-study result's plans, one block per table: every method's cost
and MV count with no SLA and at each batch latency SLA, and ASAP's MVs (sketch,
grouping, window/slide, the queries each serves).

    python3 scripts/summarize_case_studies.py RESULT.json... > summary.md
"""

import json
import sys
from collections import defaultdict

METHODS = ["asap", "perquery", "autosketch"]


def short(rqe_id):
    """`case/perf/base=0/quantile_over_time_p0.9/values/86400s` -> `quantile_over_time_p0.9[24h]`."""
    *_, query, _, lookback = rqe_id.split("/")
    secs = float(lookback.rstrip("s"))
    for unit, size in [("d", 86400), ("h", 3600), ("m", 60), ("s", 1)]:
        if secs >= size and secs % size == 0:
            return f"{query}[{int(secs // size)}{unit}]"
    return f"{query}[1 scrape]"


def grouping_of(rqe_id):
    """A query's group-by labels, from its id: `p99_by_service,region` and
    `quantile_by_job_p0.5` name theirs; `quantile_over_time` is per series."""
    query = rqe_id.split("/")[-3]
    if query.startswith("quantile_over_time"):
        return ["series"]
    return query.split("_by_")[1].split("_")[0].split(",")


def secs(value):
    return f"{value:g}s"


def mvs(plan):
    """ASAP's deployments: (description, queries) per deployment index."""
    by_deployment = defaultdict(list)
    for choice in plan["chosen"]:
        by_deployment[choice["deployment"]].append(choice)
    out = []
    for choices in by_deployment.values():
        first = choices[0]
        params = ",".join(f"{k}={v}" for k, v in first["params"].items())
        # Candidates only take the queries' own groupings, so an MV's is the
        # finest of those it serves.
        grouping = max((grouping_of(c["rqe"]) for c in choices), key=len)
        out.append((f"{first['sketch']}({params}) by ({','.join(grouping)}) "
                    f"window {secs(first['window_secs'])} "
                    f"slide {secs(first['slide_secs'])}",
                    [f"{short(c['rqe'])} (merges {c['merged_shards']})" for c in choices]))
    return out


def cheapest(result, method):
    return next(r for r in result["results"]
                if r["method"] == method and r["weights"] == "cpu" and r["bound_ms"] is None)


def describe(path):
    result = json.load(open(path))
    lines = [f"## {result['workload']}", ""]
    lines.append("| SLA (ms) | " + " | ".join(f"{m} vCPU (MVs)" for m in METHODS) + " | ASAP latency ms |")
    lines.append("|---|" + "---|" * (len(METHODS) + 1))
    plans = [("none", {m: cheapest(result, m) for m in METHODS})]
    for sla in result["sla_grid_ms"]:
        plans.append((sla, {r["method"]: r for r in result["sla_results"]
                            if r["weights"] == "cpu" and r["sla_ms"] == sla}))
    for sla, by_method in plans:
        cells = []
        for m in METHODS:
            r = by_method.get(m)
            if r is None or r.get("infeasible"):
                cells.append("infeasible")
            else:
                cells.append(f"{r['cpu']:.3g} ({r['active_deployments']})")
        asap = by_method.get("asap")
        latency = "" if asap is None or asap.get("infeasible") else f"{asap['latency_ms']:.3g}"
        lines.append(f"| {sla} | " + " | ".join(cells) + f" | {latency} |")
    lines.append("")
    seen = set()
    for sla, by_method in plans:
        asap = by_method.get("asap")
        if asap is None or asap.get("infeasible"):
            continue
        plan = mvs(asap)
        key = json.dumps(plan)
        if key in seen:
            continue
        seen.add(key)
        lines.append(f"ASAP plan at SLA {sla}:")
        lines += [f"- {mv}: {', '.join(queries)}" for mv, queries in plan]
        lines.append("")
    return "\n".join(lines)


if __name__ == "__main__":
    print("\n".join(describe(path) for path in sys.argv[1:]))
