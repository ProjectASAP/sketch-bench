# AQPBMV2 Discussion Notes, 2026-07-07

These notes summarize a design discussion about what AQPBMV2 should actually
benchmark. They are intentionally written as a decision record rather than as a
polished paper argument. The current direction is promising but not yet fully
settled.

## Starting Concern

The initial AQPBMV2 framing was too broad. It mixed several possible targets:

- sketch primitive benchmarks;
- approximate aggregate functions;
- complete AQP systems;
- SQL/PromQL/DataFusion adapters;
- broad scenario-family benchmark design inspired by recent VLDB benchmark
  papers.

That framing made it hard to answer the basic benchmark question:

```text
What concrete thing is being benchmarked?
```

The discussion converged on an important constraint:

```text
A benchmark must have a concrete unit under test.
```

The unit could be a system, a sketch instance, an adapter, or another concrete
artifact. It cannot be an abstract idea such as "AQP" or "sketches as
operators" without further specification.

## Directions Considered

### Full AQP System Benchmark

One possible target is a complete system such as ASAPQuery, ClickHouse,
DuckDB, Spark, or a DataFusion-based prototype.

This is concrete, but it is not AQPBMV2. Once a complete system is included,
the benchmark result mixes:

- parser behavior;
- optimizer choices;
- execution plans;
- partitioning and merge strategies;
- memory policy;
- serialization;
- storage layout;
- sketch implementation behavior.

That is a valid later system benchmark, but it is too large for the current
V2 scope.

### SQL-Shaped Workload Benchmark

Another idea was to use SQL-shaped workloads such as:

```sql
SELECT service, region, APPROX_PERCENTILE(latency, 0.99)
FROM logs
WHERE status >= 500
GROUP BY service, region;
```

This example is useful for explaining motivation, but it should not be part of
the AQPBMV2 benchmark definition. If SQL is actually executed, optimizer and
execution-engine decisions become uncontrolled factors. If SQL is only used as
a label for generated inputs, then SQL itself is not the contribution.

Conclusion:

```text
SQL can be explanatory material, but AQPBMV2 should not be a SQL benchmark.
```

### Adapter/Contract Benchmark

The discussion also considered benchmarking a layer that turns sketch
libraries into approximate aggregate states. This direction is interesting
because AQP systems need aggregate-state behavior, while sketch libraries often
expose only data-structure APIs.

However, without a concrete system or a concrete adapter policy, the contract
is not well-defined. If the project invents an adapter and then benchmarks it,
the measured object becomes:

```text
adapter policy + sketch implementation
```

That may be a valid framework contribution later, but it is risky for V2
because the benchmark could be criticized as evaluating an object we invented
rather than an existing system or implementation.

Conclusion:

```text
AQPBMV2 should not make adapter design the main benchmark target.
```

### Thin Binding Around Sketch Instances

The safest concrete target is:

```text
sketch implementation + fixed parameter setting + thin benchmark binding
```

Examples:

```text
KLL implementation A, k=200
t-digest implementation B, compression=100
HLL implementation C, precision=14
```

The binding is only measurement glue. It should expose local benchmark calls
such as `new_state`, `update`, `merge`, and `query`. It should not add fallback
to exact state, adaptive tuning, custom window rebuilds, or error correction.

This keeps responsibility clear:

```text
The benchmark result is attributed primarily to the concrete sketch instance,
not to a new adapter policy.
```

## Main Objection To The Thin-Binding Direction

The thin-binding direction risks being too close to existing sketch benchmarks.
If AQPBMV2 only measures:

```text
one sketch
one stream
one p99 query
error/time/memory
```

then it is not a strong contribution. Existing sketch libraries, papers, and
benchmarks already cover much of that space.

Therefore, the missing piece cannot be "another sketch benchmark harness."

## Current Working Direction

The strongest current direction is:

```text
AQPBMV2 benchmarks concrete sketch instances under canonical AQP-style sketch
kernels, with workload-level metrics over many states and many answers.
```

The key shift is the unit of evaluation:

```text
traditional sketch benchmark:
  one sketch state over one stream
  => one or a few answers

AQPBMV2:
  canonical AQP-style kernels
  => many logical states, many answers, controlled grouping/merging pressure,
     answer coverage, and failure localization
```

This is not about SQL syntax. It is about benchmark structure.

## Candidate Kernels

### Single-State Kernel

This preserves the conventional baseline:

```text
one state
one input stream
one query parameter
```

