# Blog Outline: AQPBM

> Status: currently focused on AQPBMV2.
>
> Goal: explain the problem scope of AQPBM and the contribution of the current AQPBMV2 design.

- Main AQPBMV2 claim:
  - AQPBMV2 provides a `toolkit` for benchmarking runnable approximate-function implementations.
  - It sits between raw sketch benchmarks and full system benchmarks.
  - It compares exact baselines and sketch-backed implementations under the same execution modes.
  - It organizes benchmark targets by approximate functionality.
    - Count distinct.
    - Heavy hitters.
    - Quantile.
  - A sketch instance is a candidate implementation detail.
  - A sketch instance alone is not the benchmark unit.
  - The current implementation supports count distinct, heavy hitters, and quantile.

## Query to begin with: SELECT approx_count_distinct(user_id) FROM table;

- Example query:
  - `SELECT approx_count_distinct(user_id) FROM table;`

- Reason to pick this query as a motivation:
  - It is a common approximate aggregate (already supported in many places).
  - The exact table still exists.
  - The system replaces one exact aggregate with an approximate function.
  - Users trade some fidelity for lower latency, memory, or CPU.
  - See `Existing Approximate Query Support` at the end for examples.

- Why AQPBMV2 should not benchmark this SQL query directly:
  - In that case, the benchmark target would become the whole SQL system.
  - Different databases expose different functions and execution plans.
  - The question would become:
    - Are we benchmarking Spark?
    - Are we benchmarking Trino?
    - Are we benchmarking ClickHouse?
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
  - It then evaluates runnable implementations of approximate functionality.

- Mental experiment:
  - Suppose a query is implemented using three HLL states.
  - Each individual HLL state may look bad in isolation.
    - Example: 30% relative error per raw sketch state.
  - The final query answer may still be good.
    - Example: 1% relative error after the full function logic.
  - A user would choose the full approximate function because it gives good query performance.
    - Not because every raw sketch component looks good in isolation.
  - This is why AQPBMV2 should evaluate approximate-function implementations.

## Current: AQPBMV2

- Scope:
  - AQPBMV2 is a runnable approximate-function benchmark toolkit.
  - It is not a full SQL, PromQL, or AQP system benchmark.
  - It is not a raw sketch benchmark.
  - Its benchmark unit is a runnable implementation of a functionality.
    - Example: count distinct implemented by an exact `HashSet`.
    - Example: count distinct implemented by Apache DataSketches HLL.
    - Example: quantile implemented by `asap_sketchlib` KLL.
  - A sketch can be part of the implementation.
  - The sketch API alone is not the benchmark unit.

- Main toolkit contribution:
  - AQPBMV2 should provide a reusable toolkit.
  - The toolkit should let users implement and compare approximate-function implementations.
  - The toolkit should provide execution modes and a report format.
  - The toolkit should make it easy to add:
    - more sketch libraries;
    - more exact baselines;
    - more functionality classes;
    - more sketch compositions;
    - more workload generators;
    - more comparison metrics.
  - An implementation may wrap one sketch.
  - An implementation may also compose multiple sketches.
  - This is about implementing an approximate functionality.
  - It is not yet a claim about supporting complex SQL.

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
  -> runnable approximate-function implementation
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

- Current sketch-backed implementations:
  - Apache DataSketches HLL for count distinct.
  - Apache DataSketches FrequentItems for heavy hitters.
  - Apache DataSketches TDigest for quantile.
  - `sketch_oxide` HLL for count distinct.
  - `sketch_oxide` SpaceSaving for heavy hitters.
  - `sketch_oxide` TDigest for quantile.
  - `asap_sketchlib` HLL for count distinct.
  - `asap_sketchlib` CMSHeap for heavy hitters.
  - `asap_sketchlib` KLL for quantile.

- Current benchmark-owned execution modes:
  - `grouped_state`
    - Keep one implementation state per group.
    - Update that state from all rows in the group.
    - Finalize one answer per group.
  - `partitioned_merge`
    - Build implementation states inside partitions.
    - Merge partition-local states for each group.
    - Finalize one answer per group after merge.

