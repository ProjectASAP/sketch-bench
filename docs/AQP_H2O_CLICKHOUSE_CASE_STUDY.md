# Planned AQP Case Study: H2O + ClickHouse Quantile Benchmark

This is a planned SQL-track AQP case, not a reproduced result. It should not be
used as evidence for ASAPQuery quality yet.

The idea is to adapt an ASAPQuery H2O/ClickHouse-compatible benchmark into AQP
form. Unlike the PromQL quickstart case, this path has not been run end to end
in the current workspace, and the available importer path is not enough for a
strong fidelity claim.

## Benchmark Mapping

| AQP field | Status | Planned H2O + ClickHouse value / gap |
|---|---|---|
| Track | Exists conceptually | SQL AQP is defined as a track, but this H2O case has not been reproduced. |
| Benchmark context | Planned | ClickHouse-compatible SQL query serving. Needs a runnable connector. |
| Task | Planned | Grouped p95 quantile over H2O-style group-by data. |
| Query shape | Planned | `SELECT QUANTILE(0.95, v1) ... GROUP BY id1, id2`. Needs exact reproduced query text. |
| Window | Planned | 10 second tumbling windows, if using ASAPQuery's existing pipeline. Needs verification in a run. |
| Approximate option | Planned | ASAPQuery KLL path, for example `asapquery_kll_k200`. Needs observed output. |
| Baseline option | Missing/gap | ClickHouse exact or ClickHouse-compatible exact quantile semantics must be pinned. |
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
   ClickHouse is the exact oracle, a reference implementation, or merely a
   compatible baseline.

6. The run needs a reproducible connector, equivalent in spirit to the PromQL
   `run.sh`: start services, wait for readiness, run baseline and ASAP paths,
   import AQP records, verify output shape, and clean up.

7. Requirement policy is not pinned. The case needs explicit latency and
   fidelity budgets and should emit `latency_budget_met`,
   `fidelity_budget_met`, and result-shape status.

## Why It Is Still Useful To Keep As A Planned Case

If completed, this would be a different AQP track from PromQL:

```text
SQL grouped quantile task
  x ClickHouse-compatible query-serving context
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
| Task | Planned | SQL grouped quantile. Needs reproduced query suite. |
| Benchmark context | Planned | ClickHouse-compatible SQL serving. Needs end-to-end connector. |
| Data condition | Missing/gap | H2O group-by stream/window condition must be generated or loaded reproducibly. |
| Ground truth | Missing/gap | ClickHouse baseline must be validated as exact/reference for the selected quantile semantics. |
| Requirements | Missing/gap | Latency, fidelity, and result-shape targets are not pinned for a current run. |
| Options | Planned | ClickHouse baseline and ASAPQuery KLL option. Needs observed paired outputs. |
| Cost metrics | Exists partially | Existing CSV importer can normalize latency and row-count fields if CSVs exist. |
| Fidelity metrics | Missing/gap | Strong fidelity requires full result values, not only `result_preview`. |
| Admission behavior | Exists partially | Existing importer can represent errors/timeouts/missing counterparts, but no current run exercises it. |

## Suggested Completion Plan

1. Reproduce the ASAPQuery H2O + ClickHouse benchmark end to end.
2. Make the benchmark emit full result values, not only previews.
3. Verify baseline and ASAP outputs are paired on the same query/window/data
   snapshot.
4. Import the CSVs into AQP JSONL.
5. Add a `REPRODUCE.md` with exact commands and expected output shape.
6. Update this file with observed results only after those steps pass.
