//! Quantile-algorithm ground truth comparators, one metric per algorithm, because
//! each sketch's correctness bound is defined differently in its paper.
//! [`RankErrorGT`] (KLL) reports rank error in units of `n`, range-based over
//! tie intervals.

use std::collections::BTreeMap;

use super::GroundTruth;

/// How many points the quantile grid has: `0.00, 0.01, … 1.00`. Shared with the
/// grouped rank-error comparator, so a grouped rank error and an ungrouped one
/// are read on the same ruler.
pub(crate) const GRID_POINTS: usize = 101;

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
pub struct RankErrorGT;

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
        (0..GRID_POINTS).map(|i| i as f64 / 100.0).collect()
    }

    fn score(&self, truth: &Vec<f64>, probes: &[f64], answers: &[f64]) -> BTreeMap<String, f64> {
        if truth.is_empty() || probes.is_empty() {
            return metrics_from([("items", 0.0), ("mean_rank_err", 0.0)]);
        }
        let nf = truth.len() as f64;
        let mut max_rank_err = 0.0_f64;
        let mut sum_rank_err = 0.0_f64;
        for (q, est) in probes.iter().zip(answers) {
            let err = rank_err(truth, *est, *q);
            sum_rank_err += err;
            if err > max_rank_err {
                max_rank_err = err;
            }
        }
        metrics_from([
            ("items", nf),
            ("grid_points", GRID_POINTS as f64),
            ("mean_rank_err", sum_rank_err / probes.len() as f64),
            ("max_rank_err", max_rank_err),
        ])
    }
}

// ---------- helpers ----------

/// Rank error of one answer, in units of `n`. The returned value occupies the
/// rank interval `[lower, upper]` — every rank a tie run spans — so a target
/// inside it is not an error, and outside it the distance to the near edge.
pub(crate) fn rank_err(sorted: &[f64], est: f64, q: f64) -> f64 {
    let nf = sorted.len() as f64;
    if nf == 0.0 {
        return 0.0;
    }
    let target = q * nf;
    let lower = lower_bound(sorted, est) as f64;
    let upper = upper_bound(sorted, est) as f64;
    let raw = if target < lower {
        lower - target
    } else if target > upper {
        target - upper
    } else {
        0.0
    };
    raw / nf
}

/// Count of elements strictly less than `x` in a sorted slice.
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

/// Build the flat metric map a comparator's `score` returns.
fn metrics_from<const N: usize>(pairs: [(&str, f64); N]) -> BTreeMap<String, f64> {
    pairs.into_iter().map(|(k, v)| (k.to_string(), v)).collect()
}