- Current report metrics:
  - For count distinct and quantile:
    - How many group-level answers stay within the chosen error threshold.
    - Mean and max relative error for numeric outputs.
  - For heavy hitters:
    - Whether returned top-k items are true top-k items.
    - Whether true top-k items are missing from the returned result.

- Current smoke/demo path:
  - `cargo run -p aqp-core --example aqpbmv2_functions`
  - The current generated data is only a smoke test.
  - It proves implementations can be wired into the middle layer and run.
  - It is not yet benchmark-grade workload evidence.
  - The current implementations are still close to one-sketch-per-function cases.
  - The next step is to add workloads and implementations where the middle layer matters more than the raw sketch API.

- What AQPBMV2 does not claim:
  - It does not benchmark Spark's function directly.
  - It does not benchmark Trino's function directly.
  - It does not benchmark BigQuery's function directly.
  - It does not benchmark ClickHouse's function directly.
  - Those systems show that the functionality classes are real.

- Research bar:
  - AQPBMV2 is interesting only if benchmark-owned execution modes reveal new behavior.
    - The behavior should be something raw sketch benchmarks miss.
  - If they do not, AQPBMV2 is still a useful toolkit but a weaker paper contribution.

## AQPBMV2 Preliminary Result

- Command:
  - `cargo run -q -p aqp-core --example aqpbmv2_functions`

- Functionality coverage supported by this preliminary result:
  - Count distinct:
    - Apache DataSketches HLL supports the count-distinct functionality.
    - `sketch_oxide` HLL supports the count-distinct functionality.
    - `asap_sketchlib` HLL supports the count-distinct functionality.
  - Heavy hitters:
    - Apache DataSketches FrequentItems supports the heavy-hitter functionality.
    - `sketch_oxide` SpaceSaving supports the heavy-hitter functionality.
    - `asap_sketchlib` CMSHeap supports the heavy-hitter functionality.
  - Quantile:
    - Apache DataSketches TDigest supports the quantile functionality.
    - `sketch_oxide` TDigest supports the quantile functionality.
    - `asap_sketchlib` KLL supports the quantile functionality.
  - This is a capability and integration result.
  - It does not yet support a claim about general benchmark quality or winner libraries.

- What the input is:
  - Each input row has two logical fields.
    - `group_key`
    - `value`
  - A group means one distinct value of the synthetic `group_key`.
    - Example: `group_000`, `group_001`, ..., `group_031`.
    - This mimics one output group from `GROUP BY service`.
  - Current demo group assignment is hand-written in the synthetic generator.
    - It assigns rows to groups with `idx % group_count`.
    - It is not produced by a SQL interpreter, query planner, or real data schema.
  - The benchmark groups rows by `group_key`.
    - It then runs the target functionality over the `value` field inside each group.
    - For count distinct, it counts distinct `value`s inside each synthetic group.
    - For heavy hitters, it finds frequent `value`s inside each synthetic group.
    - For quantile, it estimates the p95 of `value`s inside each synthetic group.
  - Each group owns one independent aggregate state for the implementation being tested.
  - Any group-level claim below only refers to these synthetic demo groups.
    - It is not a claim about production groups or all possible group-by workloads.
  - Count distinct input:
    - 50,000 rows.
    - 32 synthetic group-by groups.
    - Deterministic value generator with value cardinality 20,000.
  - Heavy hitter input:
    - 80,000 rows.
    - 24 synthetic group-by groups.
    - Deterministic top-k pattern.
    - Per group, the top items are intentionally clear.
      - Roughly 45%, 20%, and 15% for the top three items.
      - Remaining rows are long-tail noise.
  - Quantile input:
    - 80,000 rows.
    - 16 synthetic group-by groups.
    - Deterministic periodic values with a small tail bump.
    - Query target is p95.

