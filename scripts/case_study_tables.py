#!/usr/bin/env python3
"""Tables for the §6.3.1 case studies (performance monitoring, anomaly
detection), each swept one knob at a time, for `autosketch_vs_asap synthetic`.

Each table is one workload in the format `export_autosketch_eval_table.py
--synthetic` writes, holding only the fields the runner reads. The anomaly
metric's groups and coverage come from that script's `grouping_cardinality`
and `group_coverage`. The metric is named `requests`, not `http`, so no Hydra
grid (measured at `http`'s schema only) is a candidate.

Writes OUT/<case>-<knob>-<value>.json and OUT/plan.tsv (table, SLA grid).

    python3 scripts/case_study_tables.py --out DIR
"""

import argparse
import json
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import export_autosketch_eval_table as exporter  # noqa: E402

QUANTILE_FAMILIES = [
    {"family": "kll", "sketch": "kll-percall", "target": 0.01, "grid_param": 2.0, "grid_K": None},
    {"family": "dd", "sketch": "dd", "target": 0.01, "grid_param": 2.0, "grid_K": None},
]
# Batch latency SLAs (ms) each case's (or knob's) tables are also solved
# under: around its cheapest plan's latency (perf 24 s, anomaly 3.3 ms, with
# the drill-down 7-165 ms).
SLAS_MS = {"perf": [10, 30, 100, 300, 1_000, 3_000, 10_000],
           "anomaly": [0.3, 0.5, 1, 2, 3],
           "drilldown_endpoints": [3, 10, 30, 100]}
MIN, HOUR = 60, 3600


def quantile_rqe(dataset, query_id, stream, metric, groups, rate, lookback_secs,
                 interval_secs, grouping=None, coverage=None, covers_share=None):
    rqe = {
        "id": f"{dataset}/{query_id}/values/{lookback_secs}s",
        "query_id": query_id, "kind": "values", "capability": "quantile",
        "stream": stream, "metric": metric,
        "lookback_secs": lookback_secs, "interval_secs": interval_secs,
        "label_set": {"groups": groups, "arrival_rate_per_sec": rate, "keys_per_window": None},
        "queries_per_instance": 1, "families": QUANTILE_FAMILIES,
    }
    if grouping is not None:
        covered, min_share, max_share = coverage
        rqe.update({"grouping": grouping, "covers_share": covers_share, "covered_groups": covered,
                    "min_covered_share": min_share, "max_covered_share": max_share})
    return rqe


def table(dataset, rqes, schemas=None, samples_per_sec_per_series=None):
    workload = {"dataset": dataset, "rqes": rqes, "schemas": schemas or {}}
    if samples_per_sec_per_series:
        workload["e2e"] = {"metrics": {"data": {
            "samples_per_sec_per_series": samples_per_sec_per_series}}}
    return {"schema_version": 1, "plan": "sketch-bench case studies (§6.3.1)",
            "workloads": [workload]}


def performance(dataset, series=10_000, jobs=10, per_series=200, long_interval=30 * MIN):
    """Three dashboards on one gauge: instant p50/p90 by job and p50/p90 over
    1m every 15 s; p90 over 10m and 30m every minute; p90 over 6h and 24h
    every `long_interval`. One scrape is 1/`per_series` s."""
    rate = series * per_series
    scrape = 1 / per_series
    by_job = lambda q: quantile_rqe(dataset, f"quantile_by_job_p{q}", "job/dist", "data",
                                    jobs, rate, scrape, 15)
    over = lambda q, lookback, interval: quantile_rqe(
        dataset, f"quantile_over_time_p{q}", "series/dist", "data", series, rate,
        lookback, interval)
    rqes = [by_job("0.5"), by_job("0.9"), over("0.5", MIN, 15), over("0.9", MIN, 15),
            over("0.9", 10 * MIN, MIN), over("0.9", 30 * MIN, MIN),
            over("0.9", 6 * HOUR, long_interval), over("0.9", 24 * HOUR, long_interval)]
    return table(dataset, rqes, samples_per_sec_per_series=per_series)


ANOMALY_QUERIES = [  # (grouping, interval, covers_share)
    (["service"], 15, None), (["region"], 15, None),
    (["service", "region"], MIN, None), (["service", "endpoint"], MIN, None),
    (["service", "region", "status"], 5 * MIN, None), (["region", "endpoint"], 5 * MIN, None),
]
# A drill-down over every label, for groups holding at least 1% of requests
# (the smallest groups fall below the curves' first N).
DRILLDOWN = (["region", "service", "endpoint", "status"], 5 * MIN, 0.01)


