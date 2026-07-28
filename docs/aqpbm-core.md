# `aqpbm-core` Design

`aqpbm-core` is the measurement framework.
It names no sketch algorithm and no third-party sketch library.

## 1. Purpose

An **accumulator** is anything that takes items one at a time and builds state from them, and that is the whole of what this crate requires.
An exact `HashMap` counter qualifies as fully as a HyperLogLog does, which lets a baseline run through the same machinery as the sketch it checks.

Every part of taking a measurement is core's:

- the traits an implementation plugs into, including the capability traits that state which statistic it answers
- materialising a workload at the item type the row declares
- the warm-up, both the CPU ramp once per process and the discarded iterations once per pass
- the insert loop and the clocks around each phase, with only the row's `update` inside a timed region
- constructing the ground truth an answer is scored against, and the comparators that do the scoring
- the metrics mask, and its decomposition into one pass per metric group
- the recorders, and the sink they report through, shared with the embedded path
- the fold across a pass's runs into a mean, a stddev, the per-run samples and a count
- the platform readings: CPU time, peak resident set, and heap use
- the record every result is serialised into, and the schema version on it

The boundary is nominal knowledge.
A crate that spells the string `"hll"`, or that names a third-party sketch library, is on the other side of it.
Core learns which implementation is under measurement from the generic parameter its caller instantiates, and how to build it from a JSON object it never inspects.
That is what lets a second bundle of implementations sit beside `sketch-bench` on the same framework.

Wrapped implementations, per-algorithm parameter structs, and the catalog mapping a `(algorithm, impl)` string pair to a concrete type live in `sketch-bench`.
Argv, files on disk, and multi-process repeats live in `aqpbm-cli`, because only a process can make those decisions.
How values are drawn, and how a file is laid out, live in `aqpbm-datagen`, whose spec types core re-exports so a caller needs one dependency.
The embedded sampling path lives in `sketch-runtime`, and the pieces both paths share live here.
Microarchitectural collection lives in `sketch-profile`; core owns the record's profile slot so that one reader deserialises both kinds of line.

## 2. Inputs

Core consumes five things per cell, plus what the platform reports.
A **cell** is one row, at one parameter point, against one workload, and it is the unit one invocation measures.

- **The implementation, as a type.** The row arrives as a generic parameter satisfying `Accumulator`, `InitSketch`, `BenchImpl` and `MemoryFootprint`, plus whichever capability traits it declares.
  A **row** is a `(algorithm, impl)` pair realised as a Rust type.
  **algorithm** names the algorithm (`hll`, `cms`, `kll`), and **impl** names one library's version of it (`oxide`, `datasketches`).
  This is the only input that is not data.
  Monomorphising it in the caller's crate is what lets the row's `update` inline into the timed loop.

- **Construction parameters, as a `ParamSet`.** An algorithm name plus that algorithm's construction parameters, carried as an opaque JSON object.

```json
{"algorithm": "cms", "params": {"depth": 5, "width": 2048}}
```

  Core does not know that a count-min sketch takes a depth and a width.
  It knows the object is tagged with an algorithm, and that whatever parses the object belongs to that algorithm.

- **A workload spec.** Either a generator spec naming shape, size and seed, or a path to a file.
  A **workload** is the materialised, ordered item stream plus the provenance describing where it came from, replayed in full by every run.
  Core materialises the spec at the item type named by the row's `Accumulator::Item`, so no encoding has to be negotiated.
  A source that cannot serve that item type fails before any measurement.
  Error is a deterministic function of data and parameters, so an accuracy repetition draws its own sample, and a workload that cannot redraw measures once.

- **A run configuration.** `BenchConfig` carries the measured-run count, the warm-up count, the metrics mask, the merge shard count, the worker count for parallel rows, and a nominal seed.
  `AccuracyCfg` carries the scoring knobs: whether to score, a cap on the keys a frequency comparator probes, and whether to retain per-call samples.

- **A comparator.** The accuracy half of a cell takes a `GroundTruth` instance chosen for the row's capability.
  The timed half takes none, and does not name the type in its signature.

