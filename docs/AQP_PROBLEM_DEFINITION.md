# AQP Bench Problem Definition

> Status: working definition. This document sets aside the broader MVP notes
> and defines my current mental modep AQP benchmark.

## Core Question

AQP benchmark is trying to know:

```text
For a specific analytical task in a specific benchmark context,
what exact and approximate options are available,
how do they behave under different data conditions and requirements?
```

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

## Definition

An AQP benchmark instance is:

```text
task T
+ benchmark context C
+ options O1..On
+ data conditions D1..Dm
+ ground truth G
+ approximation requirements A
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
- `options` are the exact and approximate alternatives available in that
  benchmark context.
- `data conditions` are controlled shapes of input data that may change which
  option is good.
- `ground truth` is the exact result used to judge fidelity.
- `approximation requirements` are error, latency, memory, confidence, or
  admission targets.
- `metrics` describe latency, accuracy, cost, and other behavior.

Different benchmark contexts should not be forced into one comparison. PromQL
and ES DSL can both contain aggregates, but they are not the same benchmark
because their data models and semantics are different.

## Tracks

A track fixes the benchmark context. Comparisons are meaningful within a track.

| Track | Benchmark context | Example options |
|---|---|---|
| SQL AQP | relational analytics through SQL | exact SQL engine, AQP DB, approximate aggregate functions, sampling middleware |
| PromQL AQP | telemetry/time-series queries through PromQL | Prometheus, ASAPQuery, exact fallback, approximate PromQL execution |
| DataFusion AQP | embedded query engine/operators through DataFusion plans | DataFusion exact operators, asap-fusion operators, approximate UDAFs |
| ES aggregation AQP | document/index aggregation serving through Elasticsearch DSL | Elasticsearch reference, sketch-backed ES-compatible service |
| Sketch primitive AQP | direct aggregate primitives through aggregate/data-structure API | exact map/set/vector, HLL, KLL, CountMin, CountSketch, SpaceSaving |
| Stream/window AQP | streaming analytics through stream/window API | exact window state, sampling, sketches, approximate join summaries |

The tracks may share vocabulary, data generators, ground-truth code, metrics,
and report schema. They should not share a fake universal query interface.

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

## Data Conditions

AQP behavior is data-sensitive. One workload is not enough.

The benchmark should sweep controlled data conditions, such as:

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

The point is not only to generate realistic data. The point is to expose where
an approximate option breaks, dominates, or becomes irrelevant.

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

DataConditionSpec
  generator or trace
  shape parameters
  seed
  sweep dimensions

RequirementSpec
  fidelity targets
  cost budgets
  confidence targets

RunRecord
  task
  track
  option
  data condition
  requirement
  admission status
  cost metrics
  fidelity metrics
  ground truth id
```

This gives `sketch-bench` a concrete role: generate data conditions, run exact
ground truth and approximate options, collect cost/fidelity/admission records,
and produce the plots that reveal the metrics set.

## What `sketch-bench` Should Own

`sketch-bench` should own:

- task definitions
- data-condition generation and sweeps
- ground-truth methods
- direct sketch-vs-exact option runners
- shared metric definitions
- admission status vocabulary
- normalized run records
- plotting/report inputs for metrics set

## MVP v3

MVP v3 should prove the task-centered benchmark shape before adding external
systems.

Scope:

```text
track: Sketch primitive AQP
benchmark context: direct aggregate primitives through aggregate/data-structure API
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
task
track
option
data condition
requirement
admission status
cost metrics
fidelity metrics
ground truth id
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
- dashboard polish
