//! Count-Min wrappers — five types. Each answers a `&i64` point lookup with a
//! `u64` count estimate, which is the shape `FrequencyGT` scores; each says so
//! in its own `SketchOps` at the bottom of this file rather than by
//! implementing a shared trait.
//!
//! All of them take `(rows, cols)` and honour it, by four different routes.
//! `oxide` inverts the error bounds its API takes and checks the table it got
//! back. `datasketches` range-checks the `(u8, u32)` its API narrows to.
//! `CmsLibVector2dFast` / `CmsLibVector2dRegular` size at run time.
//! `CmsLibFixedmatrix<M>` is generic over a storage type that bakes the shape
//! in, so the shape selects a monomorphisation from the table in
//! `wrappers::fixed_matrix` and the catalog dispatches on it.

use aqpbm_core::init::{BenchImpl, BuildError, InitSketch};
use crate::params::CmsParams;
use crate::wrappers::{
    require_positive, require_range, require_resolved_shape, require_shape,
};
use aqpbm_core::config::ParamSet;

use aqpbm_core::memory_footprint::MemoryFootprint;
use sketch_oxide::Mergeable as _;

use asap_sketchlib::{
    CountMin, DataInput, DefaultXxHasher, FastPath, FastPathHasher, MatrixStorage, RegularPath,
    Vector2D,
};

/// Convert `(rows, cols)` → `(epsilon, delta)` for the oxide API, which takes
/// error bounds and derives the dimensions back out of them.
///
/// The inversion has to land on the library's own arithmetic, which is
/// `width = ceil(2/ε).next_power_of_two()` and `depth = ceil(ln(1/δ))`. Solving
/// for `ceil(2/ε) = cols` gives `ε = 2/cols`, and the half-step below keeps the
/// quotient off the integer boundary where one float ulp would tip the `ceil`
/// to `cols + 1` and the power-of-two rounding would then double the table.
/// That is exactly the bug the CountSketch side of this pair had.
///
/// This is a claim about `sketch_oxide` 0.1.6, so nothing rests on it being
/// right: [`require_resolved_shape`] checks the built sketch and refuses if the
/// library resolved the request to anything else.
fn dims_to_err(rows: usize, cols: usize) -> (f64, f64) {
    let epsilon = 2.0 / (cols as f64 - 0.5);
    let delta = (-(rows as f64 - 0.5)).exp();
    (epsilon, delta)
}

// ---------- sketch_oxide ----------
// No `rows` / `cols` field: `init` has already proven the built table matches
// the request, so the sketch itself is the one place either figure is read
// from, and a library upgrade that changed the rounding cannot slip past.
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
        require_resolved_shape(
            "oxide CMS",
            (inner.depth(), inner.width()),
            (p.rows, p.cols),
        )?;
        Ok(Self { inner })
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
//
// The API takes `(u8, u32)` where the parameter vocabulary is `(usize, usize)`,
// and it asserts rather than returning an error. Both facts have to be handled
// before the call: a bare `as u8` turns `rows = 257` into a one-row sketch, and
// `rows = 256` into a zero-row one that aborts the process inside C++.

/// What `datasketches::countmin::CountMinSketch::new` accepts. The three
/// asserts are in `countmin/sketch.rs::entries_for_config`; the row bound is
/// the `u8` the API takes.
const DS_CMS_ROWS: (usize, usize) = (1, u8::MAX as usize);
const DS_CMS_COLS: (usize, usize) = (3, u32::MAX as usize);
/// `num_hashes * num_buckets < MAX_TABLE_ENTRIES`. A product bound, so no
/// per-parameter range catches it.
const DS_CMS_MAX_ENTRIES: usize = 1 << 30;

pub struct CmsDatasketches {
    inner: datasketches::countmin::CountMinSketch,
    rows: usize,
    cols: usize,
}

