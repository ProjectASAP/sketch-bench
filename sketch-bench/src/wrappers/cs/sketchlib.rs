//! The `asap_sketchlib` implementations.
//!
//! Grouped under `wrappers/cs/` with the other implementations of this
//! algorithm; how each is driven lives beside it.

use super::*;
use crate::params::ParamSet;
use crate::wrappers::{
    partition, require_positive, require_shape, M5x32K, PARALLEL_COLS, PARALLEL_ROWS,
};
use crate::wrappers::{BuildError, Pass, QueryPass, Shared, StepPass};
use asap_sketchlib::{
    Count, DefaultXxHasher, FastPath, FastPathHasher, MatrixStorage, RegularPath,
    Vector2D,
};
use std::cell::RefCell;
use std::rc::Rc;

use crate::wrappers::frequency_value::FrequencyValue;
use std::sync::Barrier;

pub struct CsLibFixedmatrix<M: MatrixStorage>(pub Count<M, FastPath>);

pub fn build_cs_lib_fixedmatrix<M>(config: &ParamSet) -> Result<CsLibFixedmatrix<M>, BuildError>
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

pub fn build_cs_lib_vector2d_fast(config: &ParamSet) -> Result<CsLibVector2dFast, BuildError> {
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

pub fn build_cs_lib_vector2d_regular(
    config: &ParamSet,
) -> Result<CsLibVector2dRegular, BuildError> {
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

fn run_parallel_cs<T: FrequencyValue>(items: &[T], workers: usize) {
    let parts = partition(items, workers);
    let barrier = Barrier::new(parts.len());
    std::thread::scope(|s| {
        for part in &parts {
            let barrier = &barrier;
            s.spawn(move || {
                let mut sketch = Count::<M5x32K, FastPath>::from_storage(M5x32K::default());
                barrier.wait();
                for v in *part {
                    sketch.insert_emit_delta(&v.data_input(), &mut |d| {
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
) -> Result<ParallelCsFastPath, BuildError> {
    let p: CountSketchParams = config.parse()?;
    require_shape(p.rows, p.cols, PARALLEL_ROWS, PARALLEL_COLS)?;
    Ok(ParallelCsFastPath {
        workers: workers.max(1),
    })
}

pub fn memory_parallel_cs_fast_path(sketch: &ParallelCsFastPath) -> usize {
    sketch.workers * (PARALLEL_ROWS * PARALLEL_COLS * std::mem::size_of::<i32>())
}

pub fn insert_cs_lib_fixedmatrix<M, T>(
    params: &ParamSet,
    items: Rc<Vec<T>>,
    passes: usize,
) -> Result<Vec<Pass>, BuildError>
where
    M: MatrixStorage<Counter = i32> + FastPathHasher<DefaultXxHasher> + Default + Clone + 'static,
    T: FrequencyValue,
{
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        // Built here, so calling the closure is the insert and nothing else.
        let mut sketch = build_cs_lib_fixedmatrix::<M>(params)?;
        let items = items.clone();
        out.push(Box::new(move || {
            for v in items.iter() {
                sketch.0.insert(&v.data_input());
            }
            memory_cs_lib_fixedmatrix::<M>(&sketch)
        }) as Pass);
    }
    Ok(out)
}

pub fn insert_step_cs_lib_fixedmatrix<M, T>(
    params: &ParamSet,
    items: Rc<Vec<T>>,
    passes: usize,
) -> Result<Vec<StepPass>, BuildError>
where
    M: MatrixStorage<Counter = i32> + FastPathHasher<DefaultXxHasher> + Default + Clone + 'static,
    T: FrequencyValue,
{
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        let sketch: Shared<_> = Rc::new(RefCell::new(build_cs_lib_fixedmatrix::<M>(params)?));
        let (driven, read) = (sketch.clone(), sketch);
        let stream = items.clone();
        out.push(StepPass {
            steps: items.len(),
            step: Box::new(move |i| {
                let sketch = &mut *driven.borrow_mut();
                let v = &stream[i];
                sketch.0.insert(&v.data_input());
            }),
            footprint: Box::new(move || memory_cs_lib_fixedmatrix::<M>(&read.borrow())),
        });
    }
    Ok(out)
}

pub fn query_cs_lib_fixedmatrix<M, T>(
    params: &ParamSet,
    items: Rc<Vec<T>>,
    probes: Rc<Vec<T>>,
    passes: usize,
) -> Result<Vec<QueryPass<u64>>, BuildError>
where
    M: MatrixStorage<Counter = i32> + FastPathHasher<DefaultXxHasher> + Default + Clone + 'static,
    T: FrequencyValue,
{
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        // Built and fed here: the closure below asks, and only asks.
        let mut sketch = build_cs_lib_fixedmatrix::<M>(params)?;
        for v in items.iter() {
            sketch.0.insert(&v.data_input());
        }
        let probes = probes.clone();
        out.push(Box::new(move || {
            let mut answers = Vec::with_capacity(probes.len());
            for p in probes.iter() {
                answers.push(sketch.0.estimate(&p.data_input()) as u64);
            }
            let footprint = memory_cs_lib_fixedmatrix::<M>(&sketch);
            (answers, footprint)
        }) as QueryPass<u64>);
    }
    Ok(out)
}

pub fn merge_cs_lib_fixedmatrix<M, T>(
    params: &ParamSet,
    items: Rc<Vec<T>>,
    shards: usize,
    passes: usize,
) -> Result<Vec<Pass>, BuildError>
where
    M: MatrixStorage<Counter = i32> + FastPathHasher<DefaultXxHasher> + Default + Clone + 'static,
    T: FrequencyValue,
{
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        let (mut acc, rest) = cs_lib_fixedmatrix_shards(params, &items, shards)?;
        out.push(Box::new(move || {
            for other in rest.iter() {
                acc.0.merge(&other.0);
            }
            memory_cs_lib_fixedmatrix::<M>(&acc)
        }) as Pass);
    }
    Ok(out)
}

pub fn merge_step_cs_lib_fixedmatrix<M, T>(
    params: &ParamSet,
    items: Rc<Vec<T>>,
    shards: usize,
    passes: usize,
) -> Result<Vec<StepPass>, BuildError>
where
    M: MatrixStorage<Counter = i32> + FastPathHasher<DefaultXxHasher> + Default + Clone + 'static,
    T: FrequencyValue,
{
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        let (acc, rest) = cs_lib_fixedmatrix_shards(params, &items, shards)?;
        let acc: Shared<_> = Rc::new(RefCell::new(acc));
        let (driven, read) = (acc.clone(), acc);
        out.push(StepPass {
            steps: rest.len(),
            step: Box::new(move |i| {
                let acc = &mut *driven.borrow_mut();
                let other = &rest[i];
                acc.0.merge(&other.0);
            }),
            footprint: Box::new(move || memory_cs_lib_fixedmatrix::<M>(&read.borrow())),
        });
    }
    Ok(out)
}

/// The shards a fold folds: the stream split `shards` ways, one sketch each,
/// all fed. The first is the accumulator, the rest are what it folds in.
#[allow(clippy::type_complexity)]
fn cs_lib_fixedmatrix_shards<M, T>(
    params: &ParamSet,
    items: &[T],
    shards: usize,
) -> Result<(CsLibFixedmatrix<M>, Vec<CsLibFixedmatrix<M>>), BuildError>
where
    M: MatrixStorage<Counter = i32> + FastPathHasher<DefaultXxHasher> + Default + Clone + 'static,
    T: FrequencyValue,
{
    let mut parts: Vec<CsLibFixedmatrix<M>> = Vec::new();
    for shard in partition(items, shards) {
        let mut sketch = build_cs_lib_fixedmatrix::<M>(params)?;
        for v in shard {
            sketch.0.insert(&v.data_input());
        }
        parts.push(sketch);
    }
    let rest = parts.split_off(1);
    Ok((
        parts.pop().expect("a split of the stream is never empty"),
        rest,
    ))
}

pub fn insert_cs_lib_vector2d_fast<T: FrequencyValue>(
    params: &ParamSet,
    items: Rc<Vec<T>>,
    passes: usize,
) -> Result<Vec<Pass>, BuildError> {
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        // Built here, so calling the closure is the insert and nothing else.
        let mut sketch = build_cs_lib_vector2d_fast(params)?;
        let items = items.clone();
        out.push(Box::new(move || {
            for v in items.iter() {
                sketch.inner.insert(&v.data_input());
            }
            memory_cs_lib_vector2d_fast(&sketch)
        }) as Pass);
    }
    Ok(out)
}

pub fn insert_step_cs_lib_vector2d_fast<T: FrequencyValue>(
    params: &ParamSet,
    items: Rc<Vec<T>>,
    passes: usize,
) -> Result<Vec<StepPass>, BuildError> {
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        let sketch: Shared<_> = Rc::new(RefCell::new(build_cs_lib_vector2d_fast(params)?));
        let (driven, read) = (sketch.clone(), sketch);
        let stream = items.clone();
        out.push(StepPass {
            steps: items.len(),
            step: Box::new(move |i| {
                let sketch = &mut *driven.borrow_mut();
                let v = &stream[i];
                sketch.inner.insert(&v.data_input());
            }),
            footprint: Box::new(move || memory_cs_lib_vector2d_fast(&read.borrow())),
        });
    }
    Ok(out)
}

pub fn query_cs_lib_vector2d_fast<T: FrequencyValue>(
    params: &ParamSet,
    items: Rc<Vec<T>>,
    probes: Rc<Vec<T>>,
    passes: usize,
) -> Result<Vec<QueryPass<u64>>, BuildError> {
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        // Built and fed here: the closure below asks, and only asks.
        let mut sketch = build_cs_lib_vector2d_fast(params)?;
        for v in items.iter() {
            sketch.inner.insert(&v.data_input());
        }
        let probes = probes.clone();
        out.push(Box::new(move || {
            let mut answers = Vec::with_capacity(probes.len());
            for p in probes.iter() {
                answers.push(sketch.inner.estimate(&p.data_input()) as u64);
            }
            let footprint = memory_cs_lib_vector2d_fast(&sketch);
            (answers, footprint)
        }) as QueryPass<u64>);
    }
    Ok(out)
}

pub fn merge_cs_lib_vector2d_fast<T: FrequencyValue>(
    params: &ParamSet,
    items: Rc<Vec<T>>,
    shards: usize,
    passes: usize,
) -> Result<Vec<Pass>, BuildError> {
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        let (mut acc, rest) = cs_lib_vector2d_fast_shards(params, &items, shards)?;
        out.push(Box::new(move || {
            for other in rest.iter() {
                acc.inner.merge(&other.inner);
            }
            memory_cs_lib_vector2d_fast(&acc)
        }) as Pass);
    }
    Ok(out)
}

pub fn merge_step_cs_lib_vector2d_fast<T: FrequencyValue>(
    params: &ParamSet,
    items: Rc<Vec<T>>,
    shards: usize,
    passes: usize,
) -> Result<Vec<StepPass>, BuildError> {
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        let (acc, rest) = cs_lib_vector2d_fast_shards(params, &items, shards)?;
        let acc: Shared<_> = Rc::new(RefCell::new(acc));
        let (driven, read) = (acc.clone(), acc);
        out.push(StepPass {
            steps: rest.len(),
            step: Box::new(move |i| {
                let acc = &mut *driven.borrow_mut();
                let other = &rest[i];
                acc.inner.merge(&other.inner);
            }),
            footprint: Box::new(move || memory_cs_lib_vector2d_fast(&read.borrow())),
        });
    }
    Ok(out)
}

