# AQP Benchmark MVP v2 Design

> Status: proposed design.
>
> Relationship to earlier docs: this does not replace `AQP_MVP.md` or
> `AQP_MVP_SIMPLE.md`. MVP v1 proved a local exact-vs-sketch path. MVP v2
> reframes the next step around benchmark scenarios, optimized exact baselines,
> resource accounting, and approximation value.

## Design Goal

MVP v2 should answer a sharper question than MVP v1:

```text
For a concrete benchmark scenario family, under which data and workload
conditions is approximate execution worthwhile against an optimized exact
alternative?
```

"Worthwhile" means the approximate option provides a measured benefit, such as
lower query latency, lower query CPU, lower memory growth, longer retention,
higher query concurrency, or lower effective serving cost, while satisfying the
declared fidelity and result-shape requirements.

MVP v2 should not assume approximation is beneficial. Showing that exact
execution remains better for a scenario is a valid benchmark result.

The design should follow the pressure visible in recent PVLDB benchmark papers:
benchmarks are moving away from fixed single workloads and toward task-specific
suites, production-shaped workload models, explicit baselines, and diagnostic
outputs. For AQP, this means MVP v2 should produce a small scenario family with
condition sweeps, not just one exact-vs-approximate transcript.

The design translation is direct: Why TPC Is Not Enough, Cloud Analytics
Benchmark, PBench, and SQLStorm motivate workload provenance and pressure
dimensions; TFB and TAB motivate fairness around option coverage and
configuration policy; LakeBench, ScienceBenchmark, and BigVectorBench motivate
task-specific data, result shape, and fidelity metrics; the LDBC and TPCx-AI
papers motivate explicit workload versions and baseline policy.

## Non-Goals

MVP v2 should not:

- reimplement SQL-to-sketch, PromQL-to-sketch, or DataFusion-to-sketch planning;
- rewrite or replace ASAPQuery's existing quickstart/benchmark infrastructure;
- force all systems into a universal query interface;
- require every system to support exact fallback;
- claim system-wide superiority from one scenario;
- treat a weak exact implementation as a sufficient exact performance baseline.

## Core Terms

| Term | MVP v2 meaning |
|---|---|
| `scenario family` | A benchmark suite with one track, task family, workload model, value hypothesis, option set, option/config policy, data-condition matrix, baseline policy, resource accounting scope, and comparison rules. |
| `scenario` | One measured point inside a scenario family: concrete system, native interface, workload instance, data condition, exact reference execution, exact performance baseline, approximate option, and requirements. |
| `workload model` | The source and generation/replay policy for requests and data: product benchmark, real trace, controlled synthetic generator, or hybrid synthesizer. |
| `pressure dimension` | The data or workload axis the benchmark intentionally stresses, such as cardinality, group count, skew, temporal drift, query window, query concurrency, or retention horizon. |
| `exact reference execution` | The exact computation used to produce reference answers for fidelity measurement. |
| `exact performance baseline` | The optimized exact alternative used for performance and resource comparison. |
| `baseline policy` | The rule that states whether reference and performance baseline are the same system, why the performance baseline is strong enough, and what baseline gaps remain. |
| `approximate option` | The approximate system path, configuration, or direct sketch implementation being evaluated. |
| `option/config policy` | The rule for choosing exact and approximate configurations: fixed default, grid sweep, best under requirement, calibration data, tuning budget, and fairness notes. |
| `approximate deployment pattern` | How the approximate option is deployed: replacement, augmentation, tiered, precompute, fallback-capable, or primitive. |
| `resource accounting scope` | The cost categories counted when comparing exact and approximate deployments. |
| `value hypothesis` | A falsifiable claim connecting a pressure dimension to an expected benefit and a requirement boundary. |
| `outcome` | The observed result status for an option: exact reference, exact system, approximate system, fallback, unsupported, error, timeout, missing counterpart, or not applicable. |

## Scenario Manifest

MVP v2 should introduce a scenario-family manifest. The manifest is the stable
unit that makes runs interpretable and comparable. Individual run records refer
back to this manifest through `scenario_family_id` and `data_condition_id`.

Sketch:

```toml
[scenario_family]
id = "promql_grouped_quantile_latency_vs_fidelity_v2"
track = "promql_aqp"
task_family = ["aggregate", "grouped_quantile"]
description = "Grouped telemetry quantiles over PromQL-compatible serving"

[system_under_test]
name = "ASAPQuery"
interface = "promql_http"
deployment_pattern = ["augmentation", "precompute"]

[workload_model]
query_suite = "../ASAPQuery/benchmarks/queries/promql_suite.json"
provenance = "product_quickstart_plus_controlled_sweep"
request_policy = "paired_promql_instant_queries"
pressure_dimensions = [
  "series_count",
  "label_cardinality",
  "query_range",
  "temporal_drift",
  "query_concurrency",
]

[[data_conditions]]
id = "quickstart_fake_exporters_patterns_v1"
source = "asapquery_quickstart_fake_exporters"
role = "seed_condition"
notes = "Deterministic pattern exporters; not yet a broad realism sweep."

[[data_conditions]]
id = "synthetic_high_cardinality_groups_v1"
source = "sketch_bench_generator"
role = "controlled_breakpoint"
series_count = 100000
label_cardinality = 10000
seed = 1

[reference_execution]
name = "Prometheus"
role = "exact_reference"
pairing_policy = "same_promql_time"

[exact_performance_baseline]
name = "Prometheus"
role = "optimized_exact_baseline"

[baseline_policy]
reference_equals_performance_baseline = true
strength = "native_prometheus_serving_path_for_same_promql_workload"
known_gaps = ["not a tuned distributed Prometheus/Mimir/Cortex deployment"]

[approximate_option]
name = "ASAPQuery PromQL precompute path"
role = "approximate_system"

[option_config_policy]
selection = "fixed_product_defaults_for_seed_condition; grid_sweep_for_native_sketch_family"
calibration_data = "none for quickstart seed"
tuning_budget = "not yet tuned"
fairness_note = "Do not compare tuned approximate configs against untuned exact baselines."

[value_hypothesis]
pressure = "high-cardinality grouped telemetry quantiles over dashboard-style repeated queries"
expected_benefit = "precomputed approximate summaries reduce p95 latency"
accept_if = "p95 latency improves while max relative error <= 0.05 and label sets match"
reject_if = "latency gain disappears, fidelity fails, result shape changes, or incremental maintenance cost dominates"
status = "hypothesis"

[requirements]
p95_latency_ms = 1000
max_relative_error = 0.05
result_shape = "label_sets_match"

[resource_accounting]
base_exact_cost = ["query_latency"]
incremental_approx_cost = []
effective_serving_cost = ["query_latency"]
known_gaps = ["memory", "query_cpu", "ingest_cpu", "summary_storage"]

[comparison_rules]
time_pairing = "same_instant_query_time"
label_matching = "exact_label_set"
numeric_error = "relative_error_with_zero_policy"
```

This is an example shape, not a final schema. MVP v2 can store the manifest in
TOML, JSON, or Rust structs as long as the report preserves the same concepts.

## Why These Fields Are Required

| Field | Why it matters |
|---|---|
| `scenario_family` | A benchmark claim needs a stable family id, not only an ad hoc run command. |
| `system_under_test` | A benchmark without a concrete system is only a workload description. |
| `interface` | AQP behavior depends on the real API boundary users exercise. |
| `workload_model` | The benchmark must state where requests/data come from and which production pressure they model. |
| `data_condition` | AQP tradeoffs change with skew, cardinality, group count, time dynamics, and query rate. |
| `reference_execution` | Fidelity cannot be measured without a precise exact result. |
| `exact_performance_baseline` | Approximation value cannot be judged against an unoptimized or implicit exact alternative. |
| `baseline_policy` | Exact baselines become stale or weak; the manifest must say why this one is fair and what remains missing. |
| `approximate_option` | The benchmark must identify what approximate path or configuration is being evaluated. |
| `option_config_policy` | Benchmark fairness depends on how configs are chosen and how much tuning each option receives. |
| `value_hypothesis` | The run should state the pressure, expected benefit, acceptance rule, and rejection rule. |
| `requirements` | A speedup without accuracy or result-shape constraints is not an AQP result. |
| `resource_accounting` | Augmentation systems can add resource cost; the benchmark must say which costs are counted and which are missing. |
| `comparison_rules` | Pairing, group alignment, label matching, and numeric edge cases can otherwise dominate the result. |

