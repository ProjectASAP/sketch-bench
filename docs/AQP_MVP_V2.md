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
For a concrete benchmark scenario, when is approximate execution worthwhile
against an optimized exact alternative?
```

"Worthwhile" means the approximate option provides a measured benefit, such as
lower query latency, lower query CPU, lower memory growth, longer retention,
higher query concurrency, or lower effective serving cost, while satisfying the
declared fidelity and result-shape requirements.

MVP v2 should not assume approximation is beneficial. Showing that exact
execution remains better for a scenario is a valid benchmark result.

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
| `scenario` | One benchmark instance with a concrete system, native interface, workload, data condition, exact reference execution, exact performance baseline, approximate option, resource accounting scope, and comparison rules. |
| `exact reference execution` | The exact computation used to produce reference answers for fidelity measurement. |
| `exact performance baseline` | The optimized exact alternative used for performance and resource comparison. |
| `approximate option` | The approximate system path, configuration, or direct sketch implementation being evaluated. |
| `approximate deployment pattern` | How the approximate option is deployed: replacement, augmentation, tiered, precompute, fallback-capable, or primitive. |
| `resource accounting scope` | The cost categories counted when comparing exact and approximate deployments. |
| `value hypothesis` | The scenario-specific reason approximation might be useful. |
| `outcome` | The observed result status for an option: exact reference, exact system, approximate system, fallback, unsupported, error, timeout, missing counterpart, or not applicable. |

## Scenario Manifest

MVP v2 should introduce a scenario manifest. The manifest is the stable unit
that makes a run interpretable.

Sketch:

```toml
[scenario]
id = "promql_grouped_quantile_quickstart_v2"
track = "promql_aqp"
description = "Grouped telemetry quantiles over PromQL-compatible serving"

[system_under_test]
name = "ASAPQuery"
interface = "promql_http"
deployment_pattern = ["augmentation", "precompute"]

[workload]
query_suite = "../ASAPQuery/benchmarks/queries/promql_suite.json"
task_family = ["aggregate", "grouped_quantile"]

[data_condition]
id = "quickstart_fake_exporters_patterns_v1"
source = "asapquery_quickstart_fake_exporters"
notes = "Deterministic pattern exporters; not yet a broad realism sweep."

[reference_execution]
name = "Prometheus"
role = "exact_reference"
pairing_policy = "same_promql_time"

[exact_performance_baseline]
name = "Prometheus"
role = "optimized_exact_baseline"

[approximate_option]
name = "ASAPQuery PromQL precompute path"
role = "approximate_system"

[value_hypothesis]
statement = "Precomputed approximate summaries reduce p95 latency for grouped telemetry quantiles enough to justify their extra maintenance cost."
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
| `system_under_test` | A benchmark without a concrete system is only a workload description. |
| `interface` | AQP behavior depends on the real API boundary users exercise. |
| `workload` | The benchmark must state which task/query behavior is being stressed. |
| `data_condition` | AQP tradeoffs change with skew, cardinality, group count, time dynamics, and query rate. |
| `reference_execution` | Fidelity cannot be measured without a precise exact result. |
| `exact_performance_baseline` | Approximation value cannot be judged against an unoptimized or implicit exact alternative. |
| `approximate_option` | The benchmark must identify what approximate path or configuration is being evaluated. |
| `value_hypothesis` | The run should state why approximation is expected to be useful, so the result can confirm or reject that reason. |
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

MVP v2 should include two scenario types because they exercise different
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
data_conditions: sweeps over cardinality, skew, group count, and size
```

First task candidate:

```text
task: grouped count distinct or quantile
exact baseline: HashSet per group or sorted vector per group
approx option: HLL per group or KLL per group
value hypothesis: sketch memory grows more slowly than exact state after a
  cardinality/group-count threshold while staying within error budget.
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
data_condition: quickstart fake exporters first, broader sweep later
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

## Report Model

MVP v2 should emit normalized records at two levels.

### Option Run

One system or option execution.

```json
{
  "record_type": "option_run",
  "scenario_id": "promql_grouped_quantile_quickstart_v2",
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
  "scenario_id": "promql_grouped_quantile_quickstart_v2",
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
- It stresses at least one data or workload dimension where exact execution may
  become expensive.
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
2. Add common normalized `option_run` and `paired_comparison` report types.
3. Implement one native sketch primitive scenario that emits exact and
   approximate option runs plus paired comparisons.
4. Update the ASAPQuery PromQL importer to attach scenario metadata,
   deployment pattern, value hypothesis, resource accounting scope, and explicit
   missing-resource fields.
5. Add at least one data-condition sweep for the native scenario.
6. Add validation checks:
   - scenario has required fields for its track;
   - reference execution and exact performance baseline are identified;
   - comparison rules exist;
   - resource accounting status is complete or explicitly partial.
7. Keep the existing MVP v1 command paths working.

## Success Criteria

MVP v2 succeeds when:

- a native sketch primitive scenario can show exact-versus-approximate
  break-even behavior over a small data-condition sweep;
- the ASAPQuery PromQL adapter can import existing quickstart outputs into the
  same scenario/report model without claiming complete resource accounting;
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
