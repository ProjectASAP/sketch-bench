# AQP Case Study: Native Sketch Primitive Benchmark

This is the planned native `sketch-bench` AQP example. Unlike the ASAPQuery
PromQL case, this does not adapt an external query system. It uses the existing
`sketchlib bench` machinery and asks how direct sketch implementations compare
with exact in-process baselines.

This case is not fully implemented yet. The current sketch benchmark substrate
already measures useful performance and accuracy signals, but it does not yet
emit AQP-shaped `option_run` and `paired_comparison` records.

In the updated AQP design this should become the first local scenario family,
not a single KLL run. Its job is to validate the scenario-family report shape in
a controlled setting: workload provenance is a synthetic generator, pressure
dimensions are explicit sweeps, exact baselines are in-process data structures,
and option/config policy is a declared sketch parameter grid.

## Benchmark Mapping

| AQP field | Status | Current / planned sketch value |
|---|---|---|
| Scenario family | Missing/gap | Need a native sketch primitive family, for example `sketch_quantile_break_even_v1`. |
| Track | Missing/gap | Sketch primitive AQP is named in the problem definition, but no dedicated AQP report path exists yet. |
| Benchmark context | Exists conceptually | Direct aggregate/data-structure API. Existing `sketchlib bench` already runs sketches directly. |
| Workload model | Exists partially | Controlled synthetic generator and file-backed workloads exist; provenance should be recorded in the AQP manifest. |
| Pressure dimensions | Exists partially | Size, cardinality, skew, group count, and update/query ratio are available or natural sweep axes. |
| First task candidate | Exists partially | Quantile over generated numeric streams is supported by existing KLL/DDSketch-style benchmark paths. |
| First exact baseline | Exists partially | Exact sorted-vector quantile exists as an accuracy comparator, but is not recorded as a first-class AQP option run. |
| First approximate option | Exists | KLL implementations are benchmarkable, for example `asap_sketchlib::KLL(k=200)`. |
| Option/config policy | Exists partially | `sketch_config` and `--config` support sweeps, but AQP needs to record fixed/default/grid/best-under-requirement policy. |
| Data condition | Exists partially | Synthetic/file workloads exist, but they need stable AQP data-condition ids. |
| Requirement example | Missing/gap | Rank-error, memory, and latency targets need to be declared in an AQP manifest and evaluated as budget booleans. |

The existing benchmark entry point is:

```bash
sketchlib bench \
  --sketch kll \
  --impl all \
  --workload zipf \
  --size 1000000 \
  --cardinality 100000 \
  --zipf-s 1.1 \
  --seed 42 \
  --accuracy \
  --report out.jsonl
```

## What Already Exists

`sketchlib bench` already provides a strong substrate:

- sketch family and implementation selection, such as `hll`, `kll`, `cms`,
  `countsketch`, `elastic`, and `dd`;
- synthetic and file-backed workloads;
- configuration sweeps through `--config`;
- repeated measured runs and warmup runs;
- throughput, latency, CPU, memory, and optional accuracy metrics;
- ground-truth comparators for cardinality, quantile, and frequency families;
- JSONL output with `sketch`, `impl`, `workload`, `sketch_config`, and
  `bench.accuracy`.

That is enough to run a sketch benchmark. It is not yet enough to call the run a
complete AQP case.

## What AQP Would Add

The current output is option-centric:

```text
sketch + impl + config + workload
  => throughput / latency / accuracy payload
```

The AQP version should be task-centric:

```text
scenario family
  x task
  x benchmark context
  x workload model
  x data condition
  x requirement
  x option
  => cost, fidelity, admission, and failure behavior
```

For a first quantile case, the AQP report should contain:

- one `option_run` for the exact sorted-vector baseline;
- one `option_run` for each KLL option/configuration;
- one `paired_comparison` per exact-vs-KLL pair;
- one `scenario_family` summary recording workload provenance, pressure
  dimensions, option/config policy, and condition coverage;
- p95 latency speedup and latency delta;
- rank-error and value-error summaries;
- requirement status, such as `rank_error_budget_met`,
  `memory_budget_met`, and `latency_budget_met`;
- admission status, such as `baseline`, `approximated`, `unsupported`,
  `option_error`, or `missing_counterpart`.

## Why This Matches The Core Problem

`AQP_PROBLEM_DEFINITION.md` defines the benchmark shape as:

