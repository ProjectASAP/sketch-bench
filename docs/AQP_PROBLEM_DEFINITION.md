# AQP Bench Problem Definition

> Status: working definition. This document sets aside the broader MVP notes
> and defines my current mental model for an AQP benchmark.

## Core Question

AQP benchmark is trying to know:

```text
For a specific analytical task in a specific benchmark context,
what exact and approximate options are available,
how do they behave under different data conditions and requirements?
```

That question should be answered by a scenario suite, not by one benchmark row.
Recent PVLDB benchmark papers in `docs/vldb_benchmark_papers_2021_2025.md`
make the same point in different domains: fixed legacy workloads miss
production behavior, and good benchmarks expose workload pressure, data
conditions, baselines, and failure modes explicitly. For AQP this means a
benchmark must be able to show where exact execution wins, where approximation
wins, and where the approximate path should be rejected.

The output is a set of metrics:

```text
task x benchmark context x data condition x requirement x option
  => cost, fidelity, admission, and failure behavior
```

The benchmark is useful if it can explain boundaries such as:

- exact execution is still best below this cardinality
- sampling fails under this skew or rare-key regime
- this sketch is memory-efficient but only for this error target
- this approximate function meets the latency target but misses too many groups
- this system admits only this subset of the task
- this option is Pareto-dominated and should not be used

## Current Landscape: Approximate Support Is Fragmented

The right claim is not that modern data systems lack approximate query
functionality. Many popular systems expose approximate aggregate functions,
sketch-state functions, or sketch-backed extensions.

Examples as of July 2026:

- Apache DataFusion documents `approx_distinct`, `approx_median`,
  `approx_percentile_cont`, and `approx_percentile_cont_with_weight`; its
  approximate percentile path is described as using t-digest.
  <https://datafusion.apache.org/user-guide/sql/aggregate_functions.html>
- Trino documents approximate aggregate functions including
  `approx_distinct`, `approx_most_frequent`, `approx_percentile`, and
  `numeric_histogram`, plus HyperLogLog state functions such as `approx_set`
  and `merge`.
  <https://trino.io/docs/current/functions/aggregate.html>
- Spark SQL documents approximate and sketch-related functions including
  `approx_count_distinct`, `approx_percentile`, `percentile_approx`,
  `count_min_sketch`, HLL sketch functions, and KLL sketch aggregate/merge/
  query functions.
  <https://spark.apache.org/docs/latest/api/sql/index.html>
- Google BigQuery documents approximate aggregate functions including
  `APPROX_COUNT_DISTINCT`, `APPROX_QUANTILES`, `APPROX_TOP_COUNT`, and
  `APPROX_TOP_SUM`.
  <https://docs.cloud.google.com/bigquery/docs/reference/standard-sql/approximate_aggregate_functions>
- ClickHouse documents approximate aggregate functions and sketch-backed
  distinct-count variants including `uniq`, `uniqCombined`, `uniqHLL12`,
  `uniqTheta`, and quantile variants such as t-digest and GK-family functions.
  <https://clickhouse.com/docs/sql-reference/aggregate-functions/reference>
- Snowflake documents HLL, MinHash/similarity, approximate top-k, and
  approximate percentile functionality, including accumulate/combine/estimate
  style functions for some approximate states.
  <https://docs.snowflake.com/en/sql-reference/functions-aggregation>
- Apache Druid documents approximate aggregations, including DataSketches
  Theta, HLL, and Quantiles aggregators, and explicitly discusses when older
  approximate histogram/cardinality implementations are less suitable.
  <https://druid.apache.org/docs/latest/querying/aggregations/>
- Apache DataSketches itself is a production-quality sketch library rather than
  a complete AQP system. It provides sketch implementations and adaptors for
  systems such as Hive, Pig, PostgreSQL, BigQuery, and Druid, with cross-language
  implementations and binary compatibility goals.
  <https://datasketches.apache.org/>
  <https://datasketches.apache.org/docs/Architecture/SketchesByComponent.html>

The more accurate problem statement is therefore:

```text
Approximate query support is common, but fragmented across system-specific
functions, sketch-state APIs, UDFs/UDAFs, extensions, and standalone sketch
libraries. There is no widely adopted general-purpose AQP system abstraction,
and there is no clear benchmark methodology for evaluating how concrete sketch
implementations support user-level approximate query functionality.
```

