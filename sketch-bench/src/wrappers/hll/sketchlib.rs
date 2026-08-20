//! The `asap_sketchlib` implementations.
//!
//! Grouped under `wrappers/hll/` with the other implementations of this
//! algorithm; how each is driven lives beside it.

use super::*;
use crate::params::ParamSet;
use crate::wrappers::partition;
use crate::wrappers::{BuildError, Pass, QueryPass, Shared, StepPass};
use asap_sketchlib::{ErtlMLE, HyperLogLog};
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Barrier;

// `merge` lives on sketch_oxide's `Mergeable`, not on its `Accumulator`.

/// `HyperLogLog<Classic>`, the classic estimator (Flajolet et al., 2007):
/// insert bumps registers, `estimate()` scans all `2^lg_k` of them.
pub struct HllLib<R: asap_sketchlib::HllRegisterStorage = asap_sketchlib::HllBucketListP14> {
    pub(super) inner: asap_sketchlib::hll::HyperLogLogImpl<asap_sketchlib::Classic, R>,
}

pub fn build_hll_lib<R: asap_sketchlib::HllRegisterStorage>(
    config: &ParamSet,
) -> Result<HllLib<R>, BuildError> {
    let p: HllParams = config.parse()?;
    // The registry picked `R` off this same `lg_k`, so this only fires for a
    // direct caller. It fires rather than silently building at `R`, because
    // building at a precision other than the one requested is the defect
    // this row is being fixed for.
    if p.lg_k as usize != R::PRECISION {
        return Err(unsupported_precision(p.lg_k));
    }
    Ok(HllLib {
        inner: asap_sketchlib::hll::HyperLogLogImpl::<asap_sketchlib::Classic, R>::new(),
    })
}

pub fn memory_hll_lib<R: asap_sketchlib::HllRegisterStorage>(_sketch: &HllLib<R>) -> usize {
    // Off the storage type, so it tracks whichever precision was built.
    // A written-in `1 << 14` was what let three different `lg_k` values
    // report three footprints for one sketch.
    R::NUM_REGISTERS
}

/// `HyperLogLogHIP` maintains the estimate incrementally on the insert path —
/// every register upgrade pays a few fp ops — so query is O(1) rather than
/// Classic's O(m) scan. That trade is what this algorithm exists to measure,
/// and it is a different estimator, so it is its own algorithm and not an impl
/// of `hll`.
pub struct HllLibHip<R: asap_sketchlib::HllRegisterStorage = asap_sketchlib::HllBucketListP14> {
    pub(super) inner: asap_sketchlib::hll::HyperLogLogHIPImpl<R>,
}

pub fn build_hll_lib_hip<R: asap_sketchlib::HllRegisterStorage>(
    config: &ParamSet,
) -> Result<HllLibHip<R>, BuildError> {
    let p: HllParams = config.parse()?;
    if p.lg_k as usize != R::PRECISION {
        return Err(unsupported_precision(p.lg_k));
    }
    Ok(HllLibHip {
        inner: asap_sketchlib::hll::HyperLogLogHIPImpl::<R>::new(),
    })
}

pub fn memory_hll_lib_hip<R: asap_sketchlib::HllRegisterStorage>(_sketch: &HllLibHip<R>) -> usize {
    R::NUM_REGISTERS
}

/// The HIP estimate is maintained on the insert path, so the ask is a field
/// read rather than Classic's scan over `2^lg_k` registers — the trade this
/// row exists to price.
/// The `lg_k` this row is fixed at. Its per-worker sketch is a compile-time
/// type — `HyperLogLog<ErtlMLE>` is the P14 alias — so any other value is
/// unbuildable and is refused rather than run at 14 under its name.
const PARALLEL_HLL_LG_K: u8 = 14;

/// HLL, parallel-insert ErtlMLE FastPath.
pub struct ParallelHllFastPath {
    workers: usize,
}

pub fn build_parallel_hll_fast_path(
    config: &ParamSet,
    workers: usize,
) -> Result<ParallelHllFastPath, BuildError> {
    let p: HllParams = config.parse()?;
    // `HyperLogLog<ErtlMLE>` is the P14 alias, so this row exists at one
    // precision. Refuse the others instead of running at 14 under their name.
    if p.lg_k != PARALLEL_HLL_LG_K {
        return Err(BuildError(format!(
            "parallel HLL: fixed at lg_k={PARALLEL_HLL_LG_K}, requested lg_k={}",
            p.lg_k
        )));
    }
    Ok(ParallelHllFastPath {
        workers: workers.max(1),
    })
}

