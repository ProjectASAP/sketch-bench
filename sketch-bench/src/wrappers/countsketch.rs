//! CountSketch wrappers — 5 variants (`oxide` + 4× sketchlib).
//! Same family as CMS: each declares `FrequencyOps`.

use crate::accuracy::FrequencyOps;
use crate::init::{BenchImpl, BuildError, InitSketch};
use crate::params::CountSketchParams;
use crate::wrappers::require_shape;
use aqpbm_core::config::ParamSet;
use aqpbm_core::sketch::{MergeUnsupported, Sketch};
use asap_sketchlib::{Count, DataInput, FastPath, FixedMatrix, RegularPath, Vector2D};
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

impl InitSketch for CsOxide {
    fn init(config: &ParamSet) -> Result<Self, BuildError> {
        let p: CountSketchParams = config.parse()?;
        let (epsilon, delta) = dims_to_err(p.rows, p.cols);
        let inner = sketch_oxide::frequency::CountSketch::new(epsilon, delta).map_err(|e| {
            BuildError(format!(
                "oxide CountSketch rejected ε={epsilon} δ={delta}: {e:?}"
            ))
        })?;
        Ok(Self {
            inner,
            rows: p.rows,
            cols: p.cols,
        })
    }
}

impl Sketch for CsOxide {
    type Item = i64;
    #[inline(always)]
    fn update(&mut self, v: &i64) {
        self.inner.update(v, 1);
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
// Compile-time fixed at (5, 2048): builds only when the
// requested config matches that shape, else a `BuildError`.
pub struct CsLibFixedmatrixFast(pub Count<FixedMatrix, FastPath>);

impl InitSketch for CsLibFixedmatrixFast {
    fn init(config: &ParamSet) -> Result<Self, BuildError> {
        let p: CountSketchParams = config.parse()?;
        require_shape(p.rows, p.cols, CMS_FIXED_ROWS, CMS_FIXED_COLS)?;
        Ok(Self(Count::<FixedMatrix, FastPath>::default()))
    }
}

impl Sketch for CsLibFixedmatrixFast {
    type Item = i64;
    #[inline(always)]
    fn update(&mut self, v: &i64) {
        self.0.insert(&DataInput::I64(*v));
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

impl InitSketch for CsLibFixedmatrixFast32k {
    fn init(config: &ParamSet) -> Result<Self, BuildError> {
        let p: CountSketchParams = config.parse()?;
        require_shape(p.rows, p.cols, CMS_FIXED_32K_ROWS, CMS_FIXED_32K_COLS)?;
        Ok(Self(Count::<CountMinMatrix5x32K, FastPath>::from_storage(
            CountMinMatrix5x32K::default(),
        )))
    }
}

impl Sketch for CsLibFixedmatrixFast32k {
    type Item = i64;
    #[inline(always)]
    fn update(&mut self, v: &i64) {
        self.0.insert(&DataInput::I64(*v));
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

impl InitSketch for CsLibVector2dFast {
    fn init(config: &ParamSet) -> Result<Self, BuildError> {
        let p: CountSketchParams = config.parse()?;
        Ok(Self {
            inner: Count::<Vector2D<i32>, FastPath>::with_dimensions(p.rows, p.cols),
            rows: p.rows,
            cols: p.cols,
        })
    }
}

impl Sketch for CsLibVector2dFast {
    type Item = i64;
    #[inline(always)]
    fn update(&mut self, v: &i64) {
        self.inner.insert(&DataInput::I64(*v));
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

impl InitSketch for CsLibVector2dRegular {
    fn init(config: &ParamSet) -> Result<Self, BuildError> {
        let p: CountSketchParams = config.parse()?;
        Ok(Self {
            inner: Count::<Vector2D<i32>, RegularPath>::with_dimensions(p.rows, p.cols),
            rows: p.rows,
            cols: p.cols,
        })
    }
}

impl Sketch for CsLibVector2dRegular {
    type Item = i64;
    #[inline(always)]
    fn update(&mut self, v: &i64) {
        self.inner.insert(&DataInput::I64(*v));
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


// ---------- statistic membership ----------
//
// A different family from CMS (its own params) answering the same statistic.

impl FrequencyOps for CsOxide {
    type Key = i64;
    fn estimate_frequency(&self, key: &i64) -> u64 {
        // CountSketch is an unbiased estimator (median of sign·counter); a
        // small fraction of estimates can be slightly negative under collision
        // noise. Clamp to 0 to match CMS-style frequency semantics — without
        // this, `as u64` wraps -1 into u64::MAX and blows up rel-err.
        self.inner.estimate(key).max(0) as u64
    }
}

impl FrequencyOps for CsLibFixedmatrixFast {
    type Key = i64;
    fn estimate_frequency(&self, key: &i64) -> u64 {
        self.0.estimate(&DataInput::I64(*key)) as u64
    }
}

impl FrequencyOps for CsLibFixedmatrixFast32k {
    type Key = i64;
    fn estimate_frequency(&self, key: &i64) -> u64 {
        self.0.estimate(&DataInput::I64(*key)) as u64
    }
}

impl FrequencyOps for CsLibVector2dFast {
    type Key = i64;
    fn estimate_frequency(&self, key: &i64) -> u64 {
        self.inner.estimate(&DataInput::I64(*key)) as u64
    }
}

impl FrequencyOps for CsLibVector2dRegular {
    type Key = i64;
    fn estimate_frequency(&self, key: &i64) -> u64 {
        self.inner.estimate(&DataInput::I64(*key)) as u64
    }
}

// ---------- catalog identity ----------

impl BenchImpl for CsOxide { type Params = CountSketchParams; const IMPL: &'static str = "oxide"; }
impl BenchImpl for CsLibFixedmatrixFast { type Params = CountSketchParams; const IMPL: &'static str = "lib-fixedmatrix-fast"; }
impl BenchImpl for CsLibFixedmatrixFast32k { type Params = CountSketchParams; const IMPL: &'static str = "lib-fixedmatrix-fast-32k"; }
impl BenchImpl for CsLibVector2dFast { type Params = CountSketchParams; const IMPL: &'static str = "lib-vector2d-fast"; }
impl BenchImpl for CsLibVector2dRegular { type Params = CountSketchParams; const IMPL: &'static str = "lib-vector2d-regular"; }
