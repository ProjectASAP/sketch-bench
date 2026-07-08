# Blog Outline: AQPBM

> Status: currently focused on AQPBMV2.
>
> Goal: explain the problem scope of AQPBM and the contribution of the current AQPBMV2 design.

## Query to begin with: SELECT approx_count_distinct(user_id) FROM table;

- Example query:
  - `SELECT approx_count_distinct(user_id) FROM table;`

- Why this query is useful as motivation:
  - It is a common approximate aggregate.
  - The exact table still exists.
  - The system replaces one exact aggregate with an approximate function.
  - Users trade some fidelity for lower latency, memory, or CPU.
  - See `Existing Approximate Query Support` at the end for examples.

- Why AQPBMV2 should not benchmark this SQL query directly:
  - The benchmark target would become the whole SQL system.
  - Different databases expose different functions and execution plans.
  - The question would become: which database are we benchmarking?
    - Spark?
    - Trino?
    - BigQuery?
    - ClickHouse?
  - Each system-supported version of this query deserves its own system benchmark.
  - Parser, optimizer, storage, and execution engine behavior would be mixed in.
  - That belongs to a later system benchmark, not AQPBMV2.

## Raw Sketch BM

- AQPBMV1 already covers raw sketch benchmarking.
  - The benchmark target is a sketch primitive.
  - Example: HLL as one state over one stream.
  - Inputs can be controlled by distribution and cardinality.
  - Metrics include accuracy, throughput, and memory.

- The limitation:
  - Users ultimately want good query or task performance.
  - Good component or sketch performance may suggest good query performance.
    - But it is only the starting point.
  - The benchmark needs to preserve the path from sketch behavior to user-visible functionality.

- The gap:
  - A raw sketch benchmark can help users reason about candidate sketches.
  - It cannot by itself answer whether an approximate function is useful for a user-facing task.
  - AQPBMV2 starts from raw sketch evidence.
  - It then evaluates executable approximate-function candidates.

- Mental experiment:
  - Suppose a query is implemented using three HLL states.
  - Each individual HLL state may look bad in isolation.
    - Example: 30% relative error per raw sketch state.
  - The final query answer may still be good.
    - Example: 1% relative error after the full function logic.
  - A user would choose the full approximate function because it gives good query performance.
    - Not because every raw sketch component looks good in isolation.
  - This is why AQPBMV2 should evaluate executable function candidates.

## Current: AQPBMV2

- Scope:
  - AQPBMV2 is an executable approximate-function benchmark.
  - It is not a full SQL, PromQL, or AQP system benchmark.
  - It is not a raw sketch benchmark.

- Current executable interface:

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

- Middle-layer shape:

```text
user-level functionality
  -> executable approximate function candidate
  -> sketch or exact state implementation
```

- Current functionality classes:
  - Count distinct.
  - Heavy hitters.
  - Quantile.

- Current exact baselines:
  - `ExactCountDistinct<HashSet>`
  - `ExactHeavyHitters<HashMap>`
  - `ExactQuantile<Vec>`

- Current sketch-backed candidates:
  - Local demo HLL for count distinct.
  - Apache DataSketches HLL for count distinct.
  - Apache DataSketches FrequentItems for heavy hitters.
  - Apache DataSketches TDigest for quantile.
  - `sketch_oxide` HLL for count distinct.
  - `sketch_oxide` SpaceSaving for heavy hitters.
  - `sketch_oxide` TDigest for quantile.
  - `asap_sketchlib` HLL for count distinct.
  - `asap_sketchlib` CMSHeap for heavy hitters.
  - `asap_sketchlib` KLL for quantile.

- Current benchmark-owned kernels:
  - `grouped_state`
  - `partitioned_merge`

- Current report metrics:
  - Numeric relative-error answer coverage for count distinct and quantile.
  - Mean and max relative error for numeric outputs.
  - Precision@k and recall@k for heavy hitters.

- Current smoke/demo path:
  - `cargo run -p aqp-core --example aqpbmv2_functions`
  - The current generated data is only a smoke test.
  - It proves candidates can be wired into the middle layer and run.
  - It is not yet benchmark-grade workload evidence.

