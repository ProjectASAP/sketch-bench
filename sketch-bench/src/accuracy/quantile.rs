//! Quantile-family ground truth (KLL / DDSketch).
//!
//! Builds a sorted f64 copy of the stream and computes the
//! ground-truth quantile via **Type-7 linear interpolation**
//! (NumPy / R / Prometheus default — same algorithm
//! `ExactQuantile::quantile_fraction` uses). Reports value-based
//! error (absolute + relative + range-normalised) of the
//! sketch's estimate against that ground truth across a
//! 101-point quantile grid.
//!
//! Why value-error and not rank-error: rank-error is the
//! standard KLL paper bound, but `rank_of` over upper-bound
//! ties gave non-zero "error" even for the exact baseline on
//! Zipf streams — a metric artifact, not real divergence.
//! Pinning the metric to "how far off is the returned value"
//! against the canonical Type-7 quantile makes the exact
//! baseline trivially zero and lets sketches' numbers reflect
//! actual estimation error.

use serde_json::json;
use sketch_core::sketch::Sketch;
use std::time::Instant;

use super::{Comparison, GroundTruth};

/// Lossy-cast to f64. Impl for common numeric item types used
/// by quantile sketches. Separate from `Into<f64>` because the
/// standard lib refuses the `i64 -> f64` impl on precision
/// grounds, but for rank-error analysis the `as` cast is fine.
pub trait ToF64 {
    fn to_f64(self) -> f64;
}

impl ToF64 for f64 {
    fn to_f64(self) -> f64 {
        self
    }
}
impl ToF64 for f32 {
    fn to_f64(self) -> f64 {
        self as f64
    }
}
impl ToF64 for i64 {
    fn to_f64(self) -> f64 {
        self as f64
    }
}
impl ToF64 for i32 {
    fn to_f64(self) -> f64 {
        self as f64
    }
}
impl ToF64 for u64 {
    fn to_f64(self) -> f64 {
        self as f64
    }
}
impl ToF64 for u32 {
    fn to_f64(self) -> f64 {
        self as f64
    }
}

pub struct QuantileGT;

impl<S> GroundTruth<S> for QuantileGT
where
    S: Sketch<Query = f64, Answer = f64>,
    S::Item: Clone + PartialOrd + ToF64,
{
    fn compare(&self, sketch: &S, items: &[S::Item]) -> Comparison {
        if items.is_empty() {
            return Comparison {
                json: json!({ "items": 0, "mean_abs_err": 0.0 }),
                queries: 0,
                query_wall_ns: 0,
            };
        }
        let mut sorted: Vec<f64> = items.iter().cloned().map(ToF64::to_f64).collect();
        sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));

        // Time only the sketch.query() boundary; truth lookup +
        // error reduction happen after the timer stops.
        let mut estimates: [f64; 101] = [0.0; 101];
        let q_start = Instant::now();
        for (i, slot) in estimates.iter_mut().enumerate() {
            let q = i as f64 / 100.0;
            *slot = sketch.query(q);
        }
        let q_ns = q_start.elapsed().as_nanos() as u64;

        // Range used to normalise absolute error; falls back to
        // 1.0 on a constant stream so the divisor is never zero.
        let data_min = sorted[0];
        let data_max = sorted[sorted.len() - 1];
        let range = (data_max - data_min).max(1.0);

        let mut max_abs_err = 0.0_f64;
        let mut max_rel_err = 0.0_f64;
        let mut max_norm_err = 0.0_f64;
        let mut sum_abs = 0.0_f64;
        let mut sum_rel = 0.0_f64;
        let mut rel_n = 0usize;
        let mut sum_norm = 0.0_f64;

        for (i, est) in estimates.iter().enumerate() {
            let q = i as f64 / 100.0;
            let truth = type7_quantile(&sorted, q);
            let abs_err = (est - truth).abs();
            sum_abs += abs_err;
            if abs_err > max_abs_err {
                max_abs_err = abs_err;
            }
            let norm_err = abs_err / range;
            sum_norm += norm_err;
            if norm_err > max_norm_err {
                max_norm_err = norm_err;
            }
            if truth.abs() > f64::EPSILON {
                let rel = abs_err / truth.abs();
                sum_rel += rel;
                rel_n += 1;
                if rel > max_rel_err {
                    max_rel_err = rel;
                }
            }
        }
        let n_grid = estimates.len() as f64;
        let mean_abs = sum_abs / n_grid;
        let mean_norm = sum_norm / n_grid;
        let mean_rel = if rel_n == 0 {
            0.0
        } else {
            sum_rel / rel_n as f64
        };

        Comparison {
            json: json!({
                "items": items.len(),
                "grid_points": 101,
                "data_min": data_min,
                "data_max": data_max,
                "mean_abs_err": mean_abs,
                "max_abs_err": max_abs_err,
                "mean_relative_err": mean_rel,
                "max_relative_err": max_rel_err,
                "mean_normalized_err": mean_norm,
                "max_normalized_err": max_norm_err,
            }),
            queries: 101,
            query_wall_ns: q_ns,
        }
    }
}

/// Type-7 linear interpolation on a pre-sorted f64 slice. Same
/// formula `baselines::quantile::ExactQuantile::quantile_fraction`
/// uses — keeping the two in lock-step is what makes the exact
/// baseline land at exactly zero error here.
fn type7_quantile(sorted: &[f64], q: f64) -> f64 {
    if sorted.is_empty() {
        return f64::NAN;
    }
    let q = q.clamp(0.0, 1.0);
    let n = sorted.len();
    let rank = q * (n - 1) as f64;
    let lower = rank.floor() as usize;
    let upper = (lower + 1).min(n - 1);
    let weight = rank - rank.floor();
    sorted[lower] * (1.0 - weight) + sorted[upper] * weight
}
