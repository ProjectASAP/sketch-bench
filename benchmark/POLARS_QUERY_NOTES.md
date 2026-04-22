# Polars Query Notes

## Query Shape

The current query path is:

```rust
let df = DataFrame::new(...)?;
let lazy = df
    .lazy()
    .select([col("v").n_unique().alias("exact_cardinality")]);
let result = lazy.collect()?;
let estimate = result
    .column("exact_cardinality")?
    .as_materialized_series()
    .cast(&DataType::Float64)?
    .f64()?
    .get(0)
    .unwrap_or(f64::NAN);
```

This splits into three main stages:

1. Build the input `DataFrame`.
2. Build the `LazyFrame` query plan.
3. Execute the plan with `collect()` and materialize the output `DataFrame`.

## `DataFrame` vs `LazyFrame`

`DataFrame` and `LazyFrame` are related, but they are not the same kind of object.

- `DataFrame` is materialized data already held in memory.
- `LazyFrame` is a deferred query plan over data.

In this query:

- `df` is the input table.
- `lazy` is the logical query plan produced by `df.lazy().select(...)`.
- `result` is the executed output table returned by `lazy.collect()`.

The `LazyFrame` does not contain the final query result. It only describes how to produce that result.

## What `collect()` Does

`collect()` is the step that actually runs the lazy query plan.

Before `collect()`:

- the query has not been executed,
- `n_unique()` has not been computed,
- the object is still just a `LazyFrame`.

After `collect()`:

- the query has been executed,
- the result has been materialized,
- the output is a regular `DataFrame`.

For the cardinality query, that output `DataFrame` contains the exact cardinality result.

## Why the Cardinality Result Is `1 x 1`

The cardinality result is a one-row, one-column table because the query is a global aggregation with a single output expression:

```rust
select([col("v").n_unique().alias("exact_cardinality")])
```

That means:

- one row, because the aggregation is computed over the whole input column,
- one column, because only one aggregation expression is selected.

So the probe output:

```text
[DETAIL] result_shape=(1, 1)
```

refers to the materialized output `DataFrame` returned by `collect()`.

The input table is different:

```text
[DETAIL] input_df_shape=(10000000, 1)
```

This means the input has 10,000,000 rows and one column, while the output has one row and one column.

## What the Lazy Plan Output Means

The probe prints a textual representation of the lazy plan, for example:

```text
SELECT [col("v").n_unique().alias("exact_cardinality")] FROM
  DF ["v"]; PROJECT 1/1 COLUMNS
```

This is the query plan, not the query result.

It describes:

- the source input (`DF ["v"]`),
- the selected expression (`col("v").n_unique()`),
- the projected columns needed by the plan.

This textual form is what Polars planned for the `LazyFrame`.

## Why Reading the Scalar Requires Extra Steps

After `collect()`, the result is still a `DataFrame`, even though it only contains one value.

To read that single value as a Rust scalar, the code performs a sequence of extraction steps:

1. select the `"exact_cardinality"` column,
2. access its materialized `Series`,
3. cast it to `Float64`,
4. view it as `f64`,
5. read the first element.

These steps do not recompute the query. They only extract and convert the already computed result from the output `DataFrame`.

## Why Cardinality and Quantile Can Share a Lazy Query Shape

Cardinality and quantile can both be expressed as global aggregations over the full input column.

Examples:

```rust
df.lazy().select([col("v").n_unique().alias("exact_cardinality")])
```

```rust
df.lazy().select([
    col("v").quantile(lit(0.5), QuantileMethod::Linear).alias("p_50"),
    col("v").quantile(lit(0.9), QuantileMethod::Linear).alias("p_90"),
])
```

Both forms produce a single output row:

- cardinality: `1 x 1`,
- multiple quantiles: `1 x N`.

They differ only in how many aggregate expressions are selected.

## Why Frequency Is Different

Frequency is not a single global aggregation. It is a grouped aggregation:

```rust
df.lazy()
    .group_by([col("v")])
    .agg([len().alias("count")])
```

This changes the output shape:

- one row per distinct key,
- not one row for the whole input.

So frequency does not match the same single-row query shape used by cardinality and quantile.

In practice:

- cardinality output shape: `1 x 1`,
- quantile output shape: `1 x N`,
- frequency output shape: `K x 2`, where `K` is the number of distinct keys.

## Current `polars_plan_probe` Results

The following notes are based on the current output in `benchmark/polars_plan_probe_result.txt`.

The probe was run on:

- input dataset: `../input/benchmark_data_10m_int64_zipf_s11_k500000.bin`
- input row count: `10,000,000`

The reported timing breakdown for each query is:

- `load_dataset_ms`: reading the binary file into memory
- `input_df_build_ms`: building the input `DataFrame`
- `lazy_plan_build_ms`: constructing the `LazyFrame`
- `lazy_result_collect_ms`: executing the lazy plan and materializing the lazy result `DataFrame`
- `lazy_result_read_ms`: reading a summary view of the lazy result `DataFrame`
- `eager_result_collect_ms`: executing the eager path and materializing the eager result `DataFrame`
- `eager_result_read_ms`: reading a summary view of the eager result `DataFrame`
- `lazy_full_query_collect_ms`: `load_dataset_ms + input_df_build_ms + lazy_plan_build_ms + lazy_result_collect_ms`
- `eager_full_query_collect_ms`: `load_dataset_ms + input_df_build_ms + eager_result_collect_ms`
- `lazy_full_query_plus_read_ms`: `load_dataset_ms + input_df_build_ms + lazy_plan_build_ms + lazy_result_collect_ms + lazy_result_read_ms`
- `eager_full_query_plus_read_ms`: `load_dataset_ms + input_df_build_ms + eager_result_collect_ms + eager_result_read_ms`

The `collect` timings stop when the result `DataFrame` has been built.

The `read` timings measure a lightweight summary read after result construction:

- single-row results: read `shape`, `schema`, and a few representative scalar values,
- multi-row grouped results: read `shape`, `schema`, and representative values from the first and last rows.

These read timings do not do a full-table scan.

### 1. Exact Cardinality

Rust query:

```rust
let lazy = df
    .lazy()
    .select([col("v").n_unique().alias("exact_cardinality")]);
let lazy_result = lazy.collect()?;

let eager_result = {
    let exact = df.column("v")?.as_materialized_series().n_unique()? as u32;
    DataFrame::new(vec![Column::new("exact_cardinality".into(), [exact])])?
};
```

Current timing:

- `lazy_full_query_collect_ms = 306.232`
- `eager_full_query_collect_ms = 315.464`
- `lazy_full_query_plus_read_ms = 306.235`
- `eager_full_query_plus_read_ms = 315.468`
- `load_dataset_ms = 104.033`
- `input_df_build_ms = 48.585`
- `lazy_plan_build_ms = 0.047`
- `lazy_result_collect_ms = 153.567`
- `lazy_result_read_ms = 0.003`
- `eager_result_collect_ms = 162.847`
- `eager_result_read_ms = 0.004`

Current output shape:

- input: `(10000000, 1)`
- lazy result: `(1, 1)`
- eager result: `(1, 1)`

Current optimized plan:

```text
SELECT [col("v").n_unique().alias("exact_cardinality")] FROM
  DF ["v"]; PROJECT 1/1 COLUMNS
```

Observation:

- In this run, lazy and eager exact cardinality are close, with lazy slightly faster.
- The read step only touches one scalar result value.

### 2. Approximate Cardinality

Rust query:

```rust
let lazy = df
    .lazy()
    .select([col("v").approx_n_unique().alias("approx_cardinality")]);
let lazy_result = lazy.collect()?;

let eager_result = {
    let approx = df.column("v")?.approx_n_unique()? as u32;
    DataFrame::new(vec![Column::new("approx_cardinality".into(), [approx])])?
};
```

Current timing:

- `lazy_full_query_collect_ms = 216.127`
- `eager_full_query_collect_ms = 216.005`
- `lazy_full_query_plus_read_ms = 216.166`
- `eager_full_query_plus_read_ms = 216.007`
- `load_dataset_ms = 104.033`
- `input_df_build_ms = 48.277`
- `lazy_plan_build_ms = 0.005`
- `lazy_result_collect_ms = 63.813`
- `lazy_result_read_ms = 0.039`
- `eager_result_collect_ms = 63.696`
- `eager_result_read_ms = 0.002`