/// The shards a fold folds: the stream split `shards` ways, one sketch each,
/// all fed. The first is the accumulator, the rest are what it folds in.
#[allow(clippy::type_complexity)]
fn cs_lib_vector2d_fast_shards<T: FrequencyValue>(
    params: &ParamSet,
    items: &[T],
    shards: usize,
) -> Result<(CsLibVector2dFast, Vec<CsLibVector2dFast>), BuildError> {
    let mut parts: Vec<CsLibVector2dFast> = Vec::new();
    for shard in partition(items, shards) {
        let mut sketch = build_cs_lib_vector2d_fast(params)?;
        for v in shard {
            sketch.inner.insert(&v.data_input());
        }
        parts.push(sketch);
    }
    let rest = parts.split_off(1);
    Ok((
        parts.pop().expect("a split of the stream is never empty"),
        rest,
    ))
}

pub fn insert_cs_lib_vector2d_regular<T: FrequencyValue>(
    params: &ParamSet,
    items: Rc<Vec<T>>,
    passes: usize,
) -> Result<Vec<Pass>, BuildError> {
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        // Built here, so calling the closure is the insert and nothing else.
        let mut sketch = build_cs_lib_vector2d_regular(params)?;
        let items = items.clone();
        out.push(Box::new(move || {
            for v in items.iter() {
                sketch.inner.insert(&v.data_input());
            }
            memory_cs_lib_vector2d_regular(&sketch)
        }) as Pass);
    }
    Ok(out)
}

