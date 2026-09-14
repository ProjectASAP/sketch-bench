# AQP Bench Problem Definition

> Status: current problem-definition recap.
>
> The reader-facing outline for the current direction is
> `docs/AQP_BLOG_OUTLINE.md`.

## AQPBMV1 Problem Definition

AQPBMV1 was the first local attempt to move beyond isolated sketch benchmark
rows. Its problem was that a raw sketch result such as HLL update throughput or
KLL rank error does not by itself explain what happens when an approximate query
is parsed, admitted, run against a controlled data source, and compared with an
exact answer.

The V1 code therefore used a SQL-shaped frontend. DataFusion parsed the SQL and
produced a logical plan. The AQP layer admitted only a small set of aggregate
tasks and lowered them into local benchmark tasks. The supported task shapes
included count distinct, quantile, and frequency-style aggregation. Count
distinct and quantile could run either ungrouped or grouped by a small synthetic
dimension such as `region`. Frequency used a controlled key column. The local
execution layer then ran an exact backend and an approximate backend over the
same generated rows and compared the outputs.

In this version, the benchmark target was closest to an approximate query plan
slice. It included SQL parsing and lowering, synthetic relational data, exact
execution, sketch-backed execution, and an accuracy/cost report. The exact
backends used ordinary exact data structures such as sets, vectors, or maps.
The approximate backends used `asap_sketchlib` sketches such as HyperLogLog for
count distinct, KLL for quantile, and Count-Min or CountSketch-style frequency
structures for frequency tasks.

V1 was useful because it made the user-facing query shape visible. It showed
that `sketch-bench` could run a paired exact-versus-approximate local workflow
instead of only timing a sketch primitive. It also exposed the first important
boundary: once SQL parsing, admission, lowering, generated data, exact
execution, sketch execution, and reporting are all in the same benchmark, it
becomes harder to say what the benchmark unit really is.

The V1 problem definition can therefore be summarized as follows. Given a small
admitted SQL-shaped aggregate task over a controlled synthetic relation, compare
an exact local execution path with a sketch-backed local execution path, and
report whether the approximate path preserves the answer closely enough while
offering useful cost behavior. This is stronger than a raw sketch benchmark,
but it is not yet a general AQP benchmark and not a complete system benchmark.

The main weakness of V1 is that the benchmark mixes several concerns. It uses a
SQL-shaped surface, but it is not benchmarking a real SQL database. It uses
sketches, but it is not just benchmarking raw sketch primitives. It has exact
and approximate local execution, but it does not represent a full AQP system
with production planning, admission, fallback, and resource policy. That
ambiguity motivates the narrower V2 definition.

## AQPBMV2 Problem Definition

AQPBMV2 starts from a stricter question: what concrete thing is being
benchmarked? The current answer is not a database system, not SQL, and not a
raw sketch instance. The concrete target is a runnable implementation of an
approximate functionality.

A functionality is something a user can recognize at the query or task level,
such as count distinct, heavy hitters, or quantile. An implementation is the
code that owns a state, updates that state from inputs, merges another state
when needed, and finalizes an answer. The implementation may be exact, such as
a `HashSet` for count distinct. It may also be sketch-backed, such as Apache
DataSketches HLL, `sketch_oxide` HLL, or `asap_sketchlib` HLL for count
distinct. The sketch is part of the implementation, but the sketch API alone is
not the benchmark unit.

This is the middle layer between raw sketch benchmarks and full system
benchmarks. A raw sketch benchmark usually asks how one sketch state behaves on
one value stream. A full system benchmark asks how a database or AQP system
behaves after parsing, planning, optimizing, scheduling, storing, and executing
a workload. AQPBMV2 deliberately avoids both extremes. It owns local execution
modes over a common interface and asks whether exact and sketch-backed
implementations of the same functionality can be compared under those execution
modes.

The current V2 interface is the `ApproxFunction` trait. It exposes `create`,
`update`, `merge`, and `finalize`. That interface is code, not a config file.
It lets the benchmark run the same execution mode over different
implementations. The current implementation covers count distinct, heavy
hitters, and quantile. It has exact baselines and sketch-backed implementations
from Apache DataSketches, `sketch_oxide`, and `asap_sketchlib`.

The current V2 demo should be interpreted carefully. It is a capability and
integration result. It shows that these libraries can be wired into the same
middle-layer interface for the current functionality classes. It does not show
that one library is generally better than another. It does not show that the
current workload is benchmark-grade. It also does not yet prove that AQPBMV2
reveals behavior missed by raw sketch benchmarks, because the current
implementations are still close to one-sketch-per-function cases.

The current input generator is also intentionally weak. Each generated row has
a synthetic `group_key` and a `value`. The benchmark groups rows by
`group_key`, then runs the target functionality over `value` inside each group.
The current group assignment is hand-written with `idx % group_count`; it is
not produced by a SQL interpreter, query planner, real schema, or production
trace. Any current statement about all groups or group-level answers refers
only to those synthetic demo groups.

That weakness is part of the V2 problem definition rather than a hidden detail.
For AQPBMV2 to become a stronger benchmark, group-key generation must become a
first-class workload condition. The benchmark needs defensible generators for
group cardinality, group-size skew, tail heaviness, top-k gap, filter
selectivity, correlation between group key and value, partition count, and
merge shape. Only then can V2 test whether the middle layer exposes behavior
that raw sketch benchmarks do not expose.

The current V2 contribution is therefore a toolkit direction, not a final paper
claim. AQPBMV2 should provide reusable code for implementing exact and
sketch-backed approximate-function implementations, running them through
benchmark-owned execution modes, and reporting where they agree or fail against
exact baselines. The paper-level potential depends on whether the next
workloads show meaningful behavior at the functionality level that cannot be
predicted from primitive sketch benchmarks alone.

## AQPBMV3 Placeholder

## AQPBMV4 Placeholder

## AQPBMV5 Placeholder

