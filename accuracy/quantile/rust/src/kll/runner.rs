//! KLL accuracy runners — compares `sketchlib-rust` and
//! `sketch_oxide` KLL implementations against the exact quantile
//! baseline at each percentile of a 101-point grid.

use asap_sketchlib::KLL;
use sketch_oxide::quantiles::KllSketch as OxideKll;

use super::output::AccuracyRow;
use crate::baseline::BaselineData;

pub const IMPLEMENTATION_RUST_SKETCHLIB: &str = "rust_sketchlib_kll";
pub const IMPLEMENTATION_RUST_OXIDE: &str = "rust_oxide_kll";

pub const K_LIST: &[i32] = &[50, 100, 200, 400, 800];
pub const NUM_PERCENTILES: usize = 101;

pub fn run_sketchlib(baseline: &BaselineData) -> Vec<AccuracyRow> {
    let mut rows = Vec::with_capacity(K_LIST.len() * NUM_PERCENTILES);
    for &k in K_LIST {
        let mut sketch: KLL<i64> = KLL::init_kll(k);
        for &value in &baseline.values {
            sketch.update(&value);
        }
        for p in 0..NUM_PERCENTILES {
            let true_q = baseline.ground_truth_quantile(p);
            let estimate = sketch.quantile(p as f64 / 100.0);
            rows.push(build_row(
                IMPLEMENTATION_RUST_SKETCHLIB,
                k,
                p,
                baseline.total_items(),
                true_q,
                estimate,
            ));
        }
    }
    rows
}

pub fn run_oxide(baseline: &BaselineData) -> Vec<AccuracyRow> {
    let mut rows = Vec::with_capacity(K_LIST.len() * NUM_PERCENTILES);
    for &k in K_LIST {
        let mut sketch = OxideKll::new(k as u16).expect("valid KLL k");
        for &value in &baseline.values {
            sketch.update(value as f64);
        }
        for p in 0..NUM_PERCENTILES {
            let true_q = baseline.ground_truth_quantile(p);
            let estimate = sketch
                .quantile(p as f64 / 100.0)
                .expect("sketch is non-empty");
            rows.push(build_row(
                IMPLEMENTATION_RUST_OXIDE,
                k,
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
    k: i32,
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
        k,
        percentile,
        total_items,
        true_quantile,
        estimate,
        relative_error,
    }
}