pub fn memory_parallel_hll_fast_path(sketch: &ParallelHllFastPath) -> usize {
    // Each worker holds an HLL with ErtlMLE registers, one byte each.
    // `build` has already refused any lg_k other than the one baked in, so
    // this is the precision that ran and not a coarse upper bound.
    sketch.workers * (1usize << PARALLEL_HLL_LG_K)
}

fn run_parallel_hll<T: CardinalityValue>(items: &[T], workers: usize) {
    let parts = partition(items, workers);
    let barrier = Barrier::new(parts.len());
    std::thread::scope(|s| {
        for part in &parts {
            let barrier = &barrier;
            s.spawn(move || {
                let mut sketch = HyperLogLog::<ErtlMLE>::default();
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

pub fn insert_hll_lib<R: asap_sketchlib::HllRegisterStorage + 'static, T: CardinalityValue>(
    params: &ParamSet,
    items: Rc<Vec<T>>,
    passes: usize,
) -> Result<Vec<Pass>, BuildError> {
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        // Built here, so calling the closure is the insert and nothing else.
        let mut sketch = build_hll_lib::<R>(params)?;
        let items = items.clone();
        out.push(Box::new(move || {
            for v in items.iter() {
                sketch.inner.insert(&v.data_input());
            }
            memory_hll_lib(&sketch)
        }) as Pass);
    }
    Ok(out)
}

pub fn insert_step_hll_lib<R: asap_sketchlib::HllRegisterStorage + 'static, T: CardinalityValue>(
    params: &ParamSet,
    items: Rc<Vec<T>>,
    passes: usize,
) -> Result<Vec<StepPass>, BuildError> {
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        let sketch: Shared<_> = Rc::new(RefCell::new(build_hll_lib::<R>(params)?));
        let (driven, read) = (sketch.clone(), sketch);
        let stream = items.clone();
        out.push(StepPass {
            steps: items.len(),
            step: Box::new(move |i| {
                let sketch = &mut *driven.borrow_mut();
                let v = &stream[i];
                sketch.inner.insert(&v.data_input());
            }),
            footprint: Box::new(move || memory_hll_lib(&read.borrow())),
        });
    }
    Ok(out)
}

pub fn query_hll_lib<R: asap_sketchlib::HllRegisterStorage + 'static, T: CardinalityValue>(
    params: &ParamSet,
    items: Rc<Vec<T>>,
    probes: Rc<Vec<()>>,
    passes: usize,
) -> Result<Vec<QueryPass<f64>>, BuildError> {
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        // Built and fed here: the closure below asks, and only asks.
        let mut sketch = build_hll_lib::<R>(params)?;
        for v in items.iter() {
            sketch.inner.insert(&v.data_input());
        }
        let probes = probes.clone();
        out.push(Box::new(move || {
            let mut answers = Vec::with_capacity(probes.len());
            for _p in probes.iter() {
                answers.push(sketch.inner.estimate() as f64);
            }
            let footprint = memory_hll_lib(&sketch);
            (answers, footprint)
        }) as QueryPass<f64>);
    }
    Ok(out)
}

pub fn merge_hll_lib<R: asap_sketchlib::HllRegisterStorage + 'static, T: CardinalityValue>(
    params: &ParamSet,
    items: Rc<Vec<T>>,
    shards: usize,
    passes: usize,
) -> Result<Vec<Pass>, BuildError> {
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        let (mut acc, rest) = hll_lib_shards::<R, T>(params, &items, shards)?;
        out.push(Box::new(move || {
            for other in rest.iter() {
                acc.inner.merge(&other.inner);
            }
            memory_hll_lib(&acc)
        }) as Pass);
    }
    Ok(out)
}

