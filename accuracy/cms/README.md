# CMS Accuracy Comparison Framework

This variant contains Count-Min Sketch accuracy experiments for:

- Rust `datasketches`
- Rust `sketchlib-rust`
- C++ Apache DataSketches `count_min_sketch`

Shared assets live outside this directory:

- dataset: `input/benchmark_data_10m_int64_zipf_s11_k100000.bin`
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
DATASKETCHES_CPP_ROOT=~/datasketches-cpp accuracy/scripts/run_accuracy_cpp.sh
```

All implementations plus plots:

```bash
DATASKETCHES_CPP_ROOT=~/datasketches-cpp accuracy/scripts/run_accuracy_all.sh cms
```

By default, that command only runs the main Zipf accuracy comparison.

To also run the slower CAIDA/CIC dataset comparison from `pcap`/CSV inputs, opt in explicitly:

```bash
ACCURACY_CMS_ENABLE_PCAP_COMPARE=1 \
DATASKETCHES_CPP_ROOT=~/datasketches-cpp \
accuracy/scripts/run_accuracy_all.sh cms
```

When `ACCURACY_CMS_ENABLE_PCAP_COMPARE=1` and `input/equinix-nyc.dirA.20190117-125910.UTC.anon.pcap` exists, the command also:

- runs CMS seed-level accuracy on the CAIDA pcap for both Rust and C++
- writes `accuracy/cms/output/cms_accuracy_key_seed_errors_caida.csv`
- writes `accuracy/plots/cms/cms_accuracy_dataset_compare_col_65536_seed_5.png`

Optional overrides:

- `ACCURACY_CMS_ENABLE_PCAP_COMPARE=1`
- `ACCURACY_CMS_CAIDA_SOURCE_PCAP`
- `ACCURACY_CMS_CAIDA_KEY_SEED_ERRORS`
- `ACCURACY_CMS_DATASET_COMPARE_COLS`
- `ACCURACY_CMS_DATASET_COMPARE_SEED`

## Outputs

- summary CSV: `accuracy/cms/output/cms_accuracy_results.csv`
- key-error CSV: `accuracy/cms/output/cms_accuracy_key_median_errors.csv`
- seed-level key-error CSV: `accuracy/cms/output/cms_accuracy_key_seed_errors.csv`
- CAIDA seed-level key-error CSV: `accuracy/cms/output/cms_accuracy_key_seed_errors_caida.csv`
- main plot: `accuracy/plots/cms/cms_accuracy_avg_relative_error.png`
- dataset comparison plot: `accuracy/plots/cms/cms_accuracy_dataset_compare_col_65536_seed_5.png`

## Dataset Comparison Box Plot

For a fixed column count and hash seed, you can compare CMS implementations across datasets
with a grouped box plot. Pass one seed-level key-error CSV per dataset:

```bash
python3 accuracy/scripts/plot_cms_accuracy.py \
  --variant cms \
  --input-key-errors accuracy/cms/output/cms_accuracy_key_median_errors.csv \
  --output accuracy/plots/cms/cms_accuracy_avg_relative_error.png \
  --dataset-key-seed-errors zipf=accuracy/cms/output/cms_accuracy_key_seed_errors.csv \
  --dataset-key-seed-errors caida=/path/to/caida_key_seed_errors.csv \
  --dataset-boxplot-cols 65536 \
  --dataset-boxplot-seed 5 \
  --dataset-boxplot-output accuracy/plots/cms/cms_accuracy_dataset_compare_col_65536_seed_5.png
```