Current output shape:

- input: `(10000000, 1)`
- lazy result: `(1, 1)`
- eager result: `(1, 1)`

Current optimized plan:

```text
SELECT [col("v").approx_n_unique().alias("approx_cardinality")] FROM
  DF ["v"]; PROJECT 1/1 COLUMNS
```

Observation:

- In this run, lazy and eager approximate cardinality are effectively identical.
- The read step only touches one scalar result value.

### 3. Quantile Batch (`p_0` to `p_100`)

Rust query:

```rust
let lazy = df.lazy().select(build_quantile_exprs());
let lazy_result = lazy.collect()?;

let eager_result = {
    let series = df
        .column("v")?
        .as_materialized_series()
        .cast(&DataType::Float64)?;
    DataFrame::new(build_quantile_columns(&series)?)?
};
```

The quantile expression builder is:

```rust
fn build_quantile_exprs() -> Vec<Expr> {
    (0..NUM_PERCENTILES)
        .map(|percentile| {
            let rank = percentile as f64 / 100.0;
            col("v")
                .quantile(lit(rank), QuantileMethod::Linear)
                .alias(format!("p_{percentile}"))
        })
        .collect()
}
```

Current timing:

- `lazy_full_query_collect_ms = 1619.214`
- `eager_full_query_collect_ms = 10844.674`
- `lazy_full_query_plus_read_ms = 1619.235`
- `eager_full_query_plus_read_ms = 10844.689`
- `load_dataset_ms = 104.033`
- `input_df_build_ms = 47.987`
- `lazy_plan_build_ms = 0.044`
- `lazy_result_collect_ms = 1467.150`
- `lazy_result_read_ms = 0.021`
- `eager_result_collect_ms = 10692.654`
- `eager_result_read_ms = 0.015`

Current output shape:

- input: `(10000000, 1)`
- lazy result: `(1, 101)`
- eager result: `(1, 101)`

Observation:

- The plan is still a single global `SELECT`, but the eager implementation is much slower because it computes quantiles one by one at the eager layer.
- The read step only samples `p_0` and `p_100`, not all `101` output columns.

### 4. Frequency

Rust query:

```rust
let lazy = df
    .lazy()
    .group_by([col("v")])
    .agg([len().alias("count")]);
let lazy_result = lazy.collect()?;

let eager_result = df.group_by(["v"])?.select(["v"]).count()?;
```

Current timing:

- `lazy_full_query_collect_ms = 301.265`
- `eager_full_query_collect_ms = 300.141`
- `lazy_full_query_plus_read_ms = 301.276`
- `eager_full_query_plus_read_ms = 300.170`
- `load_dataset_ms = 104.033`
- `input_df_build_ms = 69.158`
- `lazy_plan_build_ms = 0.008`
- `lazy_result_collect_ms = 128.067`
- `lazy_result_read_ms = 0.011`
- `eager_result_collect_ms = 126.950`
- `eager_result_read_ms = 0.029`

Current output shape:

- input: `(10000000, 1)`
- lazy result: `(386007, 2)`
- eager result: `(386007, 2)`

Current optimized plan:

```text
AGGREGATE
	[len().alias("count")] BY [col("v")] FROM
  DF ["v"]; PROJECT 1/1 COLUMNS
```

Observation:

- This is not a single-row global aggregation.
- It returns one row per distinct value, which is why the result shape is `386007 x 2`.
- In this run, eager frequency is faster than lazy frequency.
- The read step samples `shape`, `schema`, and representative values from the first and last grouped rows.

### 5. Cardinality Plus Quantile

Rust query:

```rust
let mut exprs = vec![
    col("v").n_unique().alias("exact_cardinality"),
    col("v").approx_n_unique().alias("approx_cardinality"),
];
exprs.extend(build_quantile_exprs());
let lazy = df.lazy().select(exprs);
let lazy_result = lazy.collect()?;

let eager_result = cardinality_plus_quantile_eager(df)?;
```

Current timing:

- `lazy_full_query_collect_ms = 1673.437`
- `eager_full_query_collect_ms = 11139.487`
- `lazy_full_query_plus_read_ms = 1673.474`
- `eager_full_query_plus_read_ms = 11139.505`
- `load_dataset_ms = 104.033`
- `input_df_build_ms = 80.855`
- `lazy_plan_build_ms = 0.052`
- `lazy_result_collect_ms = 1488.498`
- `lazy_result_read_ms = 0.037`
- `eager_result_collect_ms = 10954.599`
- `eager_result_read_ms = 0.018`

Current output shape:

- input: `(10000000, 1)`
- lazy result: `(1, 103)`
- eager result: `(1, 103)`

Observation:

- This is the clearest example of a shared single-row lazy query shape.
- The query combines exact cardinality, approximate cardinality, and `101` quantiles into one global `SELECT`.
- The eager version is much slower because it manually computes the full quantile batch at the eager layer.
- The read step samples `exact_cardinality`, `approx_cardinality`, `p_0`, and `p_100`.

### 6. Cardinality Plus Frequency

Rust query:

```rust
let global = df.clone().lazy().select([
    col("v").n_unique().alias("exact_cardinality"),
    col("v").approx_n_unique().alias("approx_cardinality"),
]);
let grouped = df
    .lazy()
    .group_by([col("v")])
    .agg([len().alias("count")]);
let lazy = global.cross_join(grouped, None);
let lazy_result = lazy.collect()?;

let eager_result = cardinality_plus_frequency_eager(df)?;
```

Current timing:

- `lazy_full_query_collect_ms = 343.611`
- `eager_full_query_collect_ms = 520.596`
- `lazy_full_query_plus_read_ms = 345.317`
- `eager_full_query_plus_read_ms = 520.605`
- `load_dataset_ms = 104.033`
- `input_df_build_ms = 69.266`
- `lazy_plan_build_ms = 0.007`
- `lazy_result_collect_ms = 170.306`
- `lazy_result_read_ms = 1.706`
- `eager_result_collect_ms = 347.298`
- `eager_result_read_ms = 0.009`

Current output shape:

- input: `(10000000, 1)`
- lazy result: `(386007, 4)`
- eager result: `(386007, 4)`

Current optimized plan shape:

```text
CROSS JOIN:
LEFT PLAN ON: []
   SELECT [col("v").n_unique().alias("exact_cardinality"), col("v").approx_n_unique().alias("approx_cardinality")] FROM
    DF ["v"]; PROJECT 1/1 COLUMNS
RIGHT PLAN ON: []
  AGGREGATE
  	[len().alias("count")] BY [col("v")] FROM
    DF ["v"]; PROJECT 1/1 COLUMNS
END CROSS JOIN
```

Observation:

- Frequency cannot stay inside the same single-row `SELECT` shape as cardinality.
- In this combined probe, the query shape becomes a `CROSS JOIN` between a one-row global aggregation and a multi-row grouped aggregation.
- The lazy cross-join path is faster than the eager path in this run.
- The read step samples the global cardinality columns plus the grouped frequency columns from the first and last rows.

### 7. Quantile Plus Frequency

Rust query:

```rust
let global = df.clone().lazy().select(build_quantile_exprs());
let grouped = df
    .lazy()
    .group_by([col("v")])
    .agg([len().alias("count")]);
let lazy = global.cross_join(grouped, None);
let lazy_result = lazy.collect()?;

let eager_result = quantile_plus_frequency_eager(df)?;
```

Current timing:

- `lazy_full_query_collect_ms = 1635.642`
- `eager_full_query_collect_ms = 10907.096`
- `lazy_full_query_plus_read_ms = 1636.747`
- `eager_full_query_plus_read_ms = 10907.142`
- `load_dataset_ms = 104.033`
- `input_df_build_ms = 86.471`
- `lazy_plan_build_ms = 0.060`
- `lazy_result_collect_ms = 1445.079`
- `lazy_result_read_ms = 1.105`
- `eager_result_collect_ms = 10716.593`
- `eager_result_read_ms = 0.046`

Current output shape:

- input: `(10000000, 1)`
- lazy result: `(386007, 103)`
- eager result: `(386007, 103)`

Observation:

