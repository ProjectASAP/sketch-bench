//! Exact quantile baseline — `Vec<i64>` sorted on first query.
//! Implements `Sketch<Item=i64, Query=f64, Answer=f64>`.
//!
//! Shared by KLL and DDSketch accuracy harnesses (both want
//! nearest-rank quantiles on the sorted stream).

use std::cell::{Cell, RefCell};

use sketch_core::config::KllParams;
use sketch_core::sketch::Sketch;

#[derive(Debug, Default)]
pub struct ExactKll {
    buf: RefCell<Vec<i64>>,
    sorted: Cell<bool>,
}

impl ExactKll {
    /// Accepts a `KllParams` for dispatch-macro uniformity; the
    /// value is ignored — an exact sorted stream has no k.
    pub fn new(_p: &KllParams) -> Self {
        Self {
            buf: RefCell::new(Vec::new()),
            sorted: Cell::new(false),
        }
    }

    /// Batch ingest. Sorts eagerly so subsequent quantile calls
    /// are amortised — matches how `accuracy/kll` pre-sorted in
    /// `BaselineData`.
    pub fn ingest_all(values: &[i64]) -> Self {
        let mut buf = values.to_vec();
        buf.sort_unstable();
        Self {
            buf: RefCell::new(buf),
            sorted: Cell::new(true),
        }
    }

    /// Nearest-rank ground-truth quantile at percentile `p`
    /// (0..=100). Matches the interpolation used by the
    /// `accuracy/{kll,dd}` harnesses.
    pub fn ground_truth_quantile(&self, p: usize) -> f64 {
        assert!(p <= 100, "percentile must be 0..=100");
        self.quantile_fraction(p as f64 / 100.0)
    }

    /// Same, but keyed by fractional quantile `q ∈ [0, 1]`.
    pub fn quantile_fraction(&self, q: f64) -> f64 {
        let mut buf = self.buf.borrow_mut();
        if buf.is_empty() {
            return f64::NAN;
        }
        if !self.sorted.get() {
            buf.sort_unstable();
            self.sorted.set(true);
        }
        let n = buf.len();
        let q = q.clamp(0.0, 1.0);
        let idx = ((q * (n - 1) as f64).round() as usize).min(n - 1);
        buf[idx] as f64
    }

    pub fn total_items(&self) -> usize {
        self.buf.borrow().len()
    }
}

impl Sketch for ExactKll {
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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nearest_rank_matches_sorted_index() {
        let vals: Vec<i64> = (1..=101).collect();
        let b = ExactKll::ingest_all(&vals);
        assert_eq!(b.ground_truth_quantile(0), 1.0);
        assert_eq!(b.ground_truth_quantile(50), 51.0);
        assert_eq!(b.ground_truth_quantile(100), 101.0);
    }

    #[test]
    fn streamed_quantile_matches_batch() {
        let vals: Vec<i64> = (0..1000).rev().collect();
        let batch = ExactKll::ingest_all(&vals);
        let mut streamed = ExactKll::new(&KllParams { k: 200 });
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
