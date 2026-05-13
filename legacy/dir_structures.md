# SketchBench-Polyglot Project Structure (Current)

.
├── input/                          # Shared Benchmark Data
│   └── benchmark_data_1m_int64.bin # 1M random int64 values (shared across all languages)
│
├── cpp/                            # C++ Root (binaries built here)
│   ├── common/                     # Shared C++ timing + black-box helpers
│   ├── cs/                         # Count Sketch binaries (one per variant)
│   ├── kll/                        # KLL binaries (one per variant)
│   └── run_benchmark.sh            # C++ orchestrator
│
├── rust/                           # Rust Root (workspace)
│   ├── Cargo.toml                  # Workspace manifest
│   ├── bench_common/               # Shared timing + dataset helpers
│   ├── bench_sketches/             # Per-sketch binaries in src/bin
│   └── run_benchmark.sh            # Rust orchestrator
│
├── benchmark_cms/                  # Legacy suite (multi-sketch process)
├── benchmark_hll/                  # Legacy suite (multi-sketch process)
│
├── visualization/                  # Result aggregation + charts
└── README.md                       # Documentation on how to add a new sketch
