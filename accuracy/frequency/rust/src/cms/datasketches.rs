use crate::baseline::BaselineData;
use crate::config::{cms_impl::DATASKETCHES as IMPLEMENTATION_RUST_DATASKETCHES, COLS_LIST, ROWS};
use crate::output::{
    AccuracyRow, KeyErrorCsvWriter, KeyMedianErrorRow, KeySeedErrorCsvWriter, KeySeedErrorRow,
};
use crate::seeds::SEEDS;
use datasketches::countmin::CountMinSketch;
use std::io;

pub fn run_summary(baseline: &BaselineData) -> Vec<AccuracyRow> {
    let heavy_hitters = heavy_hitters_or_panic(baseline);
    let mut rows = Vec::with_capacity(SEEDS.len() * COLS_LIST.len());
    for &seed in &SEEDS {
        for &cols in &COLS_LIST {
            let mut sketch = CountMinSketch::with_seed(ROWS as u8, cols as u32, seed);
            for &value in &baseline.values {
                sketch.update(value);
            }
            rows.push(measure(seed, cols, baseline, &heavy_hitters, |value| {
                sketch.estimate(*value) as u64
            }));
        }
    }
    rows
}

pub fn write_key_median_errors(
    baseline: &BaselineData,
    writer: &mut KeyErrorCsvWriter,
) -> io::Result<()> {
    let heavy_hitters = heavy_hitters_or_panic(baseline);
    for &cols in &COLS_LIST {
        let sketches: Vec<CountMinSketch> = SEEDS
            .iter()
            .map(|&seed| {
                let mut sketch = CountMinSketch::with_seed(ROWS as u8, cols as u32, seed);
                for &value in &baseline.values {
                    sketch.update(value);
                }
                sketch
            })
            .collect();

        for &(key, true_count) in &heavy_hitters {
            let mut estimates = [0u64; SEEDS.len()];
            for (index, sketch) in sketches.iter().enumerate() {
                estimates[index] = sketch.estimate(key) as u64;
            }
            estimates.sort_unstable();
            let median_estimate = estimates[(estimates.len() / 2) - 1];
            let median_relative_error =
                median_estimate.abs_diff(true_count) as f64 / true_count as f64;
            writer.write_row(&KeyMedianErrorRow {
                implementation: IMPLEMENTATION_RUST_DATASKETCHES,
                language: "rust",
                rows: ROWS,
                cols,
                key,
                true_count,
                median_estimate: median_estimate as i64,
                median_relative_error,
            })?;
        }
    }
    Ok(())
}

pub fn write_key_seed_errors(
    baseline: &BaselineData,
    writer: &mut KeySeedErrorCsvWriter,
    seed_filter: Option<u64>,
    cols_filter: Option<usize>,
) -> io::Result<()> {
    let heavy_hitters = heavy_hitters_or_panic(baseline);
    for &seed in &SEEDS {
        if seed_filter.is_some_and(|required| required != seed) {
            continue;
        }
        for &cols in &COLS_LIST {
            if cols_filter.is_some_and(|required| required != cols) {
                continue;
            }
            let mut sketch = CountMinSketch::with_seed(ROWS as u8, cols as u32, seed);
            for &value in &baseline.values {
                sketch.update(value);
            }

            for &(key, true_count) in &heavy_hitters {
                let estimate = sketch.estimate(key) as u64;
                let relative_error = estimate.abs_diff(true_count) as f64 / true_count as f64;
                writer.write_row(&KeySeedErrorRow {
                    implementation: IMPLEMENTATION_RUST_DATASKETCHES,
                    language: "rust",
                    seed,
                    rows: ROWS,
                    cols,
                    key,
                    true_count,
                    estimate: estimate as i64,
                    relative_error,
                })?;
            }
        }
    }
    Ok(())
}

fn measure<F>(
    seed: u64,
    cols: usize,
    baseline: &BaselineData,
    heavy_hitters: &[(i64, u64)],
    mut estimate: F,
) -> AccuracyRow
where
    F: FnMut(&i64) -> u64,
{
    let mut total_relative_error = 0.0f64;
    let mut max_relative_error = 0.0f64;
    let mut total_absolute_error = 0.0f64;

    for &(value, true_count) in heavy_hitters {
        let estimate_value = estimate(&value);
        let absolute_error = estimate_value.abs_diff(true_count) as f64;
        let relative_error = absolute_error / true_count as f64;
        total_relative_error += relative_error;
        total_absolute_error += absolute_error;
        if relative_error > max_relative_error {
            max_relative_error = relative_error;
        }
    }

    let distinct = heavy_hitters.len() as f64;
    AccuracyRow {
        implementation: IMPLEMENTATION_RUST_DATASKETCHES,
        language: "rust",
        seed,
        rows: ROWS,
        cols,
        total_items: baseline.total_items(),
        distinct_items: heavy_hitters.len(),
        avg_relative_error: total_relative_error / distinct,
        max_relative_error,
        mean_absolute_error: total_absolute_error / distinct,
    }
}

fn heavy_hitters_or_panic(baseline: &BaselineData) -> Vec<(i64, u64)> {
    let heavy_hitters = baseline.heavy_hitters();
    assert!(
        !heavy_hitters.is_empty(),
        "baseline contains no heavy hitters with true_count >= 100"
    );
    heavy_hitters
}
