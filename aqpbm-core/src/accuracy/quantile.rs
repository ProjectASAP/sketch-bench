//! Quantile-family ground truth comparators, one metric per family, because
//! each sketch's correctness bound is defined differently in its paper.
//! [`RankErrorGT`] (KLL) reports rank error in units of `n`, range-based over
//! tie intervals; [`RelativeErrorGT`] (DDSketch) reports `|v̂ − v| / |v|`
//! against the Type-7 linear-interpolation quantile ([`type7_quantile`]).

use crate::accumulator::Accumulator;
use std::collections::BTreeMap;
use std::time::Instant;

use super::statistic::QuantileOps;
use super::{Comparison, GroundTruth};
use crate::metrics::QueryCallSample;

/// Number of times the 101-percentile sweep is repeated when `record_calls`
/// is on, matching the KLL / DD query binaries' `REPEATS_PER_RUN = 10`.
const RAW_REPEATS_PER_RUN: usize = 10;
const NUM_PERCENTILES: usize = 101;

/// Lossy-cast to f64, for the numeric item types quantile sketches take.
/// Separate from `Into<f64>` because std refuses `i64 -> f64` on precision
/// grounds, which rank-error analysis does not care about.
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

/// A value an ordered (quantile) sketch can ingest. Adds a **total** order
/// over [`ToF64`]: `f64` is only partially ordered, so `partial_cmp().unwrap()`
/// panics on NaN. Use `f64::total_cmp`; integers just use `Ord::cmp`.
pub trait QuantileValue: ToF64 + Copy {
    fn total_cmp(&self, other: &Self) -> std::cmp::Ordering;
}

impl QuantileValue for i64 {
    #[inline(always)]
    fn total_cmp(&self, other: &Self) -> std::cmp::Ordering {
        Ord::cmp(self, other)
    }
}

impl QuantileValue for f64 {
    #[inline(always)]
    fn total_cmp(&self, other: &Self) -> std::cmp::Ordering {
        f64::total_cmp(self, other)
    }
}

// ---------- KLL: rank error ----------

/// Rank-error comparator for KLL-style sketches.
#[derive(Debug, Default, Clone, Copy)]
pub struct RankErrorGT {
    /// Capture per-call samples for the legacy
    /// `kll_throughput_query_results_rust.csv` shape (one row
    /// per (run, repeat, percentile) tuple). Off by default.
    pub record_calls: bool,
}

impl<S> GroundTruth<S> for RankErrorGT
where
    S: Accumulator + QuantileOps,
    S::Item: Clone + PartialOrd + ToF64,
{
    fn compare(&self, sketch: &S, items: &[S::Item]) -> Comparison {
        if items.is_empty() {
            return Comparison {
                metrics: metrics_from([("items", 0.0), ("mean_rank_err", 0.0)]),
                queries: 0,
                query_wall_ns: 0,
                query_calls: None,
            };
        }
        let mut sorted: Vec<f64> = items.iter().cloned().map(ToF64::to_f64).collect();
        sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));

        // Time only the `estimate_quantile` boundary.
        let mut estimates: [f64; 101] = [0.0; 101];
        let q_start = Instant::now();
        for (i, slot) in estimates.iter_mut().enumerate() {
            let q = i as f64 / 100.0;
            *slot = sketch.estimate_quantile(q);
        }
        let q_ns = q_start.elapsed().as_nanos() as u64;

        let query_calls = if self.record_calls {
            Some(capture_quantile_calls(sketch))
        } else {
            None
        };

        let n = sorted.len();
        let nf = n as f64;
        let mut max_rank_err = 0.0_f64;
        let mut sum_rank_err = 0.0_f64;

        for (i, est) in estimates.iter().enumerate() {
            let q = i as f64 / 100.0;
            // The returned value occupies the rank interval `[lower, upper]`:
            // error 0 if `q*n` falls inside, else distance to the near edge.
            let lower = lower_bound(&sorted, *est);
            let upper = upper_bound(&sorted, *est);
            let target = q * nf;
            let raw_err = if (target as f64) < lower as f64 {
                lower as f64 - target
            } else if (target as f64) > upper as f64 {
                target - upper as f64
            } else {
                0.0
            };
            let err = raw_err / nf;
            sum_rank_err += err;
            if err > max_rank_err {
                max_rank_err = err;
            }
        }

        let mean = sum_rank_err / estimates.len() as f64;
        Comparison {
            metrics: metrics_from([
                ("items", n as f64),
                ("grid_points", NUM_PERCENTILES as f64),
                ("mean_rank_err", mean),
                ("max_rank_err", max_rank_err),
            ]),
            queries: 101,
            query_wall_ns: q_ns,
            query_calls,
        }
    }
}