- **The platform.** CPU time and peak resident set, both read for the whole process.
  Process heap use is read from jemalloc where the binary links it, and per-sketch allocation from a counting global allocator that the binary installs.

## 3. Outputs

A **run** is one measured iteration over a freshly constructed accumulator inside one process.
A **pass** is one metric group measured over its own population of runs: `throughput`, `latency`, `accuracy`, or `merge`.
Those four are the primary bits of the metrics mask, and metrics are not all free to collect together, which is why each takes a fresh population.
The secondary bits, CPU and memory, record at phase boundaries only, so they attach to every pass without contaminating it.

- **`BenchReport`, one per pass.** The in-process result: the row's identity, the workload's description, every measured run's `RunMetrics`, and the aggregated `BenchSection`.

- **`RunMetrics`, one per run.** Counts, the phase clocks, the optional memory and latency readings, and that run's accuracy scalars.
  It is also the unit an embedded sampler emits per window, which is what makes a live record and an offline record the same shape.

- **`Record`, one JSONL line per pass.** The serialised contract, shared with the C++ track, the runtime path, and the visualisation layer.

```json
{"schema_version": 2,
 "sketch": "cms", "impl": "oxide", "language": "rust",
 "sketch_config": {"algorithm": "cms", "params": {"depth": 5, "width": 2048}},
 "workload": {"shape": "zipf", "size": 1000000, "cardinality": 10000, "zipf_s": 1.1, "seed": 42},
 "mode": "bench", "runs": 10,
 "bench": {"pass": "throughput",
           "throughput_items_per_sec":       {"mean": 4.21e7, "stddev": 1.1e6, "n": 10},
           "build_throughput_items_per_sec": {"mean": 4.21e7, "stddev": 1.1e6, "n": 10},
           "finalize_time_ms":               {"mean": 0.0,    "stddev": 0.0,   "n": 10},
           "memory_bytes": 40960},
 "source": "cli", "timestamp": "2026-07-27T09:14:22.481Z"}
```

  An optional metric is omitted when the pass did not measure it, so "absent" and "measured as zero" stay distinguishable.
  Two throughput columns exist because no single one is comparable across a mixed panel of implementations.
  `throughput_items_per_sec` is the ingest rate over the insert loop alone; `build_throughput_items_per_sec` is the rate a queryable accumulator is produced at.
  `prepare` is the optional deferred build, called once after the last item, and it runs on its own clock.
  A row that buffers in `update` therefore reports a fast ingest column and a slower build column, and is never credited with a fast ingest loop.

- **`Comparison`, one per scored run.** A flat map of named scalars plus the timing of the estimate calls that produced them.
  Flat and named, so aggregation folds every key across runs without knowing any statistic's shape, and each key states the population it is over.

- **Per-call query samples.** One row per timed estimate call, retained when the frontend asks: call index, nanoseconds, the answer, and for quantiles the percentile and sweep index.

- **Errors.** A cell that cannot obtain its workload, or cannot build at the requested parameters, fails as one error naming which of the two happened.
  A row that provides no merge records that fact in the output, so a capability gap is a value and never a missing line.

The runs behind one pass share an allocator arena, an address-space layout, a governor ramp and one resident item slice.
No interval over them is honest, so an interval belongs to the **repeat** axis, one whole re-execution in a fresh process, which core consumes and never performs.

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
    type Params: SketchParams;                 // ALGORITHM is read off this, never written here
    const IMPL: &'static str;                  // "oxide", "lib-hip", ...
    const ALGORITHM: &'static str = <Self::Params as SketchParams>::ALGORITHM;
}
pub trait MemoryFootprint {
    fn memory_bytes(&self) -> usize;           // best-effort; a tight upper bound is fine
}
```

`update` takes its item by reference, so a wrapper carrying strings or byte slices is not charged for a clone on the hot path.
A capability trait states the query shape its own statistic wants.
A distinct count takes nothing, a frequency estimate takes a key by reference, a quantile takes a fraction, and a top-k takes `k`.
Those traits are nominal on purpose: a structural bound would also match a type that answers with a stub, and would score it.
A row that also needs a run knob, such as a worker count, takes a different construction trait and keeps its identity.

### 4.2 What a frontend calls

```rust
// The timed half: throughput, latency, the merge fold, plus CPU and memory.
run_cell::<S>(&BenchConfig, &WorkloadSpec, &ParamSet)          -> Result<Vec<BenchReport>, RunError>
run_cell_parallel::<S>(&BenchConfig, &WorkloadSpec, &ParamSet) -> Result<Vec<BenchReport>, RunError>

