# Planned AQP Case Study: H2O + ClickHouse Quantile Benchmark

This is a planned SQL-track AQP case, not a reproduced result. It should not be
used as evidence for ASAPQuery quality yet.

It is also not part of the current AQPBMV2 scope. AQPBMV2 is now scoped to
benchmark-owned sketch kernels over concrete sketch instances. This SQL case is
broader later-scope AQP system work because any real run would involve SQL
semantics, planning, execution, and system baseline policy.

The idea is to adapt an ASAPQuery H2O/ClickHouse-compatible benchmark into AQP
form. Unlike the PromQL quickstart case, this path has not been run end to end
in the current workspace, and the available importer path is not enough for a
strong fidelity claim.

Under the current AQP design this should remain a planned SQL scenario family,
not a one-off imported CSV result. The missing work is not only "run the script";
the case also needs workload provenance, pressure dimensions, baseline policy,
option/config policy, and condition coverage before it can support a benchmark
claim.

## Benchmark Mapping

| AQP field | Status | Planned H2O + ClickHouse value / gap |
|---|---|---|
| Scenario family | Missing/gap | Need a stable family id, for example `sql_h2o_grouped_quantile_latency_v1`. |
| Track | Exists conceptually | SQL AQP is defined as a track, but this H2O case has not been reproduced. |
| Benchmark context | Planned | ClickHouse-compatible SQL query serving. Needs a runnable connector. |
| Workload model | Missing/gap | Need to state whether this is an H2O product benchmark, generated stream, or replayed dataset. |
| Pressure dimensions | Missing/gap | Candidate axes: row count, group count, window size, quantile, skew, and concurrency. |
| Task | Planned | Grouped p95 quantile over H2O-style group-by data. |
| Query shape | Planned | `SELECT QUANTILE(0.95, v1) ... GROUP BY id1, id2`. Needs exact reproduced query text. |
| Window | Planned | 10 second tumbling windows, if using ASAPQuery's existing pipeline. Needs verification in a run. |
| Approximate option | Planned | ASAPQuery KLL path, for example `asapquery_kll_k200`. Needs observed output. |
| Option/config policy | Missing/gap | KLL `k`, windowing parameters, and any ASAPQuery defaults/tuning policy must be recorded. |
| Baseline option | Missing/gap | ClickHouse exact or ClickHouse-compatible exact quantile semantics must be pinned. |
| Baseline policy | Missing/gap | Need to state whether ClickHouse is exact reference, exact performance baseline, or both. |
| Data condition | Missing/gap | H2O group-by dataset and event-time stream setup need a reproduced local data condition. |
| Requirement example | Missing/gap | latency <= budget, relative error <= target, and result rows match need manifest policy and emitted budget status. |

There is an existing manifest draft:

```text
sketch-bench/aqp-workloads/asapquery/h2o_quantile.toml
```

There is also an existing CSV importer path in `sketch-cli` for ASAPQuery-style
H2O CSV outputs. That importer is not the same as a reproduced benchmark case.

## What Exists

The current repo has partial ingredients:

- an AQP manifest draft for the H2O quantile shape;
- an importer that can read ASAPQuery-style `asap_results.csv` and
  `baseline_results.csv`;
- logic to normalize latency, row counts, ASAP/baseline status, timeout/error
  status, and missing counterpart cases.

Those pieces are useful, but they do not establish a current H2O AQP result.

## What Is Missing

This case has several gaps before it can be treated like the PromQL case:

1. No reproduced local run. We have not run the H2O + ClickHouse pipeline in
   this workspace.

2. No checked output files from a current run. There is no current
   `asap_results.csv`, `baseline_results.csv`, or imported AQP JSONL report to
   cite.

3. Fidelity is not strong enough if the source CSV only has `result_preview`.
   A preview is useful for debugging, but AQP needs full baseline and
   approximate result values to compute numeric error by row/group.

4. The pairing invariant is not established. A valid AQP version must prove
   that ClickHouse and ASAPQuery answers correspond to the same query, window,
   data snapshot, and grouping key set.

5. The exact baseline semantics need to be explicit. The case must say whether
   ClickHouse is the exact reference, the exact performance baseline, both, or
   merely a compatible baseline.

6. The option/config policy is missing. The case must say whether KLL and
   ASAPQuery settings are product defaults, hand-picked, swept, calibrated, or
   chosen as the best point satisfying requirements.

7. Workload provenance and pressure dimensions are missing. The case must say
   whether the input is a standard H2O-derived benchmark, a generated stream, or
   a replayed dataset, and which axes are intentionally stressed.

8. The run needs a reproducible connector, equivalent in spirit to the PromQL
   `run.sh`: start services, wait for readiness, run baseline and ASAP paths,
   import AQP records, verify output shape, and clean up.

9. Requirement policy is not pinned. The case needs explicit latency and
   fidelity budgets and should emit `latency_budget_met`,
   `fidelity_budget_met`, and result-shape status.

## Why It Is Still Useful To Keep As A Planned Case

If completed, this would be a different AQP track from PromQL:

```text
SQL grouped quantile task
  x ClickHouse-compatible query-serving context
  x H2O workload model and provenance
  x H2O group-by data condition
  x latency/error requirements
  x {ClickHouse exact baseline, ASAPQuery KLL approximate option}
  => latency, fidelity, admission, and failure behavior
```

That would test the same AQP framework in a SQL context rather than a PromQL
telemetry context. It should remain planned/gap documentation until the run is
actually reproduced and full result values are available.

Mapped to the core problem slots:

| Core problem slot | Status | H2O + ClickHouse concrete value / gap |
|---|---|---|
| Scenario family | Missing/gap | Need stable family id, planned conditions, and coverage reporting. |
| Task | Planned | SQL grouped quantile. Needs reproduced query suite. |
| Benchmark context | Planned | ClickHouse-compatible SQL serving. Needs end-to-end connector. |
| Workload model | Missing/gap | Need provenance and generation/replay policy. |
| Pressure dimensions | Missing/gap | Need explicit axes such as rows, groups, window size, skew, and concurrency. |
| Data condition | Missing/gap | H2O group-by stream/window condition must be generated or loaded reproducibly. |
| Ground truth | Missing/gap | ClickHouse baseline must be validated as exact/reference for the selected quantile semantics. |
| Baseline policy | Missing/gap | Need to state whether ClickHouse is both reference and performance baseline. |
| Requirements | Missing/gap | Latency, fidelity, and result-shape targets are not pinned for a current run. |
| Options | Planned | ClickHouse baseline and ASAPQuery KLL option. Needs observed paired outputs. |
| Option/config policy | Missing/gap | Need fixed/sweep/tuning policy for KLL and any query-engine settings. |
| Cost metrics | Exists partially | Existing CSV importer can normalize latency and row-count fields if CSVs exist. |
| Fidelity metrics | Missing/gap | Strong fidelity requires full result values, not only `result_preview`. |
| Admission behavior | Exists partially | Existing importer can represent errors/timeouts/missing counterparts, but no current run exercises it. |

## Suggested Completion Plan

1. Reproduce the ASAPQuery H2O + ClickHouse benchmark end to end.
2. Convert the draft manifest into a scenario-family manifest with workload
   provenance, pressure dimensions, baseline policy, option/config policy, and
   planned data conditions.
3. Make the benchmark emit full result values, not only previews.
4. Verify baseline and ASAP outputs are paired on the same query/window/data
   snapshot.
5. Import the CSVs into AQP JSONL.
6. Add a `REPRODUCE.md` with exact commands and expected output shape.
7. Update this file with observed results only after those steps pass.
