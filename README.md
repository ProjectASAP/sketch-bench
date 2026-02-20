# Sketch Benchmarks

Minimal harness for comparing streaming data sketch implementations across Rust and C++. Benchmarks expect `sketch-bench/` to sit beside `Insert-Optimized-Data-Sketches/` and `sketchlib-rust/`, and they all reuse `input/benchmark_data_1m_int64.bin` (auto-generated if missing). Tooling requirements: CMake ≥3.15, a C++17 compiler, and Rust stable.

## Methodology
- Each benchmark loads the shared binary dataset into memory before timing.
- Timings cover only core insert/update loops, repeated for 10 runs.
- All C++ code builds with `-std=c++17 -O3 -march=native -fomit-frame-pointer`; Rust uses release mode with `RUSTFLAGS="-C target-cpu=native"`, `opt-level=3`, `lto=true`, `codegen-units=1`.
- Insert-Optimized C++ variants include headers directly from `~/Insert-Optimized-Data-Sketches/src`, while Apache DataSketches code is pulled via FetchContent.

## Technical Specs
- **Input file**: `input/benchmark_data_1m_int64.bin`, 1M `int64_t` values, seed 42.
- **CMS/Count Sketch params**: width 2048, depth 5, MurmurHash3.
- **HLL params**: `lg_k=14` (k=16384).
- **KLL params**: `k=200`.

## Repository Structure
```
sketch-bench/
├── cpp/                         # C++ benchmarks (CMake + binaries + output/*.jsonl)
│   ├── common/                  # shared timing + helpers
│   ├── cs/                      # Count Sketch variants
│   ├── kll/                     # KLL variants
│   ├── output/                  # JSONL benchmark output
│   └── run_benchmark.sh
├── input/                       # shared data + generator
├── rust/                        # Rust workspace (src/bin + output/*.jsonl)
│   ├── src/
│   ├── output/                  # JSONL benchmark output
│   └── run_benchmark.sh
├── visualization/               # JSON/JSONL viewer (tables + charts)
├── other_benchmark/             # auxiliary experiments / legacy runs
├── insertion_optimized_sample_output/ # saved sample outputs
├── run_all_benchmarks.sh        # top-level runner
└── README.md
```
Benchmark outputs are JSONL files with `{implementation_name, total_nanoseconds}` records under `cpp/output/` and `rust/output/`.

## How to Run
1. Generate/verify input data (done automatically by the scripts).  
2. Execute the benchmark you want (from the repo root):
   ```bash
   ./run_all_benchmarks.sh
   cd cpp  && ./run_benchmark.sh
   cd rust && ./run_benchmark.sh
   ```
   Each script builds with the flags above, runs the binaries, and writes structured output into `output/*.jsonl`.
3. Open `visualization/index.html` via a local server (see `visualization/README.md`) to view tables and charts.

## Benchmarks at a Glance
- `cpp/`: Count Sketch + KLL variants (Insert-Optimized and DataSketches).
- `rust/`: HLL, Count-Min, Count Sketch, Elastic, KLL, UnivMon, Nitro variants.
- `visualization/`: JSON/JSONL loader for charts and tables across all outputs.