- What the output is:
  - A JSON report.
  - Exact baseline outputs for each functionality class.
  - Candidate outputs under the `grouped_state` execution mode.
  - Candidate outputs under the `partitioned_merge` execution mode.
  - Numeric summaries for count distinct and quantile.
    - Fraction of groups whose approximate answer is within the chosen error threshold.
    - Mean relative error.
    - Max relative error.
  - Set summaries for heavy hitters.
    - Fraction of returned top-k items that are actually correct.
    - Fraction of exact top-k items that were returned.

- What "within threshold" means here:
  - It is a workload-level metric.
  - It is not a statistical confidence interval.
  - It means the fraction of produced answers within the declared error threshold.
  - Example:
    - 32 count-distinct groups.
    - 32 groups within the relative-error threshold.
    - The run has `100%` of answers within threshold.

- Count distinct result:
  - Exact output has 32 groups.
  - All implementations have `100%` of group-level answers within the chosen error threshold.
  - Apache DataSketches HLL:
    - Mean relative error: about `0.76%`.
    - Max relative error: about `2.40%`.
  - `sketch_oxide` HLL:
    - Mean relative error: about `0.87%`.
    - Max relative error: about `2.27%`.
  - `asap_sketchlib` HLL:
    - Mean relative error: about `0.55%`.
    - Max relative error: about `1.47%`.

- Heavy hitter result:
  - Exact output has 24 groups.
  - Apache DataSketches FrequentItems:
    - Every returned top-k item is correct.
    - No exact top-k item is missing.
  - `sketch_oxide` SpaceSaving:
    - Every returned top-k item is correct.
    - No exact top-k item is missing.
  - `asap_sketchlib` CMSHeap:
    - Every returned top-k item is correct.
    - No exact top-k item is missing.

- Quantile result:
  - Exact output has 16 groups.
  - Apache DataSketches TDigest:
    - All group-level answers are within the chosen error threshold.
    - Mean relative error: approximately `0`.
  - `sketch_oxide` TDigest:
    - All group-level answers are within the chosen error threshold.
    - Mean relative error: about `0.39%`.
    - Max relative error: about `0.63%`.
  - `asap_sketchlib` KLL:
    - All group-level answers are within the chosen error threshold.
    - Mean relative error: `0`.

- What this result represents:
  - It shows the toolkit wiring works.
  - Exact baselines and sketch-backed implementations run through the same interface.
  - Multiple sketch libraries can be compared under the same benchmark-owned execution modes.
  - `grouped_state` and `partitioned_merge` both execute successfully.
  - It is still close to raw sketch comparison because each current implementation mostly wraps one sketch.
  - Its purpose is to show the middle-layer interface can host those implementations.

- What this result does not prove:
  - It does not prove one library is generally better.
  - It does not prove AQPBMV2 is already benchmark-grade.
  - It does not yet prove that AQPBMV2 reveals behavior missed by raw sketch benchmarks.
  - It does not validate the group-generation logic.
  - The generated data is too easy.
  - The result is a smoke test and preliminary validation of the toolkit.

- Next steps:
  - Replace toy generators with benchmark-grade workload generators.
  - Use explicit distributions.
    - Uniform.
    - Zipf.
    - Lognormal.
    - Pareto or heavy-tail mixtures.
  - Add workload knobs.
    - Group cardinality.
    - Group-key generation.
    - Mapping from workload/task semantics to group keys.
    - Group-size skew.
    - Tail heaviness.
    - Top-k gap.
    - Filter selectivity.
    - Correlation between group key and value.
    - Partition count and merge shape.
  - Add resource and performance measurement.
    - Update throughput.
    - Query latency.
    - Merge latency.
    - Memory or serialized state size.
  - Check whether AQPBMV2 execution modes reveal behavior that raw sketch benchmarks miss.

## Previous AQPBMV1

- AQPBMV1 is the existing raw-sketch benchmark layer.
  - Target: sketch primitives directly.
  - Input control: data shape and distribution.
  - Metrics: throughput, accuracy, and memory usage.
  - Role: provide primitive-level evidence for AQPBMV2.

## Future AQPBMV3

Placeholder.

## Future AQPBMV4

Placeholder.

## Future AQPBMV5

Placeholder.

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
    - Runnable approximate-function implementations.
    - Benchmark-owned execution modes.
