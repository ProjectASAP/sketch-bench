# AQP Benchmark MVP v1

## Big Design Goal

The long-term goal is a deployment-neutral benchmark contract for Approximate
Query Processing (AQP).

An AQP benchmark should let us compare different systems under the same
scenario family: high-level workload intent, benchmark context, workload model,
data-condition matrix, approximation requirements, exact baseline policy, and
option/config policy, even when those systems expose different query languages
or execution models.

Examples of systems under test include:

- ASAPQuery, which exposes PromQL-facing approximate query execution.
- asap-fusion, which works through DataFusion plans and SQL-like workloads.
- ASAPController-planned deployments, where the controller owns query-to-sketch
  planning and emits deployment-specific plans.
- Direct exact or sketch baselines used for controlled comparison.
- Future non-ASAP AQP systems.

The benchmark contract should define:

- workload intent, such as grouped cardinality, quantile, frequency, or top-k
- workload provenance, such as product benchmark, trace, generator, or hybrid
  synthesizer
- pressure dimensions, such as row count, cardinality, skew, group count,
  query range, concurrency, and seed
- approximation requirements, such as target error, latency budget, or memory
  budget
- exact reference execution used for accuracy comparison
- exact performance baseline used for cost comparison
- option/config policy for exact and approximate alternatives
- cost metrics, such as latency, throughput, CPU, memory, and IO
- accuracy metrics, such as relative error, rank error, worst-group error, and
  missing groups
- admission or fallback result, such as approximated, exact fallback, rejected,
  or unsupported
- normalized report schema that makes results comparable across systems

This is different from benchmarking one approximate query path. A benchmark of
an approximate query asks whether one fixed approximate execution is fast and
accurate. An AQP benchmark asks how a system behaves as an approximate query
processing system under controlled requirements and data conditions. One run is
a scenario point; the benchmark claim comes from the scenario family.

## Ownership Boundary

This project should not own general query-to-sketch planning.

SQL, PromQL, or DataFusion-to-sketch-algebra planning is a large control-plane
problem. It includes semantic lowering, sketch binding, exact fallback, parameter
selection, budget handling, and physical placement. That work belongs in
ASAPController or in deployment-specific planner code coordinated with
ASAPController.

`sketch-bench` should own the measurement contract:

- benchmark workload definitions
- exact reference and exact performance baseline definitions
- controlled data generation
- measurement and accuracy comparison
- normalized reports
- small local execution paths needed to validate the contract

It may have thin system bindings for ASAPQuery, asap-fusion, ASAPController, or
direct baselines later. Those bindings should call each system through its
natural interface. They should not reimplement each system's planner.

## Relationship To Existing ASAP Benchmarks

ASAPQuery already has product-specific benchmark and experiment infrastructure:

- `~/ASAPQuery/benchmarks/` runs a PromQL suite against Prometheus and the ASAP
  query engine, then compares latency and result fidelity.
- `~/ASAPQuery/asap-tools/experiments/` orchestrates deployment-shaped
  experiments, including services such as Prometheus, ClickHouse, query-engine,
  exporters, monitoring, workload generation, and post-analysis.

asap-fusion also has its own benchmark and experiment paths:

- `~/asap-fusion/microbench/` contains developer microbenchmarks.
- `~/asap-fusion/experiments/` measures DataFusion and asap-fusion execution
  paths over SQL/DataFusion-style workloads.

Those are reusable experiment tools, but they are not yet a shared AQP benchmark
definition. They are centered on their host systems and deployment shapes.

The reusable piece proposed here is the cross-system comparison contract:

```text
same task/context + same workload model + same data condition
  + same requirement + explicit baseline/config policy + normalized report
```

ASAPQuery's experiment tools may be one runner for that contract. asap-fusion's
experiments may be another. ASAPController may provide the plan for a third. The
benchmark should make their results comparable without moving product ownership
into `sketch-bench`.

## MVP Goal

MVP v1 is the smallest local proof of the benchmark contract.

It does not try to compare all systems yet. It proves that `sketch-bench` can
represent a few query-shaped workload intents, generate controlled data, run an
exact reference/performance baseline and one or more `asap_sketchlib`
approximate implementations, and emit a report that has the right shape for
later cross-system comparison.

