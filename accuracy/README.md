# Accuracy Layout

`accuracy/` is split into shared assets plus per-sketch variants:

- `input`: shared datasets and dataset generators
- `accuracy/plots`: shared plot output root, with one subdirectory per variant
- `accuracy/scripts`: shared runner and plotting entry points
- `accuracy/cms`: Count-Min Sketch accuracy sources and CMS-specific outputs
- `accuracy/cs`: Count Sketch accuracy sources and CS-specific outputs
- `accuracy/hll`: HyperLogLog accuracy sources and HLL-specific outputs
- `accuracy/kll`: KLL quantile accuracy sources and outputs
- `accuracy/dd`: DDSketch quantile accuracy sources and outputs
- `accuracy/nitro`: Nitro-heavy-hitter accuracy sources and outputs
- `accuracy/octo`: OctoSketch accuracy sources and outputs

## Variants

- `cms` mirrors the original top-level accuracy implementation:
  - Rust `datasketches`
  - Rust `sketchlib-rust` Count-Min Sketch
  - C++ Apache DataSketches `count_min_sketch`
- `cs` mirrors the `cms` layout, but its Rust implementation is `sketchlib-rust` Count Sketch only and the C++ comparison target is Apache DataSketches `count_min_sketch`.
- `hll` compares fixed-size `lg_k=14` HLLs:
  - Rust `datasketches`
  - Rust `sketchlib-rust`
  - C++ Apache DataSketches
- `kll` compares KLL quantile sketch implementations across multiple `k` values.
- `dd` benchmarks Rust `DDSketch` quantile accuracy across multiple `alpha` settings.
- `nitro` benchmarks Rust Nitro Count-Min accuracy across multiple Nitro sampling rates.
- `octo` emits separate CMS/CS/HLL accuracy plots for OctoSketch worker counts.

## Run

Rust only:

```bash
accuracy/scripts/run_accuracy_rust.sh
ACCURACY_VARIANT=cs accuracy/scripts/run_accuracy_rust.sh
ACCURACY_VARIANT=hll accuracy/scripts/run_accuracy_rust.sh
ACCURACY_VARIANT=dd accuracy/scripts/run_accuracy_rust.sh
ACCURACY_VARIANT=nitro accuracy/scripts/run_accuracy_rust.sh
```

C++ only:

```bash
accuracy/scripts/run_accuracy_cpp.sh
ACCURACY_VARIANT=cs accuracy/scripts/run_accuracy_cpp.sh
DATASKETCHES_CPP_ROOT=~/datasketches-cpp accuracy/scripts/run_accuracy_cpp.sh
DATASKETCHES_CPP_ROOT=~/datasketches-cpp ACCURACY_VARIANT=hll accuracy/scripts/run_accuracy_cpp.sh
```

All implementations plus plots:

```bash
accuracy/scripts/run_accuracy_all.sh cms
accuracy/scripts/run_accuracy_all.sh cs
DATASKETCHES_CPP_ROOT=~/datasketches-cpp accuracy/scripts/run_accuracy_all.sh cms
DATASKETCHES_CPP_ROOT=~/datasketches-cpp accuracy/scripts/run_accuracy_all.sh cs
DATASKETCHES_CPP_ROOT=~/datasketches-cpp accuracy/scripts/run_accuracy_all.sh hll
accuracy/scripts/run_accuracy_all.sh dd
accuracy/scripts/run_accuracy_all.sh nitro
accuracy/scripts/run_accuracy_all.sh all
```

## Paths

- Shared dataset:
  - `input/benchmark_data_10m_int64_zipf_s11_k100000.bin`
- CMS outputs:
  - `accuracy/cms/output/cms_accuracy_results.csv`
  - `accuracy/cms/output/cms_accuracy_key_seed_errors.csv`
  - `accuracy/plots/cms/cms_accuracy_avg_relative_error.png`
- CS outputs:
  - `accuracy/cs/output/cs_accuracy_results.csv`
  - `accuracy/plots/cs/cs_accuracy_avg_relative_error.png`
- HLL outputs:
  - `accuracy/hll/output/hll_accuracy_results.csv`
  - `accuracy/plots/hll/hll_accuracy_relative_error.png`
- KLL outputs:
  - `accuracy/kll/output/kll_accuracy_results.csv`
  - `accuracy/plots/kll/kll_accuracy_relative_error.png`
- DD outputs:
  - `accuracy/dd/output/dd_accuracy_results.csv`
  - `accuracy/plots/dd/dd_accuracy_relative_error.png`
- Nitro outputs:
  - `accuracy/nitro/output/nitro_accuracy_results.csv`
  - `accuracy/plots/nitro/nitro_accuracy_relative_error.png`
- Aggregate overview:
  - `accuracy/plots/accuracy_overview.png`
