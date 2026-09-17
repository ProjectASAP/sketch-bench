# AQPBMV2 Design: Runnable Approximate-Function Toolkit

> Status: current AQPBMV2 design note.
>
> Primary reader-facing outline: `docs/AQP_BLOG_OUTLINE.md`.

## Scope

AQPBMV2 is a toolkit for benchmarking runnable implementations of approximate
functionality. It sits between raw sketch benchmarks and full system
benchmarks.

The benchmark unit is not a sketch instance by itself. The benchmark unit is a
runnable implementation of a functionality, such as count distinct, heavy
hitters, or quantile. A sketch may be the state implementation behind that
functionality, but the sketch API alone is not the benchmark target.

AQPBMV2 also does not benchmark Spark, Trino, BigQuery, ClickHouse, DataFusion,
PromQL, or ASAPQuery directly. Those systems motivate the functionality classes,
but running a real system would mix parser behavior, optimizer choices,
execution plans, storage, scheduling, and system-specific policy. That belongs
to a later system benchmark, not V2.

## Problem

Raw sketch benchmarks answer a primitive-level question: how does one sketch
state behave over one stream? That is useful, but users usually care about
functionality such as approximate count distinct, approximate top-k, or p95
quantile. They care about whether a concrete implementation can run under the
same update, merge, and finalize shape as an exact baseline, and whether it
returns acceptable answers for the workload in front of them.

The current V2 result is intentionally modest. The current implementations are
still close to one-sketch-per-function cases, so the preliminary result is
close to a raw sketch comparison. V2 should say that clearly. The current demo
is a capability and integration result: it shows that exact baselines and
sketch-backed implementations can be plugged into the same middle layer. It
does not yet prove that AQPBMV2 reveals behavior that raw sketch benchmarks
miss.

The next design step is to make the middle layer matter more. That means better
workload generators, explicit group-key generation, group-size skew,
partition/merge shapes, and eventually implementations that compose more than
one sketch or expose behavior that cannot be observed from a single raw sketch
state.

## Runnable Interface

The current code-level middle layer is the `ApproxFunction` trait:

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

This interface is deliberately smaller than SQL and smaller than a database
operator API. It gives the benchmark enough structure to run implementations
through benchmark-owned execution modes without pretending to be a complete
query engine.

The middle-layer shape is:

```text
user-level functionality
  -> runnable approximate-function implementation
  -> exact or sketch-backed state implementation
```

For count distinct, an exact implementation can use `HashSet`, while sketch
implementations can use Apache DataSketches HLL, `sketch_oxide` HLL, or
`asap_sketchlib` HLL. For heavy hitters, exact `HashMap` can be compared with
DataSketches FrequentItems, `sketch_oxide` SpaceSaving, and `asap_sketchlib`
CMSHeap. For quantile, exact sorted vectors can be compared with DataSketches
TDigest, `sketch_oxide` TDigest, and `asap_sketchlib` KLL.

## Current Implemented Functionality

Current exact baselines:

- `ExactCountDistinct<HashSet>`
- `ExactHeavyHitters<HashMap>`
- `ExactQuantile<Vec>`

Current sketch-backed implementations:

- Apache DataSketches HLL for count distinct
- Apache DataSketches FrequentItems for heavy hitters
- Apache DataSketches TDigest for quantile
- `sketch_oxide` HLL for count distinct
- `sketch_oxide` SpaceSaving for heavy hitters
- `sketch_oxide` TDigest for quantile
- `asap_sketchlib` HLL for count distinct
- `asap_sketchlib` CMSHeap for heavy hitters
- `asap_sketchlib` KLL for quantile

## Execution Modes

AQPBMV2 currently owns two execution modes.

`grouped_state` keeps one implementation state per synthetic group key, updates
that state with all rows in the group, and finalizes one answer per group. This
is meant to mimic the shape of a grouped aggregate, but the current demo does
not execute SQL.

`partitioned_merge` builds implementation states inside partitions, merges
partition-local states for each group, and finalizes one answer per group after
the merge. This checks whether the implementation can run through the same
merge shape as the benchmark expects.

