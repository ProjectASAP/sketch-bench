//! The `sketch_oxide` implementations.
//!
//! Grouped under `wrappers/kll/` with the other implementations of this
//! algorithm; how each is driven lives beside it.

use super::*;
use crate::params::ParamSet;
use crate::wrappers::partition;
use crate::wrappers::{BuildError, Pass, QueryPass, Shared, StepPass};
use std::cell::RefCell;
use std::rc::Rc;

/// uses; `sketch_oxide` takes a `u16`. Refuse out of range by name instead of
/// truncating, which would run at a `k` other than the one requested.
fn oxide_kll(k: u32) -> Result<sketch_oxide::quantiles::KllSketch, BuildError> {
    let k16 = u16::try_from(k)
        .map_err(|_| BuildError(format!("oxide KLL: k={k} exceeds the u16 its API takes")))?;
    sketch_oxide::quantiles::KllSketch::new(k16)
        .map_err(|e| BuildError(format!("oxide KLL rejected k={k16}: {e:?}")))
}

/// Generic over the item type: the inner sketch is `f64`-native, so `T = f64`
/// monomorphises `to_f64` away while `T = i64` keeps the cast — the measurement.
pub struct KllOxidePerCall<T = i64> {
    inner: sketch_oxide::quantiles::KllSketch,
    k: u32,
    _item: std::marker::PhantomData<T>,
}

pub fn build_kll_oxide_per_call<T: QuantileValue>(
    config: &ParamSet,
) -> Result<KllOxidePerCall<T>, BuildError> {
    let p: KllParams = config.parse()?;
    Ok(KllOxidePerCall {
        inner: oxide_kll(p.k)?,
        k: p.k,
        _item: std::marker::PhantomData,
    })
}

pub fn memory_kll_oxide_per_call<T: QuantileValue>(sketch: &KllOxidePerCall<T>) -> usize {
    kll_footprint::<f64>(sketch.k)
}

pub struct KllOxideCdf<T = i64> {
    inner: sketch_oxide::quantiles::KllSketch,
    k: u32,
    /// `(value, cumulative_rank)` pairs, built in `prepare`.
    cdf: Option<Vec<(f64, f64)>>,
    ends: (f64, f64),
    _item: std::marker::PhantomData<T>,
}

pub fn build_kll_oxide_cdf<T: QuantileValue>(
    config: &ParamSet,
) -> Result<KllOxideCdf<T>, BuildError> {
    let p: KllParams = config.parse()?;
    Ok(KllOxideCdf {
        inner: oxide_kll(p.k)?,
        k: p.k,
        cdf: None,
        ends: (f64::NAN, f64::NAN),
        _item: std::marker::PhantomData,
    })
}

pub fn memory_kll_oxide_cdf<T: QuantileValue>(sketch: &KllOxideCdf<T>) -> usize {
    kll_footprint::<f64>(sketch.k)
}

pub fn insert_kll_oxide_per_call<T>(
    params: &ParamSet,
    items: Rc<Vec<T>>,
    passes: usize,
) -> Result<Vec<Pass>, BuildError>
where
    T: QuantileValue + 'static,
{
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        // Built here, so calling the closure is the insert and nothing else.
        let mut sketch = build_kll_oxide_per_call::<T>(params)?;
        let items = items.clone();
        out.push(Box::new(move || {
            for v in items.iter() {
                sketch.inner.update(v.to_f64());
            }
            memory_kll_oxide_per_call::<T>(&sketch)
        }) as Pass);
    }
    Ok(out)
}

pub fn insert_step_kll_oxide_per_call<T>(
    params: &ParamSet,
    items: Rc<Vec<T>>,
    passes: usize,
) -> Result<Vec<StepPass>, BuildError>
where
    T: QuantileValue + 'static,
{
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        let sketch: Shared<_> = Rc::new(RefCell::new(build_kll_oxide_per_call::<T>(params)?));
        let (driven, read) = (sketch.clone(), sketch);
        let stream = items.clone();
        out.push(StepPass {
            steps: items.len(),
            step: Box::new(move |i| {
                let sketch = &mut *driven.borrow_mut();
                let v = &stream[i];
                sketch.inner.update(v.to_f64());
            }),
            footprint: Box::new(move || memory_kll_oxide_per_call::<T>(&read.borrow())),
        });
    }
    Ok(out)
}

pub fn query_kll_oxide_per_call<T>(
    params: &ParamSet,
    items: Rc<Vec<T>>,
    probes: Rc<Vec<f64>>,
    passes: usize,
) -> Result<Vec<QueryPass<f64>>, BuildError>
where
    T: QuantileValue + 'static,
{
    merge_query_kll_oxide_per_call::<T>(params, items, probes, 1, passes)
}

