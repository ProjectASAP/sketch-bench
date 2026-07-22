//! CountSketch wrappers — 4 variants (`oxide` + 3× sketchlib).
//! Same family as CMS: `Query = i64`, `Answer = u64`.

use asap_sketchlib::{Count, DataInput, FastPath, FixedMatrix, RegularPath, Vector2D};
use sketch_core::config::CountSketchParams;
use sketch_core::sketch::{MergeUnsupported, Sketch};
use sketch_oxide::Mergeable as _;

use crate::wrappers::cms::{
    CountMinMatrix5x32K, CMS_FIXED_32K_COLS, CMS_FIXED_32K_ROWS, CMS_FIXED_COLS, CMS_FIXED_ROWS,
};

// sketch_oxide::frequency::CountSketch sizes its table as
// `width = ceil(3/ε²).next_power_of_two()`, not `ceil(2/ε)` like CountMin.
// Inverting that for a target column count w gives ε = sqrt(3/w).
fn dims_to_err(rows: usize, cols: usize) -> (f64, f64) {
    let epsilon = (3.0 / cols as f64).sqrt();
    let delta = (-(rows as f64)).exp();
    (epsilon, delta)
}

// ---------- sketch_oxide CountSketch ----------
pub struct CsOxide {
    inner: sketch_oxide::frequency::CountSketch,
    rows: usize,
    cols: usize,
}

impl CsOxide {
    pub fn new(p: &CountSketchParams) -> Self {
        let (epsilon, delta) = dims_to_err(p.rows, p.cols);
        Self {
            inner: sketch_oxide::frequency::CountSketch::new(epsilon, delta)
                .expect("valid CountSketch parameters"),
            rows: p.rows,
            cols: p.cols,
        }
    }
}

impl Sketch for CsOxide {
    type Item = i64;
    type Query = i64;
    type Answer = u64;
    #[inline(always)]
    fn update(&mut self, v: &i64) {
        self.inner.update(v, 1);
    }
    fn query(&self, q: i64) -> u64 {
        // CountSketch is an unbiased estimator (median of sign·counter); a
        // small fraction of estimates can be slightly negative under collision
        // noise. Clamp to 0 to match CMS-style frequency semantics — without
        // this, `as u64` wraps -1 into u64::MAX and blows up rel-err.
        self.inner.estimate(&q).max(0) as u64
    }
    fn memory_bytes(&self) -> usize {
        self.rows * self.cols * std::mem::size_of::<i64>()
    }

    /// Counter-wise addition; Count Sketch is linear, so merging is exact.
    fn merge(&mut self, other: &Self) -> Result<(), MergeUnsupported> {
        self.inner
            .merge(&other.inner)
            .expect("both operands built from one ParamSet, so rows/cols match");
        Ok(())
    }
}

// ---------- asap_sketchlib: FixedMatrix + FastPath ----------
// Compile-time fixed at (5, 2048). Dispatch marks this as
// fixed-shape; sweep runs it only when requested shape matches.
pub struct CsLibFixedmatrixFast(pub Count<FixedMatrix, FastPath>);

impl CsLibFixedmatrixFast {
    pub fn new(_p: &CountSketchParams) -> Self {
        Self(Count::<FixedMatrix, FastPath>::default())
    }
}

impl Sketch for CsLibFixedmatrixFast {
    type Item = i64;
    type Query = i64;
    type Answer = u64;
    #[inline(always)]
    fn update(&mut self, v: &i64) {
        self.0.insert(&DataInput::I64(*v));
    }
    fn query(&self, q: i64) -> u64 {
        self.0.estimate(&DataInput::I64(q)) as u64
    }
    fn memory_bytes(&self) -> usize {
        CMS_FIXED_ROWS * CMS_FIXED_COLS * std::mem::size_of::<i32>()
    }

    /// Counter-wise addition; Count Sketch is linear, so merging is exact.
    fn merge(&mut self, other: &Self) -> Result<(), MergeUnsupported> {
        self.0.merge(&other.0);
        Ok(())
    }
}

