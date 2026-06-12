# AQP Benchmark MVP v1 Design

> Purpose: align the Phase-2 MVP goal before adding more query types,
> backends, or data sources.
>
> Status: implemented multi-shape slice. Sampling backend is intentionally
> deferred.

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

The MVP supports a small admitted query set:

```sql
SELECT region, COUNT(DISTINCT user_id) AS users
FROM events
GROUP BY region;

SELECT region, approx_median(value) AS p50_value
FROM events
GROUP BY region;

SELECT approx_percentile_cont(value, 0.95) AS p95_value
FROM events;

SELECT user_id, COUNT(*) AS frequency
FROM events
GROUP BY user_id;
```

This is intentionally small, but it changes the benchmark unit from isolated
sketch operations such as `HLL update throughput` to:

```text
For the same admitted SQL query and controlled data source, how do exact and
approximate execution behave?
```

## Scope

MVP v1 supports these AQP query shapes:

| SQL shape | Lowered task | Approximate backend |
|---|---|---|
| `COUNT(DISTINCT user_id)` | `CountDistinct` | `asap_sketchlib::HyperLogLog<Classic>` |
| `approx_median(value)` / `approx_percentile_cont(value, q)` | `Quantile` | `asap_sketchlib::KLL<f64>` |
| `COUNT(*) GROUP BY user_id` | `Frequency` | `asap_sketchlib::CountMin` and `asap_sketchlib::Count` |

`COUNT(DISTINCT ...)` and quantile tasks may be ungrouped or grouped by
`region`. Frequency currently admits `events.user_id` keys only.

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

AqpTask::Quantile {
    table: "events",
    column: "value",
    quantile: 0.5,
    group_by: ["region"],
}

AqpTask::Frequency {
    table: "events",
    column: "user_id",
}
```

DataFusion does **not** execute the query in this MVP. Execution is handled by
our own backends.

## Execution Backends

MVP v1 runs the same lowered query task with an exact backend and the matching
`asap_sketchlib` backend or backends:

| Policy | Implementation | Role |
|---|---|---|
| `exact` | `HashMap<region, HashSet<user_id>>` | Ground truth and exact-cost baseline |
| `sketch` | `HashMap<region, asap_sketchlib::HyperLogLog<Classic>>` | Approximate grouped distinct-count backend |
| `exact` | sorted per-group value vectors | Ground truth for quantile tasks |
| `sketch` | `HashMap<group, asap_sketchlib::KLL<f64>>` | Approximate quantile backend |
| `exact` | `HashMap<user_id, count>` | Ground truth for frequency tasks |
| `sketch` | `asap_sketchlib::CountMin` / `asap_sketchlib::Count` | Approximate point-frequency backends |

The report compares each sketch estimate against the exact result over the
admitted output keys or groups. Frequency reports include multiple sketch
comparisons in `sketches[]`.

## Synthetic Data Source

MVP v1 uses a synthetic `events` relation, not a real file or DataFusion scan.
The generated rows contain:

- `region`
- `user_id`
- `value`

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
- optional `sketches[]` entries when one task has multiple approximate backends
- mean / p95 / max relative error
- worst group

This is enough to demonstrate an AQP-style comparison:

```text
same query + same data source + exact/sketch backend => cost/error tradeoff
```

## Success Criteria

MVP v1 is successful if it can demonstrate all of the following:

- A SQL query is parsed by DataFusion into a logical plan.
- Supported aggregate plans are lowered into AQP tasks.
- The task runs on a synthetic relational data source.
- The same task runs with exact and matching `asap_sketchlib` sketch backends.
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
- more query shapes, such as filtered distinct-count or top-k
- budget concepts, such as error target, latency target, and memory target
- sampling backend after the exact/sketch comparison is clear

The key open research/design question is whether we should define an
“approximation level” that describes how approximate a query execution is. That
could mean backend policy, sketch configuration, expected error budget, or the
fraction of the query plan implemented approximately. MVP v1 exposes the need
for that concept but does not define it yet.
