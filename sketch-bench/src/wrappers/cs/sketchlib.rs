//! The `asap_sketchlib` implementations.
//!
//! Grouped under `wrappers/cs/` with the other implementations of this
//! algorithm; how each is driven lives beside it.

use super::*;
use crate::wrappers::parallel_shared::*;
use aqpbm_core::accuracy::frequency::FrequencyGT;
use aqpbm_core::cell::{RunError, WorkloadData};
use aqpbm_core::config::ParamSet;
use aqpbm_core::init::{BenchImpl, BuildError, InitSketch};
use aqpbm_core::memory_footprint::MemoryFootprint;
use aqpbm_core::ops::SketchOps;
use aqpbm_core::request::Numeric;
use aqpbm_core::runner::{BenchConfig, BenchReport};
use crate::wrappers::{require_positive, require_shape};
use asap_sketchlib::{
    Count, DataInput, DefaultXxHasher, FastPath,
    FastPathHasher, MatrixStorage, RegularPath, Vector2D};
use std::sync::Barrier;

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

impl<M> MemoryFootprint for CsLibFixedmatrix<M>
where
    M: MatrixStorage<Counter = i32> + FastPathHasher<DefaultXxHasher> + Default + Clone + 'static,
{
    fn memory_bytes(&self) -> usize {
        self.0.rows() * self.0.cols() * std::mem::size_of::<i32>()
    }
}

/// The CountSketch counterpart of `CmsFixedMatrixRow`.
pub struct CsFixedMatrixRow;

impl crate::catalog::FixedMatrixRow for CsFixedMatrixRow {
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
        insert_cs_lib_fixedmatrix(sketch, v)
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
    cols: usize}

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
            cols: p.cols})
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
    cols: usize}

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
            cols: p.cols})
    }
}

impl MemoryFootprint for CsLibVector2dRegular {
    fn memory_bytes(&self) -> usize {
        self.rows * self.cols * std::mem::size_of::<i32>()
    }
}

impl<M> CsLibFixedmatrix<M>
where
    M: MatrixStorage<Counter = i32> + FastPathHasher<DefaultXxHasher> + Default + Clone + 'static,
{
    pub fn estimate_frequency(&self, key: &i64) -> u64 {
        self.0.estimate(&DataInput::I64(*key)) as u64
    }
}

impl CsLibVector2dFast {
    pub fn estimate_frequency(&self, key: &i64) -> u64 {
        self.inner.estimate(&DataInput::I64(*key)) as u64
    }
}

impl CsLibVector2dRegular {
    pub fn estimate_frequency(&self, key: &i64) -> u64 {
        self.inner.estimate(&DataInput::I64(*key)) as u64
    }
}

impl<M> BenchImpl for CsLibFixedmatrix<M>
where
    M: MatrixStorage<Counter = i32> + FastPathHasher<DefaultXxHasher> + Default + Clone + 'static,
{
    type Params = CountSketchParams;
    const ALGORITHM: &'static str = "countsketch-fastpath-fixedmatrix";
    const IMPL: &'static str = "lib";
    const SUPPORTS_MERGE: bool = true;
}

impl BenchImpl for CsLibVector2dFast {
    type Params = CountSketchParams;
    const ALGORITHM: &'static str = "countsketch-fastpath-vector2d";
    const IMPL: &'static str = "lib";
    const SUPPORTS_MERGE: bool = true;
}

impl BenchImpl for CsLibVector2dRegular {
    type Params = CountSketchParams;
    const ALGORITHM: &'static str = "countsketch-regularpath-vector2d";
    const IMPL: &'static str = "lib";
    const SUPPORTS_MERGE: bool = true;
}

pub fn insert_cs_lib_fixedmatrix<M>(sketch: &mut CsLibFixedmatrix<M>, v: &i64)
where
    M: MatrixStorage<Counter = i32> + FastPathHasher<DefaultXxHasher> + Default + Clone + 'static,
{
        sketch.0.insert(&DataInput::I64(*v));
}

pub fn merge_cs_lib_fixedmatrix<M>(into: &mut CsLibFixedmatrix<M>, from: &CsLibFixedmatrix<M>)
where
    M: MatrixStorage<Counter = i32> + FastPathHasher<DefaultXxHasher> + Default + Clone + 'static,
{
        into.0.merge(&from.0);
}

pub fn insert_cs_lib_vector2d_fast(sketch: &mut CsLibVector2dFast, v: &i64)
{
        sketch.inner.insert(&DataInput::I64(*v));
}

pub fn merge_cs_lib_vector2d_fast(into: &mut CsLibVector2dFast, from: &CsLibVector2dFast)
{
        into.inner.merge(&from.inner);
}

pub fn insert_cs_lib_vector2d_regular(sketch: &mut CsLibVector2dRegular, v: &i64)
{
        sketch.inner.insert(&DataInput::I64(*v));
}

pub fn merge_cs_lib_vector2d_regular(into: &mut CsLibVector2dRegular, from: &CsLibVector2dRegular)
{
        into.inner.merge(&from.inner);
}

pub const VECTOR2D_FAST_OPS: SketchOps<CsLibVector2dFast, i64, i64, u64> = SketchOps {
    merge: Some(merge_cs_lib_vector2d_fast),
    prepare: None,
    ask: ask_cs_lib_vector2d_fast,
        _item: std::marker::PhantomData};