- This is the quantile analogue of the previous case.
- The global quantile batch stays on the left side of the plan and the grouped frequency result stays on the right side.
- The overall query shape is still a `CROSS JOIN`.
- As with the quantile-only query, the quantile side dominates total eager execution time.
- The read step samples `p_0`, `p_100`, and the grouped frequency columns from the first and last rows.

### 8. Cardinality Plus Quantile Plus Frequency

Rust query:

```rust
let mut exprs = vec![
    col("v").n_unique().alias("exact_cardinality"),
    col("v").approx_n_unique().alias("approx_cardinality"),
];
exprs.extend(build_quantile_exprs());
let global = df.clone().lazy().select(exprs);
let grouped = df
    .lazy()
    .group_by([col("v")])
    .agg([len().alias("count")]);
let lazy = global.cross_join(grouped, None);
let lazy_result = lazy.collect()?;

let eager_result = cardinality_plus_quantile_plus_frequency_eager(df)?;
```

Current timing:

- `lazy_full_query_collect_ms = 1617.452`
- `eager_full_query_collect_ms = 11210.888`
- `lazy_full_query_plus_read_ms = 1622.131`
- `eager_full_query_plus_read_ms = 11210.919`
- `load_dataset_ms = 104.033`
- `input_df_build_ms = 69.516`
- `lazy_plan_build_ms = 0.055`
- `lazy_result_collect_ms = 1443.849`
- `lazy_result_read_ms = 4.679`
- `eager_result_collect_ms = 11037.339`
- `eager_result_read_ms = 0.031`

Current output shape:

- input: `(10000000, 1)`
- lazy result: `(386007, 105)`
- eager result: `(386007, 105)`

Observation:

- This is the widest combined query in the current probe.
- The left side is the single-row global aggregation block and the right side is the grouped frequency block.
- The result is no longer a single row because the `CROSS JOIN` repeats the global row for every grouped frequency row.
- The eager path is again dominated by repeated eager quantile computation.
- The read step samples the global summary columns and the grouped frequency columns from the first and last rows.
<!-- 
## Current High-Level Takeaways

From the current probe output:

- `lazy_plan_build_ms` is tiny in every query.
- `n_unique` and `approx_n_unique` both fit naturally into a single-row global `SELECT`.
- the quantile batch also fits naturally into that same single-row global `SELECT` shape.
- frequency does not fit that shape because it is a grouped aggregation with one output row per distinct key.
- combining frequency with global aggregates changes the plan shape from a single `SELECT` into a `CROSS JOIN`.
- `collect` times measure result construction only; `read` times measure a later lightweight result-summary read.
- for exact cardinality and approximate cardinality, eager and lazy are close.
- for frequency without sorting, eager is faster than lazy in this run.
- for quantile-heavy queries, lazy is dramatically faster than eager.
- for single-row outputs, the read step is tiny because it touches only a few scalar values.
- for cross-join outputs, the lazy read step can be noticeably larger because even summary reads still touch columns in a much wider multi-row `DataFrame`.
- the main reason is not plan construction; it is the cost of eager repeated quantile work versus lazy batched projection.
- the current `explain()` output is useful for understanding logical plan shape, but it does not directly reveal execution-level threading or parallel scheduling. -->

## Result Collect Timing Summary

| Query | Eager result collect (ms) | Lazy result collect (ms) |
| --- | ---: | ---: |
| `df.lazy().select([col("v").n_unique().alias("exact_cardinality")])` | 162.847 | 153.567 |
| `df.lazy().select([col("v").approx_n_unique().alias("approx_cardinality")])` | 63.696 | 63.813 |
| `df.lazy().select(build_quantile_exprs())` | 10692.654 | 1467.150 |
| `df.lazy().group_by([col("v")]).agg([len().alias("count")])` | 126.950 | 128.067 |
| `df.lazy().select([col("v").n_unique().alias("exact_cardinality"), col("v").approx_n_unique().alias("approx_cardinality"), build_quantile_exprs()...])` | 10954.599 | 1488.498 |
| `global_cardinality.cross_join(grouped_frequency)` | 347.298 | 170.306 |
| `global_quantiles.cross_join(grouped_frequency)` | 10716.593 | 1445.079 |
| `global_cardinality_and_quantiles.cross_join(grouped_frequency)` | 11037.339 | 1443.849 |