- What AQPBMV2 does not claim:
  - It does not benchmark Spark's function directly.
  - It does not benchmark Trino's function directly.
  - It does not benchmark BigQuery's function directly.
  - It does not benchmark ClickHouse's function directly.
  - Those systems show that the functionality classes are real.

- Research bar:
  - AQPBMV2 is interesting only if benchmark-owned kernels reveal new behavior.
    - The behavior should be something raw sketch benchmarks miss.
  - If they do not, AQPBMV2 is still a useful toolkit but a weaker paper contribution.

## Previous AQPBMV1

- AQPBMV1 is the existing raw-sketch benchmark layer.
  - Target: sketch primitives directly.
  - Input control: data shape and distribution.
  - Metrics: throughput, accuracy, and memory usage.
  - Role: provide primitive-level evidence for AQPBMV2.

## Future AQPBMV3

## Future AQPBMV4

## Existing Approximate Query Support

- Codex found approximate-query or sketch-backed functions in several systems.
  - This matches my impression that approximation support exists in practice.
  - This list is not exhaustive.
  - More systems and functions may need to be added later.

- Apache DataFusion:
  - `approx_distinct`
  - `approx_median`
  - `approx_percentile_cont`
  - `approx_percentile_cont_with_weight`
  - Approximate percentile functionality is described as using t-digest.
  - <https://datafusion.apache.org/user-guide/sql/aggregate_functions.html>

- Trino:
  - `approx_distinct`
  - `approx_most_frequent`
  - `approx_percentile`
  - `numeric_histogram`
  - HyperLogLog state functions such as `approx_set` and `merge`
  - <https://trino.io/docs/current/functions/aggregate.html>

- Spark SQL:
  - `approx_count_distinct`
  - `approx_percentile`
  - `percentile_approx`
  - `count_min_sketch`
  - HLL sketch functions
  - KLL sketch aggregate, merge, and query functions
  - <https://spark.apache.org/docs/latest/api/sql/index.html>

- Google BigQuery:
  - `APPROX_COUNT_DISTINCT`
  - `APPROX_QUANTILES`
  - `APPROX_TOP_COUNT`
  - `APPROX_TOP_SUM`
  - <https://docs.cloud.google.com/bigquery/docs/reference/standard-sql/approximate_aggregate_functions>

- ClickHouse:
  - Approximate distinct-count variants.
    - `uniq`
    - `uniqCombined`
    - `uniqHLL12`
    - `uniqTheta`
  - Quantile variants such as t-digest and GK-family functions.
  - <https://clickhouse.com/docs/sql-reference/aggregate-functions/reference>

- Snowflake:
  - HLL functionality
  - MinHash and similarity functionality
  - Approximate top-k functionality
  - Approximate percentile functionality
  - Accumulate, combine, and estimate style functions for some approximate states
  - <https://docs.snowflake.com/en/sql-reference/functions-aggregation>

- Apache Druid:
  - DataSketches Theta aggregators
  - DataSketches HLL aggregators
  - DataSketches Quantiles aggregators
  - Documentation also discusses older approximate histogram and cardinality implementations.
  - <https://druid.apache.org/docs/latest/querying/aggregations/>

- Apache DataSketches:
  - A production-quality sketch library rather than a complete AQP system.
  - Provides sketch implementations and adaptors for multiple systems.
    - Examples: Hive, Pig, PostgreSQL, BigQuery, and Druid.
  - Provides cross-language implementations and binary compatibility goals.
  - <https://datasketches.apache.org/>
  - <https://datasketches.apache.org/docs/Architecture/SketchesByComponent.html>

- Takeaway:
  - Approximate functionality is common.
  - The support is fragmented across several forms.
    - System-specific functions.
    - Sketch-state APIs.
    - UDFs and extensions.
    - Standalone sketch libraries.
  - This motivates AQPBMV2's middle layer.
    - Executable approximate-function candidates.
    - Benchmark-owned kernels.
