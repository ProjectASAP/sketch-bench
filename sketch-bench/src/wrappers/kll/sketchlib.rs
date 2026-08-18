//! The `asap_sketchlib` implementations.
//!
//! Grouped under `wrappers/kll/` with the other implementations of this
//! algorithm; how each is driven lives beside it.

use super::*;
use crate::params::ParamSet;
use crate::wrappers::partition;
use crate::wrappers::{BuildError, Pass, QueryPass, Shared, StepPass};
use std::cell::RefCell;
use std::rc::Rc;

/// `k` this library cannot hold is an error naming both, not a run at some
/// other `k` reported as the one asked for.
fn lib_kll<T>(k: u32) -> Result<asap_sketchlib::KLL<T>, BuildError>
where
    T: asap_sketchlib::common::numerical::NumericalValue,
{
    if !(LIB_K_MIN..=LIB_K_MAX).contains(&k) {
        return Err(BuildError(format!(
            "asap KLL: k={k} outside [{LIB_K_MIN}, {LIB_K_MAX}]; the library clamps \
             to that range, so any other k would run at a value this record does not name"
        )));
    }
    Ok(asap_sketchlib::KLL::<T>::init_kll(k as i32))
}

/// Generic *in the library*: `KLL<T>` stores `T` and orders it with
/// `T::total_cmp`, converting nothing — so this row's item-type axis measures
/// the library's own choice, not a wrapper's cast.
pub struct KllLibPerCall<T: asap_sketchlib::common::numerical::NumericalValue = i64> {
    inner: asap_sketchlib::KLL<T>,
    k: u32,
}

pub fn build_kll_lib_per_call<T>(config: &ParamSet) -> Result<KllLibPerCall<T>, BuildError>
where
    T: asap_sketchlib::common::numerical::NumericalValue + QuantileValue,
{
    let p: KllParams = config.parse()?;
    Ok(KllLibPerCall {
        inner: lib_kll::<T>(p.k)?,
        k: p.k,
    })
}

pub fn memory_kll_lib_per_call<T>(sketch: &KllLibPerCall<T>) -> usize
where
    T: asap_sketchlib::common::numerical::NumericalValue + QuantileValue,
{
    kll_footprint::<T>(sketch.k)
}

pub struct KllLibCdf<T: asap_sketchlib::common::numerical::NumericalValue = i64> {
    inner: asap_sketchlib::KLL<T>,
    k: u32,
    cdf: Option<asap_sketchlib::sketches::kll::Cdf>,
}

pub fn build_kll_lib_cdf<T>(config: &ParamSet) -> Result<KllLibCdf<T>, BuildError>
where
    T: asap_sketchlib::common::numerical::NumericalValue + QuantileValue,
{
    let p: KllParams = config.parse()?;
    Ok(KllLibCdf {
        inner: lib_kll::<T>(p.k)?,
        k: p.k,
        cdf: None,
    })
}

pub fn memory_kll_lib_cdf<T>(sketch: &KllLibCdf<T>) -> usize
where
    T: asap_sketchlib::common::numerical::NumericalValue + QuantileValue,
{
    kll_footprint::<T>(sketch.k)
}

pub fn insert_kll_lib_per_call<T>(
    params: &ParamSet,
    items: Rc<Vec<T>>,
    passes: usize,
) -> Result<Vec<Pass>, BuildError>
where
    T: asap_sketchlib::common::numerical::NumericalValue + QuantileValue + 'static,
{
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        // Built here, so calling the closure is the insert and nothing else.
        let mut sketch = build_kll_lib_per_call::<T>(params)?;
        let items = items.clone();
        out.push(Box::new(move || {
            for v in items.iter() {
                sketch.inner.update(v);
            }
            memory_kll_lib_per_call::<T>(&sketch)
        }) as Pass);
    }
    Ok(out)
}

pub fn insert_step_kll_lib_per_call<T>(
    params: &ParamSet,
    items: Rc<Vec<T>>,
    passes: usize,
) -> Result<Vec<StepPass>, BuildError>
where
    T: asap_sketchlib::common::numerical::NumericalValue + QuantileValue + 'static,
{
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        let sketch: Shared<_> = Rc::new(RefCell::new(build_kll_lib_per_call::<T>(params)?));
        let (driven, read) = (sketch.clone(), sketch);
        let stream = items.clone();
        out.push(StepPass {
            steps: items.len(),
            step: Box::new(move |i| {
                let sketch = &mut *driven.borrow_mut();
                let v = &stream[i];
                sketch.inner.update(v);
            }),
            footprint: Box::new(move || memory_kll_lib_per_call::<T>(&read.borrow())),
        });
    }
    Ok(out)
}

