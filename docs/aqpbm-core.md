# `aqpbm-core` Design

## 1. Purpose

Core is a crate that contains the common functionalities.
It provides functionalites including (and may not limit to) statistical analysis, ground truth, timing, memory measurement, etc..
If an intuition about what goes to this crate is required, it will be: if anything ties to something specific, this thing should not goes to `aqpbm-core` crate.

## 2. The parts

### 2.1 Construction from an opaque parameter object

A **`ParamSet`** is an algorithm name plus that algorithm's construction parameters, carried as a JSON object.

```json
{"algorithm": "cms", "params": {"rows": 5, "cols": 32768}}
```

Core matches the name as a string and pass the parameter to an appropriate constructor (likely in some bundle). `aqpbm-core` never reads the parameters, whose vocabulary is known only to the bundle defining that family.

### 2.2 Workload materialisation

A **workload** is the materialised, ordered item stream plus its provenance, replayed in full by every run.
`aqpbm-datagen` produces the items, generated from a shape or replayed from a file, and core materialises them at the row's item type.
One carrier exists per item type: numbers, strings, bytes and labelled values.
Adding an item type means one more carrier and one more rule to materialise it.

### 2.3 The runner and the timed loop

A **cell** is one row of data, at one parameter point, against one workload.
A **run** is one measured iteration over a freshly constructed accumulator inside one process.
Core drives the runs, warms up ahead of them, and decides what sits inside a timed region.

### 2.5 Passes and the metrics mask

A **pass** is one metric group measured over its own population of runs.
Currently there are `throughput`, `latency` and `accuracy`.
User may request multiple metrics in one request.
Splitting one request into passes is what stops a metric from paying for another metric's instrumentation (e.g., throughput and accuracy needs two seperate benchmark).

### 2.6 The statistics and their comparators

A **capability** is a claim on one statistic.
Currently, **capability** includes the following:
```
cardinality         how many distinct items are there
frequency           how many times did this key occur
quantile            which value sits at this fraction
top-k               which k keys are heaviest, and how heavy
subpop-cardinality  cardinality, within the records carrying a set of labels
subpop-frequency    frequency, within the records carrying a set of labels
subpop-quantile     quantile, within the records carrying a set of labels
```

The core crate provides ground truth (the comparator) of each **capability**.
A comparator binds a capability.
Multiple algorithms can bind to the same **capability**.
One capability can carry several comparators: a quantile answer scores as a rank error or as a relative error, and the row chooses.

### 2.7 The recorders

A sink is four hooks, around update and around query, and a probe is the wrapper that wears them.

The offline runner and `sketch-runtime`, the sampler linked into a live application, want the same four hooks.
A fifth hook would change every probe that wears the sink, so the four are a fixed shape.

### 2.8 Aggregation and the record

Core folds a pass's runs into a mean, a stddev, the per-run samples and a count, then serialises the fold as one line.

```json
{"schema_version": 3,
 "sketch": "cms", "family": "cms", "impl": "oxide", "language": "rust",
 "sketch_config": {"algorithm": "cms", "params": {"rows": 5, "cols": 32768}},
 "workload": {"shape": "zipf", "size": 1000000, "cardinality": 10000, "zipf_s": 1.1, "seed": 42},
 "mode": "bench", "runs": 10,
 "bench": {"pass": "throughput",
           "throughput_items_per_sec":       {"mean": 4.21e7, "stddev": 1.1e6, "n": 10},
           "build_throughput_items_per_sec": {"mean": 4.21e7, "stddev": 1.1e6, "n": 10},
           "finalize_time_ms":               {"mean": 0.0,    "stddev": 0.0,   "n": 10},
           "memory_bytes": 40960},
 "source": "cli", "timestamp": "2026-07-27T09:14:22.481Z"}
```

`sketch` carries the algorithm with its structural variant, `family` the group it belongs to, so a reader picks an axis without parsing names.

A line names its producer: `mode` says whether a bench run, a profile run or an embedded sampler made it, and `source` says which program did.
Core owns the profile slot beside the bench one, so one reader deserialises every kind of line.
A new field is additive and optional, and changing what an existing field means is a `schema_version` bump.

## 3. What is guaranteed

- **An interval is never taken inside one process.** Core writes none, and the field stays empty until a caller that re-executes the process fills it.

- **A row is never credited with work it deferred.** Only `update` is timed as ingest, and `prepare` runs on its own clock.

- **Absent and zero stay distinguishable.** An optional metric is omitted when the pass did not measure it, so a zero in a record is a measurement.

- **An unmade claim is a value.** A row that does not merge records that fact in the output, and every other unmade claim is recorded the same way.

- **A cell fails whole or not at all.** Construction is proved once before any measurement, so a cell that cannot be built fails with a message instead of a partial result.