Some fields can be empty only if the track declares that they are not
applicable. For example, a sketch primitive scenario may not have service
deployment metadata. It still needs a reference execution, exact baseline,
approximate option, data condition, requirements, and comparison rules.

## Resource Accounting Model

MVP v2 should separate resource measurements into three views:

```text
base_exact_cost
  The cost of the optimized exact baseline serving the workload alone.

incremental_approx_cost
  The extra cost introduced by the approximate layer or approximate state.

effective_serving_cost
  The cost to satisfy the same workload/SLO under the exact deployment versus
  the approximate deployment.
```

For replacement scenarios, `incremental_approx_cost` may be less important
because the exact state is replaced. For augmentation scenarios, it is central.

MVP v2 minimum:

- report query latency or operation latency;
- report fidelity against the exact reference execution;
- report whether resource accounting is complete or partial;
- preserve missing resource dimensions as explicit gaps rather than silently
  omitting them.

Target resource dimensions:

| Dimension | Example use |
|---|---|
| `query_latency` | Measures serving benefit. |
| `query_cpu` | Distinguishes latency speedup from CPU savings. |
| `ingest_cpu` / `update_cpu` | Captures precompute or sketch maintenance cost. |
| `memory` | Captures exact state, approximate state, or added service memory. |
| `storage` | Captures raw data, summaries, sketches, or materialized views. |
| `network_bytes` | Matters for distributed or remote-write scenarios. |
| `concurrency_capacity` | Measures whether approximation serves more queries under an SLO. |
| `freshness_lag` | Captures delay caused by precompute or summary maintenance. |

## MVP v2 Scope

MVP v2 should include two scenario families because they exercise different
deployment patterns.

### Scenario A: Native Sketch Primitive

Purpose:

- prove the scenario/report model in a controlled replacement setting;
- measure break-even behavior against exact data structures;
- keep implementation local and reproducible.

Shape:

```text
track: sketch_primitive_aqp
system_under_test: direct sketch implementation
native_interface: sketch/data-structure API
deployment_pattern: replacement
exact_reference_execution: exact data structure
exact_performance_baseline: same exact data structure measured as an option
approximate_option: HLL, KLL, CountMin, or CountSketch
option_config_policy: small grid sweep plus requirement-minimal config summary
workload_model: controlled synthetic generator
pressure_dimensions: cardinality, skew, group count, size, update/query ratio
data_conditions: small matrix over those dimensions
```

First task candidate:

```text
task: grouped count distinct or quantile
exact baseline: HashSet per group or sorted vector per group
approx option: HLL per group or KLL per group
value hypothesis: sketch memory grows more slowly than exact state after a
  cardinality/group-count threshold while staying within error budget.
reject condition: exact remains faster/smaller under the tested budget, or the
  sketch violates fidelity/result-shape requirements.
```

Required outputs:

- exact option run;
- approximate option run;
- paired comparison;
- latency or throughput;
- memory estimate;
- fidelity metrics;
- requirement booleans;
- data-condition id;
- break-even summary if enough sweep points exist.
- condition coverage summary, so the result cannot be mistaken for a broad
  claim when only one point has been run.

### Scenario B: PromQL System Adapter

Purpose:

- prove that the same scenario/evaluation model can wrap an existing external
  benchmark without taking ownership of that system;
- preserve ASAPQuery's native PromQL interface;
- make augmentation/precompute resource gaps explicit.

Shape:

```text
track: promql_aqp
system_under_test: ASAPQuery PromQL path
native_interface: PromQL-compatible HTTP
deployment_pattern: augmentation/precompute, optionally fallback-capable
exact_reference_execution: Prometheus with paired query timestamp
exact_performance_baseline: Prometheus serving the same workload
approximate_option: ASAPQuery native approximate path or exact fallback path
workload_model: ASAPQuery quickstart query suite plus controlled PromQL sweeps
pressure_dimensions: series count, label cardinality, range/window, drift,
  bursts, pattern mix, concurrency
data_condition: quickstart fake exporters as seed, broader matrix later
```

