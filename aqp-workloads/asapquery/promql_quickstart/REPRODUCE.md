# Reproduce ASAPQuery PromQL AQP Run

This file records the exact workflow used to produce the ASAPQuery PromQL AQP
example. It assumes this sibling checkout layout:

```text
ProjectASAP/
  ASAPQuery/
  sketch-bench/
```

## What This Reproduces

The run uses ASAPQuery's Docker quickstart, a `sketch-bench` paired PromQL
runner, ASAPQuery's comparator, and the AQP importer. The paired runner writes
the same JSON shape as ASAPQuery's built-in benchmark scripts, but it sends one
shared PromQL `time=` value to both Prometheus and ASAPQuery for each query
repetition.

Output files:

```text
ASAPQuery/benchmarks/reports/baseline_results.json
ASAPQuery/benchmarks/reports/asap_results.json
ASAPQuery/benchmarks/reports/eval_report.md
ASAPQuery/benchmarks/reports/paired_diagnostics.json
sketch-bench/aqp-workloads/asapquery/promql_quickstart/latest_report.jsonl
```

## Prerequisites

- Docker daemon running.
- `docker compose` available.
- Rust/Cargo available for `sketch-bench`.
- Python with `requests`, or allow the runner to create a temporary virtualenv.
- On Apple Silicon, use `DOCKER_DEFAULT_PLATFORM=linux/amd64` because the
  ASAPQuery quickstart images used in this run were amd64.

The script omits Grafana and starts only benchmark-required services. This
avoids host port `3000` conflicts.

## One-Command Reproduction

From `sketch-bench`:

```bash
aqp-workloads/asapquery/promql_quickstart/run.sh \
  --output aqp-workloads/asapquery/promql_quickstart/latest_report.jsonl
```

If you already have a Python environment with `requests`:

```bash
aqp-workloads/asapquery/promql_quickstart/run.sh \
  --python /path/to/python \
  --output aqp-workloads/asapquery/promql_quickstart/latest_report.jsonl
```

To reproduce the older ASAPQuery sequential benchmark scripts instead of the
paired timestamp runner, add `--legacy-asapquery-scripts`.

The script performs:

```text
1. Start ASAPQuery quickstart Docker services:
   prometheus, asap-planner-rs, queryengine, and fake exporters.
2. Run ASAPQuery benchmarks/scripts/wait_for_stack.sh.
3. Run ASAPQuery benchmarks/scripts/ingest_wait.sh.
4. Run sketch-bench paired_promql_runner.py, producing ASAPQuery-compatible
   baseline_results.json and asap_results.json with aligned query timestamps.
5. Run ASAPQuery benchmarks/scripts/compare.py.
6. Run sketchlib aqp import-asapquery-promql.
7. Stop the Docker quickstart stack.
```

Use `--keep-stack` if you want Docker services left running for inspection.

## Manual Reproduction

Start services from `ASAPQuery/asap-quickstart`:

```bash
DOCKER_DEFAULT_PLATFORM=linux/amd64 docker compose -f docker-compose.yml up -d \
  prometheus asap-planner-rs queryengine \
  fake-exporter-constant fake-exporter-linear-up fake-exporter-linear-down \
  fake-exporter-sine fake-exporter-sine-noise fake-exporter-step fake-exporter-exp-up
```

Run the paired PromQL runner from `sketch-bench`:

```bash
cd ../ASAPQuery
bash benchmarks/scripts/wait_for_stack.sh
bash benchmarks/scripts/ingest_wait.sh
cd ../sketch-bench
python3 aqp-workloads/asapquery/promql_quickstart/paired_promql_runner.py \
  --query-suite ../ASAPQuery/benchmarks/queries/promql_suite.json \
  --baseline-output ../ASAPQuery/benchmarks/reports/baseline_results.json \
  --asap-output ../ASAPQuery/benchmarks/reports/asap_results.json \
  --diagnostics-output ../ASAPQuery/benchmarks/reports/paired_diagnostics.json
cd ../ASAPQuery
python3 benchmarks/scripts/compare.py \
  --baseline benchmarks/reports/baseline_results.json \
  --asap benchmarks/reports/asap_results.json \
  --output benchmarks/reports/eval_report.md
```

