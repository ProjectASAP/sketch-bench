//! Quantile-algorithm ground truth comparators, one metric per algorithm, because
//! each sketch's correctness bound is defined differently in its paper.
//! [`RankErrorGT`] (KLL) reports rank error in units of `n`, range-based over
//! tie intervals.

use std::collections::BTreeMap;

use aqpbm_datagen::{DataGenError, GeneratedTable};

use super::{f64_values, GroundTruth};

/// How many points the quantile grid has: `0.00, 0.01, … 1.00`. Shared with the
/// grouped rank-error comparator, so a grouped rank error and an ungrouped one
/// are read on the same ruler.
pub(crate) const GRID_POINTS: usize = 101;

// ---------- KLL: rank error ----------

/// Rank-error comparator for KLL-style sketches.
#[derive(Debug, Default, Clone, Copy)]
pub struct RankErrorGT {
    pub column: usize,
}

impl GroundTruth for RankErrorGT {
    /// Every value the stream carried, sorted. Rank is over occurrences, so
    /// nothing is deduplicated.
    type Truth = Vec<f64>;
    /// One fraction of the grid.
    type Probe = f64;
    type Answer = f64;

    fn truth(&self, table: &GeneratedTable) -> Result<Vec<f64>, DataGenError> {
        let mut sorted = f64_values(table.column(self.column)?)?;
        sorted.sort_by(f64::total_cmp);
        Ok(sorted)
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
/// rank interval `[lower, upper]` — every rank a tie run spans — so a row
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
