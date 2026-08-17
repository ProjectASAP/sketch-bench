//! The `asap_sketchlib` implementations.
//!
//! Grouped under `wrappers/cms/` with the other implementations of this
//! algorithm; how each is driven lives beside it.

use super::*;
use crate::build_error::BuildError;
use crate::wrappers::{
    partition, require_positive, require_shape, M5x32K, PARALLEL_COLS, PARALLEL_ROWS,
};
use aqpbm_core::config::ParamSet;

use asap_sketchlib::{
    CountMin, DataInput, DefaultXxHasher, FastPath, FastPathHasher, MatrixStorage, RegularPath,
    Vector2D,
};
use std::sync::Barrier;

pub struct CmsLibFixedmatrix<M: MatrixStorage>(pub CountMin<M, FastPath>);

pub fn build_cms_lib_fixedmatrix<M>(
    config: &ParamSet,
    _workers: usize,
) -> Result<CmsLibFixedmatrix<M>, BuildError>
where
    M: MatrixStorage<Counter = i32> + FastPathHasher<DefaultXxHasher> + Default + Clone + 'static,
{
    let p: CmsParams = config.parse()?;
    let inner = CountMin::<M, FastPath>::from_storage(M::default());
    // The registry selected `M` from this same pair, so this only fires for a
    // direct caller. It fires rather than silently running at `M`'s shape.
    require_shape(p.rows, p.cols, inner.rows(), inner.cols())?;
    Ok(CmsLibFixedmatrix(inner))
}

pub fn memory_cms_lib_fixedmatrix<M>(sketch: &CmsLibFixedmatrix<M>) -> usize
where
    M: MatrixStorage<Counter = i32> + FastPathHasher<DefaultXxHasher> + Default + Clone + 'static,
{
    // Off the built matrix, so it tracks whichever shape was selected.
    sketch.0.rows() * sketch.0.cols() * std::mem::size_of::<i32>()
}

/// The shape-independent half of the fixed-matrix Count-Min row: its name, and
/// how to read a shape out of a config. `registry` pairs this with the storage
/// type the requested shape selects.
pub struct CmsFixedMatrixRow;
// ---------- asap_sketchlib: Vector2D + FastPath ----------
pub struct CmsLibVector2dFast {
    inner: CountMin<Vector2D<i32>, FastPath>,
    rows: usize,
    cols: usize,
}

pub fn build_cms_lib_vector2d_fast(
    config: &ParamSet,
    _workers: usize,
) -> Result<CmsLibVector2dFast, BuildError> {
    let p: CmsParams = config.parse()?;
    // `Vector2D::init` takes `cols.ilog2()`, which aborts at 0, and a
    // zero-row matrix builds happily and then answers every query out of an
    // empty fold. Refuse both here, as the fixed-shape rows in this file
    // already refuse a shape they cannot serve.
    require_positive("asap CMS Vector2D FastPath", "rows", p.rows)?;
    require_positive("asap CMS Vector2D FastPath", "cols", p.cols)?;
    Ok(CmsLibVector2dFast {
        inner: CountMin::<Vector2D<i32>, FastPath>::with_dimensions(p.rows, p.cols),
        rows: p.rows,
        cols: p.cols,
    })
}

pub fn memory_cms_lib_vector2d_fast(sketch: &CmsLibVector2dFast) -> usize {
    sketch.rows * sketch.cols * std::mem::size_of::<i32>()
}

// ---------- asap_sketchlib: Vector2D + RegularPath ----------
pub struct CmsLibVector2dRegular {
    inner: CountMin<Vector2D<i32>, RegularPath>,
    rows: usize,
    cols: usize,
}

