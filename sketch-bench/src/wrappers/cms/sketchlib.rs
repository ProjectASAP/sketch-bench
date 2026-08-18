//! The `asap_sketchlib` implementations.
//!
//! Grouped under `wrappers/cms/` with the other implementations of this
//! algorithm; how each is driven lives beside it.

use super::*;
use crate::params::ParamSet;
use crate::wrappers::{
    partition, require_positive, require_shape, M5x32K, PARALLEL_COLS, PARALLEL_ROWS,
};
use crate::wrappers::{BuildError, Pass, QueryPass, Shared, StepPass};
use std::cell::RefCell;
use std::rc::Rc;

use asap_sketchlib::{
    CountMin, DataInput, DefaultXxHasher, FastPath, FastPathHasher, MatrixStorage, RegularPath,
    Vector2D,
};
use std::sync::Barrier;

pub struct CmsLibFixedmatrix<M: MatrixStorage>(pub CountMin<M, FastPath>);

pub fn build_cms_lib_fixedmatrix<M>(config: &ParamSet) -> Result<CmsLibFixedmatrix<M>, BuildError>
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

pub fn build_cms_lib_vector2d_fast(config: &ParamSet) -> Result<CmsLibVector2dFast, BuildError> {
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

/// CMS, parallel-insert FastPath.
pub struct ParallelCmsFastPath {
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
        workers: workers.max(1),
    })
}

pub fn memory_parallel_cms_fast_path(sketch: &ParallelCmsFastPath) -> usize {
    sketch.workers * (PARALLEL_ROWS * PARALLEL_COLS * std::mem::size_of::<i32>())
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

pub fn insert_cms_lib_fixedmatrix<M>(
    params: &ParamSet,
    items: Rc<Vec<i64>>,
    passes: usize,
) -> Result<Vec<Pass>, BuildError>
where
    M: MatrixStorage<Counter = i32> + FastPathHasher<DefaultXxHasher> + Default + Clone + 'static,
{
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        // Built here, so calling the closure is the insert and nothing else.
        let mut sketch = build_cms_lib_fixedmatrix::<M>(params)?;
        let items = items.clone();
        out.push(Box::new(move || {
            for v in items.iter() {
                sketch.0.insert(&DataInput::I64(*v));
            }
            memory_cms_lib_fixedmatrix::<M>(&sketch)
        }) as Pass);
    }
    Ok(out)
}

pub fn insert_step_cms_lib_fixedmatrix<M>(
    params: &ParamSet,
    items: Rc<Vec<i64>>,
    passes: usize,
) -> Result<Vec<StepPass>, BuildError>
where
    M: MatrixStorage<Counter = i32> + FastPathHasher<DefaultXxHasher> + Default + Clone + 'static,
{
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        let sketch: Shared<_> = Rc::new(RefCell::new(build_cms_lib_fixedmatrix::<M>(params)?));
        let (driven, read) = (sketch.clone(), sketch);
        let stream = items.clone();
        out.push(StepPass {
            steps: items.len(),
            step: Box::new(move |i| {
                let sketch = &mut *driven.borrow_mut();
                let v = &stream[i];
                sketch.0.insert(&DataInput::I64(*v));
            }),
            footprint: Box::new(move || memory_cms_lib_fixedmatrix::<M>(&read.borrow())),
        });
    }
    Ok(out)
}

pub fn query_cms_lib_fixedmatrix<M>(
    params: &ParamSet,
    items: Rc<Vec<i64>>,
    probes: Rc<Vec<i64>>,
    passes: usize,
) -> Result<Vec<QueryPass<u64>>, BuildError>
where
    M: MatrixStorage<Counter = i32> + FastPathHasher<DefaultXxHasher> + Default + Clone + 'static,
{
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        // Built and fed here: the closure below asks, and only asks.
        let mut sketch = build_cms_lib_fixedmatrix::<M>(params)?;
        for v in items.iter() {
            sketch.0.insert(&DataInput::I64(*v));
        }
        let probes = probes.clone();
        out.push(Box::new(move || {
            let mut answers = Vec::with_capacity(probes.len());
            for p in probes.iter() {
                answers.push(sketch.0.estimate(&DataInput::I64(*p)) as u64);
            }
            let footprint = memory_cms_lib_fixedmatrix::<M>(&sketch);
            (answers, footprint)
        }) as QueryPass<u64>);
    }
    Ok(out)
}