// The untimed half: accuracy and post-merge accuracy, scored against a comparator.
score_cell::<S, G>(&BenchConfig, &WorkloadSpec, &ParamSet, &G) -> Result<Vec<BenchReport>, RunError>
```

The two halves never meet, because ground truth is expensive: an exact `HashSet`, an exact counter, or a full sort.
The merge pass spans both, sharding the stream, timing only the fold, and scoring the folded result against a single-pass reference.
Each returns one report per pass the mask selected, which is why the return is a `Vec` and why every record carries a `pass` label.
Each proves construction once before any measurement, so a cell that cannot be built fails with a message instead of a partial result.
Underneath sits `BenchRunner`, which takes a workload, a factory closure and an insert closure, and exposes the same split as two methods.
The factory is a closure because independent runs require independent initial state, and sharing one accumulator across runs would correlate every sample.

### 4.3 The instrumentation surface

`MetricsSink` is four hooks, around update and around query, and `Probe<S, Sink>` is the wrapper that calls them.
`MetricsMask` selects what a sink collects and splits into passes; `FullSink` is the offline collector built from a mask, finalising into one `RunMetrics`.
`LatencyRecorder` is reachable on its own, so an embedded path records latency without linking the rest of the machinery.

### 4.4 The parameter surface

A `ParamSet` serialises as `{"algorithm": ..., "params": {...}}`, the exact shape a record's `sketch_config` field carries, and it round-trips.
Recovering a typed value fails for a set of another algorithm, or for a key the params type does not accept, which the error names.
A single point also parses from whitespace-separated `key=value` tokens, one value per key, typed by inspection: `depth=5` a number, `exact=true` a boolean.

### 4.5 The report format

One JSONL line per pass, with `schema_version` at the front of every line, which a reader checks and refuses when it does not recognise it.
`Record` is the whole line, `BenchSection` its measured content, and `RunStats` one metric's mean, stddev, optional interval, and sample count.
The grouping key for anything that pools records is `(sketch, impl, sketch_config, workload, pass)`; pooling two passes averages two experiments.

### 4.6 The build-time surface

Backing the latency recorder with a real histogram is on by default; without it the recorder counts calls and the latency summary carries counts only.
Reading process heap use requires jemalloc, which the feature forces on the linking binary, so it stays opt-in.
Per-sketch allocation accounting compiles in a counting allocator, which the linking binary installs as its global and which counts the whole process.

## 5. Open questions

- **Merge topology.** Contiguous shards folded sequentially measure one point in a space that also holds interleaved shards, hash partitioning, and tree folds.
  Admitting more means new record fields so the numbers stay comparable; fixing one keeps the column simple and measures one arrangement.

- **Who chooses the probe set.** Each comparator picks its own probe count and population, which lets a statistic ask for what it needs.
  Two algorithms' query-throughput numbers then rest on different probe counts.
  A shared probe budget in the configuration would make them comparable, at the cost of a knob that means something different for each statistic.

- **Where per-call query samples belong.** They are an order of magnitude more data than the aggregate, and their consumer is a long-format table.
  Putting them in the record exposes them to every reader and inflates the files that enable them; leaving them out keeps a second format alive.

- **Naming the deferred build.** The method is `prepare` and the metrics it feeds are `finalize_time_ms` and `build_throughput_items_per_sec`, so one of the two names should move.
  Moving the method breaks every implementation; moving the fields is a schema bump coordinated across every producer and reader.

- **Per-thread allocation accounting.** The allocation counters are process-global atomics, so a parallel-insert row's heap numbers describe the process.
  Per-thread accounting needs a thread-local shim plus a rule for which threads belong to the measurement, and the rule is the harder half.
