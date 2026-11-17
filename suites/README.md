# bench-external suites

A suite is a directory with, per workload `<w>`:

- `<w>.workload.json`: the harness `WorkloadSpec`. It holds the queries `q1..qN`, in the order the planner numbers them, and the `aqpbm-datagen` description of each metric's data.
- `<w>.planner.json`: the ASAPPlanner `PlanningWorkload` of the same queries. Its source facts describe the generated data: `series_count`, `sample_interval`, `metric_type`, `distinct_values_per_label`, and the summed `rows_per_second`.

`scripts/plan-suite.sh <suite> <run-dir>` plans every workload once per variant and writes a bench-external run directory. The variants are:

- `clickhouse.equivalent-only`
- `clickhouse.with-substitutes`
- `asap.default`
- `asap.exact`

`aqpbm-core/tests/harness_suites.rs` checks every suite: the data must match the planner's facts, the windows must match the query ranges, and evaluations must fall on the planner's phase.

All data is synthetic. Every metric has 1,000 series (`job` 10 × `instance` 100) with 1 s samples. `latency` is a gauge drawn from Uniform[0, 1000). `requests_total` is a counter of Uniform[0, 10) increments, so it has no resets. Data starts at 1,700,000,040,000 ms, a multiple of every evaluation interval.

| Suite | Windows | Evaluated every | Data |
|---|---|---|---|
| `showcase-1m` | 1 m | 10 s | 10 min |
| `showcase-1h` | 1 h | 60 s | 70 min |

Both suites have the same eight shapes:

- q1: quantile, relative error
- q1r: quantile, rank error
- q2: `sum by (job)` of `rate`
- q3: `avg by (job)` of a quantile
- q4: `sum by (job)` of a count
- q4b: max
- q4c: `sum by (job)` of `sum_over_time`
- q5: five of the above together

At 1,000 series, every `showcase-1m` workload plans as a query-time scan in every variant. In `showcase-1h`, sum, count, max and rate plan as stored windows. The exception is rate under `clickhouse.equivalent-only`: a stored rate is a substitute there, so it is computed at query time.