pub fn build_cms_lib_vector2d_regular(
    config: &ParamSet,
    _workers: usize,
) -> Result<CmsLibVector2dRegular, BuildError> {
    let p: CmsParams = config.parse()?;
    // `Vector2D::init` takes `cols.ilog2()`, which aborts at 0, and a
    // zero-row matrix builds happily and then answers every query out of an
    // empty fold. Refuse both here, as the fixed-shape rows in this file
    // already refuse a shape they cannot serve.
    require_positive("asap CMS Vector2D RegularPath", "rows", p.rows)?;
    require_positive("asap CMS Vector2D RegularPath", "cols", p.cols)?;
    Ok(CmsLibVector2dRegular {
        inner: CountMin::<Vector2D<i32>, RegularPath>::with_dimensions(p.rows, p.cols),
        rows: p.rows,
        cols: p.cols,
    })
}

pub fn memory_cms_lib_vector2d_regular(sketch: &CmsLibVector2dRegular) -> usize {
    sketch.rows * sketch.cols * std::mem::size_of::<i32>()
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

pub fn insert_cms_lib_vector2d_fast(sketch: &mut CmsLibVector2dFast, v: &i64) {
    sketch.inner.insert(&DataInput::I64(*v));
}

pub fn merge_cms_lib_vector2d_fast(into: &mut CmsLibVector2dFast, from: &CmsLibVector2dFast) {
    into.inner.merge(&from.inner);
}

pub fn insert_cms_lib_vector2d_regular(sketch: &mut CmsLibVector2dRegular, v: &i64) {
    sketch.inner.insert(&DataInput::I64(*v));
}

pub fn merge_cms_lib_vector2d_regular(
    into: &mut CmsLibVector2dRegular,
    from: &CmsLibVector2dRegular,
) {
    into.inner.merge(&from.inner);
}

pub fn ask_cms_lib_vector2d_fast(sketch: &mut CmsLibVector2dFast, key: &i64) -> u64 {
    sketch.estimate_frequency(key)
}

pub fn ask_cms_lib_vector2d_regular(sketch: &mut CmsLibVector2dRegular, key: &i64) -> u64 {
    sketch.estimate_frequency(key)
}

pub fn ask_cms_lib_fixedmatrix<M>(sketch: &mut CmsLibFixedmatrix<M>, key: &i64) -> u64
where
    M: MatrixStorage<Counter = i32> + FastPathHasher<DefaultXxHasher> + Default + Clone + 'static,
{
    sketch.estimate_frequency(key)
}

/// CMS, parallel-insert FastPath.
pub struct ParallelCmsFastPath {
    buf: Vec<i64>,
    workers: usize,
}

/// Takes the worker count, which is a run knob (`--workers`)
/// rather than a sketch parameter. The construction config is checked
/// against the baked shape and refused if it differs, exactly as the
/// single-threaded fixed-matrix rows do.
pub fn build_parallel_cms_fast_path(
    config: &ParamSet,
    workers: usize,
) -> Result<ParallelCmsFastPath, BuildError> {
    let p: CmsParams = config.parse()?;
    require_shape(p.rows, p.cols, PARALLEL_ROWS, PARALLEL_COLS)?;
    Ok(ParallelCmsFastPath {
        buf: Vec::new(),
        workers: workers.max(1),
    })
}

pub fn memory_parallel_cms_fast_path(sketch: &ParallelCmsFastPath) -> usize {
    sketch.workers * (PARALLEL_ROWS * PARALLEL_COLS * std::mem::size_of::<i32>())
        + sketch.buf.capacity() * std::mem::size_of::<i64>()
}

/// The barrier is load-bearing: a worker inserting while its peers are still
/// being spawned is not measuring a parallel insert. It costs one rendezvous
/// inside the runner's timed region, which is the honest place for it.
fn run_parallel_cms(items: &[i64], workers: usize) {
    let parts = partition(items, workers);
    let barrier = Barrier::new(parts.len());
    std::thread::scope(|s| {
        for part in &parts {
            let barrier = &barrier;
            s.spawn(move || {
                let mut sketch = CountMin::<M5x32K, FastPath>::from_storage(M5x32K::default());
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

pub fn insert_parallel_cms_fast_path(sketch: &mut ParallelCmsFastPath, v: &i64) {
    sketch.buf.push(*v);
}

pub fn prepare_parallel_cms_fast_path(sketch: &mut ParallelCmsFastPath) {
    run_parallel_cms(&sketch.buf, sketch.workers);
}