Both modes are benchmark-owned local programs. They are not database kernels,
SQL plans, or optimizer-selected execution plans.

## Current Demo Input

The current demo input is synthetic. Each row has two logical fields:

```text
group_key, value
```

The benchmark groups rows by `group_key` and runs the target functionality over
`value` inside each group. For count distinct, it counts distinct values inside
each synthetic group. For heavy hitters, it finds frequent values inside each
synthetic group. For quantile, it estimates the p95 of values inside each
synthetic group.

The current group assignment is hand-written in the generator. It uses
`idx % group_count` to assign rows to keys like `group_000`, `group_001`, and
so on. This is acceptable for a smoke test, but it is a weak part of the current
design. The group keys are not produced by SQL, a query planner, a real schema,
or a production workload.

The current demo uses:

- count distinct: 50,000 rows, 32 synthetic groups, value cardinality 20,000
- heavy hitters: 80,000 rows, 24 synthetic groups, clear top-k pattern
- quantile: 80,000 rows, 16 synthetic groups, deterministic p95 input

Any statement about all groups or group-level answers refers only to these
synthetic demo groups.

## Current Demo Output

The current demo command is:

```bash
cargo run -q -p aqp-core --example aqpbmv2_functions
```

The output is a JSON report with exact baseline outputs, sketch-backed
implementation outputs, grouped-state outputs, partitioned-merge outputs, and
summary comparisons.

The current preliminary result supports only a capability claim. It shows that
these libraries can be wired into the middle layer for these functionality
classes:

- count distinct: DataSketches HLL, `sketch_oxide` HLL, `asap_sketchlib` HLL
- heavy hitters: DataSketches FrequentItems, `sketch_oxide` SpaceSaving,
  `asap_sketchlib` CMSHeap
- quantile: DataSketches TDigest, `sketch_oxide` TDigest, `asap_sketchlib` KLL

It does not show that one library is generally better than another. It does not
show that the synthetic workload is benchmark-grade. It does not validate the
group-key generator. It does not yet establish that AQPBMV2 exposes behavior
missed by raw sketch benchmarks.

## Metrics

For count distinct and quantile, the current report compares each approximate
answer against the exact answer for the same synthetic group. It reports how
many group answers are within the chosen error threshold, plus mean and maximum
relative error.

For heavy hitters, the current report compares returned top-k items against the
exact top-k items for the same synthetic group. In reader-facing text, this
should be described as whether every returned top-k item is correct and whether
any exact top-k item is missing. The internal report fields may still use
precision and recall terminology.

These metrics are currently fidelity-oriented. Throughput, query latency, merge
latency, memory, serialized state size, CPU, and resource-accounting policy are
future work for AQPBMV2.

## Relationship To Existing Approximate Query Support

Many systems already expose approximate functionality: DataFusion, Trino,
Spark SQL, BigQuery, ClickHouse, Snowflake, Druid, and DataSketches-backed
extensions all provide examples. This supports the choice of functionality
classes such as count distinct, quantile, and heavy hitters.

AQPBMV2 does not claim to benchmark those systems. It defines a
system-independent toolkit for runnable implementations of those functionality
classes. A system-specific function can be used later only if its state or UDAF
implementation can be exposed through a compatible runnable interface, or if a
later benchmark deliberately chooses to benchmark the full system.

## Near-Term Work

The next implementation work should focus on the weak parts that the current
demo exposes:

- replace toy generators with benchmark-grade workload generators;
- make group-key generation explicit and defensible;
- map task semantics to group keys rather than hard-coding `idx % group_count`;
- add group cardinality, group-size skew, tail heaviness, top-k gap, filter
  selectivity, and correlation between group key and value;
- vary partition count and merge shape;
- add resource and performance measurement;
- test whether AQPBMV2 execution modes reveal behavior that raw sketch
  benchmarks miss.

## Research Bar

AQPBMV2 becomes more than an engineering toolkit only if the benchmark-owned
middle layer reveals behavior that raw sketch benchmarks do not reveal. The
current demo does not prove that yet. It only proves that the runnable
implementation interface is viable enough to host exact and sketch-backed
implementations from multiple libraries.