impl InitSketch for CmsDatasketches {
    fn init(config: &ParamSet) -> Result<Self, BuildError> {
        let p: CmsParams = config.parse()?;
        require_range("datasketches CMS", "rows", p.rows, DS_CMS_ROWS.0, DS_CMS_ROWS.1)?;
        require_range("datasketches CMS", "cols", p.cols, DS_CMS_COLS.0, DS_CMS_COLS.1)?;
        // Checked, because the point of the bound is that the product is what
        // overflows: `usize::MAX` rows-worth of columns must not wrap into a
        // small number that passes.
        let entries = p.rows.checked_mul(p.cols).unwrap_or(usize::MAX);
        if entries >= DS_CMS_MAX_ENTRIES {
            return Err(BuildError(format!(
                "datasketches CMS: rows x cols = {entries} counters, and this library \
                 caps a table at {DS_CMS_MAX_ENTRIES}"
            )));
        }
        // The ranges above make the casts lossless; this proves it against the
        // built sketch rather than against that reasoning, so a library that
        // starts rounding its dimensions turns into a refusal here instead of a
        // silently different table. Same guard the oxide row uses.
        let inner = datasketches::countmin::CountMinSketch::new(p.rows as u8, p.cols as u32);
        require_resolved_shape(
            "datasketches CMS",
            (inner.num_hashes() as usize, inner.num_buckets() as usize),
            (p.rows, p.cols),
        )?;
        Ok(Self {
            inner,
            rows: p.rows,
            cols: p.cols,
        })
    }
}


impl MemoryFootprint for CmsDatasketches {
    fn memory_bytes(&self) -> usize {
        // Backing store is `counts: Vec<i64>`; spell the real type so the two
        // stay in step.
        self.rows * self.cols * std::mem::size_of::<i64>()
    }
}

// ---------- asap_sketchlib: FixedMatrix + FastPath ----------
//
// One row, every compiled-in shape. The storage carries its dimensions in its
// type, so the row is generic over the storage and `catalog` picks the
// monomorphisation from the requested `(rows, cols)`. The set of shapes and the
// dispatch live in `wrappers::fixed_matrix`.
//
// This used to be three rows at three baked shapes, each refusing every config
// but its own. That put a construction parameter on the identity axis: `cols`
// is a knob this family already has, and a caller asking for a fourth shape had
// no way to say so.

pub struct CmsLibFixedmatrix<M: MatrixStorage>(pub CountMin<M, FastPath>);

impl<M> InitSketch for CmsLibFixedmatrix<M>
where
    M: MatrixStorage<Counter = i32> + FastPathHasher<DefaultXxHasher> + Default + Clone + 'static,
{
    fn init(config: &ParamSet) -> Result<Self, BuildError> {
        let p: CmsParams = config.parse()?;
        let inner = CountMin::<M, FastPath>::from_storage(M::default());
        // The catalog selected `M` from this same pair, so this only fires for a
        // direct caller. It fires rather than silently running at `M`'s shape.
        require_shape(p.rows, p.cols, inner.rows(), inner.cols())?;
        Ok(Self(inner))
    }
}


impl<M> MemoryFootprint for CmsLibFixedmatrix<M>
where
    M: MatrixStorage<Counter = i32> + FastPathHasher<DefaultXxHasher> + Default + Clone + 'static,
{
    fn memory_bytes(&self) -> usize {
        // Off the built matrix, so it tracks whichever shape was selected.
        self.0.rows() * self.0.cols() * std::mem::size_of::<i32>()
    }
}

/// The shape-independent half of the fixed-matrix Count-Min row: its name, and
/// how to read a shape out of a config. `catalog` pairs this with the storage
/// type the requested shape selects.
pub struct CmsFixedMatrixRow;

impl crate::catalog::FixedMatrixRow for CmsFixedMatrixRow {
    /// The one ask in the catalog that is not a closure at its row: `At<M>` is
    /// a GAT, so there is no single sketch type a closure could be written
    /// against. Generic over `M` instead, which is the same reason
    /// `FixedMatrixVisitor` is a trait.
    fn insert<M>(sketch: &mut Self::At<M>, v: &i64)
    where
        M: MatrixStorage<Counter = i32>
            + FastPathHasher<DefaultXxHasher>
            + Default
            + Clone
            + 'static,
    {
        insert_cms_lib_fixedmatrix(sketch, v)
    }

