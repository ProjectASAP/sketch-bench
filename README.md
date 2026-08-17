# Approximate Benchmark

<!-- To be Filled...

Current [input and output](https://github.com/ProjectASAP/sketch-bench/tree/examples-operation-metric/example) about the executable is located at a different branch from main branch. -->

## Quickstart

### Compile

At root directory of this project, just run:

```sh
% cargo build --release
```

### Example: HyperLogLog insertion throughput

```sh
% ./target/release/approxbench sketchbench \
      --algorithm hll --impl lib --config 'lg_k=14' \
      --dataset zipf --size 1000000 --zipf-s 1.1 --cardinality 100000 \
      --runs 10 --warmup-runs 3 \
      --operations insert --metrics throughput
approxbench: hll/lib config={"lg_k":14} runs=10 warmup=3
{"schema_version":3,"sketch":"hll","family":"hll","impl":"lib","language":"rust","sketch_config":{"algorithm":"hll","params":{"lg_k":14}},"workload":{"shape":"zipf","size":1000000,"cardinality":100000,"zipf_s":1.1,"seed":42},"mode":"bench","runs":10,"bench":{"metric":"throughput","operation":"insert","throughput_items_per_sec":{"mean":694468886.4950684,"stddev":3084050.9269289766,"n":10},"throughput_samples":[695289414.2186685,697695024.9460857,692321325.7122601,695894224.0779402,699361832.3280008,689377519.6748344,695732999.9380797,694163404.6771342,690428928.972124,694424190.4055575],"wall_time_ms":{"mean":1.4399749,"stddev":0.006400518945106468,"n":10},"memory_bytes":16384},"source":"cli","timestamp":"2026-08-17T09:11:20.208786Z"}

```

#### Explanaition

This example is testing HyperLogLog insertion throughput.
HyperLogLog instance comes from `asap_sketchlib`, with configuration `lg_k=14` (in short, register size is `2^14`).
Input data has zipf distribution.
Warmup the benchmark with 3 runs, and benchmark is run for 10 times.
Data is in field `throughput_items_per_sec`.

### Example: Hydra merge throughput

```sh
% ./target/release/approxbench sketchbench \
    --algorithm hydra-cms --impl lib \
    --spec configs/datagen/hydra_columns.yaml \
    --config "rows=3 cols=1024 cell_rows=3 cell_cols=1024" \
    --operations insert,merge --metrics throughput,cpu,memory \
    --merge-shards 8 --runs 5 --warmup-runs 2 --flat
approxbench: hydra-cms/lib config={"cell_cols":1024,"cell_rows":3,"cols":1024,"rows":3} runs=5 warmup=2
{"schema_version":3,"sketch":"hydra-cms","impl":"lib","language":"rust","mode":"bench","runs":5,"source":"cli","sketch_config":{"algorithm":"hydra-cms","params":{"cell_cols":1024,"cell_rows":3,"cols":1024,"rows":3}},"workload":{"shape":"columns","size":200000,"seed":1,"spec":{"column_label":["key1","key2","value"],"column_num":3,"column_spec":[{"data_type":"string","distribution":{"kind":"uniform","lower_bound":0.0,"seed":1,"upper_bound":200.0},"special_rule":0},{"data_type":"string","distribution":{"kind":"zipf","population_size":50,"seed":2,"skewness":1.1},"special_rule":0},{"data_type":"i64","distribution":{"kind":"zipf","population_size":1000,"seed":3,"skewness":1.2},"special_rule":0}],"row_num":200000}},"memory_bytes":38437088,"heap_bytes_net":null,"heap_bytes_peak":null,"insert_timestamp":"2026-08-17T09:08:16.354935Z","insert_throughput_items_per_sec":{"mean":3146644.494778336,"stddev":661990.5995087331,"n":5},"insert_throughput_samples":[3548652.8107388476,2007851.5428301138,3160803.907486936,3379170.3714341833,3636743.841401601],"insert_build_throughput_items_per_sec":null,"insert_latency_ns":null,"insert_cpu_time_ms":{"user_ms":{"mean":65.00460000000001,"stddev":12.732758353946721,"n":5},"sys_ms":{"mean":0.9918,"stddev":1.0513268759049204,"n":5}},"insert_wall_time_ms":{"mean":66.6847582,"stddev":18.67578157952414,"n":5},"insert_rss_peak_kb":null,"insert_heap_allocated_kb":18221,"query_timestamp":null,"query_throughput_items_per_sec":null,"query_latency_ns":null,"query_accuracy":null,"query_cpu_time_ms":null,"query_wall_time_ms":null,"query_rss_peak_kb":null,"query_heap_allocated_kb":null,"merge_timestamp":"2026-08-17T09:08:16.354940Z","merge_time_ms":{"mean":12.1078084,"stddev":0.2835344216392432,"n":5},"merge_folds_per_sec":{"mean":578.3938365703082,"stddev":13.58991397643068,"n":5},"merge_shards":8,"merge_supported":true,"merge_cpu_time_ms":{"user_ms":{"mean":95.2826,"stddev":2.1805422032146056,"n":5},"sys_ms":{"mean":17.5492,"stddev":2.0397143672583176,"n":5}},"merge_wall_time_ms":{"mean":12.1078084,"stddev":0.2835344216392432,"n":5},"merge_rss_peak_kb":null,"merge_heap_allocated_kb":18268,"prepare_timestamp":null,"prepare_finalize_time_ms":null,"prepare_cpu_time_ms":null,"prepare_wall_time_ms":null,"prepare_rss_peak_kb":null,"prepare_heap_allocated_kb":null}
```

#### Explanation

This example is testing Hydra-over-Count-Min insert throughput and merge cost.
Hydra instance comes from `asap_sketchlib`, with configuration `rows=3 cols=1024` (the outer sketch) and `cell_rows=3 cell_cols=1024` (the inner Count-Min sketch), which takes about 38.4 MB.
Input data is a 200k-row, 3-column table: two label columns (uniform, then zipf) and an i64 value column (zipf)
Warmup the benchmark with 2 runs, and benchmark is run for 5 times.
`merge` folds 8 shards into one.
Data is in field `insert_throughput_items_per_sec` and `merge_time_ms` / `merge_folds_per_sec` (merge/sec).

## Reference

[Overview](./docs/component_walk_through.md#graphic-view-of-structure)

[Core crate](./docs/aqpbm-core.md)

[CLI](./docs/aqpbm-cli.md)

[Data Generation](./docs/aqpbm-datagen.md)

[Sketch benchmark wrapper](./docs/sketch-bench.md)

<!-- # `sketchlib-tool` (repo: `sketch-bench`)

> Status: **Phase 2–4 + 7-lite landed**. Workspace + `sketch-core` + `sketch-bench` + unified `approxbench` CLI cover every one of the repo's 21 Rust sketch impls end-to-end against the v1 JSONL schema. `sketch-profile` (perf_event/cachegrind/VTune), `sketch-runtime` (embedded sampler), and the C++ binary migration are tracked in [`TODO.md`](TODO.md). See [`docs/DESIGN.md`](docs/DESIGN.md) and [`docs/MERGE_PLAN.md`](docs/MERGE_PLAN.md) for the full contract.

## Quick start

A measurement is named by two things: the **operation** it is taken over
(`insert`, `query`, `merge`, `prepare`) and the **metric** it reads
(`throughput`, `latency`, `accuracy`). Crossing `--operations` with `--metrics`
picks the squares a run measures, and both are required: nothing is measured
that was not asked for. `cpu` and `memory` are readings that ride along with
every square instead of forming their own.

```
cargo run -p aqpbm-cli --release -- sketchbench --list-impls

# Insert, read two ways, with cpu and memory riding along
cargo run -p aqpbm-cli --release -- sketchbench \
    --algorithm hll --impl oxide --config 'lg_k=14' \
    --workload zipf --size 1000000 --zipf-s 1.1 --cardinality 100000 \
    --runs 10 --warmup-runs 3 \
    --operations insert --metrics throughput,latency,cpu,memory \
    --report out.jsonl

# Accuracy is scored against ground truth, and the comparator issues the
# queries, so the operation is `query`. `--list-impls` names each row's
# comparators; omitted takes the row's default.
cargo run -p aqpbm-cli --release -- sketchbench \
    --algorithm cms --impl oxide --config 'rows=5 cols=4096' \
    --workload zipf --size 1000000 --cardinality 100000 \
    --runs 10 --operations query --metrics accuracy --comparator frequency \
    --report out.jsonl

# Merge reads the same fold two ways: how long one takes, and how many a second
cargo run -p aqpbm-cli --release -- sketchbench \
    --algorithm cms --impl oxide --config 'rows=5 cols=4096' \
    --workload zipf --size 1000000 --cardinality 100000 \
    --runs 10 --operations merge --metrics latency,throughput --merge-shards 8 \
    --report out.jsonl

# One record per square is the default. `--flat` folds a cell's records into
# one row instead: one slot per operation, one field per metric.
cargo run -p aqpbm-cli --release -- sketchbench \
    --algorithm hll --impl oxide --config 'lg_k=14' \
    --workload zipf --size 1000000 --cardinality 100000 \
    --runs 10 --operations insert,prepare --metrics latency --flat \
    --report out.jsonl
```

`--config` names one point, so a series is one invocation per point. A square
nothing measures is refused by name: `--operations prepare --metrics
throughput` is an error, not an empty result.

`--list-impls` enumerates every `(algorithm, impl)` pair. `sketchbench` monomorphises a `BenchRunner` over the one `(impl, config)` cell named and appends one JSONL record per square measured (schema in `aqpbm_core::report::Record`, including a `sketch_config` field that names the params used and the `operation` / `metric` pair that names the square). Impls with compile-time-fixed shapes refuse a config that doesn't match. See [`docs/BENCH_SWEEP.md`](docs/BENCH_SWEEP.md) for the full contract and the per-algorithm default grids.

Covered algorithms / impls (21 sketch + 3 exact = 24 total):

| algorithm | implementations |
|---|---|
| `hll` | `oxide`, `datasketches`, `lib` (asap_sketchlib), `exact` |
| `kll` | `oxide`, `lib`, `exact` |
| `cms`, `cms-fastpath-{fixedmatrix,vector2d}`, `cms-regularpath-vector2d` | `oxide`, `datasketches`, `lib`, `polars` |
| `countsketch`, `countsketch-fastpath-{fixedmatrix,vector2d}`, `countsketch-regularpath-vector2d` | `oxide`, `lib`, `polars` |
| `elastic` | `oxide`, `lib` |
| `nitro` | `oxide`, `lib` |
| `univmon` | `oxide`, `lib` |

The `exact` impl per (hll, kll, cms) algorithm is a zero-error baseline
that implements the same `Sketch` trait, so it benches through the
identical insert / query / accuracy pipeline. It exists to give a
side-by-side throughput / CPU / memory reference point and to
sanity-check the ground-truth wiring. The dispatch marks it
`Unparameterized`, so sweeps run it once regardless of grid size.

The ground-truth comparators live under `aqpbm-core/src/accuracy/`, organised by **statistic** and not by sketch algorithm, so a single exact algorithm serves every sketch that answers the same question:

| module             | statistic    | exact algorithm        | sketches that share this baseline |
|--------------------|--------------|------------------------|-----------------------------------|
| `cardinality.rs`   | cardinality  | `HashSet<i64>` + `len` | `hll`                             |
| `frequency.rs`     | frequency    | `HashMap<i64, u64>`    | `cms`, `countsketch`, `elastic`   |
| `quantile.rs`      | quantile     | sorted `Vec<i64>`      | `kll`, `dd` (DDSketch)            |

There is no algorithm to statistic lookup function.
A wrapper implements the capability traits in `aqpbm_core::accuracy::statistic` for the statistics it answers, and its row in `sketch_bench::catalog` names the comparator that scores it.

`sketchlib-tool` is:

1. A **CLI** (`approxbench`) for offline benchmarking and profiling of streaming-data sketch implementations across Rust and C++.
2. An **embeddable library** used by ASAP applications (asap-fusion, DataCollector, ASAPQuery) to report live sketch metrics to the ASAPController control plane.

## Benchmark vs. profile

The two concepts are kept strictly separate throughout the code, CLI, and report schema:

| | Benchmark (macro) | Profile (micro, VTune-style) |
|---|---|---|
| **What** | End-to-end behavior on a workload | Microarchitectural + allocation behavior |
| **Metrics** | Throughput, latency p50/p95/p99, CPU time, RSS, heap peak, accuracy, 95% CI over N runs | L1/L2/LLC miss rates, branch mispredicts, TLB misses, IPC, cache-simulation, allocation hotspots, flamegraphs |
| **Overhead** | Low — safe for always-on embedding | Higher — CLI-only |
| **CLI** | `approxbench sketchbench …` | not built yet |
| **Crate** | `sketch-bench` | `sketch-profile` |

The profile column is a plan, not a shipped feature.
`approxbench` today has two subcommands, `sketchbench` and `workload`, and there is no profiling subcommand or `sketch-profile` crate yet.

Both are specified to write the same JSONL schema (see [`docs/DESIGN.md §4.4`](docs/DESIGN.md)) so the visualization layer consumes either.

## Target crate layout (post-merge)

```
sketch-bench/
├── sketch-core/            # shared: workload, Sketch trait, Probe decorator, report schema
├── sketch-bench/           # macro metrics (throughput, latency, CPU, memory, accuracy, CI)
├── sketch-profile/         # micro metrics (hw counters, perf, cachegrind, heaptrack, vtune)
├── sketch-runtime/         # embedded sampler + exporters (stdout, prometheus, grpc)
├── aqpbm-cli/              # unified `approxbench` binary
├── cpp-bench/              # C++ track of sketch-bench (v1 JSONL via cpp-bench/common/)
├── input/ scripts/         # shared datasets + top-level orchestrators
├── visualization/          # JSON/JSONL viewer (tables + charts) + per-algorithm
│                           # matplotlib plot scripts under visualization/plots/
└── docs/                   # DESIGN.md, MERGE_PLAN.md
```

Downstream apps depend on `sketch-core + sketch-runtime` — not on `sketch-bench` (the offline benchmark library) and not on `sketch-profile` (CLI-only).

## Documents

- [`docs/DESIGN.md`](docs/DESIGN.md) — goals, audiences, crate layout, APIs, report schema, runtime → controller feedback loop.
- [`docs/MERGE_PLAN.md`](docs/MERGE_PLAN.md) — phased absorption of `sketch-profiler`, history preservation, risks, done criteria.

## Current behavior (pre-migration)

The legacy harness still works as before while the migration proceeds.
Everything below describes the existing repo; over time it will be superseded by the `approxbench sketchbench` subcommand.
The profiling subcommand it is meant to sit beside is not built yet.

### Methodology

- Each benchmark loads the shared binary dataset into memory before timing.
- Timings cover only core insert/update loops, repeated for 10 runs.
- C++ builds with `-std=c++17 -O3 -march=native -fomit-frame-pointer`; Rust uses release mode with `RUSTFLAGS="-C target-cpu=native"`, `opt-level=3`, `lto=true`, `codegen-units=1`.
- Insert-Optimized C++ variants include headers directly from `~/Insert-Optimized-Data-Sketches/src`; Apache DataSketches is pulled via FetchContent.

### Technical specs

- **Input file**: `input/benchmark_data_1m_int64.bin`, 1M `int64_t` values, seed 42.
- **CMS / Count Sketch params**: width 2048, depth 5, MurmurHash3.
- **HLL params**: `lg_k=14` (k=16384).
- **KLL params**: `k=200`.

### How to run

1. Generate/verify input data (done automatically by the scripts).
2. Execute the benchmark you want, from the repo root:
   ```bash
   scripts/example_config_override.py   # guided tour of --config, run this first
   scripts/run_throughput.sh            # all algorithms incl. octo + polars
   scripts/run_accuracy.sh              # all statistics, scored against ground truth
   scripts/run_all.py --workload-file …  # joint Rust + C++ run via cpp-bench/
   ```

   Both scripts wrap `approxbench sketchbench --raw-csv DIR` and dump
   long-format CSVs into `output/throughput/` and `output/accuracy/`
   respectively. The per-algorithm Rust harnesses that used to live
   under `throughput/<algorithm>/rust/` and `accuracy/<statistic>/rust/`
   have been retired in favour of `aqpbm-cli` (recoverable from
   git history). The per-algorithm C++ harnesses have likewise been
   ported onto `cpp-bench/common/` (`cpp-bench/{hll,cms,cs,kll}/`).
3. C++ benchmarks build through one top-level CMake project:
   ```bash
   cmake -S cpp-bench -B cpp-bench/build
   cmake --build cpp-bench/build
   ./cpp-bench/build/{hll,cms,cs,kll}/<impl>_<algorithm> --workload ... --report-path ...
   ```
   `scripts/run_all.py` orchestrates both tracks (Rust via `approxbench sketchbench`, C++ via the binaries above) into a single v1-JSONL report.
4. Open `visualization/index.html` via a local server (see `visualization/README.md`) to view tables and charts.

### Benchmarks at a glance

- `cpp-bench/`: HLL / CMS / Count Sketch / KLL — the Apache DataSketches
  baseline plus the Insert-Optimized "final" variant for every algorithm, and
  the full CS/KLL optimization-evolution series (naive → fastrange →
  fixed_size → final / naive → cached_level_capacities → no_min_max →
  no_self_move_protection → pcg_random → final). All emit v1 JSONL.
- `aqpbm-cli/`: unified `approxbench sketchbench` — every Rust impl + the polars
  exact baselines + the `*-parallel/lib` (octo) rows. The
  per-algorithm `throughput/<algorithm>/rust/` and `accuracy/<statistic>/rust/`
  trees have been retired into git history.
- `scripts/example_config_override.py`: a runnable tour of the `--config`
  surface, showing what each family's knobs do, that they reach the sketch,
  and what a row says when it cannot build at the point it was given.
- `scripts/run_throughput.sh`, `scripts/run_accuracy.sh`: orchestrators
  that fan `approxbench sketchbench` over all algorithms and dump CSVs the legacy
  plot scripts (now under `visualization/plots/throughput/` and
  `visualization/plots/accuracy/`) still consume unchanged.
- `visualization/`: JSON/JSONL loader for charts and tables across all
  outputs, plus per-algorithm matplotlib `plot_*.py` scripts under
  `visualization/plots/{throughput,accuracy}/`.

### Build prerequisites

CMake ≥3.15, a C++17 compiler, Rust stable. This repo expects `sketch-bench/` to sit beside `Insert-Optimized-Data-Sketches/` and `sketchlib-rust/`.

## Contributing while the migration is in flight

- New Rust impls: add a wrapper under `sketch-cli/src/wrappers/<algorithm>.rs`,
  register it in `sketch-cli/src/dispatch.rs` (one `IMPLS` row + one macro
  invocation), pick an `AccuracyKind`. The old per-algorithm `rust/` trees are
  gone; everything new flows through `approxbench sketchbench`.
- New C++ benches: land them under `cpp-bench/<algorithm>/` on the new
  v1-JSONL framework (`cpp-bench/common/`); follow `cpp-bench/kll/` or
  `cpp-bench/cs/` as templates.
- New metrics: add under `sketch-bench/metrics/` (macro) or
  `sketch-profile/hw_counters/` (micro), once those crates exist (Phase 2).
- Runtime integration in downstream apps: follow Phase 9 of
  [`docs/MERGE_PLAN.md`](docs/MERGE_PLAN.md). -->