pub fn insert_step_cs_lib_vector2d_regular<T: FrequencyValue>(
    params: &ParamSet,
    items: Rc<Vec<T>>,
    passes: usize,
) -> Result<Vec<StepPass>, BuildError> {
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        let sketch: Shared<_> = Rc::new(RefCell::new(build_cs_lib_vector2d_regular(params)?));
        let (driven, read) = (sketch.clone(), sketch);
        let stream = items.clone();
        out.push(StepPass {
            steps: items.len(),
            step: Box::new(move |i| {
                let sketch = &mut *driven.borrow_mut();
                let v = &stream[i];
                sketch.inner.insert(&v.data_input());
            }),
            footprint: Box::new(move || memory_cs_lib_vector2d_regular(&read.borrow())),
        });
    }
    Ok(out)
}

pub fn query_cs_lib_vector2d_regular<T: FrequencyValue>(
    params: &ParamSet,
    items: Rc<Vec<T>>,
    probes: Rc<Vec<T>>,
    passes: usize,
) -> Result<Vec<QueryPass<u64>>, BuildError> {
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        // Built and fed here: the closure below asks, and only asks.
        let mut sketch = build_cs_lib_vector2d_regular(params)?;
        for v in items.iter() {
            sketch.inner.insert(&v.data_input());
        }
        let probes = probes.clone();
        out.push(Box::new(move || {
            let mut answers = Vec::with_capacity(probes.len());
            for p in probes.iter() {
                answers.push(sketch.inner.estimate(&p.data_input()) as u64);
            }
            let footprint = memory_cs_lib_vector2d_regular(&sketch);
            (answers, footprint)
        }) as QueryPass<u64>);
    }
    Ok(out)
}