Import into AQP from `sketch-bench`:

```bash
cargo run -p sketch-cli --no-default-features -- aqp import-asapquery-promql \
  --manifest aqp-workloads/asapquery/promql_quickstart/manifest.toml \
  --baseline-json ../ASAPQuery/benchmarks/reports/baseline_results.json \
  --asap-json ../ASAPQuery/benchmarks/reports/asap_results.json \
  --query-suite ../ASAPQuery/benchmarks/queries/promql_suite.json \
  --report aqp-workloads/asapquery/promql_quickstart/latest_report.jsonl
```

Stop services:

```bash
cd asap-quickstart
DOCKER_DEFAULT_PLATFORM=linux/amd64 docker compose -f docker-compose.yml down
```

## Verify Output

From `sketch-bench`:

```bash
python3 - <<'PY'
import json
p = "aqp-workloads/asapquery/promql_quickstart/latest_report.jsonl"
r = json.loads(open(p).readline())
print("case", r["case_id"])
print("track", r["track"])
print("records", len(r["records"]))
print("option_runs", sum(x["record_kind"] == "option_run" for x in r["records"]))
print("comparisons", sum(x["record_kind"] == "paired_comparison" for x in r["records"]))
PY
```

Expected shape:

```text
case asapquery_promql_quickstart_v1
track PromQL AQP
records 42
option_runs 28
comparisons 14
```

Verify the paired timestamp invariant:

```bash
python3 - <<'PY'
import json
p = "../ASAPQuery/benchmarks/reports/paired_diagnostics.json"
d = json.load(open(p))
print("timestamp_policy", d["timestamp_policy"])
print("max_result_timestamp_delta_seconds", d["max_result_timestamp_delta_seconds"])
PY
```

Expected shape:

```text
timestamp_policy paired_per_query_run
max_result_timestamp_delta_seconds 0.0
```

## Observed Run Notes

The latest paired local run produced these representative AQP rows:

| Query | ASAP native | p95 speedup | p95 delta | Max relative error | Latency target | Fidelity target |
|---|:---:|---:|---:|---:|:---:|:---:|
| `q95_by_pattern` | yes | 175.56x | -2098.0 ms | 0.0056 = 0.56% | met | met |
| `q99_by_pattern` | yes | 429.59x | -2153.5 ms | 0.0066 = 0.66% | met | met |
| `q50_by_pattern` | yes | 396.46x | -2303.4 ms | 0.1000 = 10.00% | met | failed |
| `q95_all` | yes | 1.04x | -74.9 ms | 0.0000 = 0.00% | failed | met |

Older legacy runs using ASAPQuery's sequential scripts produced rows like:

| Query | ASAP native | p95 speedup | p95 delta | Max relative error | Latency target | Fidelity target |
|---|:---:|---:|---:|---:|:---:|:---:|
| `q95_by_pattern` | yes | 101.91x | -1505.4 ms | 0.7507 = 75.07% | met | failed |
| `q99_by_pattern` | yes | 295.00x | -1391.4 ms | 0.7507 = 75.07% | met | failed |
| `q50_by_pattern` | yes | 404.95x | -1455.9 ms | 0.9582 = 95.82% | met | failed |
| `q95_all` | yes | 0.69x | +691.0 ms | 0.0590 = 5.90% | failed | failed |

`Max relative error` is a fraction. The manifest target is `0.05`, meaning 5%.
`p95 delta` is `ASAPQuery p95 - Prometheus p95`, so negative means ASAPQuery was
faster and positive means it was slower.

## Important Caveats

The legacy ASAPQuery scripts run baseline and ASAP passes sequentially and each
uses a fresh `time=now`. The fake-exporter data is time-varying, so legacy
relative error can include timestamp drift. The default wrapper now avoids that
gap by pinning one shared query timestamp for both options on each query
repetition.

Latency can also vary by host load and Docker emulation. On Apple Silicon, the
amd64 image emulation can change absolute timings, so use the AQP report to
inspect per-run cost/fidelity behavior instead of expecting identical numbers.