def anomaly(dataset, endpoints=25, statuses=4, rate=2_000_000, intervals=None, drilldown=False):
    """p99 latency over 5m by six groupings of (region, service, endpoint,
    status), every group covered, plus DRILLDOWN with `drilldown`.
    `intervals` overrides ANOMALY_QUERIES'."""
    schema = {
        "labels": [
            {"name": "region", "cardinality": 4, "skew": 0.5},
            {"name": "service", "cardinality": 25, "skew": 1.1},
            {"name": "endpoint", "cardinality": endpoints, "child_of": "service", "skew": 1.1},
            {"name": "status", "cardinality": statuses, "skew": 2.0},
        ],
        "rate": rate,
        "values": {"latency": {"pareto": 2.0}},
    }
    exporter.SCHEMAS["requests"] = schema
    rqes = []
    queries = ANOMALY_QUERIES + ([DRILLDOWN] if drilldown else [])
    intervals = intervals or [None] * len(queries)
    assert len(intervals) == len(queries), (len(intervals), len(queries))
    for (grouping, interval, covers), override in zip(queries, intervals):
        coverage = exporter.group_coverage("requests", grouping, covers)
        smallest = rate * 5 * MIN * coverage[1]
        assert smallest >= exporter.SCHEMA_MIN_COVERED_N, (dataset, grouping, smallest)
        rqes.append(quantile_rqe(
            dataset, f"p99_by_{','.join(grouping)}", "requests/latency", "requests",
            exporter.grouping_cardinality("requests", grouping), rate, 5 * MIN,
            override or interval, grouping, coverage, covers))
    return table(dataset, rqes, schemas={"requests": schema})


# (case, knob, values, table builder for one value). The first case of each
# knob's default value is the paper's workload.
SWEEPS = [
    ("perf", "base", [0], lambda d, v: performance(d)),
    ("perf", "long_interval", [15, MIN, 5 * MIN, 30 * MIN],
     lambda d, v: performance(d, long_interval=v)),
    # From 1e4 series: at 1e3 the per-job instant queries (100 series per
    # job) fall below the curves' first N.
    ("perf", "series", [10_000, 30_000, 100_000], lambda d, v: performance(d, series=v)),
    # At least 20 samples/s per series, so the 1m and per-job instant
    # queries hold the curves' first N (1e3 items).
    ("perf", "per_series", [20, 50, 100, 200, 500, 1000],
     lambda d, v: performance(d, per_series=v)),
    ("anomaly", "base", [0], lambda d, v: anomaly(d)),
    ("anomaly", "endpoints", [2, 5, 10, 25, 50, 100], lambda d, v: anomaly(d, endpoints=v)),
    ("anomaly", "drilldown_endpoints", [2, 5, 10, 25, 50, 100],
     lambda d, v: anomaly(d, endpoints=v, drilldown=True)),
    ("anomaly", "statuses", [2, 4, 8, 16], lambda d, v: anomaly(d, statuses=v)),
    ("anomaly", "rate", [250_000, 500_000, 2_000_000, 20_000_000],
     lambda d, v: anomaly(d, rate=v)),
    # Every query at one interval.
    ("anomaly", "interval", [15, MIN, 5 * MIN], lambda d, v: anomaly(d, intervals=[v] * 6)),
]


def main():
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--out", required=True)
    args = parser.parse_args()
    os.makedirs(args.out, exist_ok=True)
    plan = []
    for case, knob, values, build in SWEEPS:
        for value in values:
            name = f"{case}-{knob}-{value}"
            with open(os.path.join(args.out, f"{name}.json"), "w") as f:
                json.dump(build(f"synthetic/case/{case}/{knob}={value}", value), f, indent=1)
                f.write("\n")
            plan.append((name, SLAS_MS.get(knob, SLAS_MS[case])))
    with open(os.path.join(args.out, "plan.tsv"), "w") as f:
        f.writelines(f"{name}.json\t{','.join(map(str, slas))}\n" for name, slas in plan)
    print(f"{len(plan)} tables in {args.out}")


if __name__ == "__main__":
    main()