pub fn merge_cs_lib_vector2d_regular<T: FrequencyValue>(
    params: &ParamSet,
    items: Rc<Vec<T>>,
    shards: usize,
    passes: usize,
) -> Result<Vec<Pass>, BuildError> {
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        let (mut acc, rest) = cs_lib_vector2d_regular_shards(params, &items, shards)?;
        out.push(Box::new(move || {
            for other in rest.iter() {
                acc.inner.merge(&other.inner);
            }
            memory_cs_lib_vector2d_regular(&acc)
        }) as Pass);
    }
    Ok(out)
}

pub fn merge_step_cs_lib_vector2d_regular<T: FrequencyValue>(
    params: &ParamSet,
    items: Rc<Vec<T>>,
    shards: usize,
    passes: usize,
) -> Result<Vec<StepPass>, BuildError> {
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        let (acc, rest) = cs_lib_vector2d_regular_shards(params, &items, shards)?;
        let acc: Shared<_> = Rc::new(RefCell::new(acc));
        let (driven, read) = (acc.clone(), acc);
        out.push(StepPass {
            steps: rest.len(),
            step: Box::new(move |i| {
                let acc = &mut *driven.borrow_mut();
                let other = &rest[i];
                acc.inner.merge(&other.inner);
            }),
            footprint: Box::new(move || memory_cs_lib_vector2d_regular(&read.borrow())),
        });
    }
    Ok(out)
}

/// The shards a fold folds: the stream split `shards` ways, one sketch each,
/// all fed. The first is the accumulator, the rest are what it folds in.
#[allow(clippy::type_complexity)]
fn cs_lib_vector2d_regular_shards<T: FrequencyValue>(
    params: &ParamSet,
    items: &[T],
    shards: usize,
) -> Result<(CsLibVector2dRegular, Vec<CsLibVector2dRegular>), BuildError> {
    let mut parts: Vec<CsLibVector2dRegular> = Vec::new();
    for shard in partition(items, shards) {
        let mut sketch = build_cs_lib_vector2d_regular(params)?;
        for v in shard {
            sketch.inner.insert(&v.data_input());
        }
        parts.push(sketch);
    }
    let rest = parts.split_off(1);
    Ok((
        parts.pop().expect("a split of the stream is never empty"),
        rest,
    ))
}

/// The whole stream in one call: this row's ingest *is* the parallel fan-out,
/// so there is no per-item step to buffer and none to time.
pub fn insert_parallel_cs_fast_path<T: FrequencyValue>(
    params: &ParamSet,
    workers: usize,
    items: Rc<Vec<T>>,
    passes: usize,
) -> Result<Vec<Pass>, BuildError> {
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        let sketch = build_parallel_cs_fast_path(params, workers)?;
        let items = items.clone();
        out.push(Box::new(move || {
            run_parallel_cs(&items, sketch.workers);
            memory_parallel_cs_fast_path(&sketch)
        }) as Pass);
    }
    Ok(out)
}
