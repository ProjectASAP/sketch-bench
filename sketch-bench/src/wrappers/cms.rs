//! Count-Min Accumulator wrappers — 7 variants, every one declaring
//! `FrequencyOps` (`&i64` point lookup → `u64` count estimate). `oxide`,
//! `datasketches` and the `lib_vector2d_*` pair tune via `(rows, cols)`; the
//! `lib_fixedmatrix_*` variants bake their shape in (5x65538, 5x2048, 5x32768).

use aqpbm_core::accuracy::FrequencyOps;
use aqpbm_core::init::{BenchImpl, BuildError, InitSketch};
use crate::params::CmsParams;
use crate::wrappers::require_shape;
use aqpbm_core::config::ParamSet;
use aqpbm_core::accumulator::{Accumulator, MergeUnsupported};
use aqpbm_core::memory_footprint::MemoryFootprint;
use sketch_oxide::Mergeable as _;

use asap_sketchlib::{
    impl_fixed_matrix, CountMin, DataInput, FastPath, FixedMatrix, RegularPath, Vector2D,
};

/// Convert `(rows, cols)` → `(epsilon, delta)` for the oxide / datasketches
/// APIs, which take error bounds rather than raw dimensions. Matches
/// `5 / 2048` → `0.0013 / 0.0067` within float tolerance.
fn dims_to_err(rows: usize, cols: usize) -> (f64, f64) {
    let epsilon = std::f64::consts::E / cols as f64;
    let delta = (-(rows as f64)).exp();
    (epsilon, delta)
}

// ---------- sketch_oxide ----------
// No `rows` / `cols` field: the crate rounds the width it derives from ε up to
// a power of two, so a stored request would disagree with the table actually
// allocated at every non-power-of-two `cols`. Nothing to disagree with is the
// only way to keep the footprint honest across a library upgrade too.
pub struct CmsOxide {
    inner: sketch_oxide::frequency::CountMinSketch,
}

impl InitSketch for CmsOxide {
    fn init(config: &ParamSet) -> Result<Self, BuildError> {
        let p: CmsParams = config.parse()?;
        // Native API takes an error bound, not raw dimensions — translate.
        let (epsilon, delta) = dims_to_err(p.rows, p.cols);
        let inner = sketch_oxide::frequency::CountMinSketch::new(epsilon, delta)
            .map_err(|e| BuildError(format!("oxide CMS rejected ε={epsilon} δ={delta}: {e:?}")))?;
        Ok(Self { inner })
    }
}

impl Accumulator for CmsOxide {
    type Item = i64;
    #[inline(always)]
    fn update(&mut self, v: &i64) {
        self.inner.update(v);
    }

    /// Counter-wise addition. Count-Min is linear, so merging shards is
    /// **exact** and the benchmark asserts equality rather than measuring
    /// degradation — a difference means mismatched seeds or a saturated counter.
    fn merge(&mut self, other: &Self) -> Result<(), MergeUnsupported> {
        self.inner
            .merge(&other.inner)
            .expect("both operands built from one ParamSet, so rows/cols match");
        Ok(())
    }
}

