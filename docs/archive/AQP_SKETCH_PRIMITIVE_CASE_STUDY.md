# AQPBMV2 Case Study: Approximate Function Kernel Benchmark

This is the planned native `sketch-bench` AQPBMV2 case. It does not adapt an
external query system. It benchmarks executable approximate function candidates
through local benchmark-owned kernels.

The important change from the earlier sketch-primitive plan is scope:

```text
old framing:
  wrap existing sketch primitive runs as AQP option/comparison records

current AQPBMV2 framing:
  compare exact and sketch-backed approximate function candidates under
  single-state, grouped-state, and partitioned-merge kernels
```

The goal is not to prove that sketches are approximate. The goal is to test
whether raw sketch or single-state conclusions predict behavior for
user-level approximate functionality under AQP-style workloads with many
states, many answers, grouping, and explicit merge shape.

## Concrete Unit Under Test

The unit under test is an executable approximate function candidate:

```text
functionality spec + state implementation + fixed parameter setting
```

Examples:

```text
HllCountDistinct<HllImpl, precision=14>
KllQuantile<KllImpl, k=200>
TDigestQuantile<TDigestImpl, compression=100>
```

The executable interface should be code:

```rust
trait ApproxFunction {
    type Input;
    type State;
    type Output;
    type Query;

    fn create(&self) -> Self::State;
    fn update(&self, state: &mut Self::State, input: Self::Input);
    fn merge(&self, left: &mut Self::State, right: Self::State);
    fn finalize(&self, state: &Self::State, query: Self::Query) -> Self::Output;
}
```

The thin sketch binding exists only below this layer to call a sketch
implementation. It should not add fallback-to-exact, adaptive tuning, custom
merge logic, or window rebuild policy. If those policies are added, the target
is no longer the function candidate; it is an adapter-policy benchmark.

## Baseline Kernel: Single-State

The existing sketch benchmark substrate already covers this shape:

```text
state = new_state()
for value in values:
  update(state, value)
answer = query(state, parameter)
```

This remains useful as the primitive baseline. It answers:

```text
How does the underlying sketch/state implementation behave on one stream and
one state?
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

## First New Kernel: Grouped-State Count Distinct

The first AQPBMV2 implementation should be approximate count distinct because
the exact-vs-sketch substitution is simple:

```text
ExactCountDistinct<HashSet>
HllCountDistinct<HllImpl>
```

The grouped kernel shape is:

```text
states = {}
for row in rows:
  key = key_fn(row)
  value = value_fn(row)
  update(states[key], value)

for key in states:
  answer[key] = finalize(states[key], ())
```

For an HLL-style candidate, this creates many HLL-backed count-distinct
function states rather than one raw HLL stream. The exact reference is a
per-group exact set cardinality.

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
- per-group relative/absolute error;
- answer coverage under the requirement;
- failures grouped by group size and data-condition region;
- latency and memory/state-size summaries.

The desired comparison is:

```text
What does replacing exact per-group sets with HLL-backed function states gain
and lose under grouped-state pressure?
```

## Second New Kernel: Partitioned-Merge Function

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
  answer[key] = finalize(merged, query)
```

Controlled execution dimensions should include:

- partition count;
- partitioning policy;
- merge tree shape;
- input order;
- skew across partitions.

This kernel should not be described as "running distributed SQL." The benchmark
itself owns the partitioning and merge program. That is what keeps the result
attributable to the function candidate instead of to an optimizer or query
engine.

## Example Manifest

```toml
[benchmark]
id = "aqpbmv2_grouped_count_distinct_hll_v1"
track = "function_kernel_aqp"
kernel = "grouped_count_distinct"
task = "count_distinct"

[function_candidate]
functionality = "approx_count_distinct"
implementation = "HllCountDistinct"
sketch_family = "hll"
sketch_implementation = "asap_sketchlib"
parameters = { precision = 14 }
binding = "thin_native_binding"

[candidate_metadata]
language = "rust"
version = "pinned_or_recorded_version"
native_operations = ["update", "query", "merge"]
serialization = "native_or_absent"
api_binding_effort = "thin"
metadata_status = "declared_not_benchmarked"

[data_condition]
id = "n1m_groups10k_zipf1_2_seed42"
rows = 1000000
groups = 10000
group_size_distribution = "zipf"
group_size_zipf_s = 1.2
value_cardinality_distribution = "per_group_zipf"
seed = 42

[requirements]
relative_error_max = 0.05
answer_coverage_min = 0.95

[execution_shape]
partition_count = 1
merge_shape = "none"
input_order = "generated"

[baseline]
exact_reference = "per_group_exact_hash_set"
```

## Report Shape

AQPBMV2 should add kernel-level records rather than only reusing the current
single-stream output.

Required records:

- `candidate_dossier`: candidate identity, fixed parameters, native operation
  coverage, language/runtime metadata, serialization support, and other
  declared adoption metadata;
- `kernel_run`: function candidate, kernel, data condition, execution shape,
  raw cost, and raw fidelity summary;
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
For the same approximate function candidate, do primitive/single-state results
predict grouped-state and partitioned-merge workload coverage?
```

Minimum milestone:

1. Implement `ExactCountDistinct<HashSet>` and `HllCountDistinct<HllImpl>`.
2. Run the HLL-backed candidate through grouped-state count distinct.
3. Emit exact per-group baselines.
4. Report answer coverage and failure localization.
5. State whether grouped-state behavior matched or contradicted the
   single-state result.

Stronger milestone:

1. Add at least two HLL implementations or parameter settings.
2. Sweep group count, group-size skew, and per-group cardinality.
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
