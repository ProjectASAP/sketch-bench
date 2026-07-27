//! Count-Min Sketch wrappers — 7 variants.
//!
//! Frequency family: every variant declares `FrequencyOps`
//! (`&i64` point lookup → `u64` count estimate).
//!
//! Impls:
//! * `oxide` — sketch_oxide::frequency::CountMinSketch (tunable via (rows,cols))
//! * `datasketches` — datasketches::countmin::CountMinSketch (tunable)
//! * `lib_fixedmatrix_custom_fast` — asap_sketchlib custom storage (FIXED 5x65538)
//! * `lib_fixedmatrix_fast` — asap_sketchlib FixedMatrix + FastPath (FIXED 5x2048)
//! * `lib_fixedmatrix_fast_32k` — same, FIXED 5x32768
//! * `lib_vector2d_fast` — asap_sketchlib Vector2D + FastPath (tunable)
//! * `lib_vector2d_regular` — asap_sketchlib Vector2D + RegularPath (tunable)

use aqpbm_core::accuracy::FrequencyOps;
use aqpbm_core::init::{BenchImpl, BuildError, InitSketch};
use crate::params::CmsParams;
use crate::wrappers::require_shape;
use aqpbm_core::config::ParamSet;
use aqpbm_core::sketch::{MergeUnsupported, Sketch};
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

impl InitSketch for CmsOxide {
    fn init(config: &ParamSet) -> Result<Self, BuildError> {
        let p: CmsParams = config.parse()?;
        // Native API takes an error bound, not raw dimensions — translate.
        let (epsilon, delta) = dims_to_err(p.rows, p.cols);
        let inner = sketch_oxide::frequency::CountMinSketch::new(epsilon, delta)
            .map_err(|e| BuildError(format!("oxide CMS rejected ε={epsilon} δ={delta}: {e:?}")))?;
        Ok(Self {
            inner,
            rows: p.rows,
            cols: p.cols,
        })
    }
}

impl Sketch for CmsOxide {
    type Item = i64;
    #[inline(always)]
    fn update(&mut self, v: &i64) {
        self.inner.update(v);
    }
    fn memory_bytes(&self) -> usize {
        // The crate's counters are `table: Vec<u64>`, not 32-bit. Sizing this
        // as `u32` halved every reported CMS footprint, which made CMS look
        // twice as space-efficient as CountSketch at identical accuracy.
        self.rows * self.cols * std::mem::size_of::<u64>()
    }

    /// Counter-wise addition. A Count-Min Sketch is linear in its input, so
    /// merging shards is **exact**: the result is identical to one sketch fed
    /// the whole stream. The merge benchmark therefore asserts that equality
    /// rather than measuring a degradation — a difference would mean the
    /// shards disagreed about hash seeds, or a counter saturated. The other
    /// CMS impls in this file merge on the same reasoning.
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

impl InitSketch for CmsDatasketches {
    fn init(config: &ParamSet) -> Result<Self, BuildError> {
        let p: CmsParams = config.parse()?;
        Ok(Self {
            inner: datasketches::countmin::CountMinSketch::new(p.rows as u8, p.cols as u32),
            rows: p.rows,
            cols: p.cols,
        })
    }
}

impl Sketch for CmsDatasketches {
    type Item = i64;
    #[inline(always)]
    fn update(&mut self, v: &i64) {
        self.inner.update(*v);
    }
    fn memory_bytes(&self) -> usize {
        // Backing store is `counts: Vec<i64>` — same width as the `u64` this
        // used to name, but spell the real type so the two stay in step.
        self.rows * self.cols * std::mem::size_of::<i64>()
    }

    /// Counter-wise addition; a Count-Min Sketch is linear, so merging is exact.
    fn merge(&mut self, other: &Self) -> Result<(), MergeUnsupported> {
        self.inner.merge(&other.inner);
        Ok(())
    }
}

// ---------- asap_sketchlib: FixedMatrix custom + FastPath ----------
// Shape is baked at compile time by `impl_fixed_matrix!`. Only
// runs when the requested `(rows, cols)` matches this shape;
// otherwise `init` rejects the config with a `BuildError`.
impl_fixed_matrix!(CustomCountMinMatrixI32U128, i32, 5, 65538);

pub const CMS_CUSTOM_FIXED_ROWS: usize = 5;
pub const CMS_CUSTOM_FIXED_COLS: usize = 65538;

pub struct CmsLibFixedmatrixCustomFast(pub CountMin<CustomCountMinMatrixI32U128, FastPath>);

impl InitSketch for CmsLibFixedmatrixCustomFast {
    fn init(config: &ParamSet) -> Result<Self, BuildError> {
        let p: CmsParams = config.parse()?;
        // The shape is baked into the type, so this impl exists at one shape
        // only. Reject any other config — this is what `Constraint` used to
        // decide from outside; it belongs here, where the shape is known.
        require_shape(p.rows, p.cols, CMS_CUSTOM_FIXED_ROWS, CMS_CUSTOM_FIXED_COLS)?;
        Ok(Self(
            CountMin::<CustomCountMinMatrixI32U128, FastPath>::from_storage(
                CustomCountMinMatrixI32U128::default(),
            ),
        ))
    }
}

impl Sketch for CmsLibFixedmatrixCustomFast {
    type Item = i64;
    #[inline(always)]
    fn update(&mut self, v: &i64) {
        self.0.insert(&DataInput::I64(*v));
    }
    fn memory_bytes(&self) -> usize {
        CMS_CUSTOM_FIXED_ROWS * CMS_CUSTOM_FIXED_COLS * std::mem::size_of::<i32>()
    }

