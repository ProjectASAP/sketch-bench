# CS Accuracy Comparison Framework

This variant mirrors `accuracy/cms`, but the Rust implementation is `sketchlib-rust` Count Sketch:

- Rust `sketchlib-rust` Count Sketch
- C++ Apache DataSketches `count_min_sketch` as the comparison target

Shared assets live outside this directory:

- dataset: `accuracy/data/benchmark_data_1m_int64_zipf_s11_k100000.bin`
- scripts: `accuracy/scripts/`
- plots: `accuracy/plots/cs/`

Variant-local outputs live in:

- `accuracy/cs/output/`

## Run

Rust only:

```bash
ACCURACY_VARIANT=cs accuracy/scripts/run_accuracy_rust.sh
```

C++ only:

```bash
DATASKETCHES_CPP_ROOT=~/datasketches-cpp ACCURACY_VARIANT=cs accuracy/scripts/run_accuracy_cpp.sh
```

All implementations plus plots:

```bash
DATASKETCHES_CPP_ROOT=~/datasketches-cpp accuracy/scripts/run_accuracy_all.sh cs
```

## Outputs

- summary CSV: `accuracy/cs/output/cs_accuracy_results.csv`
- key-error CSV: `accuracy/cs/output/cs_accuracy_key_median_errors.csv`
- main plot: `accuracy/plots/cs/cs_accuracy_avg_relative_error.png`