pub fn query_kll_lib_per_call<T>(
    params: &ParamSet,
    items: Rc<Vec<T>>,
    probes: Rc<Vec<f64>>,
    passes: usize,
) -> Result<Vec<QueryPass<f64>>, BuildError>
where
    T: asap_sketchlib::common::numerical::NumericalValue + QuantileValue + 'static,
{
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        // Built and fed here: the closure below asks, and only asks.
        let mut sketch = build_kll_lib_per_call::<T>(params)?;
        for v in items.iter() {
            sketch.inner.update(v);
        }
        let probes = probes.clone();
        out.push(Box::new(move || {
            let mut answers = Vec::with_capacity(probes.len());
            for p in probes.iter() {
                answers.push(sketch.inner.quantile(*p));
            }
            let footprint = memory_kll_lib_per_call::<T>(&sketch);
            (answers, footprint)
        }) as QueryPass<f64>);
    }
    Ok(out)
}

pub fn merge_kll_lib_per_call<T>(
    params: &ParamSet,
    items: Rc<Vec<T>>,
    shards: usize,
    passes: usize,
) -> Result<Vec<Pass>, BuildError>
where
    T: asap_sketchlib::common::numerical::NumericalValue + QuantileValue + 'static,
{
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        let (mut acc, rest) = kll_lib_per_call_shards(params, &items, shards)?;
        out.push(Box::new(move || {
            for other in rest.iter() {
                acc.inner.merge(&other.inner);
            }
            memory_kll_lib_per_call::<T>(&acc)
        }) as Pass);
    }
    Ok(out)
}

pub fn merge_step_kll_lib_per_call<T>(
    params: &ParamSet,
    items: Rc<Vec<T>>,
    shards: usize,
    passes: usize,
) -> Result<Vec<StepPass>, BuildError>
where
    T: asap_sketchlib::common::numerical::NumericalValue + QuantileValue + 'static,
{
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        let (acc, rest) = kll_lib_per_call_shards(params, &items, shards)?;
        let acc: Shared<_> = Rc::new(RefCell::new(acc));
        let (driven, read) = (acc.clone(), acc);
        out.push(StepPass {
            steps: rest.len(),
            step: Box::new(move |i| {
                let acc = &mut *driven.borrow_mut();
                let other = &rest[i];
                acc.inner.merge(&other.inner);
            }),
            footprint: Box::new(move || memory_kll_lib_per_call::<T>(&read.borrow())),
        });
    }
    Ok(out)
}

/// The shards a fold folds: the stream split `shards` ways, one sketch each,
/// all fed. The first is the accumulator, the rest are what it folds in.
#[allow(clippy::type_complexity)]
fn kll_lib_per_call_shards<T>(
    params: &ParamSet,
    items: &[T],
    shards: usize,
) -> Result<(KllLibPerCall<T>, Vec<KllLibPerCall<T>>), BuildError>
where
    T: asap_sketchlib::common::numerical::NumericalValue + QuantileValue + 'static,
{
    let mut parts: Vec<KllLibPerCall<T>> = Vec::new();
    for shard in partition(items, shards) {
        let mut sketch = build_kll_lib_per_call::<T>(params)?;
        for v in shard {
            sketch.inner.update(v);
        }
        parts.push(sketch);
    }
    let rest = parts.split_off(1);
    Ok((
        parts.pop().expect("a split of the stream is never empty"),
        rest,
    ))
}

pub fn insert_kll_lib_cdf<T>(
    params: &ParamSet,
    items: Rc<Vec<T>>,
    passes: usize,
) -> Result<Vec<Pass>, BuildError>
where
    T: asap_sketchlib::common::numerical::NumericalValue + QuantileValue + 'static,
{
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        // Built here, so calling the closure is the insert and nothing else.
        let mut sketch = build_kll_lib_cdf::<T>(params)?;
        let items = items.clone();
        out.push(Box::new(move || {
            for v in items.iter() {
                sketch.inner.update(v);
            }
            memory_kll_lib_cdf::<T>(&sketch)
        }) as Pass);
    }
    Ok(out)
}

pub fn insert_step_kll_lib_cdf<T>(
    params: &ParamSet,
    items: Rc<Vec<T>>,
    passes: usize,
) -> Result<Vec<StepPass>, BuildError>
where
    T: asap_sketchlib::common::numerical::NumericalValue + QuantileValue + 'static,
{
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        let sketch: Shared<_> = Rc::new(RefCell::new(build_kll_lib_cdf::<T>(params)?));
        let (driven, read) = (sketch.clone(), sketch);
        let stream = items.clone();
        out.push(StepPass {
            steps: items.len(),
            step: Box::new(move |i| {
                let sketch = &mut *driven.borrow_mut();
                let v = &stream[i];
                sketch.inner.update(v);
            }),
            footprint: Box::new(move || memory_kll_lib_cdf::<T>(&read.borrow())),
        });
    }
    Ok(out)
}