pub fn merge_cms_lib_fixedmatrix<M>(
    params: &ParamSet,
    items: Rc<Vec<i64>>,
    shards: usize,
    passes: usize,
) -> Result<Vec<Pass>, BuildError>
where
    M: MatrixStorage<Counter = i32> + FastPathHasher<DefaultXxHasher> + Default + Clone + 'static,
{
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        let (mut acc, rest) = cms_lib_fixedmatrix_shards(params, &items, shards)?;
        out.push(Box::new(move || {
            for other in rest.iter() {
                acc.0.merge(&other.0);
            }
            memory_cms_lib_fixedmatrix::<M>(&acc)
        }) as Pass);
    }
    Ok(out)
}

pub fn merge_step_cms_lib_fixedmatrix<M>(
    params: &ParamSet,
    items: Rc<Vec<i64>>,
    shards: usize,
    passes: usize,
) -> Result<Vec<StepPass>, BuildError>
where
    M: MatrixStorage<Counter = i32> + FastPathHasher<DefaultXxHasher> + Default + Clone + 'static,
{
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        let (acc, rest) = cms_lib_fixedmatrix_shards(params, &items, shards)?;
        let acc: Shared<_> = Rc::new(RefCell::new(acc));
        let (driven, read) = (acc.clone(), acc);
        out.push(StepPass {
            steps: rest.len(),
            step: Box::new(move |i| {
                let acc = &mut *driven.borrow_mut();
                let other = &rest[i];
                acc.0.merge(&other.0);
            }),
            footprint: Box::new(move || memory_cms_lib_fixedmatrix::<M>(&read.borrow())),
        });
    }
    Ok(out)
}

/// The shards a fold folds: the stream split `shards` ways, one sketch each,
/// all fed. The first is the accumulator, the rest are what it folds in.
#[allow(clippy::type_complexity)]
fn cms_lib_fixedmatrix_shards<M>(
    params: &ParamSet,
    items: &[i64],
    shards: usize,
) -> Result<(CmsLibFixedmatrix<M>, Vec<CmsLibFixedmatrix<M>>), BuildError>
where
    M: MatrixStorage<Counter = i32> + FastPathHasher<DefaultXxHasher> + Default + Clone + 'static,
{
    let mut parts: Vec<CmsLibFixedmatrix<M>> = Vec::new();
    for shard in partition(items, shards) {
        let mut sketch = build_cms_lib_fixedmatrix::<M>(params)?;
        for v in shard {
            sketch.0.insert(&DataInput::I64(*v));
        }
        parts.push(sketch);
    }
    let rest = parts.split_off(1);
    Ok((
        parts.pop().expect("a split of the stream is never empty"),
        rest,
    ))
}

pub fn insert_cms_lib_vector2d_fast(
    params: &ParamSet,
    items: Rc<Vec<i64>>,
    passes: usize,
) -> Result<Vec<Pass>, BuildError> {
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        // Built here, so calling the closure is the insert and nothing else.
        let mut sketch = build_cms_lib_vector2d_fast(params)?;
        let items = items.clone();
        out.push(Box::new(move || {
            for v in items.iter() {
                sketch.inner.insert(&DataInput::I64(*v));
            }
            memory_cms_lib_vector2d_fast(&sketch)
        }) as Pass);
    }
    Ok(out)
}

pub fn insert_step_cms_lib_vector2d_fast(
    params: &ParamSet,
    items: Rc<Vec<i64>>,
    passes: usize,
) -> Result<Vec<StepPass>, BuildError> {
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        let sketch: Shared<_> = Rc::new(RefCell::new(build_cms_lib_vector2d_fast(params)?));
        let (driven, read) = (sketch.clone(), sketch);
        let stream = items.clone();
        out.push(StepPass {
            steps: items.len(),
            step: Box::new(move |i| {
                let sketch = &mut *driven.borrow_mut();
                let v = &stream[i];
                sketch.inner.insert(&DataInput::I64(*v));
            }),
            footprint: Box::new(move || memory_cms_lib_vector2d_fast(&read.borrow())),
        });
    }
    Ok(out)
}

