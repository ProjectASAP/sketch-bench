# CMS Accuracy Comparison Framework

This directory contains Count-Min Sketch accuracy experiments for:

- Rust `datasketches`
- Rust `sketchlib-rust`
- C++ DataSketches from `Insert-Optimized-Data-Sketches`

The workflow uses the existing `input/benchmark_data_1m_int64.bin` dataset, computes exact frequencies with a hash map baseline, evaluates CMS configurations `3x2048`, `3x4096`, and `3x8192`, and writes one CSV row per `(implementation, seed, rows, cols)`.

## Metric

For each observed key, the per-key error is:

`abs(true_count - estimate) / true_count`

The per-run aggregate metrics are:

- `avg_relative_error`
- `max_relative_error`
- `mean_absolute_error`

Accuracy is evaluated only on keys present in the dataset baseline map.

## Seed List

The experiments use these fixed seeds for all implementations:

`1, 2, 3, 4, 5, 6, 7, 8, 9, 10`

## Dataset

The data source is the repo-level file:

- `input/benchmark_data_1m_int64.bin`

The scripts stage it into:

- `accuracy/data/benchmark_data_1m_int64.bin`

This dataset consists of 1,000,000 uniformly random `i64` values. That makes it reproducible, but it is not an ideal workload for stressing CMS frequency-estimation behavior because most keys are likely to appear once.

## Output

The main CSV is:

- `accuracy/output/cms_accuracy_results.csv`

The main plot is:

- `accuracy/plots/cms_accuracy_avg_relative_error.png`

An additional scatter plot is also generated:

- `accuracy/plots/cms_accuracy_seed_scatter.png`

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
accuracy/scripts/run_accuracy_all.sh
```

## Notes

- The C++ runner expects `INSERT_OPT_ROOT` to point to `Insert-Optimized-Data-Sketches`, matching the rest of this repo.
- The Rust `sketchlib-rust` comparison uses 10 custom abbreviated `SketchHasher` types (`H01` through `H10`) with one fixed experiment seed per hasher.