pub fn query_kll_lib_cdf<T>(
    params: &ParamSet,
    items: Rc<Vec<T>>,
    probes: Rc<Vec<f64>>,
    passes: usize,
) -> Result<Vec<QueryPass<f64>>, BuildError>
where
    T: asap_sketchlib::common::numerical::NumericalValue + QuantileValue + 'static,
{
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        // Built and fed here: the closure below asks, and only asks.
        let mut sketch = build_kll_lib_cdf::<T>(params)?;
        for v in items.iter() {
            sketch.inner.update(v);
        }
        let cdf = sketch.inner.cdf();
        let probes = probes.clone();
        out.push(Box::new(move || {
            let mut answers = Vec::with_capacity(probes.len());
            for p in probes.iter() {
                answers.push(cdf.query(*p));
            }
            let footprint = memory_kll_lib_cdf::<T>(&sketch);
            (answers, footprint)
        }) as QueryPass<f64>);
    }
    Ok(out)
}

pub fn merge_kll_lib_cdf<T>(
    params: &ParamSet,
    items: Rc<Vec<T>>,
    shards: usize,
    passes: usize,
) -> Result<Vec<Pass>, BuildError>
where
    T: asap_sketchlib::common::numerical::NumericalValue + QuantileValue + 'static,
{
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        let (mut acc, rest) = kll_lib_cdf_shards(params, &items, shards)?;
        out.push(Box::new(move || {
            for other in rest.iter() {
                acc.inner.merge(&other.inner);
                // A merged sketch invalidates any CDF cached from the pre-merge state.
                acc.cdf = None;
            }
            memory_kll_lib_cdf::<T>(&acc)
        }) as Pass);
    }
    Ok(out)
}

pub fn merge_step_kll_lib_cdf<T>(
    params: &ParamSet,
    items: Rc<Vec<T>>,
    shards: usize,
    passes: usize,
) -> Result<Vec<StepPass>, BuildError>
where
    T: asap_sketchlib::common::numerical::NumericalValue + QuantileValue + 'static,
{
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        let (acc, rest) = kll_lib_cdf_shards(params, &items, shards)?;
        let acc: Shared<_> = Rc::new(RefCell::new(acc));
        let (driven, read) = (acc.clone(), acc);
        out.push(StepPass {
            steps: rest.len(),
            step: Box::new(move |i| {
                let acc = &mut *driven.borrow_mut();
                let other = &rest[i];
                acc.inner.merge(&other.inner);
                // A merged sketch invalidates any CDF cached from the pre-merge state.
                acc.cdf = None;
            }),
            footprint: Box::new(move || memory_kll_lib_cdf::<T>(&read.borrow())),
        });
    }
    Ok(out)
}

/// The shards a fold folds: the stream split `shards` ways, one sketch each,
/// all fed. The first is the accumulator, the rest are what it folds in.
#[allow(clippy::type_complexity)]
fn kll_lib_cdf_shards<T>(
    params: &ParamSet,
    items: &[T],
    shards: usize,
) -> Result<(KllLibCdf<T>, Vec<KllLibCdf<T>>), BuildError>
where
    T: asap_sketchlib::common::numerical::NumericalValue + QuantileValue + 'static,
{
    let mut parts: Vec<KllLibCdf<T>> = Vec::new();
    for shard in partition(items, shards) {
        let mut sketch = build_kll_lib_cdf::<T>(params)?;
        for v in shard {
            sketch.inner.update(v);
        }
        parts.push(sketch);
    }
    let rest = parts.split_off(1);
    Ok((
        parts.pop().expect("a split of the stream is never empty"),
        rest,
    ))
}

pub fn prepare_kll_lib_cdf<T>(
    params: &ParamSet,
    items: Rc<Vec<T>>,
    passes: usize,
) -> Result<Vec<Pass>, BuildError>
where
    T: asap_sketchlib::common::numerical::NumericalValue + QuantileValue + 'static,
{
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        // Fed here: the closure below is the step that makes it ready to answer.
        let mut sketch = build_kll_lib_cdf::<T>(params)?;
        for v in items.iter() {
            sketch.inner.update(v);
        }
        out.push(Box::new(move || {
            sketch.cdf = Some(sketch.inner.cdf());
            memory_kll_lib_cdf::<T>(&sketch)
        }) as Pass);
    }
    Ok(out)
}