This fragmentation creates the gap AQP Bench should care about:

```text
user-level task/functionality
  such as approximate distinct, quantile, top-k, histogram, grouped aggregate

system-specific approximate function or sketch-state API
  such as approx_percentile, approx_set/merge, HLL_ACCUMULATE/COMBINE/ESTIMATE,
  sketch UDF/UDAF, or extension aggregator

raw sketch implementation
  such as HLL, KLL, t-digest, Count-Min, Theta, or sampling summary
```

Existing sketch benchmarks mostly evaluate the last layer directly. Existing
database benchmarks mostly evaluate a concrete system or query engine. The hard
benchmark-design question is how to compare choices across the middle gap
without pretending there is one universal AQP system interface.

## Current AQPBMV2 Boundary

This document describes the broader AQP benchmark problem. AQPBMV2 is now a
narrower design point inside that space.

Current AQPBMV2 should not try to benchmark all AQP systems or all AQP query
interfaces. Its concrete target is:

```text
executable approximate function candidate
+ exact or sketch-backed state implementation
+ fixed parameter setting
+ thin sketch binding when the candidate uses a sketch
```

AQPBMV2 should evaluate that target through benchmark-defined function kernels,
not through SQL, PromQL, DataFusion, or a database optimizer. SQL-like examples
are useful for explaining motivation, but they should not be part of the V2
benchmark definition.

The executable middle layer should be code, not only a manifest:

```rust
trait ApproxFunction {
    type Input;
    type State;
    type Output;
    type Query;

    fn create(&self) -> Self::State;
    fn update(&self, state: &mut Self::State, input: Self::Input);
    fn merge(&self, left: &mut Self::State, right: Self::State);
    fn finalize(&self, state: &Self::State, query: Self::Query) -> Self::Output;
}
```

Examples:

```text
ExactCountDistinct<HashSet>
HllCountDistinct<HllImpl>
ExactQuantile<Vec>
KllQuantile<KllImpl>
TDigestQuantile<TDigestImpl>
ExactTopK<HashMap>
SpaceSavingTopK<SpaceSavingImpl>
```

The V2 kernel set should start with:

- single-state function kernel: one state over one stream;
- grouped-state function kernel: many logical states and many answers;
- partitioned-merge function kernel: partial states plus explicit benchmark-owned
  merge shape.

This makes AQPBMV2 compatible with the rule that a benchmark must have a
concrete unit under test. It also avoids a false claim that V2 benchmarks a
complete AQP system. PromQL, SQL, DataFusion, and deployment resource
accounting remain broader or later-scope scenario-family work.

The research question for AQPBMV2 is therefore narrower than the full AQP
question:

```text
Do single-state sketch benchmark conclusions predict behavior under
AQP-style kernels with many states, many answers, grouping, and explicit merge
shape?
```

If the answer is no, AQPBMV2 can contribute a workload-kernel-level benchmark
methodology for sketch-backed approximate function candidates. If the answer is
yes, V2 is mainly an engineering toolkit and the broader AQP benchmark must
find its contribution elsewhere.

## Definition

An AQP scenario family is:

```text
task T
+ benchmark context C
+ workload model W
+ options O1..On
+ option/config policy K
+ data conditions D1..Dm
+ ground truth G
+ approximation requirements A
+ resource accounting R
+ comparison protocol P
+ metrics M
=> output metric set
```

Where:

- `task` is the analytical problem, such as frequency aggregation, quantile,
  count distinct, top-k, histogram, group-by aggregation, or window join.
- `benchmark context` fixes both the system/domain and the request interface
  options must share, such as SQL analytics through SQL, PromQL
  telemetry through PromQL, Elasticsearch aggregation through ES DSL,
  DataFusion operators through DataFusion plans, stream processing through a
  stream/window API, or direct aggregate primitives through a data-structure
  API.
- `workload model` states where the task and requests come from: a real
  workload trace, a product benchmark, a controlled generator, or a hybrid
  synthesizer. It also states which production pressure the benchmark is trying
  to model, such as high cardinality, dashboard concurrency, drift, rare
  groups, large windows, or retention pressure.
