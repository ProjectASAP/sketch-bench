# AQP Benchmark MVP v1 Design

> Purpose: align the Phase-2 MVP goal before adding more query types,
> backends, or data sources.
>
> Status: implemented first slice. Sampling backend is intentionally deferred.

## End Goal

MVP v1 proves that `sketch-bench` can benchmark an **approximate query plan**,
not just an isolated sketch. The demo should show this full path:

```text
SQL query
  -> DataFusion parser / LogicalPlan
  -> AQP task lowering
  -> synthetic relational data source
  -> exact backend + sketch backend
  -> throughput / memory / accuracy report
```

The MVP query is:

```sql
SELECT region, COUNT(DISTINCT user_id) AS users
FROM events
GROUP BY region;
```

This is intentionally small, but it changes the benchmark unit from
`HLL update throughput` to:

```text
For the same grouped distinct-count query, how do exact and approximate
execution behave under a controlled data source?
```

## Scope

MVP v1 supports one AQP query shape:

- table: `events`
- group key: `region`
- aggregate: `COUNT(DISTINCT user_id)`

The query frontend uses DataFusion only for parsing/planning:

- `DFParser::parse_sql(...)` parses SQL.
- `SqlToRel::statement_to_plan(...)` produces a DataFusion `LogicalPlan`.
- `aqp-core` lowers the supported `Aggregate` plan into:

```rust
AqpTask::CountDistinct {
    table: "events",
    column: "user_id",
    group_by: ["region"],
}
```

DataFusion does **not** execute the query in this MVP. Execution is handled by
our own backends.

## Execution Backends

MVP v1 runs the same lowered query task with two fixed backends:

| Policy | Implementation | Role |
|---|---|---|
| `exact` | `HashMap<region, HashSet<user_id>>` | Ground truth and exact-cost baseline |
| `sketch` | `HashMap<region, asap_sketchlib::HyperLogLog<Classic>>` | Approximate grouped distinct-count backend |

The exact backend returns the true per-region distinct count. The sketch
backend returns per-region HLL estimates. The report compares sketch estimates
against exact results group by group.

## Synthetic Data Source

MVP v1 uses a synthetic `events` relation, not a real file or DataFusion scan.
The generated rows contain:

- `region`
- `user_id`

The source is configurable enough to test whether data distribution matters:

- row count
- number of regions
- user cardinality
- user distribution: uniform or Zipf
- region skew toward `region_000`
- seed

Example:

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

## MVP Output

The MVP emits one JSON object with:

- lowered AQP task
- synthetic data-source parameters
- exact backend throughput and rough memory estimate
- sketch backend throughput and rough memory estimate
- mean / p95 / max relative error
- worst group

This is enough to demonstrate an AQP-style comparison:

```text
same query + same data source + different backend => cost/error tradeoff
```

## Success Criteria

MVP v1 is successful if it can demonstrate all of the following:

- A SQL query is parsed by DataFusion into a logical plan.
- The supported grouped `COUNT(DISTINCT)` plan is lowered into an AQP task.
- The task runs on a synthetic relational data source.
- The same task runs with exact and `asap_sketchlib` sketch backends.
- The report compares throughput, rough memory, and per-group error.
- Changing source knobs, such as Zipf user distribution or region skew, changes
  the measured result.

## Non-Goals

MVP v1 does not attempt to be the full AQP benchmark.

Out of scope:

- sampling backend
- hybrid backend
- budget parsing or enforcement
- real CSV/Parquet/trace data sources
- DataFusion physical execution
- multi-operator plans
- composed error across operators
- shard/merge benchmark
- visualization-ready final report schema

## Next Steps

The next phase should turn this MVP into a broader benchmark by adding:

- source-sensitivity sweeps over distribution, skew, cardinality, and group count
- a stable AQP report schema
- trace or file-backed `events` sources
- more query shapes, such as grouped quantile or top-k
- budget concepts, such as error target, latency target, and memory target
- sampling backend after the exact/sketch comparison is clear

The key open research/design question is whether we should define an
“approximation level” that describes how approximate a query execution is. That
could mean backend policy, sketch configuration, expected error budget, or the
fraction of the query plan implemented approximately. MVP v1 exposes the need
for that concept but does not define it yet.
