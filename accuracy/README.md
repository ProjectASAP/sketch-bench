# CMS Accuracy Comparison Framework

This directory contains Count-Min Sketch accuracy experiments for:

- Rust `datasketches`
- Rust `sketchlib-rust`
- C++ DataSketches from `Insert-Optimized-Data-Sketches`

The workflow uses an accuracy-local Zipf dataset, computes exact frequencies with a hash map baseline, evaluates CMS configurations `3x2048`, `3x4096`, and `3x8192`, and writes one CSV row per `(implementation, seed, rows, cols)`.

## Metric

For each observed key, the per-key error is:

`abs(true_count - estimate) / true_count`

The per-run aggregate metrics are:

- `avg_relative_error`
- `max_relative_error`
- `mean_absolute_error`

Accuracy is evaluated only on heavy hitters in the dataset baseline map:

- keys with `true_count >= 100`

This avoids tail-key relative-error blowups from tiny denominators and keeps the report focused on the high-frequency portion of the Zipf workload.

## Seed List

The experiments use these fixed seeds for all implementations:

`1, 2, 3, 4, 5, 6, 7, 8, 9, 10`

## Dataset

The default accuracy dataset is:

- `accuracy/data/benchmark_data_1m_int64_zipf_s11_k100000.bin`

It contains 1,000,000 Zipf-distributed `i64` values with:

- exponent `s = 1.1`
- support size `100,000`
- seed `42`

The emitted values are rank IDs in the range `1..100000`. This creates a reproducible heavy-tailed workload that is much better suited to stressing CMS frequency-estimation behavior than the old mostly-unique uniform dataset.

## Heavy-Hitter Evaluation

The accuracy workflow evaluates only heavy hitters, defined as:

- `true_count >= 100`

This rule applies consistently to:

- summary CSV metrics
- key-error CSV rows
- the generated plot

The CSV schema is unchanged, but the `distinct_items` column now represents the number of evaluated heavy hitters rather than the total distinct key count in the raw dataset.

If the dataset file is missing, the accuracy scripts regenerate it automatically using:

- `accuracy/data/generate_zipf_data.sh`

The repo-level benchmark dataset under `input/benchmark_data_1m_int64.bin` remains unchanged and is still used by the non-accuracy benchmark harness.

## Output

The main CSV is:

- `accuracy/output/cms_accuracy_results.csv`

The main plot is:

- `accuracy/plots/cms_accuracy_avg_relative_error.png`

An additional scatter plot is also generated:

- `accuracy/plots/cms_accuracy_seed_scatter.png`

The main plot is heavy-hitter-only and uses per-key median-over-seeds relative error for keys with `true_count >= 100`.

Additional grouped plots are also generated:

- `accuracy/plots/cms_accuracy_avg_relative_error_from_8192.png`
- `accuracy/plots/cms_accuracy_avg_relative_error_from_16384.png`

These are cropped views of the same heavy-hitter-only key-error CSV, restricted to larger column counts.

Separate per-column plots are also generated:

- `accuracy/plots/cms_accuracy_avg_relative_error_col_2048.png`
- `accuracy/plots/cms_accuracy_avg_relative_error_col_4096.png`
- `accuracy/plots/cms_accuracy_avg_relative_error_col_8192.png`
- `accuracy/plots/cms_accuracy_avg_relative_error_col_16384.png`
- `accuracy/plots/cms_accuracy_avg_relative_error_col_32768.png`
- `accuracy/plots/cms_accuracy_avg_relative_error_col_65536.png`
- `accuracy/plots/cms_accuracy_avg_relative_error_col_131072.png`

These isolate implementation differences at a fixed sketch width. All of the plots above use the same heavy-hitter-only key-error CSV.

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
- The accuracy run scripts auto-generate the default Zipf dataset if it is not already present.