/// The query, asked of the sketch a fold over `shards` shards leaves. One
/// shard is the plain query.
pub fn merge_query_kll_oxide_per_call<T>(
    params: &ParamSet,
    items: Rc<Vec<T>>,
    probes: Rc<Vec<f64>>,
    shards: usize,
    passes: usize,
) -> Result<Vec<QueryPass<f64>>, BuildError>
where
    T: QuantileValue + 'static,
{
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        // Built, fed and folded here: the closure below asks, and only asks.
        let (mut sketch, rest) = kll_oxide_per_call_shards(params, &items, shards)?;
        for other in rest.iter() {
            sketch
                .inner
                .merge(&other.inner)
                .expect("both operands built from one ParamSet, so k matches");
        }
        let probes = probes.clone();
        out.push(Box::new(move || {
            let mut answers = Vec::with_capacity(probes.len());
            for p in probes.iter() {
                answers.push(sketch.inner.quantile(*p).unwrap_or(f64::NAN));
            }
            let footprint = memory_kll_oxide_per_call::<T>(&sketch);
            (answers, footprint)
        }) as QueryPass<f64>);
    }
    Ok(out)
}

pub fn merge_kll_oxide_per_call<T>(
    params: &ParamSet,
    items: Rc<Vec<T>>,
    shards: usize,
    passes: usize,
) -> Result<Vec<Pass>, BuildError>
where
    T: QuantileValue + 'static,
{
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        let (mut acc, rest) = kll_oxide_per_call_shards(params, &items, shards)?;
        out.push(Box::new(move || {
            for other in rest.iter() {
                acc.inner
                    .merge(&other.inner)
                    .expect("both operands built from one ParamSet, so k matches");
            }
            memory_kll_oxide_per_call::<T>(&acc)
        }) as Pass);
    }
    Ok(out)
}

pub fn merge_step_kll_oxide_per_call<T>(
    params: &ParamSet,
    items: Rc<Vec<T>>,
    shards: usize,
    passes: usize,
) -> Result<Vec<StepPass>, BuildError>
where
    T: QuantileValue + 'static,
{
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        let (acc, rest) = kll_oxide_per_call_shards(params, &items, shards)?;
        let acc: Shared<_> = Rc::new(RefCell::new(acc));
        let (driven, read) = (acc.clone(), acc);
        out.push(StepPass {
            steps: rest.len(),
            step: Box::new(move |i| {
                let acc = &mut *driven.borrow_mut();
                let other = &rest[i];
                acc.inner
                    .merge(&other.inner)
                    .expect("both operands built from one ParamSet, so k matches");
            }),
            footprint: Box::new(move || memory_kll_oxide_per_call::<T>(&read.borrow())),
        });
    }
    Ok(out)
}

/// The shards a fold folds: the stream split `shards` ways, one sketch each,
/// all fed. The first is the accumulator, the rest are what it folds in.
#[allow(clippy::type_complexity)]
fn kll_oxide_per_call_shards<T>(
    params: &ParamSet,
    items: &[T],
    shards: usize,
) -> Result<(KllOxidePerCall<T>, Vec<KllOxidePerCall<T>>), BuildError>
where
    T: QuantileValue + 'static,
{
    let mut parts: Vec<KllOxidePerCall<T>> = Vec::new();
    for shard in partition(items, shards) {
        let mut sketch = build_kll_oxide_per_call::<T>(params)?;
        for v in shard {
            sketch.inner.update(v.to_f64());
        }
        parts.push(sketch);
    }
    let rest = parts.split_off(1);
    Ok((
        parts.pop().expect("a split of the stream is never empty"),
        rest,
    ))
}

pub fn insert_kll_oxide_cdf<T>(
    params: &ParamSet,
    items: Rc<Vec<T>>,
    passes: usize,
) -> Result<Vec<Pass>, BuildError>
where
    T: QuantileValue + 'static,
{
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        // Built here, so calling the closure is the insert and nothing else.
        let mut sketch = build_kll_oxide_cdf::<T>(params)?;
        let items = items.clone();
        out.push(Box::new(move || {
            for v in items.iter() {
                sketch.inner.update(v.to_f64());
            }
            memory_kll_oxide_cdf::<T>(&sketch)
        }) as Pass);
    }
    Ok(out)
}

pub fn insert_step_kll_oxide_cdf<T>(
    params: &ParamSet,
    items: Rc<Vec<T>>,
    passes: usize,
) -> Result<Vec<StepPass>, BuildError>
where
    T: QuantileValue + 'static,
{
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        let sketch: Shared<_> = Rc::new(RefCell::new(build_kll_oxide_cdf::<T>(params)?));
        let (driven, read) = (sketch.clone(), sketch);
        let stream = items.clone();
        out.push(StepPass {
            steps: items.len(),
            step: Box::new(move |i| {
                let sketch = &mut *driven.borrow_mut();
                let v = &stream[i];
                sketch.inner.update(v.to_f64());
            }),
            footprint: Box::new(move || memory_kll_oxide_cdf::<T>(&read.borrow())),
        });
    }
    Ok(out)
}

