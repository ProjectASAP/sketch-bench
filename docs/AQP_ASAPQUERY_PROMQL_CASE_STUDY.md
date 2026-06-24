# AQP Case Study: ASAPQuery PromQL Quickstart Benchmark

This is the primary ASAPQuery integration example for AQP Bench. It instantiates
`AQP_PROBLEM_DEFINITION.md` with an ASAPQuery benchmark that runs end to end
from the existing Docker quickstart. `sketch-bench` does not modify ASAPQuery;
it runs a paired-timestamp wrapper and imports ASAPQuery-compatible JSON
benchmark reports.

The bundle lives at:

```text
sketch-bench/aqp-workloads/asapquery/promql_quickstart/
```

## Benchmark Mapping

| AQP field | Concrete ASAPQuery value |
|---|---|
| Track | PromQL AQP |
| Benchmark context | Prometheus-compatible telemetry query serving |
| Task | PromQL aggregate suite over `sensor_reading` |
| Query suite | `avg`, `sum`, `min`, `max`, and `quantile`, including `by (pattern)` groupings |
| Approximate option | `asapquery_promql_precompute_v0_5_1` |
| Baseline option | `prometheus_v3_9_1_exact` |
| Data condition | ASAPQuery quickstart fake exporters with deterministic patterns |
| Requirement example | p95 latency <= 1000 ms, max relative error <= 5%, result series match |

The manifest lives at:

```text
sketch-bench/aqp-workloads/asapquery/promql_quickstart/manifest.toml
```

## Running The External Benchmark

The preferred entry point is the bundle runner:

```bash
aqp-workloads/asapquery/promql_quickstart/run.sh
```

The runner starts only the services needed by the benchmark, so it avoids the
Grafana port `3000` conflict that can happen on developer machines:

```bash
DOCKER_DEFAULT_PLATFORM=linux/amd64 docker compose -f docker-compose.yml up -d \
  prometheus asap-planner-rs queryengine \
  fake-exporter-constant fake-exporter-linear-up fake-exporter-linear-down \
  fake-exporter-sine fake-exporter-sine-noise fake-exporter-step fake-exporter-exp-up
```

It then runs the paired PromQL runner from `sketch-bench` and ASAPQuery's
comparator. The bundle script handles these working-directory changes; shown
manually, the sequence is:

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

On macOS Apple Silicon, the public images are amd64-only, so the example above
uses `DOCKER_DEFAULT_PLATFORM=linux/amd64`.

## Importing Into AQP

After the paired runner produces ASAPQuery-compatible JSON reports:

```bash
cargo run -p sketch-cli --no-default-features -- aqp import-asapquery-promql \
  --manifest aqp-workloads/asapquery/promql_quickstart/manifest.toml \
  --baseline-json ../ASAPQuery/benchmarks/reports/baseline_results.json \
  --asap-json ../ASAPQuery/benchmarks/reports/asap_results.json \
  --query-suite ../ASAPQuery/benchmarks/queries/promql_suite.json \
  --report aqp-workloads/asapquery/promql_quickstart/latest_report.jsonl
```

The importer emits one JSONL record containing:

- option runs for Prometheus and ASAPQuery per query
- paired comparisons per query
- p95 latency speedup and latency delta
- native approximate versus exact fallback admission behavior
- relative error, missing label sets, and requirement satisfaction

## Why This Matches The Core Problem

`AQP_PROBLEM_DEFINITION.md` defines the benchmark shape as:

```text
task x benchmark context x data condition x requirement x option
  => cost, fidelity, admission, and failure behavior
```

The PromQL quickstart fills each slot with concrete ASAPQuery artifacts:

| Core problem slot | PromQL quickstart concrete value |
|---|---|
| Task | PromQL aggregate and quantile queries from `benchmarks/queries/promql_suite.json` |
| Benchmark context | Prometheus-compatible telemetry query API, not SQL and not direct sketch calls |
| Data condition | Seven ASAPQuery fake exporters producing `sensor_reading` with pattern labels |
| Ground truth | Prometheus instant-query result for each PromQL expression |
| Requirements | p95 latency <= 1000 ms, max relative error <= 5%, result series match |
| Options | Prometheus exact baseline; ASAPQuery exact fallback; ASAPQuery native approximate execution |
| Cost metrics | latency samples, min/mean/p50/p95/max latency, p95 speedup, p95 latency delta |
| Fidelity metrics | absolute error, relative error, missing label sets, extra label sets |
| Admission behavior | `baseline`, `exact_fallback`, `approximated`, `option_error`, `missing_counterpart` |
| Failure behavior | query errors, missing counterparts, unparseable results, label-set mismatches |

This is why it is a concrete AQP plan rather than just a benchmark transcript:
it starts with an existing external benchmark context, preserves ASAPQuery's
PromQL interface and JSON report shape, then normalizes the result into the AQP
dimensions. It does not claim that "ASAPQuery is faster" or "ASAPQuery passes";
it records which queries use which option and which requirement each option
satisfies.

