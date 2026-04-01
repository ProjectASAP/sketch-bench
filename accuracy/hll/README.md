# HLL Accuracy Comparison Framework

This variant compares HLL implementations across multiple precision levels (`lg_k=12,14,16`, i.e. `4096`, `16384`, `65536` registers):

- Rust `sketchlib-rust` HLL (P12, P14, P16)
- Rust `datasketches` HLL
- C++ Apache DataSketches HLL

The x-axis of the plot shows the register count, with grouped box plots per implementation. The variance comes from the 10 seeded input remappings (`1..10`) applied before insertion.

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