- **The config in a record is the config that ran.** A construction that would clamp, round or ignore a requested value fails instead.
  Every plot keys on that field, so a value nothing was measured at can never reach it.

## 4. Interfaces

### 4.1 What an implementation implements

```rust
pub trait Accumulator {
    type Item;
    fn update(&mut self, v: &Self::Item);
    fn merge(&mut self, other: &Self) -> Result<(), MergeUnsupported> { /* none by default */ }
    fn prepare(&mut self) {}
}

pub trait InitSketch: Accumulator + Sized {
    fn init(config: &ParamSet) -> Result<Self, BuildError>;
}

pub trait BenchImpl: Accumulator {
    type Params: SketchParams;                 // the family's parameter vocabulary
    const IMPL: &'static str;                  // the library: "oxide", "lib", ...
    const ALGORITHM: &'static str = <Self::Params as SketchParams>::FAMILY;  // a variant overrides
    const FAMILY: &'static str = <Self::Params as SketchParams>::FAMILY;     // derived, never written
}
pub trait MemoryFootprint {
    fn memory_bytes(&self) -> usize;           // best-effort; a tight upper bound is fine
}
```

`update` takes its item by reference, so a wrapper carrying strings or byte slices is not charged for a clone on the hot path.

A row whose construction takes a run knob implements a second init trait, since a worker count is no part of a sketch's parameters.

```rust
pub trait ParallelInit: Accumulator + Sized {
    fn build(config: &ParamSet, workers: usize) -> Result<Self, BuildError>;
}
```

A row declares a statistic by implementing that statistic's trait, one per entry in §2.6, each with the query shape it wants.

```rust
pub trait CardinalityOps { fn estimate_distinct(&self) -> f64; }
pub trait FrequencyOps { type Key; fn estimate_frequency(&self, key: &Self::Key) -> u64; }
```

### 4.2 What a frontend calls

```rust
// The timed half: throughput, latency, the merge fold, plus CPU and memory.
run_cell::<S>(&BenchConfig, &WorkloadSpec, &ParamSet)          -> Result<Vec<BenchReport>, RunError>
run_cell_parallel::<S>(&BenchConfig, &WorkloadSpec, &ParamSet) -> Result<Vec<BenchReport>, RunError>

// The untimed half: accuracy and post-merge accuracy, scored against a comparator.
score_cell::<S, G>(&BenchConfig, &WorkloadSpec, &ParamSet, &G) -> Result<Vec<BenchReport>, RunError>
```

The merge pass spans both calls: `run_cell` times the fold, and `score_cell` scores its result against a single-pass reference.
`BenchConfig` carries the measured-run count, the warm-up count, the metrics mask, the merge shard count and the worker count.
Each call returns one `BenchReport` per pass the mask selected, and every record carries a `pass` label.

### 4.3 The parameter surface

Recovering a typed value from a `ParamSet` fails for a set of another algorithm, which the error names.
A single point also parses from whitespace-separated `key=value` tokens, typed by inspection: `rows=5` a number, `exact=true` a boolean.

### 4.4 The comparator and the report format

A comparator computes the exact answer from the raw items and scores the sketch against it.

```rust
pub trait GroundTruth<S: Accumulator> {                      // the `G` of `score_cell`
    fn compare(&self, sketch: &S, items: &[S::Item]) -> Comparison;
}
```

`Comparison` is a comparator's output, a flat map of named scalars, so aggregation folds every key across runs without knowing any statistic's shape.
The grouping key for anything that pools records is `(sketch, impl, sketch_config, workload, pass)`, and pooling two passes averages two experiments.

### 4.5 The build-time surface

Backing the latency recorder with a real histogram is on by default; without it the recorder counts calls and the latency summary carries counts only.
Reading process heap use requires jemalloc, which the feature forces on the linking binary, so it stays opt-in.
Per-sketch allocation accounting compiles in a counting allocator, which the linking binary installs as its global and which counts the whole process.

## 5. Open questions

- **Who chooses the probe set.** Each comparator picks its own probe count and population, which lets a statistic ask for what it needs.
  Two algorithms' query-throughput numbers then rest on different probe counts.
  A shared probe budget in the configuration would make them comparable, at the cost of a knob that means something different for each statistic.

- **Naming the deferred build.** The method is `prepare` and the metrics it feeds are `finalize_time_ms` and `build_throughput_items_per_sec`, so one of the two names should move.
  Moving the method breaks every implementation; moving the fields is a schema bump coordinated across every producer and reader.

- **Per-thread allocation accounting.** The allocation counters are process-global atomics, so a parallel-insert row's heap numbers describe the process.
  Per-thread accounting needs a thread-local shim plus a rule for which threads belong to the measurement, and the rule is the harder half.