pub fn merge_step_hll_lib<R: asap_sketchlib::HllRegisterStorage + 'static, T: CardinalityValue>(
    params: &ParamSet,
    items: Rc<Vec<T>>,
    shards: usize,
    passes: usize,
) -> Result<Vec<StepPass>, BuildError> {
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        let (acc, rest) = hll_lib_shards::<R, T>(params, &items, shards)?;
        let acc: Shared<_> = Rc::new(RefCell::new(acc));
        let (driven, read) = (acc.clone(), acc);
        out.push(StepPass {
            steps: rest.len(),
            step: Box::new(move |i| {
                let acc = &mut *driven.borrow_mut();
                let other = &rest[i];
                acc.inner.merge(&other.inner);
            }),
            footprint: Box::new(move || memory_hll_lib(&read.borrow())),
        });
    }
    Ok(out)
}

/// The shards a fold folds: the stream split `shards` ways, one sketch each,
/// all fed. The first is the accumulator, the rest are what it folds in.
#[allow(clippy::type_complexity)]
fn hll_lib_shards<R: asap_sketchlib::HllRegisterStorage + 'static, T: CardinalityValue>(
    params: &ParamSet,
    items: &[T],
    shards: usize,
) -> Result<(HllLib<R>, Vec<HllLib<R>>), BuildError> {
    let mut parts: Vec<HllLib<R>> = Vec::new();
    for shard in partition(items, shards) {
        let mut sketch = build_hll_lib::<R>(params)?;
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

pub fn insert_hll_lib_hip<R: asap_sketchlib::HllRegisterStorage + 'static, T: CardinalityValue>(
    params: &ParamSet,
    items: Rc<Vec<T>>,
    passes: usize,
) -> Result<Vec<Pass>, BuildError> {
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        // Built here, so calling the closure is the insert and nothing else.
        let mut sketch = build_hll_lib_hip::<R>(params)?;
        let items = items.clone();
        out.push(Box::new(move || {
            for v in items.iter() {
                sketch.inner.insert(&v.data_input());
            }
            memory_hll_lib_hip(&sketch)
        }) as Pass);
    }
    Ok(out)
}

pub fn insert_step_hll_lib_hip<
    R: asap_sketchlib::HllRegisterStorage + 'static,
    T: CardinalityValue,
>(
    params: &ParamSet,
    items: Rc<Vec<T>>,
    passes: usize,
) -> Result<Vec<StepPass>, BuildError> {
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        let sketch: Shared<_> = Rc::new(RefCell::new(build_hll_lib_hip::<R>(params)?));
        let (driven, read) = (sketch.clone(), sketch);
        let stream = items.clone();
        out.push(StepPass {
            steps: items.len(),
            step: Box::new(move |i| {
                let sketch = &mut *driven.borrow_mut();
                let v = &stream[i];
                sketch.inner.insert(&v.data_input());
            }),
            footprint: Box::new(move || memory_hll_lib_hip(&read.borrow())),
        });
    }
    Ok(out)
}

pub fn query_hll_lib_hip<R: asap_sketchlib::HllRegisterStorage + 'static, T: CardinalityValue>(
    params: &ParamSet,
    items: Rc<Vec<T>>,
    probes: Rc<Vec<()>>,
    passes: usize,
) -> Result<Vec<QueryPass<f64>>, BuildError> {
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        // Built and fed here: the closure below asks, and only asks.
        let mut sketch = build_hll_lib_hip::<R>(params)?;
        for v in items.iter() {
            sketch.inner.insert(&v.data_input());
        }
        let probes = probes.clone();
        out.push(Box::new(move || {
            let mut answers = Vec::with_capacity(probes.len());
            for _p in probes.iter() {
                answers.push(sketch.inner.estimate() as f64);
            }
            let footprint = memory_hll_lib_hip(&sketch);
            (answers, footprint)
        }) as QueryPass<f64>);
    }
    Ok(out)
}

/// The whole stream in one call: this row's ingest *is* the parallel fan-out,
/// so there is no per-item step to buffer and none to time.
pub fn insert_parallel_hll_fast_path<T: CardinalityValue>(
    params: &ParamSet,
    workers: usize,
    items: Rc<Vec<T>>,
    passes: usize,
) -> Result<Vec<Pass>, BuildError> {
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        let sketch = build_parallel_hll_fast_path(params, workers)?;
        let items = items.clone();
        out.push(Box::new(move || {
            run_parallel_hll(&items, sketch.workers);
            memory_parallel_hll_fast_path(&sketch)
        }) as Pass);
    }
    Ok(out)
}
