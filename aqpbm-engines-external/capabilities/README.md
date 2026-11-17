# Deployment capabilities of the external engines

Each file is an ASAPPlanner deployment-capabilities file (the `DeploymentCapabilities` JSON of ASAPPlanner main at `bc725bda8`, which merged #1374). Pass it with `asap_planner --deployment-capabilities <file>` so that the planner plans only for what the engine can run:

| File | What the planner may use |
|---|---|
| `clickhouse.equivalent-only.json` | class (a) only: same answers as the planner's semantics |
| `clickhouse.with-substitutes.json` | (a) and (b): (b) changes the guarantee as stated below |
| `victoriametrics.equivalent-only.json` | (a) only |
| `victoriametrics.with-substitutes.json` | (a) and (b) |

Classes:
- **(a) equivalent:** the engine computes the same answer.
- **(b) substitute:** the engine computes something close, with the stated difference.
- **(c) absent:** the engine cannot do it.

The translator in this crate is the check that the files are right. A plan made with a file must translate without `UnsupportedShape`. In an equivalent-only run, it must also translate without any substitution. A query-time node that several queries share is recomputed for each of them (`SharingLost`); that changes cost, not answers, so it is not a substitution. The test `public_plans_translate` checks this rule on planner-written plans of public synthetic workloads (`tests/fixtures/plans/REV` lists them). The ClickHouse files are for PromQL time-series workloads: a SQL table source is refused until the translator has a rule for it.

The tables cite engine sources at the commits of the binaries in use:
- ClickHouse 26.8.39.13 (`v26.8.39.13-lts`, git hash `d5bbb64d525197f84f9dd21489f5f6d6fdb966e5`). This is the 26.8 LTS build for x86-64 CPUs without AVX2 (`builds.clickhouse.com/26.8/amd64compat`, sha256 `aae73dc54729e837903d1769d540e9c87f58c002952f5bf2e775dfefd8fe36ca`), because the release build needs AVX2. That URL follows the 26.8 branch, so the sha256 is the pin. The golden test checks `version()` and the git hash of the binary it runs on, and the conformance tests check the cited behavior on that binary;
- VictoriaMetrics v1.154.1 (`64c2e75fe7dc`).

Prometheus `extrapolatedRate` is cited at v3.15.1.

## ClickHouse

| Plan element | Class | ClickHouse | Guarantee difference | Evidence |
|---|---|---|---|---|
| Stored windows, exact Sum / Count / Min / Max, per series or grouped | a | `sumState` / `countState` / `minState` / `maxState` in an AggregatingMergeTree via a materialized view; `-Merge` at query time | — | [`AggregateFunctionState.cpp`](https://github.com/ClickHouse/ClickHouse/blob/d5bbb64d525197f84f9dd21489f5f6d6fdb966e5/src/AggregateFunctions/Combinators/AggregateFunctionState.cpp#L1-L47) |
| Stored windows, exact Rate / Increase (per series) | b | per stored row: `count`, `min`/`max(ts)`, `argMin`/`argMax(value, ts)` and `deltaSumTimestamp(value, ts)` states, read one row at a time (`finalizeAggregation`); a series' rows are joined in time order, then Prometheus `extrapolatedRate` | A counter reset adds 0 for the step across it, where PromQL adds the first value after the reset, so the increase is lower by that value (inside a window or across windows alike). Each series' rows must be inserted in time order, one chunk after another; overlapping rows are an error. The query never merges these states across rows: `deltaSumTimestamp`'s merge joins two states by their timestamps, and a parallel read that merges non-adjacent states first counts the increase between them twice. Cost: the read finalizes each stored row and sorts a range's rows by series and time, so it grows with series × windows × unmerged parts. | [`add`](https://github.com/ClickHouse/ClickHouse/blob/d5bbb64d525197f84f9dd21489f5f6d6fdb966e5/src/AggregateFunctions/AggregateFunctionDeltaSumTimestamp.cpp#L70-L91) adds only positive deltas; [`merge`](https://github.com/ClickHouse/ClickHouse/blob/d5bbb64d525197f84f9dd21489f5f6d6fdb966e5/src/AggregateFunctions/AggregateFunctionDeltaSumTimestamp.cpp#L93-L155) joins by timestamp; tests `stored_rate_is_right_under_parallel_reads`, `stored_rate_differs_from_promql_by_the_first_value_after_each_reset` |
| Query-time Rate / Increase | a | Prometheus `extrapolatedRate` in SQL over the raw samples, with reset correction | — | Prometheus [`extrapolatedRate`](https://github.com/prometheus/prometheus/blob/v3.15.1/promql/functions.go#L452-L633) |
| Query-time Sum / Count / Min / Max, per series or grouped, and their regrouping merges | a | `sum` / `count` / `min` / `max` grouped by the kept labels; a regrouping merge re-aggregates partial values (a count of counts is their sum) | Floating-point sums run in another order than PromQL's (and without its Kahan compensation in `sum_over_time`), so they agree to rounding, not bit for bit; the conformance pass rule is 1e-9 relative | test `average_as_sum_over_count_answers` |
| Instant selector | a | `argMax(value, ts)` per series over (t − lookback, t], with the plan's lookback | The plan's lookback is the data's declared sample interval (e.g. 5 ms), not Prometheus' default 5 m. Stale markers are not modeled: the data has none. | test `instant_selector_answers` |
| `Aggregate` sum / count / min / max / avg / quantile | a | the same functions; `quantile` is `quantileExactInclusive` | — | test `grouped_query_time_sum_answers` |
| `Aggregate` top_k | a | `ORDER BY value DESC LIMIT k BY <group>` | Ties are broken by the series' labels, where PromQL's choice among equal values is unspecified; NaN ranks below every number in both | test `top_k_answers` |
| `Project` (columns, casts to Float64) and arithmetic `BinaryOp` (one-to-one on all labels) | a | a SELECT list; a join on the labels (a per-series operation across two metrics is refused: the series identity names the metric) | A checked division (e.g. avg as sum / count) with a value that is not finite is an error, where the planner's executor answers the original query exactly | test `average_as_sum_over_count_answers` |
| IRate, stored or at query time (also the `i_rate` measure) | c | — | — | not written; excluded in both files |
| HyperLogLog distinct count (`Hll`) | a | `uniqCombined64(p)`, p in 12..20 | — | [`AggregateFunctionUniqCombined.cpp`](https://github.com/ClickHouse/ClickHouse/blob/d5bbb64d525197f84f9dd21489f5f6d6fdb966e5/src/AggregateFunctions/AggregateFunctionUniqCombined.cpp#L28-L33) |
| KMV distinct count (`Kmv`) | b | `uniqTheta` | lg_k is fixed at the library default, 12 (4,096 entries) | [`ThetaSketchData.h`](https://github.com/ClickHouse/ClickHouse/blob/d5bbb64d525197f84f9dd21489f5f6d6fdb966e5/src/AggregateFunctions/ThetaSketchData.h#L35-L40) |
| KLL quantile (`Kll`) | b | `quantileGK(accuracy)` | Deterministic rank error ≤ n / accuracy instead of KLL's probabilistic bound | [`AggregateFunctionQuantileGK.cpp`](https://github.com/ClickHouse/ClickHouse/blob/d5bbb64d525197f84f9dd21489f5f6d6fdb966e5/src/AggregateFunctions/AggregateFunctionQuantileGK.cpp#L301-L319) |
| DDSketch quantile (`DDSketch`) | b | `quantileDD(α)` | Reads rank q·(n − 1); the α bound holds at that rank | [`DDSketch.h`](https://github.com/ClickHouse/ClickHouse/blob/d5bbb64d525197f84f9dd21489f5f6d6fdb966e5/src/AggregateFunctions/DDSketch.h#L79) |
| Top-k (`CmsWithHeap`, `CountSketchWithHeap`) | b | `topK(k)` (Space-Saving) | No guaranteed result | [`SpaceSaving.h`](https://github.com/ClickHouse/ClickHouse/blob/d5bbb64d525197f84f9dd21489f5f6d6fdb966e5/src/Common/SpaceSaving.h#L82) |
| Raw samples retained | a | the MergeTree source table, sorted by the filtered labels, the series identity and `ts`, with a `minmax` index on `ts`; a metric only stored chains read is a `Null` table (its views still see every insert) | — | tests `query_time_rate_is_equivalent`, `a_metric_only_chains_read_keeps_no_rows` |
| Retention | a | each table's retention is recorded (`table_retention`), and `ttl_statements()` gives a TTL per table | A TTL deletes rows by wall-clock time, so it applies only when the data's timestamps are wall-clock time. Under replayed data the tables keep every row, and stored bytes include rows past retention. | test `retention_is_recorded_and_ttl_is_separate` |
| Query-time results kept across evaluations | c | — | — | refused by the translator (`query_time_results_kept_across_evaluations_are_refused`) |

A stored read is right only at a window end, so a query that reads stored windows fails (`throwIf`) at any other evaluation time. Every insert uses `INSERT_SETTINGS`, so a NULL value is an error instead of 0. The sketch rows above give each sketch's class. The ClickHouse files list no sketch yet, because the translator does not build sketch states yet; each sketch joins `supported_summaries` in the PR that adds its translation rule.

**Conformance on a server.** `tests/clickhouse_conformance.rs` (`#[ignore]`, `AQPBM_CLICKHOUSE_URL` and `AQPBM_CGROUP_AGENT_URL`) runs one case per row through the engine adapter (`clickhouse_server`) against a real server, with hand-computed answers:

| Row | Case |
|---|---|
| stored Sum / Count / Min / Max | `stored_sum_count_min_max` |
| stored Rate / Increase (b) | `stored_rate_is_promql_minus_the_first_value_after_each_reset` (three installs fed in turn, max_threads 1 and 16) |
| query-time Rate / Increase; raw samples retained | `query_time_rate_is_promql` |
| query-time grouped sums; `Aggregate` | `grouped_sums_quantiles_and_aggregates` |
| instant selector | `instant_selector` |
| top_k | `top_k_ties_and_nan` |
| regrouping merges, `Project`, `BinaryOp`, checked division | `average_as_sum_over_count_and_checked_division` |
| IRate; query-time results kept across evaluations (c) | `class_c_rows_are_refused` |
| raw samples retained (stored-only metric); retention | `storage_and_retention` |
| window boundary (left-open) | `the_harness_conformance_kit_passes` |
| sketches | not yet: no sketch rule |

## VictoriaMetrics

The VictoriaMetrics translator comes in a later PR. Until then, these files are the planned capabilities and are checked only by the planner run in the PR's manual check.

| Plan element | Class | VictoriaMetrics | Guarantee difference | Evidence |
|---|---|---|---|---|
| Stored windows, exact Sum / Count / Min / Max | a | stream aggregation outputs `sum_samples` / `count_samples` / `min` / `max`, `enable_windows: true` | — | `lib/streamaggr/streamaggr.go` |
| Counter reset handling | b | `removeCounterResets` | A drop smaller than prev/8 adds only prev − v. A staleness gap clears the correction. `increase` with no earlier sample starts the counter at 0. | `app/vmselect/promql/rollup.go` |
| Counter extrapolation | b | none: `rate` / `increase` are not extrapolated | The increase over the samples, not extrapolated to the range's ends | `app/vmselect/promql/rollup.go` |
| IRate | b | MetricsQL `irate`, query time only | VictoriaMetrics' look-behind rule | `app/vmselect/promql/rollup.go` |
| KLL / DDSketch quantile | b | stream output `histogram_bucket` (VictoriaMetrics histogram), `histogram_quantile` | Relative error ≤ 10^(1/18) − 1 ≈ 0.136 | `lib/streamaggr/histogram_bucket.go` |
| Hll / Kmv distinct count, top-k | b | exact `count(count by (…))` / `topk` over raw samples, query time only | Exact, but no stored state | `app/vmselect/promql/aggr.go` |
| Raw samples retained | a | the raw series (`-streamAggr.keepInput`) | — | `app/vminsert/common/streamaggr.go` |

In `victoriametrics.equivalent-only.json`, both counter semantics are `false`. The planner therefore excludes every Rate / Increase / IRate state and every `rate` / `increase` / `irate` / `resets` / `delta` measure.
