# Count Sketch Throughput

This variant mirrors the `accuracy/cs` structure, but measures insertion throughput instead of error.

Compared implementations:

- Rust `sketchlib-rust`
- C++ AWS insert-optimized Count Sketch

Shared assets:

- dataset: `input/benchmark_data_10m_int64_zipf_s11_k100000.bin`
- scripts: `throughput/scripts/`
- plots: `throughput/plots/cs/`

Fixed benchmark configuration:

- rows: `5`
- cols: `2048`
- input: `10M` Zipf-distributed int64 values
- Rust sketchlib: 10 repeated runs
- C++ insert-optimized: 10 repeated runs

## Run

```bash
INSERT_OPT_ROOT=~/Insert-Optimized-Data-Sketches throughput/scripts/run_throughput_all.sh cs
```

## Outputs

- merged CSV: `throughput/cs/output/cs_throughput_results.csv`
- plot: `throughput/plots/cs/cs_throughput_insertion.png`