// ---------- asap_sketchlib: FixedMatrix 5x32768 + FastPath ----------
// Same code path as the 5x2048 FixedMatrix variant, at 5x32768
// to match the CMS+CS 32K throughput panel. Reuses the
// `CountMinMatrix5x32K` shape declared in cms.rs.
pub struct CsLibFixedmatrixFast32k(pub Count<CountMinMatrix5x32K, FastPath>);

impl CsLibFixedmatrixFast32k {
    pub fn new(_p: &CountSketchParams) -> Self {
        Self(Count::<CountMinMatrix5x32K, FastPath>::from_storage(
            CountMinMatrix5x32K::default(),
        ))
    }
}

impl Sketch for CsLibFixedmatrixFast32k {
    type Item = i64;
    type Query = i64;
    type Answer = u64;
    #[inline(always)]
    fn update(&mut self, v: &i64) {
        self.0.insert(&DataInput::I64(*v));
    }
    fn query(&self, q: i64) -> u64 {
        self.0.estimate(&DataInput::I64(q)) as u64
    }
    fn memory_bytes(&self) -> usize {
        CMS_FIXED_32K_ROWS * CMS_FIXED_32K_COLS * std::mem::size_of::<i32>()
    }

    /// Counter-wise addition; Count Sketch is linear, so merging is exact.
    fn merge(&mut self, other: &Self) -> Result<(), MergeUnsupported> {
        self.0.merge(&other.0);
        Ok(())
    }
}

// ---------- asap_sketchlib: Vector2D + FastPath ----------
pub struct CsLibVector2dFast {
    inner: Count<Vector2D<i32>, FastPath>,
    rows: usize,
    cols: usize,
}

impl CsLibVector2dFast {
    pub fn new(p: &CountSketchParams) -> Self {
        Self {
            inner: Count::<Vector2D<i32>, FastPath>::with_dimensions(p.rows, p.cols),
            rows: p.rows,
            cols: p.cols,
        }
    }
}

impl Sketch for CsLibVector2dFast {
    type Item = i64;
    type Query = i64;
    type Answer = u64;
    #[inline(always)]
    fn update(&mut self, v: &i64) {
        self.inner.insert(&DataInput::I64(*v));
    }
    fn query(&self, q: i64) -> u64 {
        self.inner.estimate(&DataInput::I64(q)) as u64
    }
    fn memory_bytes(&self) -> usize {
        self.rows * self.cols * std::mem::size_of::<i32>()
    }

    /// Counter-wise addition; Count Sketch is linear, so merging is exact.
    fn merge(&mut self, other: &Self) -> Result<(), MergeUnsupported> {
        self.inner.merge(&other.inner);
        Ok(())
    }
}

// ---------- asap_sketchlib: Vector2D + RegularPath ----------
pub struct CsLibVector2dRegular {
    inner: Count<Vector2D<i32>, RegularPath>,
    rows: usize,
    cols: usize,
}

impl CsLibVector2dRegular {
    pub fn new(p: &CountSketchParams) -> Self {
        Self {
            inner: Count::<Vector2D<i32>, RegularPath>::with_dimensions(p.rows, p.cols),
            rows: p.rows,
            cols: p.cols,
        }
    }
}

impl Sketch for CsLibVector2dRegular {
    type Item = i64;
    type Query = i64;
    type Answer = u64;
    #[inline(always)]
    fn update(&mut self, v: &i64) {
        self.inner.insert(&DataInput::I64(*v));
    }
    fn query(&self, q: i64) -> u64 {
        self.inner.estimate(&DataInput::I64(q)) as u64
    }
    fn memory_bytes(&self) -> usize {
        self.rows * self.cols * std::mem::size_of::<i32>()
    }

    /// Counter-wise addition; Count Sketch is linear, so merging is exact.
    fn merge(&mut self, other: &Self) -> Result<(), MergeUnsupported> {
        self.inner.merge(&other.inner);
        Ok(())
    }
}
