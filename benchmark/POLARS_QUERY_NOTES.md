# Polars Query Notes

This note explains the execution shape of the current Polars cardinality query used in `benchmark/src/bin/polars_cardinality_probe.rs`.

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
