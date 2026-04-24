//! DDSketch accuracy runner — compares `sketchlib-rust` DDSketch
//! against the exact quantile baseline across a grid of alpha
//! values.

use asap_sketchlib::DDSketch;

use super::output::AccuracyRow;
use crate::baseline::BaselineData;

pub const IMPLEMENTATION_RUST_SKETCHLIB: &str = "rust_sketchlib_dd";
pub const ALPHA_LIST: &[f64] = &[0.005, 0.01, 0.02, 0.05, 0.1];
pub const NUM_PERCENTILES: usize = 101;

pub fn run_sketchlib(baseline: &BaselineData) -> Vec<AccuracyRow> {
    let mut rows = Vec::with_capacity(ALPHA_LIST.len() * NUM_PERCENTILES);
    for &alpha in ALPHA_LIST {
        let mut sketch = DDSketch::new(alpha);
        for &value in &baseline.values {
            sketch.add(&(value as f64));
        }
        for p in 0..NUM_PERCENTILES {
            let true_q = baseline.ground_truth_quantile(p);
            let estimate = sketch
                .get_value_at_quantile(p as f64 / 100.0)
                .expect("DDSketch is non-empty");
            rows.push(build_row(
                IMPLEMENTATION_RUST_SKETCHLIB,
                alpha,
                p,
                baseline.total_items(),
                true_q,
                estimate,
            ));
        }
    }
    rows
}

fn build_row(
    implementation: &'static str,
    alpha: f64,
    percentile: usize,
    total_items: usize,
    true_quantile: f64,
    estimate: f64,
) -> AccuracyRow {
    let relative_error = if true_quantile.abs() < f64::EPSILON {
        if estimate.abs() < f64::EPSILON {
            0.0
        } else {
            estimate.abs()
        }
    } else {
        (estimate - true_quantile).abs() / true_quantile.abs()
    };
    AccuracyRow {
        implementation,
        language: "rust",
        alpha,
        percentile,
        total_items,
        true_quantile,
        estimate,
        relative_error,
    }
}