    /// Counter-wise addition; a Count-Min Sketch is linear, so merging is exact.
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

impl InitSketch for CmsLibFixedmatrixFast32k {
    fn init(config: &ParamSet) -> Result<Self, BuildError> {
        let p: CmsParams = config.parse()?;
        require_shape(p.rows, p.cols, CMS_FIXED_32K_ROWS, CMS_FIXED_32K_COLS)?;
        Ok(Self(
            CountMin::<CountMinMatrix5x32K, FastPath>::from_storage(CountMinMatrix5x32K::default()),
        ))
    }
}

impl Sketch for CmsLibFixedmatrixFast32k {
    type Item = i64;
    #[inline(always)]
    fn update(&mut self, v: &i64) {
        self.0.insert(&DataInput::I64(*v));
    }
    fn memory_bytes(&self) -> usize {
        CMS_FIXED_32K_ROWS * CMS_FIXED_32K_COLS * std::mem::size_of::<i32>()
    }

    /// Counter-wise addition; a Count-Min Sketch is linear, so merging is exact.
    fn merge(&mut self, other: &Self) -> Result<(), MergeUnsupported> {
        self.0.merge(&other.0);
        Ok(())
    }
}

// ---------- asap_sketchlib: FixedMatrix + FastPath ----------
// `FixedMatrix::default()` bakes in (5, 2048). A fixed-shape impl:
// its `init` accepts only `(rows, cols)` matching that, rejecting
// anything else with a `BuildError`.
pub const CMS_FIXED_ROWS: usize = 5;
pub const CMS_FIXED_COLS: usize = 2048;

pub struct CmsLibFixedmatrixFast(pub CountMin<FixedMatrix, FastPath>);

impl InitSketch for CmsLibFixedmatrixFast {
    fn init(config: &ParamSet) -> Result<Self, BuildError> {
        let p: CmsParams = config.parse()?;
        require_shape(p.rows, p.cols, CMS_FIXED_ROWS, CMS_FIXED_COLS)?;
        Ok(Self(CountMin::<FixedMatrix, FastPath>::default()))
    }
}

impl Sketch for CmsLibFixedmatrixFast {
    type Item = i64;
    #[inline(always)]
    fn update(&mut self, v: &i64) {
        self.0.insert(&DataInput::I64(*v));
    }
    fn memory_bytes(&self) -> usize {
        // `FixedMatrix` is an alias for `QuickMatrixI32`, i.e. `Box<[i32; _]>` —
        // same width as the `u32` this used to name, but the peer CountSketch
        // impl already spells it `i32`; keep the two readable side by side.
        CMS_FIXED_ROWS * CMS_FIXED_COLS * std::mem::size_of::<i32>()
    }

    /// Counter-wise addition; a Count-Min Sketch is linear, so merging is exact.
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

impl InitSketch for CmsLibVector2dFast {
    fn init(config: &ParamSet) -> Result<Self, BuildError> {
        let p: CmsParams = config.parse()?;
        Ok(Self {
            inner: CountMin::<Vector2D<i32>, FastPath>::with_dimensions(p.rows, p.cols),
            rows: p.rows,
            cols: p.cols,
        })
    }
}

impl Sketch for CmsLibVector2dFast {
    type Item = i64;
    #[inline(always)]
    fn update(&mut self, v: &i64) {
        self.inner.insert(&DataInput::I64(*v));
    }
    fn memory_bytes(&self) -> usize {
        self.rows * self.cols * std::mem::size_of::<i32>()
    }

