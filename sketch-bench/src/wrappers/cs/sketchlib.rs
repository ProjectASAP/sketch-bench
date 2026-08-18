//! The `asap_sketchlib` implementations.
//!
//! Grouped under `wrappers/cs/` with the other implementations of this
//! algorithm; how each is driven lives beside it.

use super::*;
use crate::params::ParamSet;
use crate::wrappers::{
    partition, require_positive, require_shape, M5x32K, PARALLEL_COLS, PARALLEL_ROWS,
};
use aqpbm_core::RunError;
use asap_sketchlib::{
    Count, DataInput, DefaultXxHasher, FastPath, FastPathHasher, MatrixStorage, RegularPath,
    Vector2D,
};
use std::sync::Barrier;

pub struct CsLibFixedmatrix<M: MatrixStorage>(pub Count<M, FastPath>);

pub fn build_cs_lib_fixedmatrix<M>(config: &ParamSet) -> Result<CsLibFixedmatrix<M>, RunError>
where
    M: MatrixStorage<Counter = i32> + FastPathHasher<DefaultXxHasher> + Default + Clone + 'static,
{
    let p: CountSketchParams = config.parse()?;
    let inner = Count::<M, FastPath>::from_storage(M::default());
    require_shape(p.rows, p.cols, inner.rows(), inner.cols())?;
    Ok(CsLibFixedmatrix(inner))
}

pub fn memory_cs_lib_fixedmatrix<M>(sketch: &CsLibFixedmatrix<M>) -> usize
where
    M: MatrixStorage<Counter = i32> + FastPathHasher<DefaultXxHasher> + Default + Clone + 'static,
{
    sketch.0.rows() * sketch.0.cols() * std::mem::size_of::<i32>()
}

/// The CountSketch counterpart of `CmsFixedMatrixRow`.
pub struct CsFixedMatrixRow;

// ---------- asap_sketchlib: Vector2D + FastPath ----------
pub struct CsLibVector2dFast {
    inner: Count<Vector2D<i32>, FastPath>,
    rows: usize,
    cols: usize,
}

pub fn build_cs_lib_vector2d_fast(config: &ParamSet) -> Result<CsLibVector2dFast, RunError> {
    let p: CountSketchParams = config.parse()?;
    // `Vector2D::init` takes `cols.ilog2()`, which aborts at 0, and a
    // zero-row matrix builds happily and then answers every query out of an
    // empty fold. Refuse both here, as the fixed-shape rows in this file
    // already refuse a shape they cannot serve.
    require_positive("asap Count Vector2D FastPath", "rows", p.rows)?;
    require_positive("asap Count Vector2D FastPath", "cols", p.cols)?;
    Ok(CsLibVector2dFast {
        inner: Count::<Vector2D<i32>, FastPath>::with_dimensions(p.rows, p.cols),
        rows: p.rows,
        cols: p.cols,
    })
}

pub fn memory_cs_lib_vector2d_fast(sketch: &CsLibVector2dFast) -> usize {
    sketch.rows * sketch.cols * std::mem::size_of::<i32>()
}

// ---------- asap_sketchlib: Vector2D + RegularPath ----------
pub struct CsLibVector2dRegular {
    inner: Count<Vector2D<i32>, RegularPath>,
    rows: usize,
    cols: usize,
}

pub fn build_cs_lib_vector2d_regular(config: &ParamSet) -> Result<CsLibVector2dRegular, RunError> {
    let p: CountSketchParams = config.parse()?;
    // `Vector2D::init` takes `cols.ilog2()`, which aborts at 0, and a
    // zero-row matrix builds happily and then answers every query out of an
    // empty fold. Refuse both here, as the fixed-shape rows in this file
    // already refuse a shape they cannot serve.
    require_positive("asap Count Vector2D RegularPath", "rows", p.rows)?;
    require_positive("asap Count Vector2D RegularPath", "cols", p.cols)?;
    Ok(CsLibVector2dRegular {
        inner: Count::<Vector2D<i32>, RegularPath>::with_dimensions(p.rows, p.cols),
        rows: p.rows,
        cols: p.cols,
    })
}

pub fn memory_cs_lib_vector2d_regular(sketch: &CsLibVector2dRegular) -> usize {
    sketch.rows * sketch.cols * std::mem::size_of::<i32>()
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

pub fn insert_cs_lib_vector2d_fast(sketch: &mut CsLibVector2dFast, v: &i64) {
    sketch.inner.insert(&DataInput::I64(*v));
}

pub fn merge_cs_lib_vector2d_fast(into: &mut CsLibVector2dFast, from: &CsLibVector2dFast) {
    into.inner.merge(&from.inner);
}

pub fn insert_cs_lib_vector2d_regular(sketch: &mut CsLibVector2dRegular, v: &i64) {
    sketch.inner.insert(&DataInput::I64(*v));
}

pub fn merge_cs_lib_vector2d_regular(into: &mut CsLibVector2dRegular, from: &CsLibVector2dRegular) {
    into.inner.merge(&from.inner);
}

pub fn query_cs_lib_vector2d_fast(sketch: &mut CsLibVector2dFast, key: &i64) -> u64 {
    sketch.estimate_frequency(key)
}

pub fn query_cs_lib_vector2d_regular(sketch: &mut CsLibVector2dRegular, key: &i64) -> u64 {
    sketch.estimate_frequency(key)
}

pub fn query_cs_lib_fixedmatrix<M>(sketch: &mut CsLibFixedmatrix<M>, key: &i64) -> u64
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

/// CountSketch, parallel-insert FastPath.
pub struct ParallelCsFastPath {
    workers: usize,
}

pub fn build_parallel_cs_fast_path(
    config: &ParamSet,
    workers: usize,
) -> Result<ParallelCsFastPath, RunError> {
    let p: CountSketchParams = config.parse()?;
    require_shape(p.rows, p.cols, PARALLEL_ROWS, PARALLEL_COLS)?;
    Ok(ParallelCsFastPath {
        workers: workers.max(1),
    })
}

pub fn memory_parallel_cs_fast_path(sketch: &ParallelCsFastPath) -> usize {
    sketch.workers * (PARALLEL_ROWS * PARALLEL_COLS * std::mem::size_of::<i32>())
}

/// The whole stream in one call: this row's ingest *is* the parallel fan-out,
/// so there is no per-item step to buffer and none to time.
pub fn insert_parallel_cs_fast_path(sketch: &mut ParallelCsFastPath, items: &[i64]) {
    run_parallel_cs(items, sketch.workers);
}
