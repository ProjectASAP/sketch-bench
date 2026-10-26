#!/usr/bin/env python3
"""The §6.3.1 case-study workloads, as autosketch_vs_asap synthetic tables.

- perf-monitoring: three dashboards over `cpu_usage` (the mixed workload's
  data model: 1e4 series in 10 jobs, 200 samples/s each): per-job instant
  p50/p90 and per-series p50/p90 over 1m every 15 s; per-series p90 over
  10m and 30m every 1 min; and over 6h and 24h every 30 min.
- anomaly-detection: p99 request latency over 5m on the `http` schema, by
  (service) and (region) every 15 s, by (region, service) and (service,
  endpoint) every 1 min, by (region, service, status) and (region, endpoint)
  every 5 min; every group is held to the target.

The RQEs are built with export_autosketch_eval_table.py's helpers, so they
have the evaluation's shape, targets and data. Usage:
  scripts/export_case_studies.py OUT_DIR
"""

import json
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

import export_autosketch_eval_table as ex  # noqa: E402


def write(out, name, queries, extra=None):
    workload = {"dataset": queries[0]["dataset"],
                "rqes": [ex.synthetic_entry(q) for q in queries], **(extra or {})}
    with open(os.path.join(out, f"{name}.json"), "w") as f:
        json.dump({"schema_version": 1, "plan": "§6.3.1 case studies", "inputs": {},
                   "workloads": [workload]}, f, indent=1)
    print(f"{name}: {len(workload['rqes'])} RQEs", file=sys.stderr)


def perf_monitoring(out):
    series, jobs = ex.SYNTHETIC_SERIES, ex.SYNTHETIC_JOBS
    rate = series * ex.SYNTHETIC_SAMPLES_PER_SEC
    scrape = 1 / ex.SYNTHETIC_SAMPLES_PER_SEC  # an instant query's lookback

    def rqe(query_id, stream, rng, lookback, interval, groups):
        return {"dataset": "case/perf-monitoring", "query_id": query_id, "kind": "values",
                "range": rng, "range_s": lookback, "step_s": interval,
                "theta": ex.SYNTHETIC_THETA, "K": None,
                "min_N": rate * lookback / groups, "max_N": rate * lookback / groups,
                "alpha_rank": ex.SYNTHETIC_ALPHA, "alpha_memory": ex.SYNTHETIC_ALPHA,
                "tail_class": "", "families": ex.SYNTHETIC_FAMILIES["quantile"],
                "capability": "quantile", "stream": f"cpu_usage/{stream}",
                "metric": "cpu_usage", "groups": groups, "arrival_rate": rate,
                "assumptions": [], **ex.DEFAULT_TARGETS}

    queries = [
        rqe("quantile_by_job_p0.5", "job/dist", "1s", scrape, 15, jobs),
        rqe("quantile_by_job_p0.9", "job/dist", "1s", scrape, 15, jobs),
        rqe("quantile_over_time_p0.5", "series/dist", "1m", 60, 15, series),
        rqe("quantile_over_time_p0.9", "series/dist", "1m", 60, 15, series),
        rqe("quantile_over_time_p0.9", "series/dist", "10m", 600, 60, series),
        rqe("quantile_over_time_p0.9", "series/dist", "30m", 1800, 60, series),
        rqe("quantile_over_time_p0.9", "series/dist", "6h", 21600, 1800, series),
        rqe("quantile_over_time_p0.9", "series/dist", "24h", 86400, 1800, series),
    ]
    # The runner reads the scrape rate (and so the 5 ms instant lookback)
    # from the e2e metrics block, as for the classic tables.
    metric = {"labels": [{"cardinality": jobs, "name": "job"},
                         {"cardinality": series // jobs, "child_of": "job", "name": "label_0"}],
              "samples_per_sec_per_series": ex.SYNTHETIC_SAMPLES_PER_SEC, "type": "gauge",
              "values": {"dist": {"pareto": ex.SYNTHETIC_ALPHA}}}
    write(out, "perf-monitoring", queries, {"e2e": {"metrics": {"cpu_usage": metric}}})


def anomaly_detection(out):
    ex.SCHEMA_WINDOWS = "5m"
    queries, seen = [], set()
    for interval, groupings in [(15, [["service"], ["region"]]),
                                (60, [["region", "service"], ["service", "endpoint"]]),
                                (300, [["region", "service", "status"], ["region", "endpoint"]])]:
        ex.SCHEMA_TEMPLATES = [(16, "p99_latency", "http", "latency", "quantile", groupings,
                                None)]
        queries += ex.schema_queries(interval, "case/anomaly-detection", False, seen)
    write(out, "anomaly-detection", queries,
          {"schemas": {q["metric"]: q["schema"] for q in queries}})


def main():
    out = sys.argv[1]
    os.makedirs(out, exist_ok=True)
    perf_monitoring(out)
    anomaly_detection(out)


if __name__ == "__main__":
    main()
