# AQP MVP

> Status: first executable Phase-2 slice. Sampling backend is intentionally
> deferred.

## What This Solves

Phase 1 benchmarks one sketch family at a time. That answers questions like
"how fast is this HLL update path?", but it does not answer an AQP question:

```text
Given a query plan and a data source, what happens if we execute the same
logical query with exact state versus approximate sketch state?
```

This MVP makes that unit of work a query:

```sql
SELECT region, COUNT(DISTINCT user_id) AS users
FROM events
GROUP BY region;
```

It is still deliberately small, but it is no longer just "run one HLL". It
benchmarks a plan shape:

```text
DataFusion SQL
  -> DataFusion LogicalPlan
  -> AQP CountDistinct task
  -> synthetic relational events source
  -> exact backend and sketch backend
  -> accuracy / throughput / memory summary
```

## How It Works

The query frontend lives in `aqp-core` and reuses DataFusion:

- DataFusion parses SQL and creates the `LogicalPlan` DAG.
- `aqp-core` lowers the supported plan shape into:

```rust
AqpTask::CountDistinct {
    table: "events",
    column: "user_id",
    group_by: ["region"],
}
```

The synthetic source generates an `events` relation with:

- `region`: group key
- `user_id`: distinct-count key

The first source knobs are:

- row count
- region count
- user cardinality
- user distribution: uniform or Zipf
- region skew toward `region_000`
- seed

The MVP has two execution backends:

| Policy | Implementation | Purpose |
|---|---|---|
| `exact` | `HashMap<region, HashSet<user_id>>` | Ground truth and exact cost baseline |
| `sketch` | `HashMap<region, asap_sketchlib::HyperLogLog<Classic>>` | Approximate grouped distinct-count backend |

The report compares sketch estimates against exact results per group and emits:

- exact throughput and rough memory
- sketch throughput and rough memory
- compared group count
- mean / p95 / max relative error
- worst group

## How To Run

Use the grouped query:

```bash
cargo run --config profile.dev.debug=0 --target-dir target-local \
  -p sketch-cli --no-default-features -- \
  aqp run \
  --query aqp-workloads/queries/count_distinct_users_by_region.sql \
  --workload uniform \
  --size 100000 \
  --cardinality 10000 \
  --regions 16 \
  --region-skew 0.0 \
  --report -
```

Try source sensitivity by changing only the source:

```bash
cargo run --config profile.dev.debug=0 --target-dir target-local \
  -p sketch-cli --no-default-features -- \
  aqp run \
  --query aqp-workloads/queries/count_distinct_users_by_region.sql \
  --workload zipf \
  --zipf-s 1.2 \
  --size 100000 \
  --cardinality 10000 \
  --regions 16 \
  --region-skew 0.8 \
  --report -
```

These runs answer a first AQP-style question:

```text
How do exact and sketch grouped distinct-count execution behave as the
synthetic data source changes?
```

## What This Does Not Solve Yet

This MVP is intentionally not the whole Phase 2 benchmark.

Deferred:

- sampling backend
- hybrid backend
- budget parsing and enforcement
- shard / merge benchmark
- trace-backed data sources
- interleaved streaming schedules
- multi-operator plans and composed error
- formal AQP JSONL schema shared with visualization

The next useful step is to run source-sensitivity sweeps over:

- uniform regions vs skewed regions
- uniform users vs Zipf users
- small groups vs many groups
- low vs high user cardinality

That will tell us whether the benchmark is exposing meaningful differences
between exact state and per-group sketch state before we add sampling.
