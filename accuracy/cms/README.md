# CMS Accuracy Comparison Framework

This variant contains Count-Min Sketch accuracy experiments for:

- Rust `datasketches`
- Rust `sketchlib-rust`
- C++ DataSketches from `Insert-Optimized-Data-Sketches`

Shared assets live outside this directory:

- dataset: `accuracy/data/benchmark_data_1m_int64_zipf_s11_k100000.bin`
- scripts: `accuracy/scripts/`
- plots: `accuracy/plots/cms/`

Variant-local outputs live in:

- `accuracy/cms/output/`

## Run

Rust only:

```bash
accuracy/scripts/run_accuracy_rust.sh
```

C++ only:

```bash
accuracy/scripts/run_accuracy_cpp.sh
```

All implementations plus plots:

```bash
accuracy/scripts/run_accuracy_all.sh cms
```

## Outputs

- summary CSV: `accuracy/cms/output/cms_accuracy_results.csv`
- key-error CSV: `accuracy/cms/output/cms_accuracy_key_median_errors.csv`
- main plot: `accuracy/plots/cms/cms_accuracy_avg_relative_error.png`
