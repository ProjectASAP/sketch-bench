# `aqpbm-core` Design

## 1. Purpose

Core is the place that defines the functionality used by more than one caller.
If a functionality is tied to one caller, whether that is a bundle of implementations, a frontend or an embedded path, it does **not** belong to this crate.

## 2. The parts

### 2.1 What counts as measurable

An **accumulator** is anything that takes items one at a time and builds state from them, and that is the whole of what core requires.
Ingesting items, answering a statistic, and reporting a footprint are three separate claims, and an implementation declares each one it can honour.
A **row** is a `(algorithm, impl)` pair realised as a Rust type.

### 2.2 Construction from an opaque parameter object

A **`ParamSet`** is an algorithm name plus that algorithm's construction parameters, carried as a JSON object.

```json
{"algorithm": "cms", "params": {"rows": 5, "cols": 32768}}
```

The envelope is shared and the contents belong to one bundle.
Carrying the object is therefore functionality in this crate, and inspecting it is not.

### 2.3 Workload materialisation

A **workload** is the materialised, ordered item stream plus its provenance, replayed in full by every run.
Core materialises it at the item type the row declares, and ships one carrier per item type: numbers, strings, bytes and labelled values.

### 2.4 The runner and the timed loop

A **cell** is one row, at one parameter point, against one workload.
A **run** is one measured iteration over a freshly constructed accumulator inside one process.
Driving the runs, warming up ahead of them, and deciding what sits inside a timed region are one functionality.

### 2.5 Passes and the metrics mask

A **pass** is one metric group measured over its own population of runs: `throughput`, `latency`, `accuracy`, or `merge`.
Splitting one request into passes is what stops a metric from paying for another metric's instrumentation.

### 2.6 Ground truth and the comparators

A comparator binds a capability, never an algorithm, so it scores any implementation that declares that capability.
That is what lets ground truth, and the scoring of an answer against it, live in a crate naming no algorithm.

### 2.7 The recorders

A sink is four hooks, around update and around query, and a probe is the wrapper that wears them.

The offline runner and the embedded sampler are two callers wanting the same hooks.

### 2.8 Aggregation and the record

Folding a pass's runs into a mean, a stddev, the per-run samples and a count is one functionality, and so is the line that fold serialises to.

```json
{"schema_version": 3,
 "sketch": "cms", "impl": "oxide", "language": "rust",
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

A line names its producer: `mode` says whether a bench run, a profile run or an embedded sampler made it, and `source` says which program did.
Core owns the profile slot beside the bench one, so one reader deserialises every kind of line.

## 3. What is guaranteed

- **An interval is never taken inside one process.** Core writes none, and the field stays empty until a caller that re-executes the process fills it.

- **A row is never credited with work it deferred.** Only `update` is timed as ingest, and `prepare` runs on its own clock.

- **Absent and zero stay distinguishable.** An optional metric is omitted when the pass did not measure it, so a zero in a record is a measurement.

- **A capability gap is a value.** A row that provides no merge records that fact in the output, and never as a missing line.

- **A cell fails whole or not at all.** Construction is proved once before any measurement, so a cell that cannot be built fails with a message instead of a partial result.

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
A capability trait states the query shape its own statistic wants.

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

### 4.4 The report format

`Comparison` is a comparator's output, a flat map of named scalars, so aggregation folds every key across runs without knowing any statistic's shape.
The grouping key for anything that pools records is `(sketch, impl, sketch_config, workload, pass)`, and pooling two passes averages two experiments.

### 4.5 The build-time surface

Backing the latency recorder with a real histogram is on by default; without it the recorder counts calls and the latency summary carries counts only.
Reading process heap use requires jemalloc, which the feature forces on the linking binary, so it stays opt-in.
Per-sketch allocation accounting compiles in a counting allocator, which the linking binary installs as its global and which counts the whole process.

## 5. Open questions

- **Merge topology.** Contiguous shards folded sequentially measure one point in a space that also holds interleaved shards, hash partitioning, and tree folds.
  Admitting more means new record fields so the numbers stay comparable; fixing one keeps the column simple and measures one arrangement.

- **Who chooses the probe set.** Each comparator picks its own probe count and population, which lets a statistic ask for what it needs.
  Two algorithms' query-throughput numbers then rest on different probe counts.
  A shared probe budget in the configuration would make them comparable, at the cost of a knob that means something different for each statistic.

- **Naming the deferred build.** The method is `prepare` and the metrics it feeds are `finalize_time_ms` and `build_throughput_items_per_sec`, so one of the two names should move.
  Moving the method breaks every implementation; moving the fields is a schema bump coordinated across every producer and reader.

- **Per-thread allocation accounting.** The allocation counters are process-global atomics, so a parallel-insert row's heap numbers describe the process.
  Per-thread accounting needs a thread-local shim plus a rule for which threads belong to the measurement, and the rule is the harder half.
