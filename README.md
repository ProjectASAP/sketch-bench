# `sketchlib-tool` (repo: `sketchlib-bench`)

> Status: **Phase 2–4 + 7-lite landed**. Workspace + `sketch-core` + `sketch-bench` + unified `sketchlib` CLI cover every one of the repo's 21 Rust sketch impls end-to-end against the v1 JSONL schema. `sketch-profile` (perf_event/cachegrind/VTune), `sketch-runtime` (embedded sampler), and the C++ binary migration are tracked in [`TODO.md`](TODO.md). See [`docs/DESIGN.md`](docs/DESIGN.md) and [`docs/MERGE_PLAN.md`](docs/MERGE_PLAN.md) for the full contract.

## Quick start

```
cargo run -p sketch-cli --release -- list-impls

# default: sweep all impls of a family across the family's default config grid
cargo run -p sketch-cli --release -- bench \
    --sketch hll \
    --workload zipf --size 1000000 --zipf-s 1.1 \
    --runs 10 --warmup-runs 3 \
    --metrics throughput,latency,cpu,memory \
    --report out.jsonl

# single impl, single config — same as the pre-sweep behaviour
cargo run -p sketch-cli --release -- bench \
    --sketch hll --impl oxide --config 'lg_k=14' \
    --workload zipf --size 1000000 --zipf-s 1.1 --runs 10 --report out.jsonl

# explicit grid: Cartesian product across whitespace-separated keys
cargo run -p sketch-cli --release -- bench \
    --sketch cms --config 'rows=3,5 cols=1024,2048,4096' \
    --workload zipf --size 1000000 --runs 10 --report out.jsonl
```

`list-impls` enumerates every `(family, impl)` pair. `bench` monomorphises a `BenchRunner` over each `(impl, config)` pair in the sweep and appends one v1 JSONL record per pair (schema in `sketch-core::report::Record`, includes an optional `sketch_config` field that names the params used). Impls with compile-time-fixed shapes are skipped when the requested config doesn't match; stderr logs the skip. See [`docs/BENCH_SWEEP.md`](docs/BENCH_SWEEP.md) for the full contract and the per-family default grids.

Covered families / impls (21 total):

| family | implementations |
|---|---|
| `hll` | `oxide`, `datasketches`, `lib` (asap_sketchlib) |
| `kll` | `oxide`, `lib` |
| `cms` | `oxide`, `datasketches`, `lib-{fixedmatrix-custom-fast,fixedmatrix-fast,vector2d-fast,vector2d-regular}` |
| `countsketch` | `oxide`, `lib-{fixedmatrix-fast,vector2d-fast,vector2d-regular}` |
| `elastic` | `oxide`, `lib` |
| `nitro` | `oxide`, `lib` |
| `univmon` | `oxide`, `lib` |

`sketchlib-tool` is:

1. A **CLI** (`sketchlib`) for offline benchmarking and profiling of streaming-data sketch implementations across Rust and C++.
2. An **embeddable library** used by ASAP applications (asap-fusion, DataCollector, ASAPQuery) to report live sketch metrics to the ASAPController control plane.

## Benchmark vs. profile

The two concepts are kept strictly separate throughout the code, CLI, and report schema:

| | Benchmark (macro) | Profile (micro, VTune-style) |
|---|---|---|
| **What** | End-to-end behavior on a workload | Microarchitectural + allocation behavior |
| **Metrics** | Throughput, latency p50/p95/p99, CPU time, RSS, heap peak, accuracy, 95% CI over N runs | L1/L2/LLC miss rates, branch mispredicts, TLB misses, IPC, cache-simulation, allocation hotspots, flamegraphs |
| **Overhead** | Low — safe for always-on embedding | Higher — CLI-only |
| **CLI** | `sketchlib bench …` | `sketchlib profile …` |
| **Crate** | `sketch-bench` | `sketch-profile` |

Both write the same JSONL schema (see [`docs/DESIGN.md §4.4`](docs/DESIGN.md)) so the visualization layer consumes either.

## Target crate layout (post-merge)

```
sketchlib-bench/
├── sketch-core/            # shared: workload, Sketch trait, Probe decorator, report schema
├── sketch-bench/           # macro metrics (throughput, latency, CPU, memory, accuracy, CI)
├── sketch-profile/         # micro metrics (hw counters, perf, cachegrind, heaptrack, vtune)
├── sketch-runtime/         # embedded sampler + exporters (stdout, prometheus, grpc)
├── sketch-cli/             # unified `sketchlib` binary
├── cpp/ rust/              # existing benches, migrating onto sketch-core/bench
├── accuracy/ throughput/   # existing accuracy/throughput harnesses
├── input/ scripts/         # shared datasets + generators
├── visualization/          # JSON/JSONL viewer (tables + charts)
└── docs/                   # DESIGN.md, MERGE_PLAN.md
```

Downstream apps depend on `sketch-core + sketch-bench + sketch-runtime` — not on `sketch-profile`, which is CLI-only.

## Documents

- [`docs/DESIGN.md`](docs/DESIGN.md) — goals, audiences, crate layout, APIs, report schema, runtime → controller feedback loop.
- [`docs/MERGE_PLAN.md`](docs/MERGE_PLAN.md) — phased absorption of `sketch-profiler`, history preservation, risks, done criteria.

## Current behavior (pre-migration)

The legacy harness still works as before while the migration proceeds. Everything below describes the existing repo; over time it will be superseded by `sketchlib bench` / `sketchlib profile` subcommands.

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
   ./run_all_benchmarks.sh
   cd cpp  && ./run_benchmark.sh
   cd rust && ./run_benchmark.sh
   ```
   Each script builds with the flags above, runs the binaries, and writes structured output into `output/*.jsonl`.
3. Open `visualization/index.html` via a local server (see `visualization/README.md`) to view tables and charts.

### Benchmarks at a glance

- `cpp/`: Count Sketch + KLL variants (Insert-Optimized and DataSketches).
- `rust/`: HLL, Count-Min, Count Sketch, Elastic, KLL, UnivMon, Nitro variants.
- `visualization/`: JSON/JSONL loader for charts and tables across all outputs.

### Build prerequisites

CMake ≥3.15, a C++17 compiler, Rust stable. This repo expects `sketch-bench/` to sit beside `Insert-Optimized-Data-Sketches/` and `sketchlib-rust/`.

## Contributing while the migration is in flight

- New benchmarks: land them under the existing `rust/` or `cpp/` trees for now; they'll be re-homed onto `sketch-bench` in Phase 8 of the merge plan.
- New metrics: add under `sketch-bench/metrics/` (macro) or `sketch-profile/hw_counters/` (micro), once those crates exist (Phase 2).
- Runtime integration in downstream apps: follow Phase 9 of [`docs/MERGE_PLAN.md`](docs/MERGE_PLAN.md).