MVP v2 should not modify ASAPQuery's quickstart beyond what is necessary to run
it as an adapter target. The benchmark layer should start the existing services,
drive the native workload, collect native outputs, import records, and annotate
resource-accounting gaps.

Required outputs:

- per-query exact baseline option run;
- per-query ASAPQuery option run;
- paired comparison using the same PromQL `time=`;
- latency metrics;
- relative error and label-set mismatch;
- outcome classification;
- requirement booleans;
- explicit note that memory, query CPU, ingest CPU, and summary storage are
  missing if not measured.
- workload provenance and pressure-dimension metadata for every imported run.

## Optional Scenario C: ASAPFusion / DataFusion

MVP v2 can define this as a target shape without implementing it immediately.

Shape:

```text
track: datafusion_aqp
system_under_test: asap-fusion or a DataFusion approximate path
native_interface: SQL/DataFusion logical or physical plan
deployment_pattern: replacement, precompute, or materialized-summary path
exact_reference_execution: DataFusion exact or DuckDB exact
exact_performance_baseline: optimized exact DataFusion/DuckDB path
approximate_option: asap-fusion approximate operator or summary-backed path
```

This is the natural next track for showing that the framework is not
PromQL-specific. It should not block MVP v2 unless a concrete ASAPFusion
scenario is already available.

## Data Realism Plan

MVP v2 should move from one-off synthetic inputs toward controlled realism.
The point is not to declare one workload "realistic"; the point is to make the
workload argument inspectable.

Use three tiers:

- controlled synthetic sweeps for breakpoints and sensitivity analysis;
- replay or product-benchmark conditions for native-system behavior;
- hybrid generated conditions fitted to real statistics when real traces cannot
  be shipped.

Minimum data-condition knobs:

- row count or stream length;
- cardinality;
- group count;
- group-size skew;
- Zipf/heavy-tail parameter;
- seed;
- query count or update/query ratio where applicable.

PromQL-specific future knobs:

- number of time series;
- label cardinality;
- scrape interval;
- query range/window;
- temporal drift;
- bursts;
- pattern mix;
- query concurrency.

The benchmark should be designed to produce regions, not only single rows:

```text
exact wins here;
approximation wins here if error <= target;
approximation is fast but violates fidelity here;
precompute overhead is not justified here.
```

Every plotted or summarized result should say which tier and pressure dimensions
it covers. A single quickstart condition can demonstrate the adapter, but it
should not be used as the headline benchmark result.

## Report Model

MVP v2 should emit normalized records at three levels.

### Scenario Family Summary

One record describing the benchmark suite and coverage.

```json
{
  "record_type": "scenario_family",
  "scenario_family_id": "promql_grouped_quantile_latency_vs_fidelity_v2",
  "track": "promql_aqp",
  "workload_model": {
    "provenance": "product_quickstart_plus_controlled_sweep",
    "pressure_dimensions": ["series_count", "label_cardinality", "query_range"]
  },
  "baseline_policy": {
    "exact_reference": "Prometheus",
    "exact_performance_baseline": "Prometheus",
    "strength": "native_prometheus_serving_path_for_same_promql_workload"
  },
  "option_config_policy": {
    "selection": "fixed_product_defaults_for_seed_condition",
    "tuning_budget": "not_yet_tuned"
  },
  "coverage": {
    "data_conditions_run": 1,
    "data_conditions_planned": 6
  }
}
```

### Option Run

One system or option execution.

```json
{
  "record_type": "option_run",
  "scenario_family_id": "promql_grouped_quantile_latency_vs_fidelity_v2",
  "scenario_id": "quickstart_fake_exporters_patterns_v1/q95_by_pattern",
  "track": "promql_aqp",
  "option_id": "prometheus_exact",
  "outcome": "exact_system",
  "data_condition_id": "quickstart_fake_exporters_patterns_v1",
  "cost": {
    "p95_latency_ms": 1200.0
  },
  "resources": {
    "accounting_status": "partial",
    "missing": ["memory", "query_cpu", "ingest_cpu", "summary_storage"]
  }
}
```

### Paired Comparison

