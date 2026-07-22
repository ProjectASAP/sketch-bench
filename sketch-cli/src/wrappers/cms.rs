//! Count-Min Sketch wrappers — 6 variants.
//!
//! Frequency family: `Query = i64` (point lookup),
//! `Answer = u64` (count estimate).
//!
//! Impls:
//! * `oxide` — sketch_oxide::frequency::CountMinSketch (tunable via (rows,cols))
//! * `datasketches` — datasketches::countmin::CountMinSketch (tunable)
//! * `lib_fixedmatrix_custom_fast` — asap_sketchlib custom storage (FIXED 5x65538)
//! * `lib_fixedmatrix_fast` — asap_sketchlib FixedMatrix + FastPath (FIXED 5x2048)
//! * `lib_vector2d_fast` — asap_sketchlib Vector2D + FastPath (tunable)
//! * `lib_vector2d_regular` — asap_sketchlib Vector2D + RegularPath (tunable)

use crate::params::CmsParams;
use sketch_core::sketch::{MergeUnsupported, Sketch};
use sketch_oxide::Mergeable as _;

use asap_sketchlib::{
    impl_fixed_matrix, CountMin, DataInput, FastPath, FixedMatrix, RegularPath, Vector2D,
};

/// Convert `(rows, cols)` → `(epsilon, delta)` for the oxide /
/// datasketches style API (they accept error bounds, not raw
/// dimensions). Matches the `CMS_ROWS=5 / CMS_COLS=2048` →
/// `CMS_EPSILON=0.0013 / CMS_DELTA=0.0067` defaults used by the
/// legacy binaries within float tolerance.
fn dims_to_err(rows: usize, cols: usize) -> (f64, f64) {
    let epsilon = std::f64::consts::E / cols as f64;
    let delta = (-(rows as f64)).exp();
    (epsilon, delta)
}

// ---------- sketch_oxide ----------
pub struct CmsOxide {
    inner: sketch_oxide::frequency::CountMinSketch,
    rows: usize,
    cols: usize,
}

impl CmsOxide {
    pub fn new(p: &CmsParams) -> Self {
        let (epsilon, delta) = dims_to_err(p.rows, p.cols);
        Self {
            inner: sketch_oxide::frequency::CountMinSketch::new(epsilon, delta)
                .expect("valid CMS parameters"),
            rows: p.rows,
            cols: p.cols,
        }
    }
}

impl Sketch for CmsOxide {
    type Item = i64;
    type Query = i64;
    type Answer = u64;
    #[inline(always)]
    fn update(&mut self, v: &i64) {
        self.inner.update(v);
    }
    fn query(&self, q: i64) -> u64 {
        self.inner.estimate(&q)
    }
    fn memory_bytes(&self) -> usize {
        self.rows * self.cols * std::mem::size_of::<u32>()
    }

    /// Counter-wise addition. A Count-Min Sketch is linear in its input, so
    /// merging shards is **exact**: the result is identical to one sketch fed
    /// the whole stream. The merge benchmark therefore asserts that equality
    /// rather than measuring a degradation — a difference would mean the
    /// shards disagreed about hash seeds, or a counter saturated.
    fn merge(&mut self, other: &Self) -> Result<(), MergeUnsupported> {
        self.inner
            .merge(&other.inner)
            .expect("both operands built from one ParamSet, so rows/cols match");
        Ok(())
    }
}

// ---------- datasketches ----------
pub struct CmsDatasketches {
    inner: datasketches::countmin::CountMinSketch,
    rows: usize,
    cols: usize,
}

impl CmsDatasketches {
    pub fn new(p: &CmsParams) -> Self {
        Self {
            inner: datasketches::countmin::CountMinSketch::new(p.rows as u8, p.cols as u32),
            rows: p.rows,
            cols: p.cols,
        }
    }
}

impl Sketch for CmsDatasketches {
    type Item = i64;
    type Query = i64;
    type Answer = u64;
    #[inline(always)]
    fn update(&mut self, v: &i64) {
        self.inner.update(*v);
    }
    fn query(&self, q: i64) -> u64 {
        self.inner.estimate(q).max(0) as u64
    }
    fn memory_bytes(&self) -> usize {
        self.rows * self.cols * std::mem::size_of::<u64>()
    }

    /// Counter-wise addition. A Count-Min Sketch is linear in its input, so
    /// merging shards is **exact**: the result is identical to one sketch fed
    /// the whole stream. The merge benchmark therefore asserts that equality
    /// rather than measuring a degradation — a difference would mean the
    /// shards disagreed about hash seeds, or a counter saturated.
    fn merge(&mut self, other: &Self) -> Result<(), MergeUnsupported> {
        self.inner.merge(&other.inner);
        Ok(())
    }
}

// ---------- asap_sketchlib: FixedMatrix custom + FastPath ----------
// Shape is baked at compile time by `impl_fixed_matrix!`. Only
// runs when the requested `(rows, cols)` matches this shape;
// otherwise the dispatch skips it.
impl_fixed_matrix!(CustomCountMinMatrixI32U128, i32, 5, 65538);

pub const CMS_CUSTOM_FIXED_ROWS: usize = 5;
pub const CMS_CUSTOM_FIXED_COLS: usize = 65538;

pub struct CmsLibFixedmatrixCustomFast(pub CountMin<CustomCountMinMatrixI32U128, FastPath>);

impl CmsLibFixedmatrixCustomFast {
    pub fn new(_p: &CmsParams) -> Self {
        Self(
            CountMin::<CustomCountMinMatrixI32U128, FastPath>::from_storage(
                CustomCountMinMatrixI32U128::default(),
            ),
        )
    }
}