pub fn query_cms_lib_vector2d_fast(
    params: &ParamSet,
    items: Rc<Vec<i64>>,
    probes: Rc<Vec<i64>>,
    passes: usize,
) -> Result<Vec<QueryPass<u64>>, BuildError> {
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        // Built and fed here: the closure below asks, and only asks.
        let mut sketch = build_cms_lib_vector2d_fast(params)?;
        for v in items.iter() {
            sketch.inner.insert(&DataInput::I64(*v));
        }
        let probes = probes.clone();
        out.push(Box::new(move || {
            let mut answers = Vec::with_capacity(probes.len());
            for p in probes.iter() {
                answers.push(sketch.inner.estimate(&DataInput::I64(*p)) as u64);
            }
            let footprint = memory_cms_lib_vector2d_fast(&sketch);
            (answers, footprint)
        }) as QueryPass<u64>);
    }
    Ok(out)
}

pub fn merge_cms_lib_vector2d_fast(
    params: &ParamSet,
    items: Rc<Vec<i64>>,
    shards: usize,
    passes: usize,
) -> Result<Vec<Pass>, BuildError> {
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        let (mut acc, rest) = cms_lib_vector2d_fast_shards(params, &items, shards)?;
        out.push(Box::new(move || {
            for other in rest.iter() {
                acc.inner.merge(&other.inner);
            }
            memory_cms_lib_vector2d_fast(&acc)
        }) as Pass);
    }
    Ok(out)
}

pub fn merge_step_cms_lib_vector2d_fast(
    params: &ParamSet,
    items: Rc<Vec<i64>>,
    shards: usize,
    passes: usize,
) -> Result<Vec<StepPass>, BuildError> {
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        let (acc, rest) = cms_lib_vector2d_fast_shards(params, &items, shards)?;
        let acc: Shared<_> = Rc::new(RefCell::new(acc));
        let (driven, read) = (acc.clone(), acc);
        out.push(StepPass {
            steps: rest.len(),
            step: Box::new(move |i| {
                let acc = &mut *driven.borrow_mut();
                let other = &rest[i];
                acc.inner.merge(&other.inner);
            }),
            footprint: Box::new(move || memory_cms_lib_vector2d_fast(&read.borrow())),
        });
    }
    Ok(out)
}

/// The shards a fold folds: the stream split `shards` ways, one sketch each,
/// all fed. The first is the accumulator, the rest are what it folds in.
#[allow(clippy::type_complexity)]
fn cms_lib_vector2d_fast_shards(
    params: &ParamSet,
    items: &[i64],
    shards: usize,
) -> Result<(CmsLibVector2dFast, Vec<CmsLibVector2dFast>), BuildError> {
    let mut parts: Vec<CmsLibVector2dFast> = Vec::new();
    for shard in partition(items, shards) {
        let mut sketch = build_cms_lib_vector2d_fast(params)?;
        for v in shard {
            sketch.inner.insert(&DataInput::I64(*v));
        }
        parts.push(sketch);
    }
    let rest = parts.split_off(1);
    Ok((
        parts.pop().expect("a split of the stream is never empty"),
        rest,
    ))
}

pub fn insert_cms_lib_vector2d_regular(
    params: &ParamSet,
    items: Rc<Vec<i64>>,
    passes: usize,
) -> Result<Vec<Pass>, BuildError> {
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        // Built here, so calling the closure is the insert and nothing else.
        let mut sketch = build_cms_lib_vector2d_regular(params)?;
        let items = items.clone();
        out.push(Box::new(move || {
            for v in items.iter() {
                sketch.inner.insert(&DataInput::I64(*v));
            }
            memory_cms_lib_vector2d_regular(&sketch)
        }) as Pass);
    }
    Ok(out)
}