It gives a point of comparison against existing sketch benchmark behavior.

### Grouped-State Kernel

This models a common approximate aggregate pressure:

```text
many groups
one sketch state per group
many answers
```

The controlled dimensions can include:

- group cardinality;
- group-size skew;
- per-group value distribution;
- heavy tails;
- duplicate rate;
- key-value correlation;
- selectivity when the generated rows include a filter-like field.

The central result is not one error value. The central result is answer
coverage and failure localization:

```text
How many group answers met the requirement?
Which groups failed?
Do failures concentrate in small groups, rare groups, skewed groups, or
tail-heavy groups?
```

### Partitioned-Merge Kernel

This models partial aggregation and merge sensitivity without involving a SQL
optimizer:

```text
partition rows
build partial sketch states
merge states for the same logical group
query merged states
```

The controlled dimensions can include:

- partition count;
- partitioning policy;
- merge tree;
- input order;
- skew across partitions.

This is a benchmark-defined kernel, not a database-chosen plan.

## What Would Make This Interesting

AQPBMV2 becomes interesting if it shows that conclusions from single-state
sketch benchmarks do not reliably predict behavior under grouped or
partitioned AQP-style kernels.

Examples of paper-worthy findings would include:

- a sketch instance that looks strong on single-state streams but fails many
  small or skewed groups;
- candidate rankings that change under group-size skew or partitioned merge;
- accuracy failures that concentrate in specific workload regions rather than
  appearing uniformly;
- a parameter setting that satisfies global p99 but provides poor workload
  coverage across many grouped p99 answers;
- merge sensitivity that is invisible in a single-stream benchmark.

If the grouped and partitioned kernels merely reproduce the same conclusions as
single-state tests, then AQPBMV2 is still a useful engineering toolkit but has
a weaker research contribution.

## Current Contribution Statement

Working version:

```text
AQPBMV2 contributes an AQP-workload-level benchmark methodology and toolkit
for concrete sketch instances, replacing single-stream sketch evaluation with
canonical grouped and partitioned sketch kernels that measure answer coverage,
failure localization, and sensitivity to workload shape.
```

Shorter version:

```text
AQPBMV2 evaluates concrete sketch instances at the workload-kernel level, not
only at the single-stream primitive level.
```

## Refinement: Selection Criteria, Not Just Scores

A later comment suggested listing criteria for choosing between sketch
implementations or libraries, such as performance, API ease, number of
implemented sketches, and implementation language. This is useful, but it
should not change the benchmark target.

The design lesson is:

```text
AQPBMV2 should produce candidate-selection evidence, not only benchmark rows.
```

The evidence should be separated into three layers:

1. Declared adoption metadata:
   language, runtime, license, implemented sketch families, API shape,
   native merge support, serialization support, documentation, packaging,
   dependency footprint, and maintenance status.

2. Measured primitive behavior:
   update throughput, query latency, merge latency, memory/state size, raw
   fidelity, and parameter sensitivity on single-state workloads.

3. Measured AQP-kernel behavior:
   answer coverage, failure localization, many-state scaling, unsupported
   kernel coverage, and sensitivity to group skew, partition count, merge tree,
   and input distribution.

The important boundary is that AQPBMV2 should not turn qualitative adoption
metadata into fake benchmark measurements. API ergonomics, documentation, and
maintenance are still relevant for choosing a sketch implementation, but they
should be labeled as declared or manually assessed metadata. The kernel
benchmark measures behavior under controlled kernels.

This suggests a practical output form:

```text
candidate dossier
  = identity and fixed parameters
  + declared adoption metadata
  + primitive benchmark results
  + AQP-kernel coverage and failure summaries
```

This makes the project more useful without weakening the concrete unit under
test.

## Remaining Doubts

This direction is not fully proven. The main doubts are:

- Are the proposed kernels canonical enough, or are they arbitrary synthetic
  programs?
- Do existing sketch benchmarks already cover enough of grouped and
  partitioned behavior to weaken the novelty claim?
- Can the empirical study produce findings that are not obvious from existing
  sketch theory or implementation documentation?
- How should fixed memory or parameter budgets be applied fairly when grouped
  workloads create many states?
- Which concrete sketch implementations in the repo are sufficient for a
  first convincing comparison?

These doubts should stay visible in the design. The next implementation step
should be small and empirical: implement the grouped-state kernel, compare it
against the single-state baseline, and see whether it actually changes the
conclusions.
