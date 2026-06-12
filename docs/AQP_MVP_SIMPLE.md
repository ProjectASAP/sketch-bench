# AQP Benchmark MVP v1

## Purpose

MVP v1 defines the smallest useful version of query-level AQP benchmarking in
`sketch-bench`.

The goal is not to build a full approximate query engine yet. The goal is to
prove that the benchmark can start from a SQL query, lower it into a benchmark
task, run that task with an exact implementation and an approximate
`asap_sketchlib` implementation, then report the cost and accuracy tradeoff.

## End Goal

The MVP should answer this question:

```text
For a supported SQL query, how does exact execution compare with an
asap_sketchlib approximate execution on the same data?
```

MVP v1 supports a small set of admitted aggregate query shapes. The implemented
set is intentionally sketch-shaped rather than general SQL:

```sql
SELECT region, COUNT(DISTINCT user_id) AS users
FROM events
GROUP BY region;

SELECT region, approx_median(value) AS p50_value
FROM events
GROUP BY region;

SELECT approx_percentile_cont(value, 0.95) AS p95_value
FROM events;

SELECT user_id, COUNT(*) AS frequency
FROM events
GROUP BY user_id;
```

For each supported query, the benchmark should produce a report that shows:

- exact execution throughput
- exact execution memory estimate
- approximate execution throughput
- approximate execution memory estimate
- approximate accuracy compared with the exact result
- query-specific error details, such as worst group for grouped aggregates

This moves the project from benchmarking an isolated sketch operation to
benchmarking small approximate query plans.

## MVP Flow

```text
SQL query
  -> DataFusion logical plan
  -> AQP task
  -> generated events data
  -> exact backend for that task
  -> one or more asap_sketchlib approximate backends for that task
  -> throughput, memory, and accuracy report
```

DataFusion is used for SQL parsing and logical planning only. The MVP execution
path is owned by `sketch-bench`.

## What We Currently Have

The current slice establishes the core shape of the MVP:

- A SQL frontend using DataFusion parsing and logical planning.
- A narrow lowering path from a supported aggregate plan into an AQP task.
- Three task families over `events`:
  - `COUNT(DISTINCT user_id)`, optionally grouped by `region`, backed by HLL.
  - `approx_median(value)` / `approx_percentile_cont(value, q)`, optionally
    grouped by `region`, backed by KLL.
  - `COUNT(*) GROUP BY user_id`, backed by CountMin and CountSketch.
- A synthetic `events` source with configurable row count, regions, user
  cardinality, distribution, skew, and seed.
- Exact backends using sets, sorted value vectors, or exact count maps.
- Sketch backends using `asap_sketchlib` HyperLogLog, KLL, CountMin, and
  CountSketch.
- A JSON report with backend performance, rough memory estimates, and relative
  error summary.

This is enough to demonstrate the basic MVP claim:

```text
same query + same data + different exact/sketch backend => cost/error comparison
```

## What Is Missing For MVP v1

The current slice is still intentionally narrow. The remaining work is not
general SQL support; it is making the admitted query set and report schema more
useful while keeping each query shape explicit and benchmarkable.

MVP v1 still needs:

- Clear admission rules for each supported query shape, so unsupported SQL fails
  clearly instead of running an unintended benchmark.
- A report format that can describe multiple query/task types and multiple
  sketch backends consistently.
- Additional admitted shapes such as filtered distinct-count queries or top-k,
  if they have a clear exact backend, sketch backend, and accuracy metric.

Larger follow-up work, likely after MVP v1, includes:

- Real input sources, such as CSV, Parquet, Arrow batches, or telemetry traces.
- DataFusion physical execution as a backend or baseline.
- Sampling and hybrid approximate backends.
- Budget concepts, such as target error, memory limit, or latency target.
- Stable report schema for downstream dashboards or comparisons.
- Shard, merge, and distributed execution benchmarks.
- Error modeling across composed operators.

The line for MVP v1 should be multiple supported query shapes, not general SQL.
Each supported shape should have a known exact baseline, a known approximate
backend, and a reportable accuracy metric.

## MVP Boundary

MVP v1 should stay focused on a small number of complete paths:

```text
supported SQL query -> exact result -> asap_sketchlib estimate -> benchmark report
```

Unsupported SQL should fail clearly. The benchmark should avoid implying that
it supports general SQL, general AQP planning, or production query execution.

## Success Criteria

MVP v1 is successful when a user can run multiple supported SQL query shapes and
get a clear comparison between exact execution and the matching
`asap_sketchlib` approximate execution on the same generated data.

The report should make the tradeoff visible enough for the user to understand
whether the approximate backend is faster, smaller, and accurate enough for the
workload being tested.
