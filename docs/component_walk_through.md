# Component Walk Through

This is the document for expected components and expected functionality of each component.
Currently it's a rough outline.
More detail will be added during manual code review and refactoring process.

## Overview

The repo is structured as a cargo workspace with following crates:

```
members = [
    "aqpbm-datagen",
    "aqpbm-core",
    "sketch-bench",
    "sketch-runtime",
    "aqpbm-cli",
]
```

### Idea to keep in mind

The benchmark framework should add minimal overhead as well as not optimize away what is being benchmarked.

### Graphic View of structure

Solid arrows are dependencies that exist today, as reported by `cargo tree -e normal`.
Dashed arrows are planned.
An arrow points from a crate to what it depends on.

```mermaid
flowchart TB
        cli["<b>aqpbm-cli</b><br/><i>argument parsing, calling necessary function from other crates, output</i>"]

        sb["<b>sketch-bench</b><br/><i>the sketches themselves: wrappers,<br/>params, exact baselines, the catalog</i>"]
        ab["<b>aqp-bench</b><br/><i>V2 placeholder</i>"]

        core["<b>aqpbm-core</b><br/><i>the framework: traits, runner,<br/>metrics, comparators</i>"]
        dg["<b>aqpbm-datagen</b><br/><i>synthetic data generation</i>"]

    rt["<b>sketch-runtime</b><br/><i>embedded sampler + exporters</i>"]
    app(["downstream applications"])

    cli --> sb
    cli --> core
    cli --> dg
    cli -.-> ab

    sb --> core
    ab -.-> core

    rt --> core
    app -.-> rt

    core --> dg

    classDef future stroke-dasharray: 5 5
    class ab,app future
```

## aqpbm-core

This is where core functionality lives.

**Target.** Candidate functionality is:
- runner/sweeper that performs multiple run of something
- invocation of what to be executed
- warm-up of cpu
- timing functionality
- possibly more

**Today.** The generic benchmark engine lives here:
- `hot_loop`: cpu warm-up + the timed insert loop
- `runner`: `BenchRunner` (the multi-pass runner/sweeper) + `BenchConfig`
- `metrics`: the recorders (wall / CPU / RSS / jemalloc / heap-track / throughput) + `MetricsMask`, which picks the ones a run collects
  - `FullSink` is the collector built from that mask, and it finalises into one `RunMetrics` per run
- `aggregation`: the Welford accumulator (running mean + variance, one pass, O(1) memory) + N-run `aggregate` → `BenchSection`
- `accuracy`: the `GroundTruth` comparator trait + `Comparison`, the per-statistic capability traits (`CardinalityOps`, `FrequencyOps`, `QuantileOps`, `TopKOps`) and the comparators
  - a comparator computes the exact answer from the whole item slice at once, so it is not an `Accumulator`