// ---------- DDSketch: relative error ----------

/// Relative-error comparator for DDSketch-style sketches.
#[derive(Debug, Default, Clone, Copy)]
pub struct RelativeErrorGT {
    /// Capture per-call samples for the legacy
    /// `dd_throughput_query_results_rust.csv` shape. Off by default.
    pub record_calls: bool,
}

impl<S> GroundTruth<S> for RelativeErrorGT
where
    S: Accumulator + QuantileOps,
    S::Item: Clone + PartialOrd + ToF64,
{
    fn compare(&self, sketch: &S, items: &[S::Item]) -> Comparison {
        if items.is_empty() {
            return Comparison {
                metrics: metrics_from([("items", 0.0), ("mean_relative_err", 0.0)]),
                queries: 0,
                query_wall_ns: 0,
                query_calls: None,
            };
        }
        let mut sorted: Vec<f64> = items.iter().cloned().map(ToF64::to_f64).collect();
        sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));

        let mut estimates: [f64; 101] = [0.0; 101];
        let q_start = Instant::now();
        for (i, slot) in estimates.iter_mut().enumerate() {
            let q = i as f64 / 100.0;
            *slot = sketch.estimate_quantile(q);
        }
        let q_ns = q_start.elapsed().as_nanos() as u64;

        let query_calls = if self.record_calls {
            Some(capture_quantile_calls(sketch))
        } else {
            None
        };

        let mut max_rel_err = 0.0_f64;
        let mut sum_rel = 0.0_f64;
        let mut n_rel = 0usize;
        for (i, est) in estimates.iter().enumerate() {
            let q = i as f64 / 100.0;
            let truth = type7_quantile(&sorted, q);
            // Skip grid points where truth is ~0 — relative
            // error is undefined there. DDSketch's guarantee is
            // for non-zero quantiles anyway.
            if truth.abs() <= f64::EPSILON {
                continue;
            }
            let rel = (est - truth).abs() / truth.abs();
            sum_rel += rel;
            n_rel += 1;
            if rel > max_rel_err {
                max_rel_err = rel;
            }
        }
        let mean = if n_rel == 0 {
            0.0
        } else {
            sum_rel / n_rel as f64
        };

        Comparison {
            metrics: metrics_from([
                ("items", items.len() as f64),
                ("grid_points", NUM_PERCENTILES as f64),
                ("evaluated_points", n_rel as f64),
                ("mean_relative_err", mean),
                ("max_relative_err", max_rel_err),
            ]),
            queries: 101,
            query_wall_ns: q_ns,
            query_calls,
        }
    }
}

// ---------- helpers ----------

/// Count of elements strictly less than `x` in a sorted slice.
fn lower_bound(sorted: &[f64], x: f64) -> usize {
    let mut lo = 0usize;
    let mut hi = sorted.len();
    while lo < hi {
        let mid = (lo + hi) / 2;
        if sorted[mid] < x {
            lo = mid + 1;
        } else {
            hi = mid;
        }
    }
    lo
}

/// Count of elements `<= x` in a sorted slice.
fn upper_bound(sorted: &[f64], x: f64) -> usize {
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
    lo
}

/// Independently-time each `estimate_quantile` call across
/// `RAW_REPEATS_PER_RUN` × `NUM_PERCENTILES` for the per-call CSV shape. The
/// sweep is by `p`, so the recorded `percentile` is `p as f64 / 100.0`.
fn capture_quantile_calls<S>(sketch: &S) -> Vec<QueryCallSample>
where
    S: QuantileOps,
{
    let mut samples = Vec::with_capacity(RAW_REPEATS_PER_RUN * NUM_PERCENTILES);
    let mut call_index = 0usize;
    for repeat in 1..=RAW_REPEATS_PER_RUN {
        for p in 0..NUM_PERCENTILES {
            call_index += 1;
            let rank = p as f64 / 100.0;
            let t0 = Instant::now();
            let q = sketch.estimate_quantile(rank);
            let ns = t0.elapsed().as_nanos() as u64;
            std::hint::black_box(&q);
            samples.push(QueryCallSample {
                call_index,
                nanoseconds: ns,
                estimate: q,
                percentile: rank,
                repeat,
            });
        }
    }
    samples
}

/// Type-7 linear interpolation on a pre-sorted f64 slice — the
/// NumPy / R / Prometheus default quantile. This is the ground
/// truth `RelativeErrorGT` scores DDSketch against.
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

/// Build the flat metric map a `Comparison` carries.
fn metrics_from<const N: usize>(pairs: [(&str, f64); N]) -> BTreeMap<String, f64> {
    pairs.into_iter().map(|(k, v)| (k.to_string(), v)).collect()
}
