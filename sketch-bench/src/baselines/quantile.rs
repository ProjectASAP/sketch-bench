//! Exact quantile baseline — `Vec<i64>` sorted on first query.
//! Provides ground truth for any sketch family that answers a
//! quantile / rank query. Shared between all quantile sketches.
//!
//! Quantile definition: **Type-7 linear interpolation** (NumPy
//! / R / Prometheus default). When `q*(n-1)` falls between two
//! adjacent samples, the answer is their weighted average. The
//! accuracy comparator uses this same formula on its own sorted
//! copy, so the exact baseline sees zero value-error against
//! ground truth by construction.
//!
//! Current consumers: `kll`, `dd` (DDSketch) — see
//! `baselines::Statistic::Quantile`.

use std::cell::{Cell, RefCell};

use sketch_core::config::KllParams;
use sketch_core::sketch::{MergeUnsupported, Sketch};

#[derive(Debug, Default)]
pub struct ExactQuantile {
    buf: RefCell<Vec<i64>>,
    sorted: Cell<bool>,
}

impl ExactQuantile {
    /// Accepts a `KllParams` for dispatch-macro uniformity; the
    /// value is ignored — an exact sorted stream has no k.
    pub fn new(_p: &KllParams) -> Self {
        Self {
            buf: RefCell::new(Vec::new()),
            sorted: Cell::new(false),
        }
    }

    /// Batch ingest. Sorts eagerly so subsequent quantile calls
    /// are amortised — matches how `accuracy/{kll,dd}` pre-sorted
    /// in `BaselineData`.
    pub fn ingest_all(values: &[i64]) -> Self {
        let mut buf = values.to_vec();
        buf.sort_unstable();
        Self {
            buf: RefCell::new(buf),
            sorted: Cell::new(true),
        }
    }

    /// Type-7 ground-truth quantile at percentile `p` (0..=100).
    pub fn ground_truth_quantile(&self, p: usize) -> f64 {
        assert!(p <= 100, "percentile must be 0..=100");
        self.quantile_fraction(p as f64 / 100.0)
    }

    /// Type-7 linear-interpolation quantile keyed by fraction
    /// `q ∈ [0, 1]`. NaN / out-of-range inputs follow the
    /// Prometheus reference: NaN→NaN, q<0→-Inf, q>1→+Inf.
    pub fn quantile_fraction(&self, q: f64) -> f64 {
        let mut buf = self.buf.borrow_mut();
        if buf.is_empty() {
            return f64::NAN;
        }
        if !self.sorted.get() {
            buf.sort_unstable();
            self.sorted.set(true);
        }
        if q.is_nan() {
            return f64::NAN;
        }
        if q < 0.0 {
            return f64::NEG_INFINITY;
        }
        if q > 1.0 {
            return f64::INFINITY;
        }
        let n = buf.len();
        let rank = q * (n - 1) as f64;
        let lower = rank.floor() as usize;
        let upper = (lower + 1).min(n - 1);
        let weight = rank - rank.floor();
        let lo = buf[lower] as f64;
        let hi = buf[upper] as f64;
        lo * (1.0 - weight) + hi * weight
    }

    pub fn total_items(&self) -> usize {
        self.buf.borrow().len()
    }
}

impl Sketch for ExactQuantile {
    type Item = i64;
    type Query = f64;
    type Answer = f64;

    fn update(&mut self, v: &i64) {
        self.buf.get_mut().push(*v);
        self.sorted.set(false);
    }

    fn query(&self, q: f64) -> f64 {
        self.quantile_fraction(q)
    }

    fn memory_bytes(&self) -> usize {
        self.buf.borrow().capacity() * std::mem::size_of::<i64>()
    }

    /// Concatenate the retained values. Exact: the multiset union of two
    /// samples is the sample of the union, so unlike KLL the baseline loses
    /// nothing to merging and gives the merge benchmark a zero-error floor.
    fn merge(&mut self, other: &Self) -> Result<(), MergeUnsupported> {
        self.buf.borrow_mut().extend_from_slice(&other.buf.borrow());
        self.sorted.set(false);
        Ok(())
    }

    /// Sort eagerly so the query phase only pays Type-7 lookup
    /// cost. Mirrors how KLL/DD lib sketches maintain a queryable
    /// structure during update.
    fn finalize_for_query(&mut self) {
        if !self.sorted.get() {
            self.buf.get_mut().sort_unstable();
            self.sorted.set(true);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn type7_quantile_at_grid_points() {
        let vals: Vec<i64> = (1..=101).collect();
        let b = ExactQuantile::ingest_all(&vals);
        // Each percentile maps exactly onto a grid index, so
        // linear interpolation collapses to the sorted value.
        assert_eq!(b.ground_truth_quantile(0), 1.0);
        assert_eq!(b.ground_truth_quantile(50), 51.0);
        assert_eq!(b.ground_truth_quantile(100), 101.0);
    }

    #[test]
    fn type7_interpolates_between_adjacent_samples() {
        // Two-element vec: q=0.5 falls between, expect the mean.
        let b = ExactQuantile::ingest_all(&[10i64, 20]);
        assert_eq!(b.quantile_fraction(0.0), 10.0);
        assert_eq!(b.quantile_fraction(1.0), 20.0);
        assert!((b.quantile_fraction(0.5) - 15.0).abs() < 1e-9);
        assert!((b.quantile_fraction(0.25) - 12.5).abs() < 1e-9);
    }

    #[test]
    fn streamed_quantile_matches_batch() {
        let vals: Vec<i64> = (0..1000).rev().collect();
        let batch = ExactQuantile::ingest_all(&vals);
        let mut streamed = ExactQuantile::new(&KllParams { k: 200 });
        for v in &vals {
            streamed.update(v);
        }
        for p in [0, 25, 50, 75, 100] {
            assert_eq!(
                batch.ground_truth_quantile(p),
                streamed.ground_truth_quantile(p),
                "percentile {p}"
            );
        }
    }
}
