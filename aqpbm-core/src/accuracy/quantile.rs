//! Quantile-algorithm ground truth comparators, one metric per algorithm, because
//! each sketch's correctness bound is defined differently in its paper.
//! [`RankErrorGT`] (KLL) reports rank error in units of `n`, range-based over
//! tie intervals; [`RelativeErrorGT`] (DDSketch) reports `|v̂ − v| / |v|`
//! against the Type-7 linear-interpolation quantile ([`type7_quantile`]).

use std::collections::BTreeMap;

use super::GroundTruth;

/// Number of times the 101-percentile sweep is repeated when `record_calls`
/// is on, matching the KLL / DD query binaries' `REPEATS_PER_RUN = 10`.
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
}

impl<I> GroundTruth<I> for RankErrorGT
where
    I: Clone + PartialOrd + ToF64,
{
    /// Every value the stream carried, sorted. Rank is over occurrences, so
    /// nothing is deduplicated.
    type Truth = Vec<f64>;
    /// One fraction of the grid.
    type Probe = f64;
    type Answer = f64;

    fn truth(&self, items: &[I]) -> Vec<f64> {
        let mut sorted: Vec<f64> = items.iter().cloned().map(ToF64::to_f64).collect();
        sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        sorted
    }

    fn probes(&self, truth: &Vec<f64>) -> Vec<f64> {
        if truth.is_empty() {
            return Vec::new();
        }
        (0..NUM_PERCENTILES).map(|i| i as f64 / 100.0).collect()
    }

    fn probe_as_f64(&self, probe: &f64) -> f64 {
        *probe
    }

    fn answer_as_f64(&self, answer: &f64) -> f64 {
        *answer
    }

    fn score(&self, truth: &Vec<f64>, probes: &[f64], answers: &[f64]) -> BTreeMap<String, f64> {
        if truth.is_empty() || probes.is_empty() {
            return metrics_from([("items", 0.0), ("mean_rank_err", 0.0)]);
        }
        let nf = truth.len() as f64;
        let mut max_rank_err = 0.0_f64;
        let mut sum_rank_err = 0.0_f64;
        for (q, est) in probes.iter().zip(answers) {
            // The returned value occupies the rank interval `[lower, upper]`:
            // error 0 if `q*n` falls inside, else distance to the near edge.
            let lower = lower_bound(truth, *est);
            let upper = upper_bound(truth, *est);
            let target = q * nf;
            let raw_err = if target < lower as f64 {
                lower as f64 - target
            } else if target > upper as f64 {
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
        metrics_from([
            ("items", nf),
            ("grid_points", NUM_PERCENTILES as f64),
            ("mean_rank_err", sum_rank_err / probes.len() as f64),
            ("max_rank_err", max_rank_err),
        ])
    }
}

// ---------- DDSketch: relative error ----------

/// Relative-error comparator for DDSketch-style sketches.
#[derive(Debug, Default, Clone, Copy)]
pub struct RelativeErrorGT {
}

impl<I> GroundTruth<I> for RelativeErrorGT
where
    I: Clone + PartialOrd + ToF64,
{
    type Truth = Vec<f64>;
    type Probe = f64;
    type Answer = f64;

    fn truth(&self, items: &[I]) -> Vec<f64> {
        let mut sorted: Vec<f64> = items.iter().cloned().map(ToF64::to_f64).collect();
        sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        sorted
    }

    fn probes(&self, truth: &Vec<f64>) -> Vec<f64> {
        if truth.is_empty() {
            return Vec::new();
        }
        (0..NUM_PERCENTILES).map(|i| i as f64 / 100.0).collect()
    }

    fn probe_as_f64(&self, probe: &f64) -> f64 {
        *probe
    }

    fn answer_as_f64(&self, answer: &f64) -> f64 {
        *answer
    }

    fn score(&self, truth: &Vec<f64>, probes: &[f64], answers: &[f64]) -> BTreeMap<String, f64> {
        if truth.is_empty() || probes.is_empty() {
            return metrics_from([("items", 0.0), ("mean_relative_err", 0.0)]);
        }
        let mut max_rel_err = 0.0_f64;
        let mut sum_rel = 0.0_f64;
        let mut n_rel = 0usize;
        for (q, est) in probes.iter().zip(answers) {
            let t = type7_quantile(truth, *q);
            // Skip grid points where truth is ~0 — relative error is undefined
            // there, and DDSketch's guarantee is for non-zero quantiles anyway.
            if t.abs() <= f64::EPSILON {
                continue;
            }
            let rel = (est - t).abs() / t.abs();
            sum_rel += rel;
            n_rel += 1;
            if rel > max_rel_err {
                max_rel_err = rel;
            }
        }
        metrics_from([
            ("items", truth.len() as f64),
            ("grid_points", NUM_PERCENTILES as f64),
            ("evaluated_points", n_rel as f64),
            (
                "mean_relative_err",
                if n_rel == 0 { 0.0 } else { sum_rel / n_rel as f64 },
            ),
            ("max_relative_err", max_rel_err),
        ])
    }
}

// ---------- helpers ----------

/// Count of elements strictly less than `x` in a sorted slice.
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

pub(crate) fn lower_bound(sorted: &[f64], x: f64) -> usize {
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
pub(crate) fn upper_bound(sorted: &[f64], x: f64) -> usize {
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


/// Build the flat metric map a `Comparison` carries.
fn metrics_from<const N: usize>(pairs: [(&str, f64); N]) -> BTreeMap<String, f64> {
    pairs.into_iter().map(|(k, v)| (k.to_string(), v)).collect()
}
