# HLL Accuracy Comparison Framework

This variant compares fixed-size HLL implementations at `lg_k=14` (`16384` registers):

- Rust `sketchlib-rust` HLL
- Rust `datasketches` HLL
- C++ Apache DataSketches HLL

Unlike CMS/CS, HLL has a single fixed-size group in this repo, so the plot is a single grouped box plot. The variance comes from the 10 seeded input remappings (`1..10`) applied before insertion.

Shared assets live outside this directory:

- dataset: `input/benchmark_data_10m_int64_zipf_s11_k100000.bin`
- scripts: `accuracy/scripts/`
- plots: `accuracy/plots/hll/`

Variant-local outputs live in:

- `accuracy/hll/output/`

## Run

Rust only:

```bash
ACCURACY_VARIANT=hll accuracy/scripts/run_accuracy_rust.sh
```

C++ only:

```bash
DATASKETCHES_CPP_ROOT=~/datasketches-cpp ACCURACY_VARIANT=hll accuracy/scripts/run_accuracy_cpp.sh
```

All implementations plus plot:

```bash
DATASKETCHES_CPP_ROOT=~/datasketches-cpp accuracy/scripts/run_accuracy_all.sh hll
```

## Outputs

- summary CSV: `accuracy/hll/output/hll_accuracy_results.csv`
- main plot: `accuracy/plots/hll/hll_accuracy_relative_error.png`
