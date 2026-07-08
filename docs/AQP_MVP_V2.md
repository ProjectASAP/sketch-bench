# AQPBMV2 Design: Executable Approximate Function Benchmark

> Status: active design direction after the July 2026 discussion.
>
> This document intentionally narrows MVP v2. Earlier drafts treated v2 as a
> broad AQP scenario-family framework that could include sketch primitives,
> PromQL systems, SQL systems, DataFusion plans, and resource-accounting
> studies. That broader direction remains useful, but it is not the current
> AQPBMV2 scope.

## One-Sentence Scope

AQPBMV2 benchmarks executable approximate function candidates under canonical
AQP-style kernels, measuring what is gained and lost when exact aggregate state
is replaced by sketch-backed state.

## The Problem

Most sketch benchmarks can be reduced to this shape:

```text
one sketch instance
+ one value stream
+ one query or small query set
=> error, throughput, latency, memory
```

For example:

```text
N = 10M values
candidate = KLL(k=200)
query = p99
measure = rank error, value error, time, memory
```

That is a valid sketch benchmark. It is not enough to answer the AQP-facing
question:

```text
Which concrete implementation of a user-level approximate function is suitable
for which workload shape?
```

Approximate query workloads often involve many logical aggregate states and
many answers, not only one sketch over one stream. A grouped quantile workload,
for example, creates one quantile state per group. A distributed execution
shape creates partial states and merges them. Under those shapes, the important
result is not just "what was the p99 error for one stream?" but:

- how many answers met the declared fidelity requirement;
- which groups or partitions failed;
- whether failures concentrate in small groups, skewed groups, tail-heavy
  groups, or filtered subsets;
- whether the candidate ranking changes under group skew, partition count,
  merge shape, or input distribution;
- how much workload coverage the candidate gives under the same parameter or
  budget.

The current AQPBMV2 hypothesis is:

```text
Single-state sketch benchmarks are insufficient to predict sketch behavior for
AQP-style workloads with many states, many answers, grouping, and merging.
```

AQPBMV2 should test that hypothesis rather than assume it.

## Unit Under Test

The benchmark target must be concrete. AQPBMV2 does not benchmark the abstract
idea of AQP, SQL, or a sketch algorithm in a paper.

The unit under test is:

```text
approximate function candidate
= functionality spec + state implementation + fixed parameters
```

Examples:

```text
HllCountDistinct<HllImpl, precision=14>
KllQuantile<KllImpl, k=200>
TDigestQuantile<TDigestImpl, compression=100>
SpaceSavingTopK<SpaceSavingImpl, k=100>
```

The raw sketch is not the benchmark target. The target is executable code that
implements a user-level approximate functionality, such as approximate distinct
count, approximate quantile, or approximate top-k.

The minimal executable middle layer should look like this:

```rust
trait ApproxFunction {
    type Input;
    type State;
    type Output;
    type Query;

    fn create(&self) -> Self::State;
    fn update(&self, state: &mut Self::State, input: Self::Input);
    fn merge(&self, left: &mut Self::State, right: Self::State);
    fn finalize(&self, state: &mut Self::State, query: Self::Query) -> Self::Output;
}
```

Then AQPBMV2 can provide paired exact and sketch-backed implementations:

```text
ExactCountDistinct<HashSet>
HllCountDistinct<HllImpl>

ExactQuantile<Vec>
KllQuantile<KllImpl>
TDigestQuantile<TDigestImpl>

ExactTopK<HashMap>
SpaceSavingTopK<SpaceSavingImpl>
```

This is the middle layer. It is code, not a config file or protocol sketch.
It is also not a system-specific SQL function. A system function such as
Spark's `approx_count_distinct` or Trino's `approx_percentile` can motivate the
functionality class, but AQPBMV2 does not claim to benchmark that system's
implementation unless the implementation is exposed as an executable candidate.

The thin binding still exists underneath this layer so a candidate can call a
particular sketch library. That binding is measurement glue. It must not add
policy such as fallback to exact state, adaptive parameter tuning, custom
window rebuilds, error correction, or rewritten merge behavior. If those
policies are added, the benchmark target becomes "adapter policy plus sketch",
which is a different project.

## Candidate Selection Model

AQPBMV2 should support approximate-function candidate selection, not only
produce benchmark numbers. A user choosing between candidates such as
`HllCountDistinct<DataSketchesHll>` and `HllCountDistinct<SketchlibHll>` needs
more than one error/throughput row. The benchmark output should therefore
separate three kinds of evidence.

### 1. Declared Adoption Metadata