impl Sketch for CmsLibFixedmatrixCustomFast {
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
        CMS_CUSTOM_FIXED_ROWS * CMS_CUSTOM_FIXED_COLS * std::mem::size_of::<i32>()
    }

    /// Counter-wise addition. A Count-Min Sketch is linear in its input, so
    /// merging shards is **exact**: the result is identical to one sketch fed
    /// the whole stream. The merge benchmark therefore asserts that equality
    /// rather than measuring a degradation — a difference would mean the
    /// shards disagreed about hash seeds, or a counter saturated.
    fn merge(&mut self, other: &Self) -> Result<(), MergeUnsupported> {
        self.0.merge(&other.0);
        Ok(())
    }
}

// ---------- asap_sketchlib: FixedMatrix 5x32768 + FastPath ----------
// Same code path as the 5x2048 FixedMatrix (compile-time baked
// shape via `impl_fixed_matrix!`), at 5x32768 to mirror the
// CMS+CS 32K panel.
impl_fixed_matrix!(CountMinMatrix5x32K, i32, 5, 32768);

pub const CMS_FIXED_32K_ROWS: usize = 5;
pub const CMS_FIXED_32K_COLS: usize = 32768;

pub struct CmsLibFixedmatrixFast32k(pub CountMin<CountMinMatrix5x32K, FastPath>);

impl CmsLibFixedmatrixFast32k {
    pub fn new(_p: &CmsParams) -> Self {
        Self(CountMin::<CountMinMatrix5x32K, FastPath>::from_storage(
            CountMinMatrix5x32K::default(),
        ))
    }
}

impl Sketch for CmsLibFixedmatrixFast32k {
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

    /// Counter-wise addition. A Count-Min Sketch is linear in its input, so
    /// merging shards is **exact**: the result is identical to one sketch fed
    /// the whole stream. The merge benchmark therefore asserts that equality
    /// rather than measuring a degradation — a difference would mean the
    /// shards disagreed about hash seeds, or a counter saturated.
    fn merge(&mut self, other: &Self) -> Result<(), MergeUnsupported> {
        self.0.merge(&other.0);
        Ok(())
    }
}

// ---------- asap_sketchlib: FixedMatrix + FastPath ----------
// `FixedMatrix::default()` bakes in (5, 2048). Treated as a
// fixed-shape impl in dispatch; only runs when `(rows, cols)`
// matches that.
pub const CMS_FIXED_ROWS: usize = 5;
pub const CMS_FIXED_COLS: usize = 2048;

pub struct CmsLibFixedmatrixFast(pub CountMin<FixedMatrix, FastPath>);

impl CmsLibFixedmatrixFast {
    pub fn new(_p: &CmsParams) -> Self {
        Self(CountMin::<FixedMatrix, FastPath>::default())
    }
}

impl Sketch for CmsLibFixedmatrixFast {
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
        CMS_FIXED_ROWS * CMS_FIXED_COLS * std::mem::size_of::<u32>()
    }

    /// Counter-wise addition. A Count-Min Sketch is linear in its input, so
    /// merging shards is **exact**: the result is identical to one sketch fed
    /// the whole stream. The merge benchmark therefore asserts that equality
    /// rather than measuring a degradation — a difference would mean the
    /// shards disagreed about hash seeds, or a counter saturated.
    fn merge(&mut self, other: &Self) -> Result<(), MergeUnsupported> {
        self.0.merge(&other.0);
        Ok(())
    }
}

// ---------- asap_sketchlib: Vector2D + FastPath ----------
pub struct CmsLibVector2dFast {
    inner: CountMin<Vector2D<i32>, FastPath>,
    rows: usize,
    cols: usize,
}

impl CmsLibVector2dFast {
    pub fn new(p: &CmsParams) -> Self {
        Self {
            inner: CountMin::<Vector2D<i32>, FastPath>::with_dimensions(p.rows, p.cols),
            rows: p.rows,
            cols: p.cols,
        }
    }
}

impl Sketch for CmsLibVector2dFast {
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

    /// Counter-wise addition. A Count-Min Sketch is linear in its input, so
    /// merging shards is **exact**: the result is identical to one sketch fed
    /// the whole stream. The merge benchmark therefore asserts that equality
    /// rather than measuring a degradation — a difference would mean the
    /// shards disagreed about hash seeds, or a counter saturated.
    fn merge(&mut self, other: &Self) -> Result<(), MergeUnsupported> {
        self.inner.merge(&other.inner);
        Ok(())
    }
}

// ---------- asap_sketchlib: Vector2D + RegularPath ----------
pub struct CmsLibVector2dRegular {
    inner: CountMin<Vector2D<i32>, RegularPath>,
    rows: usize,
    cols: usize,
}

impl CmsLibVector2dRegular {
    pub fn new(p: &CmsParams) -> Self {
        Self {
            inner: CountMin::<Vector2D<i32>, RegularPath>::with_dimensions(p.rows, p.cols),
            rows: p.rows,
            cols: p.cols,
        }
    }
}

impl Sketch for CmsLibVector2dRegular {
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

    /// Counter-wise addition. A Count-Min Sketch is linear in its input, so
    /// merging shards is **exact**: the result is identical to one sketch fed
    /// the whole stream. The merge benchmark therefore asserts that equality
    /// rather than measuring a degradation — a difference would mean the
    /// shards disagreed about hash seeds, or a counter saturated.
    fn merge(&mut self, other: &Self) -> Result<(), MergeUnsupported> {
        self.inner.merge(&other.inner);
        Ok(())
    }
}