- `options` are the exact and approximate alternatives available in that
  benchmark context.
- `option/config policy` states which configurations are tried, how defaults
  are chosen, whether tuning/calibration data is allowed, and how much tuning
  budget each option receives.
- `data conditions` are controlled shapes of input data that may change which
  option is good.
- `ground truth` is the exact result used to judge fidelity.
- `approximation requirements` are error, latency, memory, confidence, or
  admission targets.
- `resource accounting` states which exact and approximate costs are counted
  and which are known gaps.
- `comparison protocol` states how runs are paired, how timestamps/windows are
  aligned, how groups are matched, and how numeric edge cases are handled.
- `metrics` describe latency, accuracy, cost, and other behavior.

Different benchmark contexts should not be forced into one comparison. PromQL
and ES DSL can both contain aggregates, but they are not the same benchmark
because their data models and semantics are different.

A full benchmark should define a scenario family:

```text
one task/context/workload model
  x multiple data conditions
  x exact and approximate options
  x explicit requirements
```

A single scenario is useful as a smoke test. It is not enough for a benchmark
claim unless it is clearly labeled as one point in the family.

## Tracks

A track fixes the benchmark context. Comparisons are meaningful within a track.

| Track | Benchmark context | Example options |
|---|---|---|
| SQL AQP | relational analytics through SQL | exact SQL engine, AQP DB, approximate aggregate functions, sampling middleware |
| PromQL AQP | telemetry/time-series queries through PromQL | Prometheus, ASAPQuery, exact fallback, approximate PromQL execution |
| DataFusion AQP | embedded query engine/operators through DataFusion plans | DataFusion exact operators, asap-fusion operators, approximate UDAFs |
| ES aggregation AQP | document/index aggregation serving through Elasticsearch DSL | Elasticsearch reference, sketch-backed ES-compatible service |
| Sketch kernel AQP | benchmark-owned single-state, grouped-state, and partitioned-merge kernels over direct aggregate/data-structure APIs | exact map/set/vector, HLL, KLL, CountMin, CountSketch, SpaceSaving |
| Stream/window AQP | streaming analytics through stream/window API | exact window state, sampling, sketches, approximate join summaries |

The tracks may share vocabulary, data generators, ground-truth code, metrics,
and report schema. They should not share a fake universal query interface.
Within a track, the scenario family owns the workload model and data-condition
matrix. The track only says which native interface and option types are
admissible. For AQPBMV2, the sketch-kernel track also owns the execution shape:
the benchmark explicitly defines grouping and merge kernels instead of relying
on a SQL optimizer or external query engine to choose a plan.

## Primary Concrete Example

The primary ASAPQuery integration example is the PromQL quickstart bundle:

```text
sketch-bench/aqp-workloads/asapquery/promql_quickstart/
```

It runs ASAPQuery's existing Docker quickstart, uses a `sketch-bench`
paired-timestamp PromQL runner against Prometheus and ASAPQuery, then imports
the JSON reports into AQP records. The paired runner preserves ASAPQuery's
PromQL API and report shape, but sends the same PromQL `time=` value to both
systems for each query repetition. This is the preferred concrete example
because it is runnable from the public quickstart path and it produced a
concrete AQP report:

```text
sketch-bench/aqp-workloads/asapquery/promql_quickstart/latest_report.jsonl
```

The latest local run imported 42 AQP records:

- 14 Prometheus baseline option runs
- 14 ASAPQuery option runs
- 14 paired comparisons

The case fills the AQP dimensions with specific values:

- `task`: PromQL aggregates and grouped quantiles over `sensor_reading`.
- `benchmark context`: Prometheus-compatible telemetry query serving.
- `options`: Prometheus exact baseline, ASAPQuery exact fallback, and ASAPQuery
  native approximate execution.
- `data condition`: deterministic fake exporters with multiple pattern groups.
- `requirements`: p95 latency budget, relative-error target, and result-series
  matching.
- `metrics`: latency, speedup, numeric error, missing/extra label sets,
  admission status, and requirement satisfaction.

The PromQL quickstart should be read as the seed scenario for this definition,
not as the full benchmark suite. It gives the benchmark a real external system
boundary:

```text
PromQL task x telemetry context x fake-exporter data condition
  x latency/error requirements
  x {Prometheus, ASAPQuery exact fallback, ASAPQuery native approximate}
  => latency, fidelity, admission, and failure behavior
```

That example fills in the previously vague pieces as follows:

- Metrics are no longer abstract: the imported record contains p95 latency,
  p95 speedup, relative error, label-set mismatch counts, and per-requirement
  booleans.
- Failure behavior is no longer just a note: each query records `baseline`,
  `exact_fallback`, `approximated`, `option_error`, or missing counterpart
  behavior.
- Requirements are checked per dimension, so a query can be latency-good but
  fidelity-bad, or fidelity-good but latency-bad.
- Fidelity is measured against the same query timestamp for both options; the
  runner also emits `paired_diagnostics.json` so timestamp alignment can be
  checked explicitly.
- The output is inspectable data, not only a narrative: the bundle includes a
  sample AQP JSONL report and a runner that can regenerate it.

What it does not yet provide is the broader scenario family. It still needs a
matrix over series count, label cardinality, query range, pattern mix, drift,
burstiness, and query concurrency before it can support a general claim about
PromQL AQP behavior.

Concrete rows from the latest paired run show the point:

| Query | ASAP native | p95 speedup | p95 delta | Max relative error | Latency target | Fidelity target |
|---|:---:|---:|---:|---:|:---:|:---:|
| `q95_by_pattern` | yes | 175.56x | -2098.0 ms | 0.0056 = 0.56% | met | met |
| `q99_by_pattern` | yes | 429.59x | -2153.5 ms | 0.0066 = 0.66% | met | met |
| `q50_by_pattern` | yes | 396.46x | -2303.4 ms | 0.1000 = 10.00% | met | failed |
| `q95_all` | yes | 1.04x | -74.9 ms | 0.0000 = 0.00% | failed | met |

These rows are useful precisely because they do not collapse into one score.
The grouped `q95` and `q99` quantiles are much faster and satisfy the 5%
fidelity target. The grouped `q50` quantile is also much faster but misses that
target in this run. The ungrouped `q95_all` row preserves fidelity but misses
the fixed 1000 ms latency budget. That is the kind of boundary an AQP benchmark
is meant to expose.

`Max relative error` is reported as a fraction, not as a percent. The target in
this manifest is `0.05`, i.e. 5%. Therefore `0.0056` means about 0.56% relative
error and meets the fidelity target, while `0.1000` means 10.00% and fails.
`p95 delta` is `ASAPQuery p95 - Prometheus p95`, so negative values mean
ASAPQuery was faster and positive values mean it was slower.

The older sequential ASAPQuery scripts chose one fresh `time=now` for the
baseline pass and a later `time=now` for the ASAPQuery pass. On time-varying
fake-exporter data, that can inflate relative error by comparing Prometheus at
time `T` with ASAPQuery at `T + gap`. The legacy high-error rows around 75% to
95% should be read as evidence that the benchmark protocol needed pairing, not
as the headline ASAPQuery result.

## Planned SQL Example

The H2O + ClickHouse path is planned as a SQL-track example, documented in
`docs/AQP_H2O_CLICKHOUSE_CASE_STUDY.md`. It is not a current reproduced result.
It still needs an end-to-end run, full result values for fidelity comparison,
and a clear baseline/ASAP pairing invariant before it can be presented like the
PromQL quickstart.

## Tasks

Tasks are the center of the benchmark. A track chooses which tasks it supports.

Initial task candidates:

| Task | Exact option | Approximate options | Important data conditions |
|---|---|---|---|
| frequency aggregation | exact count map | CountMin, CountSketch, sampling, heavy-hitter sketches | cardinality, skew, tail mass, signed updates, update/query ratio |
| count distinct | exact set | HLL, sampling, bitmap variants | cardinality, duplicate rate, group count, small vs large groups |
| quantile | sorted values or exact order statistic | KLL, t-digest, sampling | distribution shape, tails, group size, query quantile |
| top-k | exact count map plus sort | SpaceSaving, CountMin+heap, CountSketch+heap, sampling | skew, near-ties, tail size, k |
| histogram | exact bucket counts | fixed buckets, adaptive histograms, sketches | bucket count, range skew, outliers |
| group-by aggregation | exact grouped aggregate | sketches per group, samples, approximate aggregate functions | group count, group skew, missing small groups |
| window join | exact windowed join state | sampling, sketches/summaries, approximate join policies | window size, join-key skew, correlation, out-of-order data |

