//! Exact-baseline wrappers used by `dispatch.rs`.
//!
//! The algorithms live in [`sketch_bench::baselines`], organised
//! by statistic (cardinality / frequency / quantile). This file
//! re-exports them and adds CLI-side newtype wrappers when a
//! family's `ParamSet` variant differs from the baseline's
//! constructor signature (e.g. CountSketch shares
//! `ExactFrequency` with CMS; DDSketch shares `ExactQuantile`
//! with KLL).

use sketch_core::config::{CountSketchParams, DdParams};
use sketch_core::sketch::Sketch;

pub use sketch_bench::baselines::{ExactCardinality, ExactFrequency, ExactQuantile};

/// CountSketch view of [`ExactFrequency`]. Same map-of-counts
/// algorithm as the CMS exact baseline; the wrapper exists only
/// so the dispatch macro can call `<W>::new(&CountSketchParams)`
/// uniformly with the rest of the family.
#[derive(Debug, Default)]
pub struct ExactFrequencyCs(pub ExactFrequency);

impl ExactFrequencyCs {
    pub fn new(_p: &CountSketchParams) -> Self {
        Self(ExactFrequency::default())
    }
}

impl Sketch for ExactFrequencyCs {
    type Item = i64;
    type Query = i64;
    type Answer = u64;
    #[inline]
    fn update(&mut self, v: &i64) {
        self.0.update(v);
    }
    fn query(&self, q: i64) -> u64 {
        self.0.query(q)
    }
    fn memory_bytes(&self) -> usize {
        self.0.memory_bytes()
    }
    fn finalize_for_query(&mut self) {
        self.0.finalize_for_query();
    }
}

/// DDSketch view of [`ExactQuantile`]. Same sorted-stream
/// algorithm as the KLL exact baseline; wraps it so the dispatch
/// macro can construct it from `&DdParams`.
#[derive(Debug, Default)]
pub struct ExactQuantileDd(pub ExactQuantile);

impl ExactQuantileDd {
    pub fn new(_p: &DdParams) -> Self {
        Self(ExactQuantile::default())
    }
}

impl Sketch for ExactQuantileDd {
    type Item = i64;
    type Query = f64;
    type Answer = f64;
    #[inline]
    fn update(&mut self, v: &i64) {
        self.0.update(v);
    }
    fn query(&self, q: f64) -> f64 {
        self.0.query(q)
    }
    fn memory_bytes(&self) -> usize {
        self.0.memory_bytes()
    }
    fn finalize_for_query(&mut self) {
        self.0.finalize_for_query();
    }
}