One exact-versus-approximate comparison under the same scenario and data
condition.

```json
{
  "record_type": "paired_comparison",
  "scenario_family_id": "promql_grouped_quantile_latency_vs_fidelity_v2",
  "scenario_id": "quickstart_fake_exporters_patterns_v1/q95_by_pattern",
  "baseline_option_id": "prometheus_exact",
  "candidate_option_id": "asapquery_precompute",
  "cost_delta": {
    "p95_latency_ms": -2098.0,
    "p95_speedup": 175.56
  },
  "fidelity": {
    "max_relative_error": 0.0056,
    "missing_result_keys": 0,
    "extra_result_keys": 0
  },
  "requirements": {
    "latency_met": true,
    "fidelity_met": true,
    "result_shape_met": true
  },
  "coverage": {
    "workload_tier": "product_quickstart_seed",
    "pressure_dimensions": ["pattern_mix"]
  },
  "interpretation": {
    "value_hypothesis_status": "supported_for_this_condition"
  }
}
```

The `interpretation` field should be conservative. Allowed values should be
local to the measured scenario, such as:

- `supported_for_this_condition`;
- `rejected_for_this_condition`;
- `inconclusive_resource_accounting_partial`;
- `fidelity_violation`;
- `latency_violation`;
- `shape_violation`.

## How To Judge A Good Scenario

A scenario is valuable if it satisfies all of these:

- It represents a real or plausible reason to use approximation.
- It has a strong exact performance baseline.
- It has an exact reference result for fidelity.
- It belongs to a scenario family that stresses at least one data or workload
  dimension where exact execution may become expensive.
- It has explicit resource accounting scope.
- It can produce an informative result even when approximation loses.

A scenario is well executed if:

- exact and approximate results refer to the same data snapshot/window;
- unsupported queries and failed runs are preserved as outcomes;
- comparison rules are explicit;
- resource gaps are recorded;
- repeated runs or variance are reported when possible;
- no paper-level claim depends on a field marked as missing or partial.

## Implementation Plan

1. Add a Rust or TOML scenario manifest model.
2. Make the manifest a scenario-family model with workload provenance,
   pressure dimensions, baseline policy, value hypothesis, and condition matrix.
3. Add common normalized `scenario_family`, `option_run`, and
   `paired_comparison` report types.
4. Implement one native sketch primitive scenario family that emits exact and
   approximate option runs plus paired comparisons.
5. Update the ASAPQuery PromQL importer to attach scenario-family metadata,
   workload provenance, deployment pattern, value hypothesis, resource
   accounting scope, and explicit missing-resource fields.
6. Add at least one data-condition sweep for the native scenario family.
7. Add validation checks:
   - scenario family has required fields for its track;
   - reference execution and exact performance baseline are identified;
   - baseline policy states whether the baseline is strong, partial, or weak;
   - option/config policy states how parameters were chosen;
   - workload provenance and pressure dimensions are present;
   - comparison rules exist;
   - resource accounting status is complete or explicitly partial.
8. Keep the existing MVP v1 command paths working.

## Success Criteria

MVP v2 succeeds when:

- a native sketch primitive scenario can show exact-versus-approximate
  break-even behavior over a small data-condition sweep;
- the ASAPQuery PromQL adapter can import existing quickstart outputs into the
  same scenario/report model without claiming complete resource accounting;
- each scenario family records workload provenance, pressure dimensions,
  baseline policy, option/config policy, and coverage;
- every report states the exact reference execution, exact performance
  baseline, approximate deployment pattern, and resource accounting status;
- approximation can win, lose, or be inconclusive without changing the schema;
- the design can explain why a result supports or rejects the scenario's value
  hypothesis for that data condition.

## Open Questions

- What is the minimum exact baseline strength required for each track?
- Should the exact reference execution and exact performance baseline be allowed
  to differ in MVP v2, or only in later scenarios?
- Which resource dimensions are mandatory for a VLDB-quality PromQL scenario?
- What is the first ASAPFusion scenario concrete enough to implement?
- Should `value_hypothesis` be a required manifest field for sketch primitives,
  or only for system scenarios?
- How should the report encode confidence intervals and variance across
  repeated runs?