This table is not a final workload list. It is a way to make the problem
concrete: each task has different good and bad approximate options.

## Options

An option is one way to answer the task in the chosen benchmark context.

Examples:

- exact/traditional execution
- exact data structure baseline
- approximate aggregate function
- sampling method
- sketch method
- synopsis or precomputed summary
- approximate planner policy
- exact fallback policy
- hybrid exact-plus-approximate strategy

Options should include exact baselines. Without exact baselines, the benchmark
cannot show whether approximation is worthwhile.

The exact baseline should be treated as a design object, not as a default
implementation detail. PVLDB benchmark papers that criticize standard
benchmarks or synthesize production-like workloads are usually reacting to weak
or stale baselines. AQP Bench should therefore record:

- the exact reference used for fidelity;
- the exact performance baseline used for cost comparison;
- whether those are the same system or different systems;
- why the exact performance baseline is considered strong enough for the
  scenario.

Approximate options need the same discipline. A benchmark should not compare a
carefully tuned exact system against an arbitrary sketch configuration, or a
heavily tuned approximate system against a default exact baseline. Each scenario
family should record:

- the configuration grid or default configuration policy;
- whether the option is evaluated at one fixed config, best-of-grid, or
  requirement-minimal config;
- any calibration/training data used to choose parameters;
- tuning budget and stopping rule;
- whether the same policy is applied to all comparable options.

## Data Conditions

AQP behavior is data-sensitive. One workload is not enough.

The benchmark should define data conditions at two levels.

First, it should state workload provenance:

- real trace or product benchmark;
- controlled synthetic generator;
- hybrid generator fitted to real statistics;
- manually curated edge-case workload.

Second, it should sweep controlled data conditions, such as:

- row count or stream length
- cardinality
- duplicate rate
- Zipf/skew parameter
- heavy-hitter strength
- tail size
- group count
- group-size skew
- selectivity
- update/query ratio
- burstiness
- time-window size
- join-key skew
- correlation between fields
- out-of-order or late events

Every data condition should have an identifier, seed or source version,
generation/replay policy, and pressure dimensions. The point is not only to
generate realistic data. The point is to expose where an approximate option
breaks, dominates, or becomes irrelevant.

This is also where AQP should differ from a narrow microbenchmark. A useful
suite includes both controlled breakpoints and workload-like conditions. The
controlled cases explain why a result changes; the workload-like cases test
whether the breakpoints matter under a realistic query and data mix.

## Requirements

AQP is defined by requirements on answer fidelity and resource cost.

Examples:

- relative error <= 1%, 5%, or 10%
- rank error <= target
- precision/recall@k >= target
- confidence interval coverage >= target
- latency <= target
- memory <= budget
- storage <= budget
- candidate must not miss groups above a threshold

The benchmark should record whether each option satisfies the requirement, not
only its raw latency or raw error.

## Metrics

Metrics should be task-specific but comparable in structure.

Cost metrics:

- build time
- update throughput
- query latency
- CPU time
- memory
- storage
- scan bytes
- network bytes
- merge cost
- maintenance cost

Fidelity metrics:

- absolute error
- relative error
- rank error
- confidence interval width and coverage
- missing groups
- false positive or false negative keys
- precision/recall@k
- ordering error

Admission and behavior metrics:

- exact
- approximate
- exact fallback
- unsupported
- rejected
- timeout
- requirement met
- requirement violated

## Outputs

AQP Bench should produce data that supports analysis, not just a score.

Useful outputs:

- cost-error curves
- Pareto frontiers
- heatmaps over data conditions
- requirement satisfaction regions
- admission/fallback matrices
- break-even points against exact execution
- sensitivity plots for skew, cardinality, group count, or window size
- per-task summaries of when each option is appropriate
- coverage matrices showing which tasks, data conditions, and options were
  actually exercised
- baseline-strength notes explaining whether the exact comparison is complete,
  partial, or still weak

Example conclusion shape:

```text
For frequency aggregation, exact maps dominate at low cardinality.
CountMin is efficient under positive counts and strong skew, but loses accuracy
for rare keys. Sampling fails when heavy-tail rare keys matter. CountSketch is
worth considering when signed/noisy updates are part of the task.
```

Another example:

```text
For window joins, sampling reduces cost only when join-key skew and correlation
are mild. Under skewed keys or large windows, exact state or task-specific
summaries may be necessary to meet the error requirement.
```

## Component Design

The benchmark can be built from these components:

```text
ScenarioFamilySpec
  track
  value hypothesis
  workload provenance
  pressure dimensions
  condition matrix
  option/config policy

TaskSpec
  name
  result shape
  admitted operations
  ground truth method
  task-specific fidelity metrics

TrackSpec
  benchmark context
  candidate option set
  binding/admission rules
  non-goals

WorkloadModelSpec
  query/request source
  generator or replay policy
  production pressure being modeled
  workload version

DataConditionSpec
  generator or trace
  shape parameters
  seed
  sweep dimensions

BaselineSpec
  exact reference execution
  exact performance baseline
  baseline-strength note

OptionPolicySpec
  configuration grid
  default selection rule
  tuning budget
  calibration data
  fairness notes

RequirementSpec
  fidelity targets
  cost budgets
  confidence targets

ResourceAccountingSpec
  base exact costs
  incremental approximate costs
  effective serving costs
  missing dimensions

ComparisonProtocolSpec
  time/window pairing
  group/key matching
  numeric error rules
  repeated-run policy

RunRecord
  scenario family
  task
  track
  option
  data condition
  requirement
  admission status
  cost metrics
  fidelity metrics
  exact reference id
```

This gives `sketch-bench` a concrete role: generate data conditions, run exact
reference/baseline and approximate options, collect cost/fidelity/admission
records, and produce the plots that reveal the metrics set.

## What `sketch-bench` Should Own

`sketch-bench` should own:

- task definitions
- scenario-family manifests
- data-condition generation and sweeps
- workload provenance metadata
- ground-truth methods
- exact-baseline strength metadata
- option/config sweep policy
- direct sketch-vs-exact option runners
- shared metric definitions
- admission status vocabulary
- comparison protocol validation
- resource-accounting gap tracking
- normalized run records
- plotting/report inputs for metrics set

<!-- ## MVP v3

MVP v3 should prove the scenario-family benchmark shape before adding external
systems.

Scope:

```text
scenario family: Sketch primitive AQP break-even suite
track: Sketch primitive AQP
benchmark context: direct aggregate primitives through aggregate/data-structure API
workload model: controlled synthetic generator
pressure dimensions: stream length, cardinality, skew, group count, update/query ratio
option/config policy: fixed default first, then declared config grid
```

Tasks:

- frequency aggregation
- count distinct
- quantile
- top-k

Options:

- exact count map, exact set, exact sorted values, exact top-k
- CountMin
- CountSketch
- HLL
- KLL
- SpaceSaving or CountMin+heap for top-k

Data conditions:

- row count or stream length
- cardinality
- Zipf/skew parameter
- heavy-hitter strength
- group count where applicable
- update/query ratio where applicable
- seed

Requirements:

- memory budget
- relative error target
- rank error target for quantile
- precision/recall@k target for top-k

Run output:

```text
scenario family
task
track
option
workload provenance
data condition
baseline policy
option/config policy
requirement
admission status
cost metrics
fidelity metrics
exact reference id
```

Required analysis outputs:

- cost-error curves per task
- memory-error curves per task
- heatmaps over skew and cardinality
- requirement satisfaction tables
- break-even points against exact data structures
- one short conclusion per task describing when each option is appropriate

MVP v3 success means we can make claims like:

```text
For frequency aggregation, exact maps dominate at low cardinality.
CountMin becomes useful under memory pressure and positive-count workloads, but
rare-key accuracy degrades under heavy tails.
```

and:

```text
For count distinct, HLL is useful once exact sets exceed the memory budget, but
small groups remain better served by exact sets.
```

Non-goals:

- SQL, PromQL, DataFusion, or Elasticsearch bindings
- cross-track comparisons
- a leaderboard
- distributed execution
- planner evaluation
- dashboard polish -->
