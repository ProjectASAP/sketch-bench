//! CountSketch wrappers — four types (`oxide` + 3× sketchlib), each declaring
//! `FrequencyOps`. Same shapes and the same three routes as the Count-Min
//! wrappers next door, including one generic row over the shared shape table.

use aqpbm_core::accuracy::FrequencyOps;
use aqpbm_core::init::{BenchImpl, BuildError, InitSketch};
use crate::params::CountSketchParams;
use crate::wrappers::{require_positive, require_resolved_shape, require_shape};
use aqpbm_core::config::ParamSet;
use aqpbm_core::accumulator::{Accumulator, MergeUnsupported};
use aqpbm_core::memory_footprint::MemoryFootprint;
use asap_sketchlib::{
    Count, DataInput, DefaultXxHasher, FastPath, FastPathHasher, MatrixStorage, RegularPath,
    Vector2D,
};
use sketch_oxide::Mergeable as _;

// sketch_oxide::frequency::CountSketch sizes its table as
// `width = ceil(3/ε²).next_power_of_two()`, not `ceil(2/ε)` like CountMin.
//
// Inverting that for a target width needs `3/ε²` to land *on* `cols`, and
// `ε = sqrt(3/cols)` does not: the square root and the square do not round-trip
// in `f64`, so `3/ε²` came out a hair above `cols`, `ceil` took it to `cols + 1`
// and the power-of-two rounding doubled the table. Every request built twice the
// counters it named. Solving against `cols - 0.5` puts the quotient half an
// integer below the boundary, which no single ulp can cross.
fn dims_to_err(rows: usize, cols: usize) -> (f64, f64) {
    let epsilon = (3.0 / (cols as f64 - 0.5)).sqrt();
    let delta = (-(rows as f64 - 0.5)).exp();
    (epsilon, delta)
}

// ---------- sketch_oxide CountSketch ----------
// No `rows` / `cols` field, for the same reason as `CmsOxide`: `init` proves the
// built table matches the request, so the sketch is the only place either
// figure is read from.
//
// One bound this row has and the Count-Min one does not: the crate floors its
// depth at 3, so the median is taken over enough estimates to be one. `rows < 3`
// is therefore unreachable, and refused by name.
pub struct CsOxide {
    inner: sketch_oxide::frequency::CountSketch,
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
        require_resolved_shape(
            "oxide CountSketch",
            (inner.depth(), inner.width()),
            (p.rows, p.cols),
        )?;
        Ok(Self { inner })
    }
}

impl Accumulator for CsOxide {
    type Item = i64;
    #[inline(always)]
    fn update(&mut self, v: &i64) {
        self.inner.update(v, 1);
    }

    /// Counter-wise addition; Count Accumulator is linear, so merging is exact.
    fn merge(&mut self, other: &Self) -> Result<(), MergeUnsupported> {
        self.inner
            .merge(&other.inner)
            .expect("both operands built from one ParamSet, so rows/cols match");
        Ok(())
    }
}

impl MemoryFootprint for CsOxide {
    fn memory_bytes(&self) -> usize {
        // Off the built sketch: the crate rounds the width up to a power of two
        // and floors the depth at 3, so at `rows=2` a request-derived figure
        // under-reports by a third. Backing store is `table: Vec<i64>`.
        self.inner.depth() * self.inner.width() * std::mem::size_of::<i64>()
    }
}

// ---------- asap_sketchlib: FixedMatrix + FastPath ----------
//
// One row over every compiled-in shape, exactly as the Count-Min side. The
// shapes are shared: a matrix is a matrix, and both sketches read the same table
// in `wrappers::fixed_matrix`.

pub struct CsLibFixedmatrix<M: MatrixStorage>(pub Count<M, FastPath>);

impl<M> InitSketch for CsLibFixedmatrix<M>
where
    M: MatrixStorage<Counter = i32> + FastPathHasher<DefaultXxHasher> + Default + Clone + 'static,
{
    fn init(config: &ParamSet) -> Result<Self, BuildError> {
        let p: CountSketchParams = config.parse()?;
        let inner = Count::<M, FastPath>::from_storage(M::default());
        require_shape(p.rows, p.cols, inner.rows(), inner.cols())?;
        Ok(Self(inner))
    }
}

impl<M> Accumulator for CsLibFixedmatrix<M>
where
    M: MatrixStorage<Counter = i32> + FastPathHasher<DefaultXxHasher> + Default + Clone + 'static,
{
    type Item = i64;
    #[inline(always)]
    fn update(&mut self, v: &i64) {
        self.0.insert(&DataInput::I64(*v));
    }

    /// Counter-wise addition; Count Accumulator is linear, so merging is exact.
    fn merge(&mut self, other: &Self) -> Result<(), MergeUnsupported> {
        self.0.merge(&other.0);
        Ok(())
    }
}

impl<M> MemoryFootprint for CsLibFixedmatrix<M>
where
    M: MatrixStorage<Counter = i32> + FastPathHasher<DefaultXxHasher> + Default + Clone + 'static,
{
    fn memory_bytes(&self) -> usize {
        self.0.rows() * self.0.cols() * std::mem::size_of::<i32>()
    }
}

