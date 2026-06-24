#!/usr/bin/env python3
"""Run ASAPQuery PromQL benchmark queries with paired timestamps.

This script intentionally writes the same JSON shape as ASAPQuery's
benchmarks/scripts/run_baseline.py and run_asap.py so the existing ASAPQuery
comparator and sketch-bench AQP importer can consume the outputs unchanged.
"""

from __future__ import annotations

import argparse
import json
import os
import time
import urllib.parse
from datetime import datetime, timezone
from typing import Any


DEFAULT_PROMETHEUS_URL = "http://localhost:9090"
DEFAULT_ASAP_URL = "http://localhost:8088"
DEFAULT_RUNS_PER_QUERY = 3


def query_endpoint(base_url: str, expr: str, query_ts: float) -> tuple[dict[str, Any], float]:
    import requests

    encoded = urllib.parse.quote(expr, safe="")
    url = f"{base_url}/api/v1/query?query={encoded}&time={query_ts:.3f}"
    t0 = time.monotonic()
    resp = requests.get(url, timeout=30)
    latency_ms = (time.monotonic() - t0) * 1000.0
    resp.raise_for_status()
    return resp.json(), latency_ms


def result_data(payload: dict[str, Any]) -> list[dict[str, Any]]:
    data = payload.get("data", {})
    if not isinstance(data, dict):
        return []
    result = data.get("result", [])
    if not isinstance(result, list):
        return []
    return result


def result_timestamp(result: list[dict[str, Any]]) -> float | None:
    for entry in result:
        try:
            return float(entry["value"][0])
        except (KeyError, IndexError, TypeError, ValueError):
            continue
    return None


def run_one(
    base_url: str,
    expr: str,
    query_ts: float,
) -> tuple[str, list[dict[str, Any]], str | None, float | None, float | None]:
    try:
        payload, latency_ms = query_endpoint(base_url, expr, query_ts)
    except Exception as exc:  # noqa: BLE001
        return "error", [], str(exc), None, None

    if payload.get("status") != "success":
        error = payload.get("error", "unknown error")
        return "error", [], str(error), latency_ms, None

    data = result_data(payload)
    return "success", data, None, latency_ms, result_timestamp(data)