- `init`: `InitSketch` (build one from a `ParamSet`) / `BenchImpl` (a row's identity — its impl name, and the family it belongs to)
- `cell`: `run_cell` / `score_cell` / `run_cell_parallel`, `BenchItem` / `ParallelInit` traits, `WorkloadSpec`, `AccuracyCfg`, `RunError`
  - consider the data table where each cell is the result of one benchmark
- `accumulator`: the trait a sketch implements about how the sketch ingests one item, and optionally how it merges and finalises (finalises is none for most sketches)
- `memory_footprint`, `latency`, `probe`, `workload`, `report`, `config` (params)

Together these make this crate the whole framework: implement its traits on your own sketch and the runners will drive, score and report it.

It also re-exports the generator's public types, so a consumer that needs a `GenSpec` does not have to depend on `aqpbm-datagen` separately.

## aqpbm-datagen

Crate for input data generation.
It defines three orthogonal axes:
- `shape`: the structure — *what* the values mean (keys / categorical / monotonic series)
- `dist`: the distribution — *how* they are spread (uniform / zipf / …)
- `sink`: the destination — where they go (`MemorySink`, `BinSink`)

Data are generated in memory or streamed to disk, chunk by chunk.
On-disk output is a header-less little-endian `.bin` stream plus a `.meta.json` provenance sidecar (`io`, `stats`).

This crate depends on nothing else in the workspace.
`input/` still exists as a directory of pre-generated data files and their C++ generator.

## aqpbm-cli

A common front-end that process input including command line argument and preferrably config file.

**Target.** The only functionality of this crate is to invoke corresponding functionality of `sketch-bench` or `aqp-bench` or `other-bench`.

**Today.** Cleaned up.
- `cli`: parse the command-line argument
- call corresponding function in `sketch-bench`
- `workload_cmd`: operate datagen for workload passed to benchmark (`sketchlib workload generate|describe`)
- `raw_csv`: write output received from `sketch-bench`
- `repeat`: re-execute the whole benchmark in N separate processes, and report the spread across their means

## sketch-bench

Where sketch-primitive related instance is defined.
This crate connects to `aqpbm-cli` such that user can use this benchmark.

**Target.** Wrappers, sketch definition, and the measurement that is specific to this domain.

**Today.** A bundle of implementations:
- `wrappers/`: the wrapped sketch implementations under test
- `catalog`: which `(family, impl)` pairs exist and how to run one
- `params`: per-family construction parameters, and where the family names are declared
- `legacy_csv`: the domain-specific long-format CSV rendering

Note: A *family* is the algorithm (`hll`, `cms`, `kll`) and an *impl* is one library's version of it (`oxide`, `datasketches`, `polars`, `lib-hip`), so `(hll, oxide)` names one benchmarkable row.

Family names are written in exactly two places: the `FAMILY` consts in `params`, and `legacy_csv`'s per-family header table.
The second is pinned rather than derived, because plot scripts read those columns positionally and renaming one silently shifts every later column.
Everywhere else the family comes from the type: a row's `BenchImpl` derives it from `Params::FAMILY`, so `"hll"` is written once and `catalog` spells no family name at all.

**Benchmarking a sketch of your own means depending on `aqpbm-core`, not on this crate**: you implement its traits and call `cell::run_cell`.

This crate exists for the CLI.
`sketchlib bench --sketch hll --impl oxide` arrives holding two strings, and `catalog` is what turns them into a concrete Rust type.
That is the only reason a list of implementations exists at all — and it is also what lets a future `aqp-bench` sit parallel to `sketch-bench` on the same core, each shipping its own catalog.

## sketch-runtime

**Discussion needed**
This crate is expected to use functionalities provided by `aqpbm-core`.
Anything here is adjustable.
Remove and rewrite of this crate is also an option.

This crate contains downstream application connection.
This will be adjusted later with more detail about how downstream application is expected to be connected.
The position of this crate may be changed: whether it depends on sketch-bench or is parallel to sketch-bench is unclear at this moment. Today it is parallel: it depends on `aqpbm-core` only.

**Today.**
- `sampler`: the embedded `MetricsSink` a downstream app hands to a `Probe<S, Sampler>`, with count- and time-based sampling and emit windows
- `exporter`: where the sampled records go — `stdout`, `file`, `grpc`, `noop`, `fanout`
- `switch`: a runtime toggle the controller can flip without restarting the host

Sampling has three independent off-switches: compile-time (`default-features = false`), construction-time (`Sampler::disabled`), and runtime (`RuntimeSwitch`).

## aqp-bench (placeholder)

This part is postponed to V2, even name may be changed to "xxx-bench" (where xxx means something else).
It is listed here to indicate structure: this is a component parallel to sketch-bench and how it will connect to `aqpbm-cli` and backboned by `aqpbm-core`.

## Other language

This part is not started yet.
It is reasonable to have benchmark in other language.
Yet, not started.

## Terminology

"Name change?" is about the *name*, not the thing.
**No** means settled.
**Likely** means the name already disagrees with something else in the tree, and one of the two should move.
**Open** means the component itself is still under discussion.
Module names not listed below (`accumulator`, `accuracy`, `aggregation`, `cell`, `config`, `init`, `metrics`, `probe`, `runner`) are named after the trait or type they hold, so the row for that trait covers them.

| Terminology | Meaning | Name change? |
|---|---|---|
| `aqpbm-core` | The framework: traits, runner, metrics, comparators. Names no family. | No |
| `aqpbm-datagen` | The generator: shape × dist × sink. Depends on nothing else in the workspace. | No |
| `aqpbm-cli` | The `sketchlib` binary — parse argv, call `sketch-bench`, write output. | No |
| `sketch-bench` | The bundle of wrapped implementations, plus the catalog that names them. | maybe? |
| `sketch-runtime` | Embedded sampler + exporters for downstream apps. | Open — the crate itself is under discussion |
| `aqp-bench` | V2 placeholder: a second bundle sitting parallel to `sketch-bench`. | Open — stated above as "may be changed" |
| `sketchlib` | The CLI binary name. | Likely — same "sketch" question as the crates |
| family | The algorithm: `hll`, `cms`, `countsketch`, `kll`, `dd`, `topk`, `elastic`, `nitro`, `univmon`. | likely |
| impl | One library's version of a family: `oxide`, `datasketches`, `polars`, `lib`, `lib-hip`, `lib-fixedmatrix-fast`, … | likely |
| row | One `(family, impl)` pair — the unit that can be benchmarked. | No |
| cell | One row measured against one workload at one config. What `run_cell` runs. | likely |
| run | One measured iteration inside a cell (`--runs N`), all in the same process. | likely |
| repeat | One whole re-execution in a fresh process (`--repeats R`). | likely |
| pass | One metric group measured over its own sketch population: throughput, latency, accuracy, merge. | likely |
| `Accumulator` | The trait a sketch implements: ingest one item, optionally merge and `prepare`. | No |
| `InitSketch` | Build one from a `ParamSet`, or say why not. | likely |
| `BenchImpl` | A row's identity: its `IMPL` name plus its `Params` type, from which `FAMILY` is derived. | likely |
| `ParallelInit` | Same, for rows that need the worker count and not just a `ParamSet`. | likely |
| `BenchItem` | How a sketch's item type materialises a workload. | likely |
| `CardinalityOps` / `FrequencyOps` / `QuantileOps` / `TopKOps` | Capability traits — which statistic a row can answer, and therefore which comparator applies. | No |
| `SketchParams` | The trait a per-family params struct implements; carries `const FAMILY`. | likely |
| `ParamSet` | A family name plus its params as JSON. The type-erased form that keeps the family axis open. | likely |
| `FAMILY` | The const on a params struct. One of the two places a family name is written. | likely |
| `hot_loop` / `insert_loop` | The single timed insert loop. Nothing else is inside the timed region. | likelly |
| `BenchRunner` | Drives warm-up plus measured iterations, and emits one report per pass. | likely |
| `BenchConfig` | Knobs for a run: runs, warm-up runs, metrics mask, merge shards, threads, seed. | likely |
| `run_cell` / `score_cell` / `run_cell_parallel` | The timed half, the accuracy half, and the threaded variant. | likely |
| `WorkloadSpec` | Where a cell's items come from: generated in-process, or loaded from a file. | likely |
| `AccuracyCfg` | The accuracy knobs the frontend fills in: on/off, probe cap, per-call recording. | No |
| `RunError` | Why a cell could not run — a config an impl cannot build at, or a width it cannot take. | No |
| `prepare()` | Deferred build after the last `update` — a polars sort, a KLL CDF. No-op for most sketches. | **Likely** — the method is `prepare`, the metric it feeds is `finalize_*`; the two should agree |
| `Probe` | The decorator that reports per-update events to a `MetricsSink`. | likely |
| `MetricsSink` | What receives those events: `FullSink` offline, `Sampler` embedded, `NoopSink` when off. | likely |
| `MetricsMask` | Which metric families a run collects. | likely |
| `FullSink` | The collector built from that mask; finalises into one `RunMetrics`. | likely |
| `RunMetrics` | What one run measured. | likely |
| `Welford` | Running mean + variance, one pass, O(1) memory. | likely |
| `aggregate` | Folds N `RunMetrics` into one `BenchSection`. | likely |
| `BenchSection` | The aggregated numbers inside a report record. | likely |
| `Record` / `report` | One JSONL line: the row, its config, its workload, and its `BenchSection`. | likely |
| `latency` / `LatencyRecorder` | Per-update latency histogram, `hdrhistogram`-backed, a no-op shim when the feature is off. | No |
| `memory_footprint` / `MemoryFootprint` | A sketch's param-derived logical size, reported beside the measured heap. | No |
| `GroundTruth` | A comparator: computes the exact answer from the whole item slice and scores a sketch against it. | No |
| `Comparison` | What a comparator returns — named scalars plus query timing. | likely |
| `shape` | Structure — what the generated values mean (keys / categorical / monotonic). | likely |
| `dist` | Distribution — how they are spread (uniform / zipf / …). | likely |
| `sink` | Destination — `MemorySink` (resident) or `BinSink` (streamed to `.bin`). | No |
| `GenSpec` | One reproducible generation: shape, size, seed. | likely |
| `workload` | The materialised item stream plus its provenance, ready to be replayed per run. | likely |
| `io` / `stats` | Datagen's `.bin` writer + `.meta.json` sidecar, and the column summary inside it. | likely |
| `cli` / `workload_cmd` / `raw_csv` / `repeat` | The `aqpbm-cli` modules: argv parsing, `workload generate\|describe`, the CSV sink, the multi-process re-runner. | No |
| `wrappers/` | The wrapped implementations under test. | No |
| `catalog` | The list of rows and how to run one. Spells no family name itself. | likely |
| `params` | Per-family construction parameters, and where family names are declared. | likely |
| `legacy_csv` | The long-format CSV the plot scripts read. | **Likely** — "legacy" names a format that is still the live output |
| `sampler` | The embedded `MetricsSink` a downstream app hands to a `Probe`. | Open — with the crate |
| `exporter` | Where sampled records go: `stdout`, `file`, `grpc`, `noop`, `fanout`. | Open — with the crate |
| `switch` / `RuntimeSwitch` | A runtime toggle the controller can flip without restarting the host. | Open — with the crate |