These fields describe whether a candidate is practical to adopt. They are not
proved by the kernel benchmark, but they should be recorded so comparisons are
interpretable:

- function families implemented, such as approximate distinct count, quantile,
  frequency, or top-k;
- sketch families used by each function candidate;
- implementation language and runtime requirements;
- public API shape and whether a thin binding is straightforward;
- native support for merge, serialization, deserialization, and reset;
- supported input types and query parameters;
- build, packaging, and dependency footprint;
- license and redistribution constraints;
- documentation, examples, and maintenance status;
- cross-language or cross-process portability if serialized states are meant
  to move across boundaries.

The design rule is that adoption metadata must be labeled as declared or
manually assessed. AQPBMV2 should not pretend that documentation quality or API
ergonomics are measured by a sketch kernel.

### 2. Measured Primitive Behavior

These are conventional sketch/function-state measurements. AQPBMV2 should keep
them because they explain the base behavior of the concrete candidate:

- update throughput;
- query latency;
- merge latency where merge is native;
- memory footprint or serialized state size;
- raw fidelity metrics such as rank error, value error, count error, or
  precision/recall;
- parameter sensitivity under single-state workloads.

This layer answers:

```text
How does the concrete function-state implementation behave as a primitive?
```

### 3. Measured AQP-Kernel Behavior

This is the AQPBMV2-specific layer. It measures whether primitive-level
behavior remains useful when the benchmark creates many states, many answers,
grouping pressure, and merge pressure:

- answer coverage across all logical outputs;
- failure localization by group size, tail region, distribution shape, or
  partition shape;
- many-state scaling, including per-state overhead;
- sensitivity to group skew, input order, partition count, and merge tree;
- unsupported kernel coverage when a candidate lacks a native operation;
- ranking changes between single-state, grouped-state, and partitioned-merge
  kernels.

This layer answers:

```text
Which approximate function candidate is suitable for this AQP-style workload
kernel?
```

The final comparison should be a candidate dossier, not a single leaderboard
score. A strong candidate may have excellent kernel coverage but poor adoption
metadata. Another may be easy to integrate but lack merge support or fail
under grouped-state pressure. AQPBMV2 should make those tradeoffs explicit.

## What AQPBMV2 Is Not

AQPBMV2 is not:

- a SQL benchmark;
- a SQL parser, optimizer, or execution benchmark;
- a benchmark of a complete AQP database system;
- a raw sketch benchmark;
- a benchmark of ASAPQuery, ClickHouse, DuckDB, Spark, or DataFusion;
- a benchmark of a newly invented adapter policy;
- a claim that every AQP workload can be represented by sketch kernels.

SQL may appear in papers or notes only as motivation for why a kernel is
relevant. It should not enter the AQPBMV2 benchmark definition. Once SQL
execution is included, the result mixes the sketch with parser behavior,
optimizer choices, physical plans, partitioning, memory policy, and execution
engine details. That belongs to a later system benchmark, not AQPBMV2.

## Canonical Kernel Families

AQPBMV2 should define explicit benchmark kernels. A kernel is a fixed local
execution program owned by the benchmark harness, not a plan chosen by an
optimizer.

### Kernel 1: Single-State Sketch

Purpose: keep the conventional sketch benchmark as a baseline.

```text
state = new_state()
for value in values:
  update(state, value)
answer = query(state, parameter)
```

This kernel measures the familiar case: one logical state over one stream.

### Kernel 2: Grouped-State Sketch

Purpose: model the core pressure created by grouped approximate aggregates.

```text
states = {}
for row in rows:
  key = key_fn(row)
  value = value_fn(row)
  update(states[key], value)

for key in states:
  answer[key] = query(states[key], parameter)
```

Controlled dimensions include:

- number of groups;
- group-size skew;
- per-group value distribution;
- heavy tails;
- duplicate rate;
- key-value correlation;
- selectivity if the row generator includes an admitted filter field.

The benchmark output is a set of answers, not one answer. Metrics should
therefore include answer coverage, per-group failures, and failure localization
by group/data condition.

### Kernel 3: Partitioned-Merge Sketch

Purpose: model partial aggregation and merge sensitivity without invoking a
database optimizer.

```text
partial_states = {}
for row in rows:
  partition = partition_fn(row)
  key = key_fn(row)
  value = value_fn(row)
  update(partial_states[partition][key], value)

for key in all_keys:
  merged = merge_all(partial_states[*][key], merge_shape)
  answer[key] = query(merged, parameter)
```

Controlled dimensions include:

- partition count;
- partitioning policy;
- merge tree shape;
- input order;
- group skew across partitions;
- whether a candidate lacks native merge support.