pub fn query_kll_oxide_cdf<T>(
    params: &ParamSet,
    items: Rc<Vec<T>>,
    probes: Rc<Vec<f64>>,
    passes: usize,
) -> Result<Vec<QueryPass<f64>>, BuildError>
where
    T: QuantileValue + 'static,
{
    merge_query_kll_oxide_cdf::<T>(params, items, probes, 1, passes)
}

/// The query, asked of the sketch a fold over `shards` shards leaves. One
/// shard is the plain query.
pub fn merge_query_kll_oxide_cdf<T>(
    params: &ParamSet,
    items: Rc<Vec<T>>,
    probes: Rc<Vec<f64>>,
    shards: usize,
    passes: usize,
) -> Result<Vec<QueryPass<f64>>, BuildError>
where
    T: QuantileValue + 'static,
{
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        // Built, fed and folded here: the closure below asks, and only asks.
        let (mut sketch, rest) = kll_oxide_cdf_shards(params, &items, shards)?;
        for other in rest.iter() {
            sketch
                .inner
                .merge(&other.inner)
                .expect("both operands built from one ParamSet, so k matches");
            // A merged sketch invalidates any table cached from the pre-merge state.
            sketch.cdf = None;
        }
        let (table, min, max) = (sketch.inner.cdf(), sketch.inner.min(), sketch.inner.max());
        let probes = probes.clone();
        out.push(Box::new(move || {
            let mut answers = Vec::with_capacity(probes.len());
            for p in probes.iter() {
                answers.push(query_cdf(&table, *p, min, max));
            }
            let footprint = memory_kll_oxide_cdf::<T>(&sketch);
            (answers, footprint)
        }) as QueryPass<f64>);
    }
    Ok(out)
}

pub fn merge_kll_oxide_cdf<T>(
    params: &ParamSet,
    items: Rc<Vec<T>>,
    shards: usize,
    passes: usize,
) -> Result<Vec<Pass>, BuildError>
where
    T: QuantileValue + 'static,
{
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        let (mut acc, rest) = kll_oxide_cdf_shards(params, &items, shards)?;
        out.push(Box::new(move || {
            for other in rest.iter() {
                acc.inner
                    .merge(&other.inner)
                    .expect("both operands built from one ParamSet, so k matches");
                // A merged sketch invalidates any table cached from the pre-merge state.
                acc.cdf = None;
            }
            memory_kll_oxide_cdf::<T>(&acc)
        }) as Pass);
    }
    Ok(out)
}

pub fn merge_step_kll_oxide_cdf<T>(
    params: &ParamSet,
    items: Rc<Vec<T>>,
    shards: usize,
    passes: usize,
) -> Result<Vec<StepPass>, BuildError>
where
    T: QuantileValue + 'static,
{
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        let (acc, rest) = kll_oxide_cdf_shards(params, &items, shards)?;
        let acc: Shared<_> = Rc::new(RefCell::new(acc));
        let (driven, read) = (acc.clone(), acc);
        out.push(StepPass {
            steps: rest.len(),
            step: Box::new(move |i| {
                let acc = &mut *driven.borrow_mut();
                let other = &rest[i];
                acc.inner
                    .merge(&other.inner)
                    .expect("both operands built from one ParamSet, so k matches");
                // A merged sketch invalidates any table cached from the pre-merge state.
                acc.cdf = None;
            }),
            footprint: Box::new(move || memory_kll_oxide_cdf::<T>(&read.borrow())),
        });
    }
    Ok(out)
}

/// The shards a fold folds: the stream split `shards` ways, one sketch each,
/// all fed. The first is the accumulator, the rest are what it folds in.
#[allow(clippy::type_complexity)]
fn kll_oxide_cdf_shards<T>(
    params: &ParamSet,
    items: &[T],
    shards: usize,
) -> Result<(KllOxideCdf<T>, Vec<KllOxideCdf<T>>), BuildError>
where
    T: QuantileValue + 'static,
{
    let mut parts: Vec<KllOxideCdf<T>> = Vec::new();
    for shard in partition(items, shards) {
        let mut sketch = build_kll_oxide_cdf::<T>(params)?;
        for v in shard {
            sketch.inner.update(v.to_f64());
        }
        parts.push(sketch);
    }
    let rest = parts.split_off(1);
    Ok((
        parts.pop().expect("a split of the stream is never empty"),
        rest,
    ))
}

pub fn prepare_kll_oxide_cdf<T>(
    params: &ParamSet,
    items: Rc<Vec<T>>,
    passes: usize,
) -> Result<Vec<Pass>, BuildError>
where
    T: QuantileValue + 'static,
{
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        // Fed here: the closure below is the step that makes it ready to answer.
        let mut sketch = build_kll_oxide_cdf::<T>(params)?;
        for v in items.iter() {
            sketch.inner.update(v.to_f64());
        }
        out.push(Box::new(move || {
            sketch.cdf = Some(sketch.inner.cdf());
            sketch.ends = (sketch.inner.min(), sketch.inner.max());
            memory_kll_oxide_cdf::<T>(&sketch)
        }) as Pass);
    }
    Ok(out)
}