    fn ops<M>() -> aqpbm_core::ops::SketchOps<Self::At<M>, i64, i64, u64>
    where
        M: MatrixStorage<Counter = i32>
            + FastPathHasher<DefaultXxHasher>
            + Default
            + Clone
            + 'static,
    {
        fixedmatrix_ops::<M>()
    }
    // Both fixed-matrix rows are frequency rows at every shape, and both
    // wrap a library structure that folds losslessly.
    const CAPABILITY: aqpbm_core::request::Capability =
        aqpbm_core::request::Capability::Frequency;
    const SUPPORTS_MERGE: bool = true;
    const ALGORITHM: &'static str = "cms-fastpath-fixedmatrix";
    type At<
        M: MatrixStorage<Counter = i32>
            + FastPathHasher<DefaultXxHasher>
            + Default
            + Clone
            + 'static,
    > = CmsLibFixedmatrix<M>;
    fn shape(params: &ParamSet) -> Result<(usize, usize), aqpbm_core::cell::RunError> {
        let p: CmsParams = params.parse().map_err(BuildError::from)?;
        Ok((p.rows, p.cols))
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
        // `Vector2D::init` takes `cols.ilog2()`, which aborts at 0, and a
        // zero-row matrix builds happily and then answers every query out of an
        // empty fold. Refuse both here, as the fixed-shape rows in this file
        // already refuse a shape they cannot serve.
        require_positive("asap CMS Vector2D FastPath", "rows", p.rows)?;
        require_positive("asap CMS Vector2D FastPath", "cols", p.cols)?;
        Ok(Self {
            inner: CountMin::<Vector2D<i32>, FastPath>::with_dimensions(p.rows, p.cols),
            rows: p.rows,
            cols: p.cols,
        })
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
        // `Vector2D::init` takes `cols.ilog2()`, which aborts at 0, and a
        // zero-row matrix builds happily and then answers every query out of an
        // empty fold. Refuse both here, as the fixed-shape rows in this file
        // already refuse a shape they cannot serve.
        require_positive("asap CMS Vector2D RegularPath", "rows", p.rows)?;
        require_positive("asap CMS Vector2D RegularPath", "cols", p.cols)?;
        Ok(Self {
            inner: CountMin::<Vector2D<i32>, RegularPath>::with_dimensions(p.rows, p.cols),
            rows: p.rows,
            cols: p.cols,
        })
    }
}


impl MemoryFootprint for CmsLibVector2dRegular {
    fn memory_bytes(&self) -> usize {
        self.rows * self.cols * std::mem::size_of::<i32>()
    }
}


// ---------- statistic membership ----------

impl CmsOxide {
    pub fn estimate_frequency(&self, key: &i64) -> u64 {
        self.inner.estimate(key)
    }
}

impl CmsDatasketches {
    pub fn estimate_frequency(&self, key: &i64) -> u64 {
        self.inner.estimate(*key).max(0) as u64
    }
}

impl<M> CmsLibFixedmatrix<M>
where
    M: MatrixStorage<Counter = i32> + FastPathHasher<DefaultXxHasher> + Default + Clone + 'static,
{
    pub fn estimate_frequency(&self, key: &i64) -> u64 {
        self.0.estimate(&DataInput::I64(*key)) as u64
    }
}

impl CmsLibVector2dFast {
    pub fn estimate_frequency(&self, key: &i64) -> u64 {
        self.inner.estimate(&DataInput::I64(*key)) as u64
    }
}

