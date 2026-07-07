# AQPBMV2 Case Study: Sketch Kernel Benchmark

This is the planned native `sketch-bench` AQPBMV2 case. It does not adapt an
external query system. It benchmarks concrete sketch implementation instances
through local benchmark-owned kernels.

The important change from the earlier sketch-primitive plan is scope:

```text
old framing:
  wrap existing sketch primitive runs as AQP option/comparison records

current AQPBMV2 framing:
  compare single-state sketch behavior against grouped-state and
  partitioned-merge sketch kernels
```

The goal is not to prove that sketches are approximate. The goal is to test
whether single-stream sketch benchmark conclusions predict behavior for
AQP-style workloads with many states, many answers, grouping, and explicit
merge shape.

## Concrete Unit Under Test

The unit under test is a concrete sketch instance:

```text
sketch implementation + fixed parameter setting + thin benchmark binding
```

Examples:

```text
KLL implementation, k=200
t-digest implementation, compression=100
HLL implementation, precision=14
```

The thin binding exists only to call the implementation from the benchmark
harness. It should not add fallback-to-exact, adaptive tuning, custom merge
logic, or window rebuild policy. If those policies are added, the target is no
longer the sketch instance; it is an adapter-policy benchmark.

## Baseline Kernel: Single-State

The existing sketch benchmark substrate already covers this shape:

```text
state = new_state()
for value in values:
  update(state, value)
answer = query(state, parameter)
```

This remains useful as the baseline. It answers:

```text
How does this sketch instance behave on one stream and one state?
```

Existing `sketchlib bench` support is already close to this baseline:

- sketch family and implementation selection;
- synthetic and file-backed workloads;
- configuration sweeps through `--config`;
- repeated measured runs and warmups;
- throughput, latency, CPU, memory, and optional accuracy metrics;
- ground-truth comparators for cardinality, quantile, and frequency families;
- JSONL output with `sketch`, `impl`, `workload`, `sketch_config`, and
  `bench.accuracy`.

This is necessary, but it is not sufficient for AQPBMV2's intended claim.

## First New Kernel: Grouped-State Quantile

The first AQPBMV2 implementation should add a grouped quantile kernel:

```text
states = {}
for row in rows:
  key = key_fn(row)
  value = value_fn(row)
  update(states[key], value)

for key in states:
  answer[key] = quantile(states[key], q)
```

For a KLL-style candidate, this creates many KLL states rather than one KLL
state. The exact reference is a per-group exact order statistic.

Controlled data dimensions should include:

- number of rows;
- number of groups;
- group-size skew;
- per-group value distribution;
- tail heaviness;
- duplicate rate;
- key-value correlation;
- seed.

The benchmark should report:

- per-group approximate answer;
- per-group exact answer;
- per-group rank/value error;
- answer coverage under the requirement;
- failures grouped by group size and data-condition region;
- latency and memory/state-size summaries.

The desired comparison is:

```text
Does the candidate that looked best in the single-state kernel still provide
the best answer coverage under grouped-state pressure?
```

## Second New Kernel: Partitioned-Merge Quantile

After grouped-state is stable, add partitioned merge for candidates that
natively support merge:

```text
partial_states = {}
for row in rows:
  partition = partition_fn(row)
  key = key_fn(row)
  update(partial_states[partition][key], value_fn(row))

for key in all_keys:
  merged = merge_all(partial_states[*][key], merge_shape)
  answer[key] = quantile(merged, q)
```

Controlled execution dimensions should include:

- partition count;
- partitioning policy;
- merge tree shape;
- input order;
- skew across partitions.

This kernel should not be described as "running distributed SQL." The benchmark
itself owns the partitioning and merge program. That is what keeps the result
attributable to the sketch instance instead of to an optimizer or query engine.

## Example Manifest

```toml
[benchmark]
id = "aqpbmv2_grouped_quantile_kll_v1"
track = "sketch_kernel_aqp"
kernel = "grouped_quantile"
task = "quantile"

[candidate]
family = "kll"
implementation = "asap_sketchlib"
parameters = { k = 200 }
binding = "thin_native_binding"

[candidate_metadata]
language = "rust"
version = "pinned_or_recorded_version"
native_operations = ["update", "query", "merge"]
serialization = "native_or_absent"
api_binding_effort = "thin"
metadata_status = "declared_not_benchmarked"

[data_condition]
id = "n1m_groups10k_zipf1_2_lognormal_tail_seed42"
rows = 1000000
groups = 10000
group_size_distribution = "zipf"
group_size_zipf_s = 1.2
value_distribution = "per_group_lognormal_tail"
key_value_correlation = "strong"
seed = 42

[query]
quantile = 0.99

[requirements]
rank_error_max = 0.01
answer_coverage_min = 0.95

[execution_shape]
partition_count = 1
merge_shape = "none"
input_order = "generated"

[baseline]
exact_reference = "per_group_exact_order_statistics"
```

## Report Shape

AQPBMV2 should add kernel-level records rather than only reusing the current
single-stream output.

Required records:

- `candidate_dossier`: candidate identity, fixed parameters, native operation
  coverage, language/runtime metadata, serialization support, and other
  declared adoption metadata;
- `kernel_run`: candidate, kernel, data condition, execution shape, raw cost,
  and raw fidelity summary;
- `answer_record`: exact and approximate answer for one logical output key
  when storage volume is acceptable, or sampled/aggregated answer records when
  it is not;
- `coverage_summary`: answer coverage, requirement pass/fail, and failure
  localization;
- `single_vs_grouped_comparison`: whether the grouped or partitioned result
  agrees with the single-state baseline.

The candidate dossier should distinguish measured fields from declared fields.
For example, update throughput and answer coverage are measured. Language,
license, documentation quality, public API shape, and maintenance status are
adoption metadata. They are useful for choosing between candidates, but they
should not be mixed into a single benchmark score.

## Success Criteria For This Case

This case succeeds when it can answer:

```text
For the same concrete sketch instance, do single-state benchmark results
predict grouped-state and partitioned-merge workload coverage?
```

Minimum milestone:

1. Run one KLL-like quantile candidate through the existing single-state path.
2. Run the same candidate through grouped-state quantile.
3. Emit exact per-group baselines.
4. Report answer coverage and failure localization.
5. State whether grouped-state behavior matched or contradicted the
   single-state result.

Stronger milestone:

1. Add at least two quantile candidates or parameter settings.
2. Sweep group count, group-size skew, and tail heaviness.
3. Show whether candidate ranking changes between single-state and grouped
   kernels.
4. Add partitioned-merge for candidates with native merge support.

## Remaining Risks

- If grouped and partitioned kernels do not reveal behavior that differs from
  single-state tests, AQPBMV2 is useful engineering but a weaker research
  contribution.
- If the kernels are not argued as canonical AQP execution pressures, they may
  look like arbitrary synthetic programs.
- If the binding adds policy, the benchmark target becomes ambiguous.
- If existing sketch benchmarks already cover these exact kernel shapes, the
  novelty claim must be narrowed to tooling and reproducibility.
