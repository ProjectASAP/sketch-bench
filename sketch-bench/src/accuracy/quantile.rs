//! Quantile-family ground truth (KLL). Exact sorted copy;
//! reports max rank error across a 101-point quantile grid.

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
                json: json!({ "items": 0, "max_rank_err": 0.0 }),
                queries: 0,
                query_wall_ns: 0,
            };
        }
        let mut sorted: Vec<f64> = items.iter().cloned().map(ToF64::to_f64).collect();
        sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));

        // Time only the sketch.query() boundary; rank lookup +
        // error reduction happen after the timer stops.
        let mut estimates: [f64; 101] = [0.0; 101];
        let q_start = Instant::now();
        for (i, slot) in estimates.iter_mut().enumerate() {
            let q = i as f64 / 100.0;
            *slot = sketch.query(q);
        }
        let q_ns = q_start.elapsed().as_nanos() as u64;

        let n = sorted.len() as f64;
        let mut max_rank_err = 0.0_f64;
        let mut errs = Vec::with_capacity(101);
        for (i, est) in estimates.iter().enumerate() {
            let q = i as f64 / 100.0;
            let rank = rank_of(&sorted, *est);
            let true_q = rank / n;
            let rank_err = (true_q - q).abs();
            errs.push(rank_err);
            if rank_err > max_rank_err {
                max_rank_err = rank_err;
            }
        }
        let mean = errs.iter().sum::<f64>() / errs.len() as f64;
        Comparison {
            json: json!({
                "items": items.len(),
                "max_rank_err": max_rank_err,
                "mean_rank_err": mean,
                "grid_points": 101,
            }),
            queries: 101,
            query_wall_ns: q_ns,
        }
    }
}

fn rank_of(sorted: &[f64], x: f64) -> f64 {
    let mut lo = 0usize;
    let mut hi = sorted.len();
    while lo < hi {
        let mid = (lo + hi) / 2;
        if sorted[mid] <= x {
            lo = mid + 1;
        } else {
            hi = mid;
        }
    }
    lo as f64
}