## What Became Less Vague

The original problem definition left several details intentionally broad. The
PromQL quickstart makes the first version concrete:

- Metrics: p95 latency and max relative error are the first cost/fidelity pair.
- Requirement status: latency, fidelity, and result shape are checked
  separately.
- Admission model: ASAPQuery rows are separated into native approximate
  execution and exact fallback.
- Failure model: errors and missing counterpart rows are preserved as benchmark
  outcomes.
- Timestamp policy: each Prometheus/ASAPQuery pair uses the same PromQL
  `time=` value, so fidelity does not include artificial timestamp drift.
- Report shape: one AQP JSONL object contains all option runs and paired
  comparisons for the query suite.

The key point is that the AQP record can represent disagreement between
metrics. A query can improve latency and miss fidelity, preserve fidelity and
miss latency, or fall back to exact execution. Those mixed outcomes are exactly
what the benchmark is supposed to reveal.

## Observed Local Run

On June 24, 2026 America/New_York, the paired quickstart import ran
successfully with Grafana omitted. The paired diagnostics reported
`max_result_timestamp_delta_seconds = 0.0` across 42 Prometheus/ASAPQuery query
pairs.

The sample imported AQP report is checked into the bundle at:

```text
sketch-bench/aqp-workloads/asapquery/promql_quickstart/sample_report_2026-06-23.jsonl
```

That sample report came from the older legacy sequential scripts and remains
useful for explaining the report shape, but it should not be used as clean
fidelity evidence because baseline and ASAP used different `time=now` values.
Use `latest_report.jsonl` from the paired runner for current fidelity
interpretation.

The clearest paired AQP example is the grouped quantile path:

| Query | ASAP native | p95 speedup | p95 delta | Max relative error | Latency target | Fidelity target | AQP interpretation |
|---|:---:|---:|---:|---:|:---:|:---:|---|
| `q95_by_pattern` | yes | 175.56x | -2098.0 ms | 0.0056 = 0.56% | met | met | approximate option is much faster and satisfies the 5% fidelity target |
| `q99_by_pattern` | yes | 429.59x | -2153.5 ms | 0.0066 = 0.66% | met | met | approximate option is much faster and satisfies the 5% fidelity target |
| `q50_by_pattern` | yes | 396.46x | -2303.4 ms | 0.1000 = 10.00% | met | failed | approximate option is much faster but violates the 5% fidelity target |
| `q95_all` | yes | 1.04x | -74.9 ms | 0.0000 = 0.00% | failed | met | approximate option preserves fidelity but misses the fixed 1000 ms latency budget |

This is the core AQP benchmark shape: for the same task and context, cost,
fidelity, admission, and requirement satisfaction are separate outputs. The
paired runner is required before interpreting those numbers as ASAPQuery
quality evidence.

`Max relative error` is a fraction. The manifest target is `0.05`, meaning 5%,
so `0.0056` is about 0.56% relative error, not 0.0056%. `p95 delta` is
`ASAPQuery p95 - Prometheus p95`; negative values mean ASAPQuery was faster,
positive values mean it was slower.

## Remaining Gaps

This bundle is enough to make the core problem concrete, but it is not a
complete AQP benchmark suite yet.

1. Data-condition sweep is still shallow. The quickstart fixes one synthetic
   exporter setup. A fuller AQP benchmark should vary series count, label
   cardinality, group skew, pattern mix, scrape duration, and query timestamp.

2. Requirement policy is still simple. The current manifest uses one latency
   budget and one relative-error target for all queries. A better policy would
   allow per-query or per-task requirements, such as tighter error for
   ungrouped quantiles and separate rules for fallback queries.

3. Fidelity semantics are value-based, not PromQL-semantic complete. The
   importer compares numeric vector values by label set. It does not yet model
   PromQL range vectors, staleness, histogram samples, timestamps, NaN/Inf
   behavior, or query-step semantics.

4. The paired runner fixes timestamp drift for instant-vector comparisons, but
   it still uses a small quickstart workload and only three repetitions by
   default.

5. Resource cost is latency-only. The bundle does not yet import memory,
   sketch storage, CPU time, network bytes, or remote-write overhead.

6. Reproducibility metadata is partial. The manifest records file paths and
   option IDs, but the imported report should also record Docker image digests,
   ASAPQuery commit, sketch-bench commit, host architecture, Docker platform,
   and whether Grafana was omitted.

7. The connector is a script, not a typed connector registry. `run.sh` is a
   useful first connector, but AQP should eventually have a small connector
   schema for external benchmarks: startup, readiness, benchmark commands,
   expected outputs, import command, cleanup, and environment metadata.

8. The H2O + ClickHouse SQL path is planned separately in
   `AQP_H2O_CLICKHOUSE_CASE_STUDY.md`. It maps a different track, but it has
   not been reproduced end to end and should be treated as a gap, not as a
   current result.
