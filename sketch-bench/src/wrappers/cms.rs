//! Count-Min Accumulator wrappers — five types, every one declaring
//! `FrequencyOps` (`&i64` point lookup → `u64` count estimate).
//!
//! All of them take `(rows, cols)` and honour it, by three different routes.
//! `oxide` inverts the error bounds its API takes and checks the table it got
//! back. `datasketches` range-checks the `(u8, u32)` its API narrows to.
//! `CmsLibVector2dFast` / `CmsLibVector2dRegular` size at run time.
//! `CmsLibFixedmatrix<M>` is generic over a storage type that bakes the shape
//! in, so the shape selects a monomorphisation from the table in
//! `wrappers::fixed_matrix` and the catalog dispatches on it.

use aqpbm_core::accuracy::FrequencyOps;
use aqpbm_core::init::{BenchImpl, BuildError, InitSketch};
use crate::params::CmsParams;
use crate::wrappers::{
    require_positive, require_range, require_resolved_shape, require_shape,
};
use aqpbm_core::config::ParamSet;
use aqpbm_core::accumulator::{Accumulator, MergeUnsupported};
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

impl<M> Accumulator for CmsLibFixedmatrix<M>
where
    M: MatrixStorage<Counter = i32> + FastPathHasher<DefaultXxHasher> + Default + Clone + 'static,
{
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

impl<M> FrequencyOps for CmsLibFixedmatrix<M>
where
    M: MatrixStorage<Counter = i32> + FastPathHasher<DefaultXxHasher> + Default + Clone + 'static,
{
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

impl<M> BenchImpl for CmsLibFixedmatrix<M>
where
    M: MatrixStorage<Counter = i32> + FastPathHasher<DefaultXxHasher> + Default + Clone + 'static,
{
    type Params = CmsParams;
    const ALGORITHM: &'static str = "cms-fastpath-fixedmatrix";
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