def main() -> None:
    parser = argparse.ArgumentParser(
        description="Run paired Prometheus/ASAPQuery PromQL instant queries"
    )
    parser.add_argument("--query-suite", required=True, help="ASAPQuery promql_suite.json")
    parser.add_argument("--baseline-output", required=True, help="Prometheus JSON output")
    parser.add_argument("--asap-output", required=True, help="ASAPQuery JSON output")
    parser.add_argument("--diagnostics-output", help="Optional paired timestamp diagnostics JSON")
    parser.add_argument(
        "--prometheus-url",
        default=DEFAULT_PROMETHEUS_URL,
        help=f"Prometheus base URL (default: {DEFAULT_PROMETHEUS_URL})",
    )
    parser.add_argument(
        "--asap-url",
        default=DEFAULT_ASAP_URL,
        help=f"ASAPQuery base URL (default: {DEFAULT_ASAP_URL})",
    )
    parser.add_argument(
        "--runs",
        type=int,
        default=DEFAULT_RUNS_PER_QUERY,
        help=f"Runs per query (default: {DEFAULT_RUNS_PER_QUERY})",
    )
    args = parser.parse_args()

    if args.runs <= 0:
        raise SystemExit("--runs must be positive")

    with open(args.query_suite) as f:
        suite = json.load(f)

    baseline_results: dict[str, dict[str, Any]] = {}
    asap_results: dict[str, dict[str, Any]] = {}
    diagnostics: list[dict[str, Any]] = []
    max_result_timestamp_delta = 0.0

    for q in suite["queries"]:
        qid = q["id"]
        expr = q["expr"]
        approximate = q.get("approximate", False)

        baseline_latencies: list[float | None] = []
        asap_latencies: list[float | None] = []
        baseline_data: list[dict[str, Any]] = []
        asap_data: list[dict[str, Any]] = []
        baseline_status = "success"
        asap_status = "success"
        baseline_error: str | None = None
        asap_error: str | None = None

        print(f"[paired] Running query '{qid}' (approximate={approximate}): {expr}")
        for run in range(1, args.runs + 1):
            query_ts = time.time()
            b_status, b_data, b_error, b_latency, b_result_ts = run_one(
                args.prometheus_url, expr, query_ts
            )
            a_status, a_data, a_error, a_latency, a_result_ts = run_one(
                args.asap_url, expr, query_ts
            )

            baseline_latencies.append(b_latency)
            asap_latencies.append(a_latency)
            baseline_status = b_status
            asap_status = a_status
            baseline_error = b_error
            asap_error = a_error
            if b_status == "success":
                baseline_data = b_data
            if a_status == "success":
                asap_data = a_data

            result_delta = None
            if b_result_ts is not None and a_result_ts is not None:
                result_delta = abs(a_result_ts - b_result_ts)
                max_result_timestamp_delta = max(max_result_timestamp_delta, result_delta)

            diagnostics.append(
                {
                    "query_id": qid,
                    "run": run,
                    "query_timestamp": query_ts,
                    "baseline_result_timestamp": b_result_ts,
                    "asap_result_timestamp": a_result_ts,
                    "result_timestamp_delta_seconds": result_delta,
                    "baseline_status": b_status,
                    "asap_status": a_status,
                }
            )
            print(
                "  run {}/{}: baseline={} {} ms  asap={} {} ms  result_delta_s={}".format(
                    run,
                    args.runs,
                    b_status,
                    "n/a" if b_latency is None else f"{b_latency:.1f}",
                    a_status,
                    "n/a" if a_latency is None else f"{a_latency:.1f}",
                    "n/a" if result_delta is None else f"{result_delta:.3f}",
                )
            )

        baseline_results[qid] = {
            "status": baseline_status,
            "latencies_ms": baseline_latencies,
            "data": baseline_data,
            "error": baseline_error,
        }
        asap_results[qid] = {
            "status": asap_status,
            "approximate": approximate,
            "latencies_ms": asap_latencies,
            "data": asap_data,
            "error": asap_error,
        }

    generated_at = datetime.now(timezone.utc).isoformat()
    baseline_output = {
        "timestamp": generated_at,
        "prometheus_url": args.prometheus_url,
        "timestamp_policy": "paired_per_query_run",
        "results": baseline_results,
    }
    asap_output = {
        "timestamp": generated_at,
        "asap_url": args.asap_url,
        "timestamp_policy": "paired_per_query_run",
        "results": asap_results,
    }

    os.makedirs(os.path.dirname(os.path.abspath(args.baseline_output)), exist_ok=True)
    os.makedirs(os.path.dirname(os.path.abspath(args.asap_output)), exist_ok=True)
    with open(args.baseline_output, "w") as f:
        json.dump(baseline_output, f, indent=2)
    with open(args.asap_output, "w") as f:
        json.dump(asap_output, f, indent=2)

    if args.diagnostics_output:
        diagnostics_output = {
            "timestamp": generated_at,
            "timestamp_policy": "paired_per_query_run",
            "runs_per_query": args.runs,
            "query_count": len(suite["queries"]),
            "max_result_timestamp_delta_seconds": max_result_timestamp_delta,
            "pairs": diagnostics,
        }
        os.makedirs(os.path.dirname(os.path.abspath(args.diagnostics_output)), exist_ok=True)
        with open(args.diagnostics_output, "w") as f:
            json.dump(diagnostics_output, f, indent=2)
        print(f"[paired] Diagnostics saved to {args.diagnostics_output}")

    print(f"[paired] Baseline results saved to {args.baseline_output}")
    print(f"[paired] ASAP results saved to {args.asap_output}")
    print(f"[paired] Max result timestamp delta: {max_result_timestamp_delta:.3f}s")


if __name__ == "__main__":
    main()
