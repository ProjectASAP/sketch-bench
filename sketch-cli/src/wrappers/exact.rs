//! Exact-baseline wrappers used by `dispatch.rs`.
//!
//! The algorithms live in [`sketch_bench::baselines`], organised
//! by statistic (cardinality / frequency / quantile). This file
//! re-exports them and adds CLI-side newtype wrappers when a
//! family's `ParamSet` variant differs from the baseline's
//! constructor signature (e.g. CountSketch shares
//! `ExactFrequency` with CMS; DDSketch shares `ExactQuantile`
//! with KLL).

use crate::params::{CmsParams, CountSketchParams, DdParams, HllParams};
use sketch_bench::accuracy::quantile::QuantileValue;
use sketch_core::sketch::{MergeUnsupported, Sketch};

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
    #[inline(always)]
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
#[derive(Debug)]
pub struct ExactQuantileDd<T = i64>(pub ExactQuantile<T>);

impl<T> Default for ExactQuantileDd<T> {
    fn default() -> Self {
        Self(ExactQuantile::default())
    }
}

impl<T: QuantileValue> ExactQuantileDd<T> {
    pub fn new(_p: &DdParams) -> Self {
        Self(ExactQuantile::default())
    }
}

impl<T: QuantileValue> Sketch for ExactQuantileDd<T> {
    type Item = T;
    type Query = f64;
    type Answer = f64;
    #[inline(always)]
    fn update(&mut self, v: &T) {
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

// ---------- null baselines: the estimator that does no work ----------
//
// These answer 0 for every query. They exist as permanent rows in the matrix
// because they pin the scale of every accuracy metric: for both frequency and
// cardinality, answering 0 scores an average relative error of **exactly
// 1.0**, since |0 - f| / f = 1 for every key with a positive true count.
//
// That constant is the disqualifying threshold. Any metric under which a real
// sketch scores worse than `null` is measuring the wrong population, not
// exposing a bad sketch — the trap SALSA (ICDE 2021) names outright: "for CMS,
// and this dataset, it is better to estimate all sizes as 0 without performing
// any measurement". This repo's own unfiltered ARE hit 22.31 for a working
// Count-Min Sketch, i.e. 22x worse than these rows, which is what motivated
// reporting error over top-k prefixes instead.
//
// Keeping them in the sweep makes that check automatic rather than something
// a reader has to remember to perform.

/// Frequency: every count is zero.
#[derive(Default)]
pub struct NullFrequency;

impl NullFrequency {
    pub fn new(_p: &CmsParams) -> Self {
        Self
    }
}

impl Sketch for NullFrequency {
    type Item = i64;
    type Query = i64;
    type Answer = u64;
    #[inline(always)]
    fn update(&mut self, _: &i64) {}
    fn query(&self, _: i64) -> u64 {
        0
    }
    fn memory_bytes(&self) -> usize {
        0
    }

    /// Merging two empty summaries yields an empty summary. Supported so the
    /// null row still anchors the 1.0 reference in a merge sweep.
    fn merge(&mut self, _other: &Self) -> Result<(), MergeUnsupported> {
        Ok(())
    }
}

/// Same, registered under CountSketch's `ParamSet` variant.
#[derive(Default)]
pub struct NullFrequencyCs;

impl NullFrequencyCs {
    pub fn new(_p: &CountSketchParams) -> Self {
        Self
    }
}

impl Sketch for NullFrequencyCs {
    type Item = i64;
    type Query = i64;
    type Answer = u64;
    #[inline(always)]
    fn update(&mut self, _: &i64) {}
    fn query(&self, _: i64) -> u64 {
        0
    }
    fn memory_bytes(&self) -> usize {
        0
    }

    /// Merging two empty summaries yields an empty summary. Supported so the
    /// null row still anchors the 1.0 reference in a merge sweep.
    fn merge(&mut self, _other: &Self) -> Result<(), MergeUnsupported> {
        Ok(())
    }
}

/// Cardinality: the estimate is always zero, so relative error is 1.0.
#[derive(Default)]
pub struct NullCardinality;

impl NullCardinality {
    pub fn new(_p: &HllParams) -> Self {
        Self
    }
}

impl Sketch for NullCardinality {
    type Item = i64;
    type Query = ();
    type Answer = f64;
    #[inline(always)]
    fn update(&mut self, _: &i64) {}
    fn query(&self, _: ()) -> f64 {
        0.0
    }
    fn memory_bytes(&self) -> usize {
        0
    }

    /// Merging two empty summaries yields an empty summary. Supported so the
    /// null row still anchors the 1.0 reference in a merge sweep.
    fn merge(&mut self, _other: &Self) -> Result<(), MergeUnsupported> {
        Ok(())
    }
}