The MVP question is:

```text
For an admitted query-shaped workload intent, how does an exact baseline compare
with an asap_sketchlib approximate implementation on the same generated data
condition and under the same requirements?
```

MVP v1 supports a small set of admitted aggregate shapes over a synthetic
`events` relation:

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

The SQL surface is an MVP convenience. It is an admission format for these
benchmark shapes, not a claim that `sketch-bench` owns SQL planning.

## MVP Flow

```text
admitted workload intent
  -> generated events data
  -> exact reference / exact baseline
  -> asap_sketchlib approximate implementation
  -> normalized cost and accuracy report
```

If SQL is accepted, DataFusion is used only as a local parsing/admission helper.
Execution is owned by the benchmark path. General SQL lowering and sketch
planning remain outside this framework.

## Current Slice

The current implementation already demonstrates the narrow local path:

- A SQL-shaped frontend using DataFusion parsing and logical planning.
- A narrow lowering path from supported aggregate shapes into AQP benchmark
  tasks.
- Three task families over `events`:
  - `COUNT(DISTINCT user_id)`, optionally grouped by `region`, backed by HLL.
  - `approx_median(value)` / `approx_percentile_cont(value, q)`, optionally
    grouped by `region`, backed by KLL.
  - `COUNT(*) GROUP BY user_id`, backed by CountMin and CountSketch.
- A synthetic `events` source with configurable row count, regions, user
  cardinality, distribution, skew, and seed.
- Exact oracles using sets, sorted value vectors, or exact count maps.
- Exact baselines currently reuse the same in-process exact structures as the
  reference execution.
- Sketch implementations using `asap_sketchlib` HyperLogLog, KLL, CountMin, and
  CountSketch.
- A JSON report with backend performance, rough memory estimates, and relative
  error summary.

This is enough to show:

```text
same intent + same generated data + exact/sketch implementations
  => cost and accuracy comparison
```

It is not enough to claim a full AQP benchmark yet.

## Gap To MVP Goal

The current slice still needs to become a clearer benchmark contract rather than
only a working demo path.

MVP v1 still needs:

- Explicit workload-intent names independent of SQL spelling.
- Scenario-family identifiers, even if each family starts with one condition.
- Workload provenance and pressure dimensions for generated data.
- Clear admission rules for each supported shape, so unsupported SQL fails
  clearly instead of running an unintended benchmark.
- A report schema that separates workload intent, workload provenance, data
  condition, system/backend, cost metrics, accuracy metrics, and
  admission/fallback status.
- Baseline policy that states when the exact reference and exact performance
  baseline are the same in-process structure.
- Option/config policy that records sketch parameters, config grids, and tuning
  or default-selection rules.
- Query-specific accuracy details, such as worst group for grouped aggregates,
  missing groups, rank error for quantiles, and heavy-hitter error for frequency
  tasks.
- A stable way to identify exact reference executions, exact performance
  baselines, and approximate implementations.

Useful follow-up work after MVP v1 includes:

- Real input sources, such as CSV, Parquet, Arrow batches, or telemetry traces.
- Multiple system bindings, such as ASAPQuery, asap-fusion, ASAPController, and
  direct sketch baselines.
- Budget concepts, such as target error, memory limit, or latency target.
- Sampling and hybrid approximate baselines.
- Shard, merge, and distributed execution benchmarks.
- Error modeling across composed operators.
- A dashboard or report consumer for comparing runs.

Follow-up work should not include an independent general SQL/DataFusion-to-sketch
planner inside `sketch-bench`.

## Success Criteria

MVP v1 is successful when a user can run multiple admitted workload intents and
get a clear normalized report comparing exact baseline results with
`asap_sketchlib` approximate results on the same controlled data.

The report should make the tradeoff visible enough to answer:

```text
For this workload intent and data condition, was the approximate implementation
faster, smaller, and accurate enough relative to the exact baseline/reference?
```

That gives `sketch-bench` a concrete MVP while keeping the long-term AQP
benchmark direction compatible with ASAPQuery, asap-fusion, and ASAPController.