pub fn insert_step_cms_lib_vector2d_regular(
    params: &ParamSet,
    items: Rc<Vec<i64>>,
    passes: usize,
) -> Result<Vec<StepPass>, BuildError> {
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        let sketch: Shared<_> = Rc::new(RefCell::new(build_cms_lib_vector2d_regular(params)?));
        let (driven, read) = (sketch.clone(), sketch);
        let stream = items.clone();
        out.push(StepPass {
            steps: items.len(),
            step: Box::new(move |i| {
                let sketch = &mut *driven.borrow_mut();
                let v = &stream[i];
                sketch.inner.insert(&DataInput::I64(*v));
            }),
            footprint: Box::new(move || memory_cms_lib_vector2d_regular(&read.borrow())),
        });
    }
    Ok(out)
}

pub fn query_cms_lib_vector2d_regular(
    params: &ParamSet,
    items: Rc<Vec<i64>>,
    probes: Rc<Vec<i64>>,
    passes: usize,
) -> Result<Vec<QueryPass<u64>>, BuildError> {
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        // Built and fed here: the closure below asks, and only asks.
        let mut sketch = build_cms_lib_vector2d_regular(params)?;
        for v in items.iter() {
            sketch.inner.insert(&DataInput::I64(*v));
        }
        let probes = probes.clone();
        out.push(Box::new(move || {
            let mut answers = Vec::with_capacity(probes.len());
            for p in probes.iter() {
                answers.push(sketch.inner.estimate(&DataInput::I64(*p)) as u64);
            }
            let footprint = memory_cms_lib_vector2d_regular(&sketch);
            (answers, footprint)
        }) as QueryPass<u64>);
    }
    Ok(out)
}

pub fn merge_cms_lib_vector2d_regular(
    params: &ParamSet,
    items: Rc<Vec<i64>>,
    shards: usize,
    passes: usize,
) -> Result<Vec<Pass>, BuildError> {
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        let (mut acc, rest) = cms_lib_vector2d_regular_shards(params, &items, shards)?;
        out.push(Box::new(move || {
            for other in rest.iter() {
                acc.inner.merge(&other.inner);
            }
            memory_cms_lib_vector2d_regular(&acc)
        }) as Pass);
    }
    Ok(out)
}

pub fn merge_step_cms_lib_vector2d_regular(
    params: &ParamSet,
    items: Rc<Vec<i64>>,
    shards: usize,
    passes: usize,
) -> Result<Vec<StepPass>, BuildError> {
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        let (acc, rest) = cms_lib_vector2d_regular_shards(params, &items, shards)?;
        let acc: Shared<_> = Rc::new(RefCell::new(acc));
        let (driven, read) = (acc.clone(), acc);
        out.push(StepPass {
            steps: rest.len(),
            step: Box::new(move |i| {
                let acc = &mut *driven.borrow_mut();
                let other = &rest[i];
                acc.inner.merge(&other.inner);
            }),
            footprint: Box::new(move || memory_cms_lib_vector2d_regular(&read.borrow())),
        });
    }
    Ok(out)
}

/// The shards a fold folds: the stream split `shards` ways, one sketch each,
/// all fed. The first is the accumulator, the rest are what it folds in.
#[allow(clippy::type_complexity)]
fn cms_lib_vector2d_regular_shards(
    params: &ParamSet,
    items: &[i64],
    shards: usize,
) -> Result<(CmsLibVector2dRegular, Vec<CmsLibVector2dRegular>), BuildError> {
    let mut parts: Vec<CmsLibVector2dRegular> = Vec::new();
    for shard in partition(items, shards) {
        let mut sketch = build_cms_lib_vector2d_regular(params)?;
        for v in shard {
            sketch.inner.insert(&DataInput::I64(*v));
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
pub fn insert_parallel_cms_fast_path(
    params: &ParamSet,
    workers: usize,
    items: Rc<Vec<i64>>,
    passes: usize,
) -> Result<Vec<Pass>, BuildError> {
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        let sketch = build_parallel_cms_fast_path(params, workers)?;
        let items = items.clone();
        out.push(Box::new(move || {
            run_parallel_cms(&items, sketch.workers);
            memory_parallel_cms_fast_path(&sketch)
        }) as Pass);
    }
    Ok(out)
}