This kernel makes execution shape an explicit benchmark variable. It does not
ask a SQL optimizer to choose a plan.

## Manifest Shape

AQPBMV2 should use a small manifest centered on kernels, candidates, and data
conditions.

Example:

```toml
[benchmark]
id = "aqpbmv2_grouped_quantile_kll_v1"
track = "sketch_kernel_aqp"
kernel = "grouped_quantile"
task = "quantile"

[function_candidate]
functionality = "approx_quantile"
implementation = "KllQuantile"
sketch_family = "kll"
sketch_implementation = "implementation_a"
parameters = { k = 200 }
binding = "thin_native_binding"

[candidate_metadata]
language = "rust"
version = "pinned_or_recorded_version"
native_operations = ["update", "query", "merge"]
serialization = "native_or_absent"
api_binding_effort = "thin"
metadata_status = "declared_not_benchmarked"

[data_condition]
id = "n10m_groups10k_zipf1_2_lognormal_tail_seed1"
rows = 10000000
groups = 10000
group_size_distribution = "zipf"
group_size_zipf_s = 1.2
value_distribution = "per_group_lognormal_tail"
key_value_correlation = "strong"
seed = 1

[query]
parameter = 0.99

[requirements]
rank_error_max = 0.01
answer_coverage_min = 0.95

[execution_shape]
partition_count = 1
merge_shape = "none"
input_order = "generated"

[baseline]
exact_reference = "per_group_exact_order_statistics"
exact_performance_baseline = "optional_exact_state_run"
```

For the partitioned kernel, `partition_count` and `merge_shape` become active
fields. For single-state kernels, group fields are absent or set to one.
The `candidate_metadata` section is descriptive metadata, not a measured
benchmark result.

## Metrics

AQPBMV2 should report raw metrics, but the key contribution should be the
workload-level interpretation.

Candidate metrics:

- update throughput;
- query latency;
- merge latency where applicable;
- memory or serialized state size where measurable;
- raw fidelity metrics such as rank error, value error, relative error,
  precision/recall, or count error.

Workload-level metrics:

- answer coverage: fraction of answers meeting the requirement;
- failure localization: failed answers grouped by group size, distribution
  region, partition shape, or other controlled condition;
- sensitivity: change in coverage or ranking across group skew, partition
  count, merge shape, and input distribution;
- unsupported coverage: fraction of workload shapes that cannot be run because
  the candidate lacks required native operations such as merge;
- budget frontier: optional summary of coverage under a fixed parameter,
  memory, or latency budget.

The benchmark should preserve exact losses. If the exact baseline is faster,
smaller, or more reliable for a condition, that is a valid result.

## Difference From Existing Sketch And DB Benchmarks

The intended distinction is not "AQPBMV2 uses SQL" or "AQPBMV2 has an adapter".
Those are not meaningful contributions.

The intended distinction is the evaluation unit:

```text
traditional sketch benchmark:
  one sketch state over one stream
  => one or a few answers

traditional database benchmark:
  one concrete database system
  => system-level query/runtime behavior

AQPBMV2:
  executable approximate function candidate
  => exact-vs-sketch state substitution, many logical states, many answers,
     controlled grouping/merging pressure, answer coverage, and failure
     localization
```

If an existing benchmark already provides grouped states, partitioned merge
shapes, controlled group/data distributions, exact per-answer baselines, and
workload-level coverage metrics, then it is close to AQPBMV2 and the novelty
claim must be narrowed. AQPBMV2 should not claim novelty merely for wrapping
sketch libraries or measuring error/throughput.

## Relationship To Broader AQP Design

The broader AQP benchmark design remains useful, but it is later scope:

- PromQL/ASAPQuery scenarios benchmark a concrete external system boundary.
- SQL/ClickHouse/DataFusion scenarios benchmark a concrete SQL or engine path.
- Resource-accounting studies benchmark deployment-level value.

Those are not AQPBMV2 unless they are explicitly reduced to executable
function candidates and local kernels. AQPBMV2 should use the function-kernel
track to make one narrow, defensible step before claiming a full AQP system
benchmark.

## Optional DB-Supported Function Parity Probe

AQPBMV2 should not benchmark a database system in V2, but it can use
DB-supported approximate functions as a parity probe.

Purpose:

```text
Check whether AQPBMV2's function specs resemble functionality that real systems
already expose.
```

Non-purpose:

```text
Do not use this probe to claim that AQPBMV2 benchmarks Spark, Trino,
DataFusion, ClickHouse, BigQuery, Snowflake, or Druid.
```

The probe should be small:

1. Pick one functionality class, preferably approximate count distinct.
2. Generate a small controlled dataset.
3. Run AQPBMV2's `ExactCountDistinct` and HLL-backed count-distinct
   implementations through the local kernels.
4. Separately run one DB-supported approximate function on the same logical
   input, if the DB is easy to run locally.
5. Compare only semantics and result shape: input type, null handling,
   grouping behavior, output type, and rough answer compatibility.
6. Do not compare database latency, optimizer behavior, scan cost, storage, or
   execution engine performance.

This gives AQPBMV2 a sanity check against real user-facing functionality
without letting the work collapse into a database benchmark.

## Why This Could Become A Paper

The V2 toolkit alone is not automatically a VLDB-level contribution. It becomes
interesting only if the empirical study shows that AQP-style kernel structure
changes conclusions that single-state sketch benchmarks would suggest.

A credible paper-shaped contribution would need:

1. A clear benchmark target: executable approximate function candidates under
   fixed parameters and thin sketch bindings.
2. A canonical kernel set with an argument for why the kernels represent common
   AQP execution pressures: single aggregate, grouped aggregate, and partial
   aggregation plus merge.
3. A controlled data-condition generator for group cardinality, group skew,
   tails, correlation, selectivity, and partitioning.
4. A result model based on answer coverage, failure localization, and
   sensitivity, not only average error and throughput.
5. An empirical study over multiple real sketch-backed function candidates
   showing non-obvious differences or ranking changes that would be hidden by
   raw sketch or single-state benchmarks.

Without item 5, AQPBMV2 is a useful engineering toolkit but probably not a
VLDB-strength benchmark paper.

## Implementation Plan

1. Define the `ApproxFunction` trait and the function-candidate record model.
2. Implement one exact reference and one sketch-backed candidate for
   approximate distinct count.
3. Keep the existing single-state sketch benchmark path as a primitive baseline.
4. Add a kernel manifest format with `kernel`, `function_candidate`,
   `data_condition`, `query`, `requirements`, `execution_shape`, and
   `baseline` sections.
5. Implement grouped and partitioned kernels over `ApproxFunction`, not over
   raw sketches directly.
6. Emit exact per-group reference answers and candidate per-group answers.
7. Add answer coverage and failure-localization summaries.
8. Add KLL/t-digest quantile candidates after count-distinct proves the shape.
9. Add a DB-supported function parity probe as optional validation, not as the
   primary benchmark.
10. Keep PromQL, SQL, DataFusion, and external-system adapters out of the V2
   success criteria.

Initial code landing:

```text
aqp-core/src/function.rs
```

Runnable smoke path:

```bash
cargo run -p aqp-core --example aqpbmv2_functions
```

The broader `aqpbmv2_functions` example runs three functionality classes through
the same grouped and partitioned-merge kernels:

- distinct count: exact `HashSet`, Apache DataSketches HLL,
  `sketch_oxide` HLL, and `asap_sketchlib` HLL;
- heavy hitters: exact `HashMap`, Apache DataSketches FrequentItems, and
  `sketch_oxide` SpaceSaving, and `asap_sketchlib` CMSHeap;
- quantile: exact sorted vector, Apache DataSketches TDigest, and
  `sketch_oxide` TDigest, and `asap_sketchlib` KLL.

The JSON report includes exact outputs, candidate outputs, and coverage
summaries. Count-distinct and quantile use numeric relative-error coverage;
heavy hitters use precision/recall at k. Bare Count-Min Sketch is a point
frequency estimator, so the executable heavy-hitter candidate uses
`asap_sketchlib` CMSHeap rather than pretending that CMS alone discovers top-k
items.

## Success Criteria

AQPBMV2 succeeds as a design and engineering milestone when:

- an executable approximate function candidate can be run through single-state
  and grouped-state kernels;
- each run has a stable data-condition id and exact reference answers;
- reports include answer coverage and failure localization;
- the benchmark can show whether grouped-state behavior agrees or disagrees
  with single-state behavior;
- the docs clearly say that SQL/system benchmarking is later scope;
- negative results are preserved, including exact wins and unsupported
  function-candidate/kernel pairs.

## Open Questions

- Are the single-state, grouped-state, and partitioned-merge kernels sufficient
  as the first canonical set, or is another kernel needed to represent AQP
  pressure without invoking a database system?
- Which existing sketch implementations in the repo are concrete enough for the
  first empirical comparison?
- What minimum set of data-condition sweeps is needed before the result is more
  than a demo?
- How should a fixed memory budget be applied fairly when grouped workloads
  create many states?
- Can the empirical study produce a genuinely new finding, or will it only
  reproduce known sketch behavior in a different harness?