impl MemoryFootprint for CmsOxide {
    fn memory_bytes(&self) -> usize {
        // Read off the built sketch, not off the requested `(rows, cols)`: the
        // crate derives its width from ε and rounds it up to a power of two, so
        // `cols = 3000` allocates 4096 and a request-derived figure under-reports
        // by 27%. The counters are `table: Vec<u64>`, not 32-bit — sizing them
        // as `u32` once halved every reported CMS footprint, which made CMS look
        // twice as space-efficient as CountSketch at identical accuracy.
        self.inner.depth() * self.inner.width() * std::mem::size_of::<u64>()
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

impl Accumulator for CmsDatasketches {
    type Item = i64;
    #[inline(always)]
    fn update(&mut self, v: &i64) {
        self.inner.update(*v);
    }

    /// Counter-wise addition; a Count-Min Accumulator is linear, so merging is exact.
    fn merge(&mut self, other: &Self) -> Result<(), MergeUnsupported> {
        self.inner.merge(&other.inner);
        Ok(())
    }
}

impl MemoryFootprint for CmsDatasketches {
    fn memory_bytes(&self) -> usize {
        // Backing store is `counts: Vec<i64>`; spell the real type so the two
        // stay in step.
        self.rows * self.cols * std::mem::size_of::<i64>()
    }
}

// ---------- asap_sketchlib: FixedMatrix custom + FastPath ----------
// Shape baked at compile time by `impl_fixed_matrix!`; `init` rejects any
// `(rows, cols)` that does not match it with a `BuildError`.
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

impl Accumulator for CmsLibFixedmatrixCustomFast {
    type Item = i64;
    #[inline(always)]
    fn update(&mut self, v: &i64) {
        self.0.insert(&DataInput::I64(*v));
    }

    /// Counter-wise addition; a Count-Min Accumulator is linear, so merging is exact.
    fn merge(&mut self, other: &Self) -> Result<(), MergeUnsupported> {
        self.0.merge(&other.0);
        Ok(())
    }
}

impl MemoryFootprint for CmsLibFixedmatrixCustomFast {
    fn memory_bytes(&self) -> usize {
        CMS_CUSTOM_FIXED_ROWS * CMS_CUSTOM_FIXED_COLS * std::mem::size_of::<i32>()
    }
}

// ---------- asap_sketchlib: FixedMatrix 5x32768 + FastPath ----------
// Same code path as the 5x2048 FixedMatrix, at 5x32768 to mirror the
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

impl Accumulator for CmsLibFixedmatrixFast32k {
    type Item = i64;
    #[inline(always)]
    fn update(&mut self, v: &i64) {
        self.0.insert(&DataInput::I64(*v));
    }

    /// Counter-wise addition; a Count-Min Accumulator is linear, so merging is exact.
    fn merge(&mut self, other: &Self) -> Result<(), MergeUnsupported> {
        self.0.merge(&other.0);
        Ok(())
    }
}

impl MemoryFootprint for CmsLibFixedmatrixFast32k {
    fn memory_bytes(&self) -> usize {
        CMS_FIXED_32K_ROWS * CMS_FIXED_32K_COLS * std::mem::size_of::<i32>()
    }
}

// ---------- asap_sketchlib: FixedMatrix + FastPath ----------
// `FixedMatrix::default()` bakes in (5, 2048); `init` accepts only a matching
// `(rows, cols)` and rejects anything else with a `BuildError`.
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

impl Accumulator for CmsLibFixedmatrixFast {
    type Item = i64;
    #[inline(always)]
    fn update(&mut self, v: &i64) {
        self.0.insert(&DataInput::I64(*v));
    }

    /// Counter-wise addition; a Count-Min Accumulator is linear, so merging is exact.
    fn merge(&mut self, other: &Self) -> Result<(), MergeUnsupported> {
        self.0.merge(&other.0);
        Ok(())
    }
}

impl MemoryFootprint for CmsLibFixedmatrixFast {
    fn memory_bytes(&self) -> usize {
        // `FixedMatrix` aliases `QuickMatrixI32`, i.e. `Box<[i32; _]>`. The peer
        // CountSketch impl spells it `i32` too; keep the two readable together.
        CMS_FIXED_ROWS * CMS_FIXED_COLS * std::mem::size_of::<i32>()
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

impl Accumulator for CmsLibVector2dFast {
    type Item = i64;
    #[inline(always)]
    fn update(&mut self, v: &i64) {
        self.inner.insert(&DataInput::I64(*v));
    }

    /// Counter-wise addition; a Count-Min Accumulator is linear, so merging is exact.
    fn merge(&mut self, other: &Self) -> Result<(), MergeUnsupported> {
        self.inner.merge(&other.inner);
        Ok(())
    }
}

impl MemoryFootprint for CmsLibVector2dFast {
    fn memory_bytes(&self) -> usize {
        self.rows * self.cols * std::mem::size_of::<i32>()
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

impl Accumulator for CmsLibVector2dRegular {
    type Item = i64;
    #[inline(always)]
    fn update(&mut self, v: &i64) {
        self.inner.insert(&DataInput::I64(*v));
    }

    /// Counter-wise addition; a Count-Min Accumulator is linear, so merging is exact.
    fn merge(&mut self, other: &Self) -> Result<(), MergeUnsupported> {
        self.inner.merge(&other.inner);
        Ok(())
    }
}

impl MemoryFootprint for CmsLibVector2dRegular {
    fn memory_bytes(&self) -> usize {
        self.rows * self.cols * std::mem::size_of::<i32>()
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
// `IMPL` is the library. The hash strategy (`fastpath` packs one hash and slices
// a column out of it per row, `regularpath` hashes once per row) and the storage
// backend (`vector2d` sizes at runtime, `fixedmatrix` bakes the shape into the
// type) both change what is being measured, not who wrote it, so they name the
// algorithm. `CmsParams` is the vocabulary all of them share, which is what
// keeps them one family.

impl BenchImpl for CmsOxide { type Params = CmsParams; const IMPL: &'static str = "oxide"; }
impl BenchImpl for CmsDatasketches { type Params = CmsParams; const IMPL: &'static str = "datasketches"; }

impl BenchImpl for CmsLibFixedmatrixCustomFast {
    type Params = CmsParams;
    const ALGORITHM: &'static str = "cms-fastpath-fixedmatrix-custom";
    const IMPL: &'static str = "lib";
}
impl BenchImpl for CmsLibFixedmatrixFast {
    type Params = CmsParams;
    const ALGORITHM: &'static str = "cms-fastpath-fixedmatrix-2k";
    const IMPL: &'static str = "lib";
}
impl BenchImpl for CmsLibFixedmatrixFast32k {
    type Params = CmsParams;
    const ALGORITHM: &'static str = "cms-fastpath-fixedmatrix-32k";
    const IMPL: &'static str = "lib";
}
impl BenchImpl for CmsLibVector2dFast {
    type Params = CmsParams;
    const ALGORITHM: &'static str = "cms-fastpath-vector2d";
    const IMPL: &'static str = "lib";
}
impl BenchImpl for CmsLibVector2dRegular {
    type Params = CmsParams;
    const ALGORITHM: &'static str = "cms-regularpath-vector2d";
    const IMPL: &'static str = "lib";
}

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

    /// Two configs the crate resolves to one table must report one footprint.
    /// `cols` is a request: the width is derived from ε and rounded up to a
    /// power of two, so 3000 and 4096 build the same sketch and score the same
    /// error. Reporting the request would put those two identical measurements
    /// at x-positions 27% apart on every accuracy-vs-memory plot.
    ///
    /// Stated as an equality between two configs rather than a pinned number, so
    /// it keeps holding if the crate changes how it rounds.
    #[test]
    fn oxide_cms_reports_the_table_it_built_not_the_one_requested() {
        let of = |cols: usize| {
            CmsOxide::init(&ParamSet::of(&CmsParams { rows: 5, cols }))
                .expect("both are valid oxide shapes")
                .memory_bytes()
        };
        assert_eq!(
            of(3000),
            of(4096),
            "cols=3000 and cols=4096 resolve to one table, so one footprint"
        );
    }

    /// The same property for CountSketch, where the crate also floors the depth
    /// at 3 so the median has enough estimates to be one. A `rows=2` request
    /// therefore builds 3 rows and must say so.
    #[test]
    fn oxide_countsketch_reports_the_depth_it_built_not_the_one_requested() {
        let of = |rows: usize| {
            CsOxide::init(&ParamSet::of(&crate::params::CountSketchParams { rows, cols: 2048 }))
                .expect("both are valid oxide shapes")
                .memory_bytes()
        };
        assert_eq!(
            of(2),
            of(3),
            "rows=2 and rows=3 resolve to one table, so one footprint"
        );
    }

    /// The two oxide rows do **not** land on one table at one nominal shape, and
    /// this pins by how much.
    ///
    /// `CmsOxide` asks for `cols` through `ε = e/cols`, and the crate's
    /// `ceil(2/ε).next_power_of_two()` returns exactly `cols` at every power of
    /// two. `CsOxide` asks through `ε = sqrt(3/cols)`, and `3/ε²` does not
    /// round-trip in `f64`: it lands a hair above `cols`, `ceil` takes it to
    /// `cols + 1`, and the power-of-two rounding then doubles it. So `cols=2048`
    /// builds 2048 columns of Count-Min and 4096 of CountSketch.
    ///
    /// Both footprints below are truthful about what was allocated. What is not
    /// settled is whether `cols` should mean the same thing to both rows, which
    /// is a question about the panel, not about this formula. Until it is
    /// settled, a CMS-vs-CountSketch comparison at one `--config` is comparing
    /// two counter budgets, and this test is where that fact is written down.
    #[test]
    fn the_two_oxide_rows_resolve_one_config_to_different_tables() {
        let cms = CmsOxide::init(&shape()).expect("5x2048 is a valid oxide shape");
        let cs = CsOxide::init(&ParamSet::of(&crate::params::CountSketchParams {
            rows: 5,
            cols: 2048,
        }))
        .expect("5x2048 is a valid oxide shape");

        // 8 bytes per counter on both sides: `table: Vec<u64>` and `Vec<i64>`.
        // Sizing either as 32-bit once halved a reported footprint and made one
        // algorithm look twice as space-efficient at identical measured accuracy.
        assert_eq!(cms.memory_bytes(), 5 * 2048 * 8, "CMS builds the width asked for");
        assert_eq!(
            cs.memory_bytes(),
            5 * 4096 * 8,
            "CountSketch's ε round-trip doubles the width, and the footprint says so"
        );
    }
}