    /// Counter-wise addition; a Count-Min Sketch is linear, so merging is exact.
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

impl InitSketch for CmsLibVector2dRegular {
    fn init(config: &ParamSet) -> Result<Self, BuildError> {
        let p: CmsParams = config.parse()?;
        Ok(Self {
            inner: CountMin::<Vector2D<i32>, RegularPath>::with_dimensions(p.rows, p.cols),
            rows: p.rows,
            cols: p.cols,
        })
    }
}

impl Sketch for CmsLibVector2dRegular {
    type Item = i64;
    #[inline(always)]
    fn update(&mut self, v: &i64) {
        self.inner.insert(&DataInput::I64(*v));
    }
    fn memory_bytes(&self) -> usize {
        self.rows * self.cols * std::mem::size_of::<i32>()
    }

    /// Counter-wise addition; a Count-Min Sketch is linear, so merging is exact.
    fn merge(&mut self, other: &Self) -> Result<(), MergeUnsupported> {
        self.inner.merge(&other.inner);
        Ok(())
    }
}


// ---------- statistic membership ----------

impl FrequencyOps for CmsOxide {
    type Key = i64;
    fn estimate_frequency(&self, key: &i64) -> u64 {
        self.inner.estimate(key)
    }
}

impl FrequencyOps for CmsDatasketches {
    type Key = i64;
    fn estimate_frequency(&self, key: &i64) -> u64 {
        self.inner.estimate(*key).max(0) as u64
    }
}

impl FrequencyOps for CmsLibFixedmatrixCustomFast {
    type Key = i64;
    fn estimate_frequency(&self, key: &i64) -> u64 {
        self.0.estimate(&DataInput::I64(*key)) as u64
    }
}

impl FrequencyOps for CmsLibFixedmatrixFast {
    type Key = i64;
    fn estimate_frequency(&self, key: &i64) -> u64 {
        self.0.estimate(&DataInput::I64(*key)) as u64
    }
}

impl FrequencyOps for CmsLibFixedmatrixFast32k {
    type Key = i64;
    fn estimate_frequency(&self, key: &i64) -> u64 {
        self.0.estimate(&DataInput::I64(*key)) as u64
    }
}

impl FrequencyOps for CmsLibVector2dFast {
    type Key = i64;
    fn estimate_frequency(&self, key: &i64) -> u64 {
        self.inner.estimate(&DataInput::I64(*key)) as u64
    }
}

impl FrequencyOps for CmsLibVector2dRegular {
    type Key = i64;
    fn estimate_frequency(&self, key: &i64) -> u64 {
        self.inner.estimate(&DataInput::I64(*key)) as u64
    }
}

// ---------- catalog identity ----------

impl BenchImpl for CmsOxide { type Params = CmsParams; const IMPL: &'static str = "oxide"; }
impl BenchImpl for CmsDatasketches { type Params = CmsParams; const IMPL: &'static str = "datasketches"; }
impl BenchImpl for CmsLibFixedmatrixCustomFast { type Params = CmsParams; const IMPL: &'static str = "lib-fixedmatrix-custom-fast"; }
impl BenchImpl for CmsLibFixedmatrixFast { type Params = CmsParams; const IMPL: &'static str = "lib-fixedmatrix-fast"; }
impl BenchImpl for CmsLibFixedmatrixFast32k { type Params = CmsParams; const IMPL: &'static str = "lib-fixedmatrix-fast-32k"; }
impl BenchImpl for CmsLibVector2dFast { type Params = CmsParams; const IMPL: &'static str = "lib-vector2d-fast"; }
impl BenchImpl for CmsLibVector2dRegular { type Params = CmsParams; const IMPL: &'static str = "lib-vector2d-regular"; }

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wrappers::countsketch::CsOxide;

    fn shape() -> ParamSet {
        ParamSet::of(&CmsParams {
            rows: 5,
            cols: 2048,
        })
    }

    /// `sketch_oxide` stores its counters as `Vec<u64>`. Sizing them at 4 bytes
    /// halved the reported footprint, which is the number the accuracy-vs-memory
    /// plots divide by.
    #[test]
    fn oxide_sizes_counters_at_the_real_width() {
        let sketch = CmsOxide::init(&shape()).expect("5x2048 is a valid oxide shape");
        assert_eq!(sketch.memory_bytes(), 5 * 2048 * 8);
    }

    /// Both oxide sketches allocate one 8-byte counter per cell, so at one shape
    /// they must report one footprint — CMS looked 2× cheaper than CountSketch
    /// at identical measured accuracy while this was skewed.
    #[test]
    fn oxide_cms_and_countsketch_agree_at_one_shape() {
        let cms = CmsOxide::init(&shape()).expect("5x2048 is a valid oxide shape");
        let cs = CsOxide::init(&ParamSet::of(&crate::params::CountSketchParams {
            rows: 5,
            cols: 2048,
        }))
        .expect("5x2048 is a valid oxide shape");
        assert_eq!(cms.memory_bytes(), cs.memory_bytes());
    }
}
