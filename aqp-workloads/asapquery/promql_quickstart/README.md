# ASAPQuery PromQL Quickstart AQP Bundle

This is the primary ASAPQuery integration example for AQP Bench.

It uses ASAPQuery's existing Docker quickstart, runs a `sketch-bench` paired
PromQL benchmark against Prometheus and ASAPQuery, then imports the generated
JSON reports into the AQP report schema. The integration is intentionally
external: `sketch-bench` does not modify ASAPQuery or vendor its runtime.

## Contents

| File | Purpose |
|---|---|
| `manifest.toml` | AQP identity for the task, context, data condition, options, and requirements. |
| `run.sh` | End-to-end connector script for the ASAPQuery quickstart benchmark. |
| `paired_promql_runner.py` | Paired-timestamp PromQL runner that writes ASAPQuery-compatible JSON reports. |
| `sample_report_2026-06-23.jsonl` | Sample imported AQP output from a local run. |

## Run

From the `sketch-bench` repository root:

```bash
aqp-workloads/asapquery/promql_quickstart/run.sh
```

The script performs:

```text
ASAPQuery Docker quickstart up
ASAPQuery wait_for_stack.sh
ASAPQuery ingest_wait.sh
sketch-bench paired_promql_runner.py
ASAPQuery compare.py
sketchlib aqp import-asapquery-promql
ASAPQuery Docker quickstart down
```

By default it expects the sibling checkout layout:

```text
ProjectASAP/
  ASAPQuery/
  sketch-bench/
```

Use `--asapquery-dir` for a different checkout:

```bash
aqp-workloads/asapquery/promql_quickstart/run.sh \
  --asapquery-dir /path/to/ASAPQuery \
  --output aqp-workloads/asapquery/promql_quickstart/latest_report.jsonl
```

Use `--keep-stack` when you want to inspect Prometheus or QueryEngine after the
run. Without it, the script stops the Docker stack on success or failure.

Use `--legacy-asapquery-scripts` only when you want to reproduce ASAPQuery's
original sequential `run_baseline.py` then `run_asap.py` behavior. The default
paired runner sends the same PromQL `time=` value to Prometheus and ASAPQuery
for each query repetition, so fidelity is not polluted by comparing different
fake-exporter timestamps.

## AQP Interpretation

This bundle is useful because ASAPQuery naturally exercises the AQP fields:

- Prometheus is the exact baseline option.
- ASAPQuery may use native approximate execution or exact fallback.
- Grouped quantile queries can exercise the native approximate path.
- The paired runner makes fidelity comparisons at the same query timestamp.
- Requirement status is therefore multi-dimensional: latency, fidelity, result
  shape, admission behavior, and overall usefulness are not the same signal.

The report records mixed outcomes directly: one query can satisfy latency and
miss fidelity, preserve fidelity and miss latency, or fall back to exact
execution. Those outcomes are AQP observations, not a single global claim that
ASAPQuery passed or failed.

In the terms of `docs/AQP_PROBLEM_DEFINITION.md`, this bundle is:

```text
PromQL aggregate task
  x Prometheus-compatible telemetry context
  x ASAPQuery quickstart fake-exporter data condition
  x {p95 latency <= 1000 ms, max relative error <= 5%, result series match}
  x {Prometheus exact, ASAPQuery exact fallback, ASAPQuery approximate}
  => latency, speedup, numeric error, label-set mismatch, admission, requirement status
```

## Known Gaps

This is a primary integration example, not the final benchmark suite:

- It uses one fixed synthetic data condition instead of a sweep.
- It records latency and numeric fidelity, but not memory, CPU, storage, or
  network cost.
- It compares PromQL instant-vector values by label set; full PromQL semantic
  coverage for range vectors, staleness, histograms, and NaN/Inf is not yet
  implemented.
- The paired runner fixes query timestamp drift, but it still uses a small
  quickstart workload and only three repetitions per query by default.
- `run.sh` is the first external connector. A typed connector registry would be
  cleaner once there are multiple external systems.