```text
scenario family
  x task
  x benchmark context
  x workload model
  x data condition
  x requirement
  x option
  => cost, fidelity, admission, and failure behavior
```

The native sketch primitive case would fill those slots as:

| Core problem slot | Status | Native sketch concrete value / gap |
|---|---|---|
| Scenario family | Missing/gap | Need a stable family id and manifest, not only CLI arguments. |
| Task | Exists partially | Quantile, frequency, and cardinality are supported by existing benchmark/comparator code, but not packaged as AQP task manifests. |
| Benchmark context | Exists | Direct sketch/data-structure API, not SQL and not PromQL. |
| Workload model | Exists partially | Controlled generator/file workload exists, but provenance and replay/generation policy are not first-class AQP fields. |
| Pressure dimensions | Exists partially | Size/cardinality/skew are available; group count and update/query ratio need task-specific support. |
| Data condition | Exists partially | Generated/file-backed streams have size/cardinality/skew/seed metadata, but need stable AQP condition ids. |
| Ground truth | Exists partially | Exact sorted vector, hash map, and set baselines exist as comparators, but are not consistently emitted as AQP option runs. |
| Baseline policy | Missing/gap | Need to state when exact structures are both reference and performance baseline, and when they are reference only. |
| Requirements | Missing/gap | Rank error, relative error, memory, and latency budgets need explicit manifest fields and pass/fail evaluation. |
| Options | Exists partially | Sketch implementations/configs exist, but exact-vs-approx pairings are not emitted as AQP comparisons. |
| Option/config policy | Missing/gap | Need to record whether configs are fixed defaults, grid-swept, or selected as minimum config satisfying requirements. |
| Cost metrics | Exists | update throughput, query latency, memory, and CPU are already measured by `sketchlib bench`. |
| Fidelity metrics | Exists partially | rank/relative/absolute error exist for some families; unsupported families need explicit `no_fidelity_observation` behavior. |
| Admission behavior | Missing/gap | AQP labels such as `baseline`, `approximated`, `unsupported`, and `option_error` are not emitted yet. |

The important distinction is that this track does not evaluate an external
query system. It evaluates sketch primitives directly. That makes it a good
native AQP example after the ASAPQuery PromQL external-system example.

## Gaps

This case is not done. The current missing pieces are:

1. No AQP scenario-family manifest for a native sketch case. The run needs
   explicit task, context, workload provenance, pressure dimensions, data
   conditions, options, option/config policy, and requirements.

2. Exact baselines are currently used by accuracy comparators, but they are not
   consistently recorded as first-class AQP option runs.

3. Current JSONL records are not paired-comparison records. They need explicit
   exact-vs-approx comparisons with speedup, error summary, and requirement
   status.

4. Raw accuracy numbers are present for some families, but AQP budget fields
   such as `rank_error_budget_met` or `relative_error_budget_met` are not yet
   emitted.

5. Some implementations have no accuracy comparator. Those should be recorded
   as `unsupported` or `no_fidelity_observation`, not silently treated as
   comparable.

6. Option/config policy is not first-class. AQP should record the parameter grid
   or default rule, tuning budget, calibration data, and whether a result is a
   fixed deployment choice or a best-of-grid point.

7. Data-condition identity is still mostly a workload descriptor. AQP should
   give each controlled condition a stable id, such as
   `zipf_s1.1_n1m_card100k_seed42`.

8. The low-level `sketchlib bench` schema should remain intact. The AQP layer
   should be a new adapter/importer or a separate AQP command, not a breaking
   rewrite of existing benchmark output.

## Suggested First Implementation

Start with one narrow native case:

```text
task: quantile
context: direct sketch primitive API
scenario family: sketch_quantile_break_even_v1
workload model: controlled synthetic generator
pressure dimensions: stream length, cardinality, Zipf skew, KLL k
data condition: zipf_s1.1_n1m_card100k_seed42
baseline: exact sorted vector quantile
approximate option: KLL(k=200)
option/config policy: fixed KLL(k=200) first, then grid sweep over k
requirements: rank error <= 0.01, p95 query latency <= budget, memory <= budget
```

That should emit exactly one exact `option_run`, one KLL `option_run`, and one
`paired_comparison`, plus a `scenario_family` summary showing that only one
condition has been run. After that shape is stable, expand to KLL config
sweeps, DDSketch, frequency sketches, and HLL/cardinality.
