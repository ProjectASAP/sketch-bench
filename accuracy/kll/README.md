# KLL Accuracy Comparison Framework

This variant compares KLL quantile sketch implementations across multiple k values (`50, 100, 200, 400, 800`):

- Rust `sketchlib-rust` KLL
- Rust `sketch_oxide` KLL
- C++ Apache DataSketches KLL

For each (implementation, k) pair, the benchmark inserts 10M Zipf-distributed int64 values, then queries quantiles at 101 percentiles (p0 through p100). The ground truth is computed from a sorted copy of the data. The 101 relative errors form the distribution for each box in the grouped box plot.

Shared assets live outside this directory:

- dataset: `input/benchmark_data_10m_int64_zipf_s11_k100000.bin`
- scripts: `accuracy/scripts/`
- plots: `accuracy/plots/kll/`

Variant-local outputs live in:

- `accuracy/kll/output/`

## Run

Rust only:

```bash
ACCURACY_VARIANT=kll accuracy/scripts/run_accuracy_rust.sh
```

C++ only:

```bash
DATASKETCHES_CPP_ROOT=~/datasketches-cpp ACCURACY_VARIANT=kll accuracy/scripts/run_accuracy_cpp.sh
```

All implementations plus plot:

```bash
DATASKETCHES_CPP_ROOT=~/datasketches-cpp accuracy/scripts/run_accuracy_all.sh kll
```

## Outputs

- summary CSV: `accuracy/kll/output/kll_accuracy_results.csv`
- main plot: `accuracy/plots/kll/kll_accuracy_relative_error.png`
