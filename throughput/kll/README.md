# KLL Throughput Benchmark

Measures insertion throughput for KLL quantile sketches across four implementations.

## Implementations

| Label | Language | Library | Notes |
|-------|----------|---------|-------|
| `rust_sketchlib_kll` | Rust | `sketchlib-rust` (local) | Our KLL |
| `rust_oxide_kll` | Rust | `sketch_oxide 0.1.5` | Third-party Rust crate |
| `cpp_datasketches_kll` | C++ | Apache `datasketches-cpp` | Reference C++ implementation |
| `cpp_insert_optimized_kll` | C++ | AWS `Insert-Optimized-Data-Sketches` | Optimized variant from AWS |

## Parameters

- **k**: 200 (default KLL accuracy parameter)
- **Data**: 10M Zipf-distributed `int64` values (s=1.1, support=100k)
- **Runs**: 10 per implementation

## CSV Schema

```
implementation,language,run,k,total_items,total_nanoseconds,throughput_items_per_sec
```

## Running

```bash
# From the throughput/scripts/ directory:
bash run_throughput_all.sh kll
```

This will:
1. Build and run the Rust benchmarks (sketchlib-rust + sketch_oxide)
2. Build and run the C++ benchmarks (DataSketches + Insert-Optimized)
3. Merge results into `kll/output/kll_throughput_results.csv`
4. Generate `plots/kll/kll_throughput_insertion.png`

## Dependencies

- **Rust**: `sketchlib-rust` (local path), `sketch_oxide` (crates.io)
- **C++**: `datasketches-cpp` at `$DATASKETCHES_CPP_ROOT` or `~/datasketches-cpp`
- **C++**: `Insert-Optimized-Data-Sketches` at `$INSERT_OPT_ROOT` or `~/Insert-Optimized-Data-Sketches`
