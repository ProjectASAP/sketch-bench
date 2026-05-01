# Frequency Accuracy Comparison Framework

This variant provides accuracy ground truth for the **frequency**
statistic, using `sketch_bench::baselines::ExactFrequency` as the
exact reference.

Two sketch families share this crate:

- **Count-Min Sketch** (`--sketch cms`)
  - Rust `datasketches`
  - Rust `sketchlib-rust`
  - Rust `sketch_oxide`
  - C++ Apache DataSketches (under `cpp/cms/`)
- **Count Sketch** (`--sketch countsketch` or `--sketch cs`)
  - Rust `sketchlib-rust`
  - Rust `sketch_oxide`
  - C++ comparison target: Apache DataSketches `count_min_sketch`
    (under `cpp/countsketch/`)

The Rust runners live under `rust/src/{cms,countsketch}/*.rs` and
share `baseline.rs`, `config.rs`, `output.rs`, `seeds.rs` at the
crate root. The binary dispatches on `--sketch`. Rows in the
output CSVs are tagged with family-specific implementation names
(`rust_oxide_cms` vs `rust_oxide_cs`) so plot scripts can group
them.

Shared assets live outside this directory:

- dataset: `input/benchmark_data_10m_int64_zipf_s11_k100000.bin`
- scripts: `accuracy/scripts/`
- plots: `accuracy/plots/frequency/` (or the legacy per-family
  `plots/cms/` + `plots/cs/` directories)

## Run

Rust only:

```bash
# CMS (default when --sketch omitted)
ACCURACY_VARIANT=cms accuracy/scripts/run_accuracy_rust.sh
# Count Sketch
ACCURACY_VARIANT=cs  accuracy/scripts/run_accuracy_rust.sh
```

C++ only:

```bash
DATASKETCHES_CPP_ROOT=~/datasketches-cpp ACCURACY_VARIANT=cms \
    accuracy/scripts/run_accuracy_cpp.sh
```

All implementations plus plots:

```bash
DATASKETCHES_CPP_ROOT=~/datasketches-cpp accuracy/scripts/run_accuracy_all.sh cms
DATASKETCHES_CPP_ROOT=~/datasketches-cpp accuracy/scripts/run_accuracy_all.sh cs
```

## CAIDA / pcap comparison (CMS only)

The pcap-driven dataset comparison plot is CMS-specific. Opt in
via `ACCURACY_CMS_ENABLE_PCAP_COMPARE=1` — see
`accuracy/scripts/run_accuracy_all.sh` for the full env var list.

## Outputs

Per-variant, outputs land in `accuracy/{cms,cs}/output/` and plot
files in `accuracy/plots/{cms,cs}/`. File schemas:

- summary CSV: `{variant}_accuracy_results.csv`
- key-error CSV: `{variant}_accuracy_key_median_errors.csv`
- seed-level key-error CSV (CMS only):
  `cms_accuracy_key_seed_errors.csv`

The row schemas are unchanged from the pre-merge crates, except
`median_estimate` / `estimate` are now `i64` so Count-Sketch's
signed estimates round-trip cleanly. All observed CMS values fit
in `i64`.
