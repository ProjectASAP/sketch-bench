# Accuracy Layout

`accuracy/` is split into shared assets plus per-sketch variants:

- `accuracy/data`: shared datasets and dataset generators
- `accuracy/plots`: shared plot output root, with one subdirectory per variant
- `accuracy/scripts`: shared runner and plotting entry points
- `accuracy/cms`: Count-Min Sketch accuracy sources and CMS-specific outputs
- `accuracy/cs`: Count Sketch accuracy sources and CS-specific outputs

## Variants

- `cms` mirrors the original top-level accuracy implementation:
  - Rust `datasketches`
  - Rust `sketchlib-rust` Count-Min Sketch
  - C++ DataSketches CMS from `Insert-Optimized-Data-Sketches`
- `cs` mirrors the `cms` layout, but its Rust implementation is `sketchlib-rust` Count Sketch only.

## Run

Rust only:

```bash
accuracy/scripts/run_accuracy_rust.sh
ACCURACY_VARIANT=cs accuracy/scripts/run_accuracy_rust.sh
```

C++ only:

```bash
accuracy/scripts/run_accuracy_cpp.sh
ACCURACY_VARIANT=cs accuracy/scripts/run_accuracy_cpp.sh
```

All implementations plus plots:

```bash
accuracy/scripts/run_accuracy_all.sh cms
accuracy/scripts/run_accuracy_all.sh cs
```

## Paths

- Shared dataset:
  - `accuracy/data/benchmark_data_1m_int64_zipf_s11_k100000.bin`
- CMS outputs:
  - `accuracy/cms/output/cms_accuracy_results.csv`
  - `accuracy/plots/cms/cms_accuracy_avg_relative_error.png`
- CS outputs:
  - `accuracy/cs/output/cs_accuracy_results.csv`
  - `accuracy/plots/cs/cs_accuracy_avg_relative_error.png`