/// The CountSketch counterpart of `CmsFixedMatrix`.
pub struct CsFixedMatrix;

impl crate::wrappers::fixed_matrix::FixedMatrixRegistration for CsFixedMatrix {
    const ALGORITHM: &'static str = "countsketch-fastpath-fixedmatrix";
    type At<
        M: MatrixStorage<Counter = i32>
            + FastPathHasher<DefaultXxHasher>
            + Default
            + Clone
            + 'static,
    > = CsLibFixedmatrix<M>;
    fn shape(params: &ParamSet) -> Result<(usize, usize), aqpbm_core::cell::RunError> {
        let p: CountSketchParams = params.parse().map_err(BuildError::from)?;
        Ok((p.rows, p.cols))
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
        // `Vector2D::init` takes `cols.ilog2()`, which aborts at 0, and a
        // zero-row matrix builds happily and then answers every query out of an
        // empty fold. Refuse both here, as the fixed-shape rows in this file
        // already refuse a shape they cannot serve.
        require_positive("asap Count Vector2D FastPath", "rows", p.rows)?;
        require_positive("asap Count Vector2D FastPath", "cols", p.cols)?;
        Ok(Self {
            inner: Count::<Vector2D<i32>, FastPath>::with_dimensions(p.rows, p.cols),
            rows: p.rows,
            cols: p.cols,
        })
    }
}

impl Accumulator for CsLibVector2dFast {
    type Item = i64;
    #[inline(always)]
    fn update(&mut self, v: &i64) {
        self.inner.insert(&DataInput::I64(*v));
    }

    /// Counter-wise addition; Count Accumulator is linear, so merging is exact.
    fn merge(&mut self, other: &Self) -> Result<(), MergeUnsupported> {
        self.inner.merge(&other.inner);
        Ok(())
    }
}

impl MemoryFootprint for CsLibVector2dFast {
    fn memory_bytes(&self) -> usize {
        self.rows * self.cols * std::mem::size_of::<i32>()
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
        // `Vector2D::init` takes `cols.ilog2()`, which aborts at 0, and a
        // zero-row matrix builds happily and then answers every query out of an
        // empty fold. Refuse both here, as the fixed-shape rows in this file
        // already refuse a shape they cannot serve.
        require_positive("asap Count Vector2D RegularPath", "rows", p.rows)?;
        require_positive("asap Count Vector2D RegularPath", "cols", p.cols)?;
        Ok(Self {
            inner: Count::<Vector2D<i32>, RegularPath>::with_dimensions(p.rows, p.cols),
            rows: p.rows,
            cols: p.cols,
        })
    }
}

impl Accumulator for CsLibVector2dRegular {
    type Item = i64;
    #[inline(always)]
    fn update(&mut self, v: &i64) {
        self.inner.insert(&DataInput::I64(*v));
    }

    /// Counter-wise addition; Count Accumulator is linear, so merging is exact.
    fn merge(&mut self, other: &Self) -> Result<(), MergeUnsupported> {
        self.inner.merge(&other.inner);
        Ok(())
    }
}

impl MemoryFootprint for CsLibVector2dRegular {
    fn memory_bytes(&self) -> usize {
        self.rows * self.cols * std::mem::size_of::<i32>()
    }
}


// ---------- statistic membership ----------
//
// A different algorithm from CMS (its own params) answering the same statistic.

impl FrequencyOps for CsOxide {
    type Key = i64;
    fn estimate_frequency(&self, key: &i64) -> u64 {
        // CountSketch is unbiased (median of sign·counter), so collision noise
        // can push an estimate slightly negative. Clamp to 0 for CMS-style
        // semantics — otherwise `as u64` wraps -1 into u64::MAX.
        self.inner.estimate(key).max(0) as u64
    }
}

impl<M> FrequencyOps for CsLibFixedmatrix<M>
where
    M: MatrixStorage<Counter = i32> + FastPathHasher<DefaultXxHasher> + Default + Clone + 'static,
{
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
// Named on the same rule as the Count-Min rows: hash strategy and storage
// backend name the algorithm, the library names the impl.

impl BenchImpl for CsOxide { type Params = CountSketchParams; const IMPL: &'static str = "oxide"; }

impl<M> BenchImpl for CsLibFixedmatrix<M>
where
    M: MatrixStorage<Counter = i32> + FastPathHasher<DefaultXxHasher> + Default + Clone + 'static,
{
    type Params = CountSketchParams;
    const ALGORITHM: &'static str = "countsketch-fastpath-fixedmatrix";
    const IMPL: &'static str = "lib";
}
impl BenchImpl for CsLibVector2dFast {
    type Params = CountSketchParams;
    const ALGORITHM: &'static str = "countsketch-fastpath-vector2d";
    const IMPL: &'static str = "lib";
}
impl BenchImpl for CsLibVector2dRegular {
    type Params = CountSketchParams;
    const ALGORITHM: &'static str = "countsketch-regularpath-vector2d";
    const IMPL: &'static str = "lib";
}