pub fn ask_cs_lib_vector2d_fast(sketch: &mut CsLibVector2dFast, key: &i64) -> u64 {
    sketch.estimate_frequency(key)
}

pub const VECTOR2D_REGULAR_OPS: SketchOps<CsLibVector2dRegular, i64, i64, u64> = SketchOps {
    merge: Some(merge_cs_lib_vector2d_regular),
    prepare: None,
    ask: ask_cs_lib_vector2d_regular,
        _item: std::marker::PhantomData};

pub fn ask_cs_lib_vector2d_regular(sketch: &mut CsLibVector2dRegular, key: &i64) -> u64 {
    sketch.estimate_frequency(key)
}

pub const fn fixedmatrix_ops<M>() -> SketchOps<CsLibFixedmatrix<M>, i64, i64, u64>
where
    M: MatrixStorage<Counter = i32> + FastPathHasher<DefaultXxHasher> + Default + Clone + 'static,
{
    SketchOps {
        merge: Some(merge_cs_lib_fixedmatrix),
        prepare: None,
        ask: ask_cs_lib_fixedmatrix,
        _item: std::marker::PhantomData}
}

pub fn ask_cs_lib_fixedmatrix<M>(sketch: &mut CsLibFixedmatrix<M>, key: &i64) -> u64
where
    M: MatrixStorage<Counter = i32> + FastPathHasher<DefaultXxHasher> + Default + Clone + 'static,
{
    sketch.estimate_frequency(key)
}

fn run_parallel_cs(items: &[i64], workers: usize) {
    let parts = partition(items, workers);
    let barrier = Barrier::new(parts.len());
    std::thread::scope(|s| {
        for part in &parts {
            let barrier = &barrier;
            s.spawn(move || {
                let mut sketch = Count::<M5x32K, FastPath>::from_storage(M5x32K::default());
                barrier.wait();
                for &v in *part {
                    sketch.insert_emit_delta(&DataInput::I64(v), &mut |d| {
                        std::hint::black_box(&d);
                    });
                }
                std::hint::black_box(&sketch);
            });
        }
    });
}

pub const CS_OPS: SketchOps<ParallelCsFastPath, i64, (), ()> = SketchOps {
    merge: None,
    prepare: Some(prepare_parallel_cs_fast_path),
    ask: |_, _| (),
        _item: std::marker::PhantomData};

pub fn run_cs(
    cfg: &BenchConfig,
    data: WorkloadData,
    params: &ParamSet,
    width: Numeric,
) -> Result<Vec<BenchReport>, RunError> {
    crate::catalog::run_parallel::<ParallelCsFastPath, i64, _>(cfg, data, params, width, insert_parallel_cs_fast_path, &CS_OPS)
}

/// CountSketch, parallel-insert FastPath.
pub struct ParallelCsFastPath {
    buf: Vec<i64>,
    workers: usize}

impl aqpbm_core::cell::ParallelInit for ParallelCsFastPath {
    fn build(config: &ParamSet, workers: usize) -> Result<Self, BuildError> {
        let p: CountSketchParams = config.parse()?;
        require_shape(p.rows, p.cols, PARALLEL_ROWS, PARALLEL_COLS)?;
        Ok(Self {
            buf: Vec::new(),
            workers: workers.max(1)})
    }
}

impl MemoryFootprint for ParallelCsFastPath {
    fn memory_bytes(&self) -> usize {
        self.workers * (PARALLEL_ROWS * PARALLEL_COLS * std::mem::size_of::<i32>())
            + self.buf.capacity() * std::mem::size_of::<i64>()
    }
}

impl BenchImpl for ParallelCsFastPath {
    type Params = CountSketchParams;
    const ALGORITHM: &'static str = "countsketch-fastpath-fixedmatrix-32k-parallel";
    const IMPL: &'static str = "lib";
    const SUPPORTS_PREPARE: bool = true;
}

pub fn run_vector2d_fast(
    cfg: &BenchConfig,
    data: WorkloadData,
    params: &ParamSet,
    width: Numeric,
) -> Result<Vec<BenchReport>, RunError> {
    crate::catalog::run_scored::<CsLibVector2dFast, i64, FrequencyGT, _>(
        cfg, data, params, width, insert_cs_lib_vector2d_fast,
        &VECTOR2D_FAST_OPS,
    )
}

pub fn run_vector2d_regular(
    cfg: &BenchConfig,
    data: WorkloadData,
    params: &ParamSet,
    width: Numeric,
) -> Result<Vec<BenchReport>, RunError> {
    crate::catalog::run_scored::<CsLibVector2dRegular, i64, FrequencyGT, _>(
        cfg, data, params, width, insert_cs_lib_vector2d_regular,
        &VECTOR2D_REGULAR_OPS,
    )
}

pub fn insert_parallel_cs_fast_path(sketch: &mut ParallelCsFastPath, v: &i64)
{
        sketch.buf.push(*v);
}

pub fn prepare_parallel_cs_fast_path(sketch: &mut ParallelCsFastPath)
{
        run_parallel_cs(&sketch.buf, sketch.workers);
}
