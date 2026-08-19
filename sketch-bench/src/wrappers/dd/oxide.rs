use super::*;
use crate::params::ParamSet;
use crate::wrappers::partition;
use crate::wrappers::{BuildError, Pass, QueryPass, Shared, StepPass};
use sketch_oxide::Mergeable as _;
use std::cell::RefCell;
use std::rc::Rc;

pub struct DdOxide<T = i64> {
    inner: sketch_oxide::quantiles::DDSketch,
    alpha: f64,
    _item: std::marker::PhantomData<T>,
}

pub fn build_dd_oxide<T: QuantileValue>(config: &ParamSet) -> Result<DdOxide<T>, BuildError> {
    let p: DdParams = config.parse()?;
    require_alpha("oxide DDSketch", p.alpha)?;
    let inner = sketch_oxide::quantiles::DDSketch::new(p.alpha)
        .map_err(|e| BuildError(format!("oxide DDSketch rejected alpha={}: {e:?}", p.alpha)))?;
    Ok(DdOxide {
        inner,
        alpha: p.alpha,
        _item: std::marker::PhantomData,
    })
}

pub fn memory_dd_oxide<T: QuantileValue>(sketch: &DdOxide<T>) -> usize {
    dd_footprint(
        sketch.alpha,
        sketch.inner.min(),
        sketch.inner.max(),
        sketch.inner.count(),
    )
}

pub fn insert_dd_oxide<T>(
    params: &ParamSet,
    items: Rc<Vec<T>>,
    passes: usize,
) -> Result<Vec<Pass>, BuildError>
where
    T: QuantileValue + 'static,
{
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        let mut sketch = build_dd_oxide::<T>(params)?;
        let items = items.clone();
        out.push(Box::new(move || {
            for v in items.iter() {
                sketch.inner.add(v.to_f64());
            }
            memory_dd_oxide::<T>(&sketch)
        }) as Pass);
    }
    Ok(out)
}

pub fn insert_step_dd_oxide<T>(
    params: &ParamSet,
    items: Rc<Vec<T>>,
    passes: usize,
) -> Result<Vec<StepPass>, BuildError>
where
    T: QuantileValue + 'static,
{
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        let sketch: Shared<_> = Rc::new(RefCell::new(build_dd_oxide::<T>(params)?));
        let (driven, read) = (sketch.clone(), sketch);
        let stream = items.clone();
        out.push(StepPass {
            steps: items.len(),
            step: Box::new(move |i| {
                let sketch = &mut *driven.borrow_mut();
                let v = &stream[i];
                sketch.inner.add(v.to_f64());
            }),
            footprint: Box::new(move || memory_dd_oxide::<T>(&read.borrow())),
        });
    }
    Ok(out)
}

pub fn query_dd_oxide<T>(
    params: &ParamSet,
    items: Rc<Vec<T>>,
    probes: Rc<Vec<f64>>,
    passes: usize,
) -> Result<Vec<QueryPass<f64>>, BuildError>
where
    T: QuantileValue + 'static,
{
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        let mut sketch = build_dd_oxide::<T>(params)?;
        for v in items.iter() {
            sketch.inner.add(v.to_f64());
        }
        let probes = probes.clone();
        out.push(Box::new(move || {
            let mut answers = Vec::with_capacity(probes.len());
            for p in probes.iter() {
                answers.push(sketch.inner.quantile(*p).unwrap_or(f64::NAN));
            }
            let footprint = memory_dd_oxide::<T>(&sketch);
            (answers, footprint)
        }) as QueryPass<f64>);
    }
    Ok(out)
}

pub fn merge_dd_oxide<T>(
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
        let (mut acc, rest) = dd_oxide_shards(params, &items, shards)?;
        out.push(Box::new(move || {
            for other in rest.iter() {
                acc.inner
                    .merge(&other.inner)
                    .expect("both operands built from one ParamSet, so alpha matches");
            }
            memory_dd_oxide::<T>(&acc)
        }) as Pass);
    }
    Ok(out)
}

pub fn merge_step_dd_oxide<T>(
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
        let (acc, rest) = dd_oxide_shards(params, &items, shards)?;
        let acc: Shared<_> = Rc::new(RefCell::new(acc));
        let (driven, read) = (acc.clone(), acc);
        out.push(StepPass {
            steps: rest.len(),
            step: Box::new(move |i| {
                let acc = &mut *driven.borrow_mut();
                let other = &rest[i];
                acc.inner
                    .merge(&other.inner)
                    .expect("both operands built from one ParamSet, so alpha matches");
            }),
            footprint: Box::new(move || memory_dd_oxide::<T>(&read.borrow())),
        });
    }
    Ok(out)
}

#[allow(clippy::type_complexity)]
fn dd_oxide_shards<T>(
    params: &ParamSet,
    items: &[T],
    shards: usize,
) -> Result<(DdOxide<T>, Vec<DdOxide<T>>), BuildError>
where
    T: QuantileValue + 'static,
{
    let mut parts: Vec<DdOxide<T>> = Vec::new();
    for shard in partition(items, shards) {
        let mut sketch = build_dd_oxide::<T>(params)?;
        for v in shard {
            sketch.inner.add(v.to_f64());
        }
        parts.push(sketch);
    }
    let rest = parts.split_off(1);
    Ok((
        parts.pop().expect("a split of the stream is never empty"),
        rest,
    ))
}
