# Polars Throughput Tests Explained

This document covers the Polars benchmarks under `throughput`, namely:

- `polars_freq`
- `polars_cardinality`
- `polars_quantile`

These are not sketch implementations. They are exact-computation reference baselines built with Polars. The point of including them in the throughput plots is not to claim that Polars is a better sketch, but to show what throughput looks like when the same task is solved with exact columnar operations instead of approximate summaries.

## Overall structure

Each Polars family has two benchmark entry points:

- `src/main.rs`: insertion-stage throughput
- `src/bin/query.rs`: query-stage throughput

The three Polars families correspond to three sketch task types:

- `polars_freq` corresponds to frequency and is compared with the `cms` / `cs` panel.
- `polars_cardinality` corresponds to cardinality and is compared with the `hll` panel.
- `polars_quantile` corresponds to quantiles and is compared with the `kll` / `dd` panel.

`throughput/scripts/run_throughput_polars.sh` wires them into the throughput pipeline, and `throughput/scripts/plot_throughput_overview.py` places their results into the overview figures.

## 1. Frequency: `polars_freq`

### What insertion measures

File: `throughput/polars_freq/src/main.rs`

The insertion benchmark is not a sketch-style per-item update loop. Instead, it measures the throughput of building an exact frequency table from the full dataset:

```rust
df
    .lazy()
    .group_by([col("v")])
    .agg([len().alias("count")])
    .collect()?;
```

This means:

- group by `v`
- count rows in each group
- produce a complete exact `value -> count` table

So this benchmark measures how fast Polars can transform the raw column into an exact grouped frequency result.

### What query measures

File: `throughput/polars_freq/src/bin/query.rs`

The query benchmark answers this question: for a batch of keys, what are their exact frequencies?

Lazy version:

```rust
let counts_lazy = df
    .lazy()
    .group_by([col("v")])
    .agg([len().alias("count")]);

keys_df
    .lazy()
    .join(counts_lazy, [col("v")], [col("v")], JoinArgs::new(JoinType::Left))
    .collect()?;
```

Eager version:

```rust
let counts_df = df.group_by(["v"])?.select(["v"]).count()?;
keys_df.left_join(&counts_df, ["v"], ["v"])?;
```

Both versions perform the same logical task:

1. Build an exact count table from the full dataset.
2. Join the query keys against that table.
3. Return the exact count for each key.

Here, `keys` are the distinct values extracted from the dataset. This is therefore closer to a batched "look up all candidate keys" workload than to repeated random point queries.

### Why it is measured this way

There are two reasons:

- A sketch frequency query is conceptually "given a key, return a count estimate". The closest exact Polars counterpart is to build counts with `group_by + count` and then perform key lookup.
- Using `group_by` lets Polars run the task through its natural columnar aggregation path rather than through an artificial custom loop.

## 2. Cardinality: `polars_cardinality`

### What insertion measures

File: `throughput/polars_cardinality/src/main.rs`

The operation here is:

```rust
df
    .lazy()
    .select([col("v").n_unique().alias("exact_cardinality")])
    .collect()?;
```

This computes:

- the exact number of distinct values in the full column

It corresponds to the same high-level question that HLL answers, except Polars returns the exact cardinality instead of an approximation.

### What query measures

File: `throughput/polars_cardinality/src/bin/query.rs`

Lazy version:

```rust
df
    .lazy()
    .select([col("v").n_unique().alias("exact_cardinality")])
    .collect()?;
```

Eager version:

```rust
let series = value_col.as_materialized_series();
let estimate = series.n_unique()? as f64;
```

Both versions answer the same query:

- "How many distinct values are present in the full dataset?"

This is a global aggregate query, not a point lookup query.

### Why it is measured this way

The natural HLL query semantic is "give me a cardinality estimate". The most direct exact equivalent in Polars is `n_unique()`.

So this benchmark is not trying to simulate a general OLAP workload. It is intentionally measuring the exact baseline that is closest to the HLL query itself.

## 3. Quantile: `polars_quantile`

### What insertion measures

File: `throughput/polars_quantile/src/main.rs`

This benchmark first builds 101 quantile expressions for `p0..p100`:

```rust
col("v")
    .quantile(lit(rank), QuantileMethod::Linear)
    .alias(format!("p_{percentile}"))
```

It then executes:

```rust
df.lazy().select(quantile_exprs.clone()).collect()?;
```

So the insertion benchmark effectively measures:

- given the full column
- compute all exact quantiles from `0%` to `100%` in one batch

This is not analogous to a sketch incrementally maintaining a compact summary during updates. It is a full exact batch quantile aggregation.

### What query measures

File: `throughput/polars_quantile/src/bin/query.rs`

The lazy version still computes `p0..p100` in one `select(...).collect()` call.

The eager version instead performs repeated quantile calls on the same materialized series:

```rust
series
    .f64()?
    .quantile(percentile as f64 / 100.0, QuantileMethod::Linear)?
```

That means the eager path computes `p0..p100` as 101 separate eager quantile calls.

The query CSV stores one row per percentile, but the overview plot merges all 101 rows from the same `(run, repeat)` batch and converts them into an effective "queries per second" value.

### Why it is measured this way

This case is the most important to interpret carefully:

- A sketch quantile query is usually "given a rank, return an estimate".
- If Polars only computed one percentile per run, the measurement would be dominated more heavily by framework overhead and would be harder to compare against repeated sketch queries.
- The lazy version therefore computes the full `p0..p100` set in one batched query.
- The eager version computes them one by one to reflect a non-lazy exact query style.

This is also why Polars quantile queries often look especially slow: Polars is doing exact quantile work over the real data, while the sketch answers from a compressed approximate summary.

## Lazy versus eager

In the current code:

- insertion benchmarks remain lazy
- query benchmarks support `--engine lazy|eager`

The eager query semantics are:

- `freq`: eagerly build exact grouped counts, then `left_join` on the query keys
- `cardinality`: call `n_unique()` directly on the materialized `Series`
- `quantile`: call `.quantile(...)` repeatedly on the materialized `Series`

So eager does not change the mathematical task. It changes the Polars execution style from lazy query planning plus `collect()` to eager dataframe/series operations.

<!-- ## What the `variant` argument actually means

All Polars families accept a `--variant` argument, but for Polars this is mainly about fitting into the existing benchmark and output layout. It does not mean that Polars switches to a different sketch-like internal algorithm.

Examples:

- `polars_freq --variant cms|cs`
- `polars_quantile --variant kll|dd`
- `polars_cardinality --variant hll`

The main purpose of these variants is:

- to place outputs in the matching comparison family
- to make the plots line up with the corresponding sketch family

Polars itself is still computing exact frequency, exact cardinality, and exact quantiles. -->

## Short summary

The Polars throughput tests use exact columnar operations as baselines:

- frequency: `group_by + count`, then `join` for query
- cardinality: `n_unique`
- quantile: `quantile`

They are measured this way because these operations are the closest exact counterparts to the semantics of the corresponding sketch queries. The lazy versus eager distinction then measures the same exact task under different Polars execution models.
