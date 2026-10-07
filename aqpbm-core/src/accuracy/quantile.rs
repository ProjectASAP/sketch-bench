//! Quantile-algorithm ground truth comparators, one metric per algorithm, because
//! each sketch's correctness bound is defined differently in its paper.
//! [`RankErrorGT`] (KLL) reports rank error in units of `n`, range-based over
//! tie intervals. [`RelativeValueErrorGT`] (DDSketch) reports relative value
//! error against the exact order statistic each DDSketch implementation asks.

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

// ---------- DDSketch: relative value error ----------

/// Relative-value-error comparator for DDSketch-style sketches.
///
/// Each probe's denominator is the exact quantile value at the zero-based rank
/// `ceil(q * n) - 1`; `q = 0` names the first item. Both DDSketch backends use
/// that convention. Exact zeroes have undefined relative error and are
/// excluded from the mean.
#[derive(Debug, Default, Clone, Copy)]
pub struct RelativeValueErrorGT {
    pub column: usize,
}

impl GroundTruth for RelativeValueErrorGT {
    type Truth = Vec<f64>;
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
            return metrics_from([("items", 0.0), ("grid_points", 0.0)]);
        }
        let mut sum = 0.0;
        let mut scored = 0usize;
        for (q, estimate) in probes.iter().zip(answers) {
            let exact = quantile_value(truth, *q);
            if exact == 0.0 {
                continue;
            }
            sum += (estimate - exact).abs() / exact.abs();
            scored += 1;
        }
        let mut metrics = metrics_from([
            ("items", truth.len() as f64),
            ("grid_points", scored as f64),
        ]);
        if scored > 0 {
            metrics.insert("mean_relative_value_error".into(), sum / scored as f64);
        }
        metrics
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

fn quantile_value(sorted: &[f64], q: f64) -> f64 {
    let rank = if q == 0.0 {
        1
    } else {
        (q * sorted.len() as f64).ceil() as usize
    };
    sorted[rank - 1]
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relative_value_error_averages_nonzero_exact_quantiles_only() {
        let gt = RelativeValueErrorGT { column: 0 };
        let truth = vec![0.0, 10.0, 20.0];
        let scores = gt.score(&truth, &[0.0, 0.5, 1.0], &[100.0, 11.0, 16.0]);

        assert_eq!(scores["items"], 3.0);
        assert_eq!(scores["grid_points"], 2.0);
        assert!((scores["mean_relative_value_error"] - 0.15).abs() < f64::EPSILON);
        assert!(!scores.contains_key("mean_rank_err"));
    }

    #[test]
    fn relative_value_error_omits_the_mean_when_every_exact_quantile_is_zero() {
        let gt = RelativeValueErrorGT { column: 0 };
        let scores = gt.score(&vec![0.0; 3], &[0.0, 0.5, 1.0], &[0.0, 1.0, -1.0]);

        assert_eq!(scores["grid_points"], 0.0);
        assert!(!scores.contains_key("mean_relative_value_error"));
    }

    #[test]
    fn relative_value_error_uses_ddsketchs_quantile_rank() {
        let gt = RelativeValueErrorGT { column: 0 };
        let scores = gt.score(&vec![10.0, 20.0, 40.0, 80.0], &[0.3], &[24.0]);

        // ceil(0.3 * 4) - 1 is 1, so the exact value is 20, not 10.
        assert!((scores["mean_relative_value_error"] - 0.2).abs() < f64::EPSILON);
    }

    #[test]
    fn relative_value_error_uses_the_first_item_at_zero_quantile() {
        let gt = RelativeValueErrorGT { column: 0 };
        let scores = gt.score(&vec![10.0, 20.0, 40.0], &[0.0], &[11.0]);

        assert!((scores["mean_relative_value_error"] - 0.1).abs() < f64::EPSILON);
    }

    #[test]
    fn relative_value_error_uses_the_last_item_at_one_quantile() {
        let gt = RelativeValueErrorGT { column: 0 };
        let scores = gt.score(&vec![10.0, 20.0, 40.0], &[1.0], &[44.0]);

        assert!((scores["mean_relative_value_error"] - 0.1).abs() < f64::EPSILON);
    }
}
