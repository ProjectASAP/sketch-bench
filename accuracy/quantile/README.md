# Quantile Accuracy Comparison Framework

This variant provides accuracy ground truth for the **quantile**
statistic, using `sketch_bench::baselines::ExactQuantile` as the
exact reference.

Two sketch families share this crate:

- **KLL** (`--sketch kll`)
  - Rust `sketchlib-rust` KLL
  - Rust `sketch_oxide` KLL
  - C++ Apache DataSketches KLL (under `cpp/kll/`)
- **DDSketch** (`--sketch dd`)
  - Rust `sketchlib-rust` DDSketch

The Rust runners live under `rust/src/{kll,dd}/runner.rs`. Each
has its own row schema in `rust/src/{kll,dd}/output.rs` because
the parameter column differs (`k` for KLL, `alpha` for DDSketch);
`baseline.rs` is shared. The binary dispatches on `--sketch`.

KLL runs at `k ∈ {50, 100, 200, 400, 800}`; DDSketch at
`alpha ∈ {0.005, 0.01, 0.02, 0.05, 0.1}`. Both query 101
percentiles per configuration against the exact sorted ground
truth.

Shared assets live outside this directory:

- dataset: `input/benchmark_data_10m_int64_zipf_s11_k100000.bin`
- scripts: `accuracy/scripts/`
- plots: `accuracy/plots/{kll,dd}/`

## Run

Rust only:

```bash
ACCURACY_VARIANT=kll accuracy/scripts/run_accuracy_rust.sh
ACCURACY_VARIANT=dd  accuracy/scripts/run_accuracy_rust.sh
```

C++ only (KLL only):

```bash
DATASKETCHES_CPP_ROOT=~/datasketches-cpp ACCURACY_VARIANT=kll \
    accuracy/scripts/run_accuracy_cpp.sh
```

All implementations plus plot:

```bash
DATASKETCHES_CPP_ROOT=~/datasketches-cpp accuracy/scripts/run_accuracy_all.sh kll
accuracy/scripts/run_accuracy_all.sh dd
```

## Outputs

- KLL summary CSV: `accuracy/kll/output/kll_accuracy_results.csv`
- DDSketch summary CSV: `accuracy/dd/output/dd_accuracy_results.csv`
- plots: `accuracy/plots/kll/kll_accuracy_relative_error.png`,
  `accuracy/plots/dd/dd_accuracy_relative_error.png`