impl CmsLibVector2dRegular {
    pub fn estimate_frequency(&self, key: &i64) -> u64 {
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

impl BenchImpl for CmsOxide { type Params = CmsParams; const IMPL: &'static str = "oxide"; const SUPPORTS_MERGE: bool = true; }
impl BenchImpl for CmsDatasketches { type Params = CmsParams; const IMPL: &'static str = "datasketches"; const SUPPORTS_MERGE: bool = true; }

impl<M> BenchImpl for CmsLibFixedmatrix<M>
where
    M: MatrixStorage<Counter = i32> + FastPathHasher<DefaultXxHasher> + Default + Clone + 'static,
{
    type Params = CmsParams;
    const ALGORITHM: &'static str = "cms-fastpath-fixedmatrix";
    const IMPL: &'static str = "lib";
    const SUPPORTS_MERGE: bool = true;
}
impl BenchImpl for CmsLibVector2dFast {
    type Params = CmsParams;
    const ALGORITHM: &'static str = "cms-fastpath-vector2d";
    const IMPL: &'static str = "lib";
    const SUPPORTS_MERGE: bool = true;
}
impl BenchImpl for CmsLibVector2dRegular {
    type Params = CmsParams;
    const ALGORITHM: &'static str = "cms-regularpath-vector2d";
    const IMPL: &'static str = "lib";
    const SUPPORTS_MERGE: bool = true;
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

    /// A `cols` the crate cannot resolve exactly is refused, naming the table it
    /// would have built.
    ///
    /// This used to build: `cols = 3000` and `cols = 4096` both allocated 4096
    /// columns, scored identical error, and were recorded as two different
    /// configs. That put one measurement at two x-positions 27% apart on every
    /// accuracy-vs-memory plot. A refusal is the only honest answer, since the
    /// error bound the API takes cannot express 3000 columns.
    #[test]
    fn oxide_cms_refuses_a_cols_it_cannot_resolve_exactly() {
        let Err(err) = CmsOxide::init(&ParamSet::of(&CmsParams {
            rows: 5,
            cols: 3000,
        })) else {
            panic!("cols=3000 resolves to a 4096-wide table, so it must be refused");
        };
        let err = err.to_string();
        assert!(err.contains("3000") && err.contains("4096"), "{err}");
    }

    /// The same for CountSketch's depth floor: the crate takes a median across
    /// rows and refuses to do it over fewer than 3, so `rows = 2` cannot be
    /// honoured and is refused instead of quietly building 3.
    #[test]
    fn oxide_countsketch_refuses_a_depth_below_its_floor() {
        let Err(err) = CsOxide::init(&ParamSet::of(&crate::params::CountSketchParams {
            rows: 2,
            cols: 2048,
        })) else {
            panic!("rows=2 resolves to a 3-row table, so it must be refused");
        };
        assert!(err.to_string().contains("2048"), "{err}");
    }

    /// One `--config`, one counter budget, across the two oxide rows.
    ///
    /// This is the property the ε inversions exist to hold. It did not hold
    /// before: CountSketch asked through `ε = sqrt(3/cols)`, `3/ε²` did not
    /// round-trip in `f64`, `ceil` took it to `cols + 1` and the power-of-two
    /// rounding doubled it, so `cols = 2048` built 2048 columns of Count-Min and
    /// 4096 of CountSketch. A CMS-vs-CountSketch comparison at one config was
    /// comparing two budgets.
    #[test]
    fn the_two_oxide_rows_resolve_one_config_to_one_shape() {
        let cms = CmsOxide::init(&shape()).expect("5x2048 is a valid oxide shape");
        let cs = CsOxide::init(&ParamSet::of(&crate::params::CountSketchParams {
            rows: 5,
            cols: 2048,
        }))
        .expect("5x2048 is a valid oxide shape");

        // 8 bytes per counter on both sides: `table: Vec<u64>` and `Vec<i64>`.
        // Sizing either as 32-bit once halved a reported footprint and made one
        // algorithm look twice as space-efficient at identical measured accuracy.
        assert_eq!(cms.memory_bytes(), 5 * 2048 * 8);
        assert_eq!(cs.memory_bytes(), 5 * 2048 * 8);
    }

    /// Every power-of-two width in the range a sweep would walk resolves
    /// exactly, on both rows. The inversion is arithmetic on floats, so the
    /// property worth pinning is that it holds across the range and not just at
    /// the one shape the tests above happen to use.
    #[test]
    fn every_power_of_two_shape_round_trips_on_both_oxide_rows() {
        for lg in 3..=16u32 {
            let cols = 1usize << lg;
            for rows in 3..=8usize {
                let cms = CmsOxide::init(&ParamSet::of(&CmsParams { rows, cols }))
                    .unwrap_or_else(|e| panic!("cms {rows}x{cols}: {e}"));
                assert_eq!(cms.memory_bytes(), rows * cols * 8, "cms {rows}x{cols}");
                let cs = CsOxide::init(&ParamSet::of(&crate::params::CountSketchParams {
                    rows,
                    cols,
                }))
                .unwrap_or_else(|e| panic!("countsketch {rows}x{cols}: {e}"));
                assert_eq!(cs.memory_bytes(), rows * cols * 8, "countsketch {rows}x{cols}");
            }
        }
    }

    /// The datasketches API takes `(u8, u32)` and asserts inside C++, so every
    /// bound has to be checked on this side. Each value below reproduced a
    /// distinct failure before the guards existed.
    #[test]
    fn datasketches_refuses_what_its_api_cannot_take() {
        let build = |rows: usize, cols: usize| CmsDatasketches::init(&ParamSet::of(&CmsParams { rows, cols }));
        for (rows, cols, why) in [
            (0usize, 1024usize, "aborted inside C++: num_hashes must be at least 1"),
            (256, 1024, "`as u8` made it 0, then the same abort"),
            (257, 1024, "`as u8` made it 1: a one-row sketch labelled 257"),
            (5, 2, "aborted: num_buckets must be at least 3"),
            (5, 4_294_967_296, "`as u32` made it 0, then abort"),
            (5, 4_294_968_320, "`as u32` made it 1024: reported 160 GiB, allocated 682 KiB"),
            (3, 1 << 30, "aborted: the table-entry cap is a product bound"),
        ] {
            let err = build(rows, cols)
                .err()
                .unwrap_or_else(|| panic!("{rows}x{cols} must be refused ({why})"));
            let err = err.to_string();
            assert!(
                err.contains(&rows.to_string()) || err.contains(&cols.to_string()),
                "the refusal should name the value: {err}"
            );
        }
        // The whole legal domain still builds, including both ends.
        for (rows, cols) in [(1usize, 3usize), (5, 2048), (255, 4096)] {
            assert!(build(rows, cols).is_ok(), "{rows}x{cols} is legal");
        }
    }

    /// A `Vector2D` row used to call the library straight through. `cols = 0`
    /// aborted in `ilog2`, and `rows = 0` was worse: it built, ingested, and
    /// wrote a *scored* record whose error was `i32::MAX` — a garbage number
    /// that survived into the output.
    #[test]
    fn vector2d_rows_refuse_a_degenerate_shape() {
        for (rows, cols) in [(0usize, 1024usize), (5, 0), (0, 0)] {
            let p = ParamSet::of(&CmsParams { rows, cols });
            assert!(CmsLibVector2dFast::init(&p).is_err(), "fastpath {rows}x{cols}");
            assert!(CmsLibVector2dRegular::init(&p).is_err(), "regularpath {rows}x{cols}");
            let q = ParamSet::of(&crate::params::CountSketchParams { rows, cols });
            assert!(
                crate::wrappers::countsketch::CsLibVector2dFast::init(&q).is_err(),
                "cs fastpath {rows}x{cols}"
            );
            assert!(
                crate::wrappers::countsketch::CsLibVector2dRegular::init(&q).is_err(),
                "cs regularpath {rows}x{cols}"
            );
        }
        assert!(CmsLibVector2dFast::init(&ParamSet::of(&CmsParams { rows: 1, cols: 1 })).is_ok());
    }

    /// Every row in the family agrees on a degenerate shape. This is the
    /// property the whole guard pass exists to restore: one config used to give
    /// four different answers across seven rows — refused, aborted, silently
    /// accepted, and accepted-with-a-scored-record.
    #[test]
    fn the_frequency_rows_agree_on_a_degenerate_shape() {
        let p = ParamSet::of(&CmsParams { rows: 0, cols: 1024 });
        assert!(CmsOxide::init(&p).is_err(), "oxide");
        assert!(CmsDatasketches::init(&p).is_err(), "datasketches");
        assert!(CmsLibVector2dFast::init(&p).is_err(), "vector2d fastpath");
        assert!(CmsLibVector2dRegular::init(&p).is_err(), "vector2d regularpath");
        // The fixed-shape and parallel rows already refused it, via require_shape.
        assert!(CmsLibFixedmatrix::<crate::wrappers::fixed_matrix::M5x2048>::init(&p).is_err());
    }
}

// ---------- how this sketch is driven ----------
//
// One function per operation, per sketch. These used to be an
// `impl Accumulator for X` block, which fixed one signature for every
// implementation in the repo. As free functions each states its own
// terms, and `catalog` names them in the row's `SketchOps`.
    #[inline(always)]
pub fn insert_cms_oxide(sketch: &mut CmsOxide, v: &i64)
{
        sketch.inner.update(v);
}

pub fn merge_cms_oxide(into: &mut CmsOxide, from: &CmsOxide)
{
        into.inner
            .merge(&from.inner)
            .expect("both operands built from one ParamSet, so rows/cols match");
}
    #[inline(always)]
pub fn insert_cms_datasketches(sketch: &mut CmsDatasketches, v: &i64)
{
        sketch.inner.update(*v);
}

pub fn merge_cms_datasketches(into: &mut CmsDatasketches, from: &CmsDatasketches)
{
        into.inner.merge(&from.inner);
}
    #[inline(always)]
pub fn insert_cms_lib_fixedmatrix<M>(sketch: &mut CmsLibFixedmatrix<M>, v: &i64)
where
    M: MatrixStorage<Counter = i32> + FastPathHasher<DefaultXxHasher> + Default + Clone + 'static,
{
        sketch.0.insert(&DataInput::I64(*v));
}

pub fn merge_cms_lib_fixedmatrix<M>(into: &mut CmsLibFixedmatrix<M>, from: &CmsLibFixedmatrix<M>)
where
    M: MatrixStorage<Counter = i32> + FastPathHasher<DefaultXxHasher> + Default + Clone + 'static,
{
        into.0.merge(&from.0);
}
    #[inline(always)]
pub fn insert_cms_lib_vector2d_fast(sketch: &mut CmsLibVector2dFast, v: &i64)
{
        sketch.inner.insert(&DataInput::I64(*v));
}

pub fn merge_cms_lib_vector2d_fast(into: &mut CmsLibVector2dFast, from: &CmsLibVector2dFast)
{
        into.inner.merge(&from.inner);
}
    #[inline(always)]
pub fn insert_cms_lib_vector2d_regular(sketch: &mut CmsLibVector2dRegular, v: &i64)
{
        sketch.inner.insert(&DataInput::I64(*v));
}

pub fn merge_cms_lib_vector2d_regular(into: &mut CmsLibVector2dRegular, from: &CmsLibVector2dRegular)
{
        into.inner.merge(&from.inner);
}

// ---------- the rows this file provides ----------
//
// One `SketchOps` per row: build is `InitSketch::init`, the rest are the
// functions above. `catalog::ROWS` names `run_*` and nothing else — there is no
// macro, and nothing here has to agree with any other wrapper.

use aqpbm_core::cell::WorkloadData;
use aqpbm_core::ops::SketchOps;
use aqpbm_core::request::Numeric;
use aqpbm_core::runner::{BenchConfig, BenchReport};
use aqpbm_core::cell::RunError;
use aqpbm_core::accuracy::frequency::FrequencyGT;

/// `cms/oxide`. Probe is a key, answer is a count — the shape `FrequencyGT` asks in.
pub const OXIDE_OPS: SketchOps<CmsOxide, i64, i64, u64> = SketchOps {
    merge: Some(merge_cms_oxide),
    prepare: None,
    ask: ask_cms_oxide,
        _item: std::marker::PhantomData,
};
pub fn ask_cms_oxide(sketch: &mut CmsOxide, key: &i64) -> u64 {
    sketch.estimate_frequency(key)
}
pub fn run_oxide(
    cfg: &BenchConfig,
    data: WorkloadData,
    params: &ParamSet,
    width: Numeric,
) -> Result<Vec<BenchReport>, RunError> {
    crate::catalog::run_scored::<CmsOxide, i64, FrequencyGT, _>(cfg, data, params, width, insert_cms_oxide, &OXIDE_OPS)
}

pub const DATASKETCHES_OPS: SketchOps<CmsDatasketches, i64, i64, u64> = SketchOps {
    merge: Some(merge_cms_datasketches),
    prepare: None,
    ask: ask_cms_datasketches,
        _item: std::marker::PhantomData,
};
pub fn ask_cms_datasketches(sketch: &mut CmsDatasketches, key: &i64) -> u64 {
    sketch.estimate_frequency(key)
}
pub fn run_datasketches(
    cfg: &BenchConfig,
    data: WorkloadData,
    params: &ParamSet,
    width: Numeric,
) -> Result<Vec<BenchReport>, RunError> {
    crate::catalog::run_scored::<CmsDatasketches, i64, FrequencyGT, _>(
        cfg,
        data,
        params,
        width,
        insert_cms_datasketches,
        &DATASKETCHES_OPS,
    )
}

pub const VECTOR2D_FAST_OPS: SketchOps<CmsLibVector2dFast, i64, i64, u64> = SketchOps {
    merge: Some(merge_cms_lib_vector2d_fast),
    prepare: None,
    ask: ask_cms_lib_vector2d_fast,
        _item: std::marker::PhantomData,
};
pub fn ask_cms_lib_vector2d_fast(sketch: &mut CmsLibVector2dFast, key: &i64) -> u64 {
    sketch.estimate_frequency(key)
}
pub fn run_vector2d_fast(
    cfg: &BenchConfig,
    data: WorkloadData,
    params: &ParamSet,
    width: Numeric,
) -> Result<Vec<BenchReport>, RunError> {
    crate::catalog::run_scored::<CmsLibVector2dFast, i64, FrequencyGT, _>(
        cfg,
        data,
        params,
        width,
        insert_cms_lib_vector2d_fast,
        &VECTOR2D_FAST_OPS,
    )
}

pub const VECTOR2D_REGULAR_OPS: SketchOps<CmsLibVector2dRegular, i64, i64, u64> = SketchOps {
    merge: Some(merge_cms_lib_vector2d_regular),
    prepare: None,
    ask: ask_cms_lib_vector2d_regular,
        _item: std::marker::PhantomData,
};
pub fn ask_cms_lib_vector2d_regular(sketch: &mut CmsLibVector2dRegular, key: &i64) -> u64 {
    sketch.estimate_frequency(key)
}
pub fn run_vector2d_regular(
    cfg: &BenchConfig,
    data: WorkloadData,
    params: &ParamSet,
    width: Numeric,
) -> Result<Vec<BenchReport>, RunError> {
    crate::catalog::run_scored::<CmsLibVector2dRegular, i64, FrequencyGT, _>(
        cfg,
        data,
        params,
        width,
        insert_cms_lib_vector2d_regular,
        &VECTOR2D_REGULAR_OPS,
    )
}

/// The fixed-matrix row's ops, generic over the storage the shape selected.
/// A `const fn` rather than a `const`, because there is one per `M`.
pub const fn fixedmatrix_ops<M>() -> SketchOps<CmsLibFixedmatrix<M>, i64, i64, u64>
where
    M: MatrixStorage<Counter = i32> + FastPathHasher<DefaultXxHasher> + Default + Clone + 'static,
{
    SketchOps {
        merge: Some(merge_cms_lib_fixedmatrix),
        prepare: None,
        ask: ask_cms_lib_fixedmatrix,
        _item: std::marker::PhantomData,
    }
}
pub fn ask_cms_lib_fixedmatrix<M>(sketch: &mut CmsLibFixedmatrix<M>, key: &i64) -> u64
where
    M: MatrixStorage<Counter = i32> + FastPathHasher<DefaultXxHasher> + Default + Clone + 'static,
{
    sketch.estimate_frequency(key)
}
