# Cardinality Accuracy Comparison Framework

This variant provides accuracy ground truth for the **cardinality**
statistic, using `sketch_bench::baselines::ExactCardinality` as the
exact reference.

Currently the only sketch family on the cardinality axis is HLL.
Other cardinality sketches (CPC, Theta, …) would join the same
crate rather than getting their own directory — the organisation
mirrors `sketch-bench/src/baselines/` so each statistic has one
home.

Sketches compared, at multiple precisions (`lg_k=12,14,16` →
`4096`, `16384`, `65536` registers):

- Rust `sketchlib-rust` HLL (P12, P14, P16)
- Rust `datasketches` HLL
- C++ Apache DataSketches HLL

The x-axis of the plot shows the register count, with grouped box
plots per implementation. Variance comes from the 10 seeded input
remappings (`1..10`) applied before insertion.

Shared assets live outside this directory:

- dataset: `input/benchmark_data_10m_int64_zipf_s11_k100000.bin`
- scripts: `accuracy/scripts/`
- plots: `accuracy/plots/cardinality/`

Variant-local outputs live in:

- `accuracy/cardinality/output/`

## Run

Rust only:

```bash
ACCURACY_VARIANT=cardinality accuracy/scripts/run_accuracy_rust.sh
# legacy alias — still works
ACCURACY_VARIANT=hll accuracy/scripts/run_accuracy_rust.sh
```

C++ only:

```bash
DATASKETCHES_CPP_ROOT=~/datasketches-cpp ACCURACY_VARIANT=cardinality \
    accuracy/scripts/run_accuracy_cpp.sh
```

All implementations plus plot:

```bash
DATASKETCHES_CPP_ROOT=~/datasketches-cpp accuracy/scripts/run_accuracy_all.sh cardinality
```

## Outputs

- summary CSV: `accuracy/cardinality/output/cardinality_accuracy_results.csv`
- main plot: `accuracy/plots/cardinality/cardinality_accuracy_relative_error.png`
