use super::*;
use crate::params::ParamSet;
use crate::wrappers::partition;
use crate::wrappers::{BuildError, Pass, QueryPass, Shared, StepPass};
use std::cell::RefCell;
use std::rc::Rc;

pub struct DdLib<T = i64> {
    inner: asap_sketchlib::DDSketch,
    alpha: f64,
    _item: std::marker::PhantomData<T>,
}

pub fn build_dd_lib<T>(config: &ParamSet) -> Result<DdLib<T>, BuildError>
where
    T: asap_sketchlib::common::numerical::NumericalValue,
{
    let p: DdParams = config.parse()?;
    require_alpha("asap DDSketch", p.alpha)?;
    Ok(DdLib {
        inner: asap_sketchlib::DDSketch::new(p.alpha),
        alpha: p.alpha,
        _item: std::marker::PhantomData,
    })
}

pub fn memory_dd_lib<T>(sketch: &DdLib<T>) -> usize
where
    T: asap_sketchlib::common::numerical::NumericalValue,
{
    dd_footprint(
        sketch.alpha,
        sketch.inner.min(),
        sketch.inner.max(),
        sketch.inner.get_count(),
    )
}

pub fn insert_dd_lib<T>(
    params: &ParamSet,
    items: Rc<Vec<T>>,
    passes: usize,
) -> Result<Vec<Pass>, BuildError>
where
    T: asap_sketchlib::common::numerical::NumericalValue + 'static,
{
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        let mut sketch = build_dd_lib::<T>(params)?;
        let items = items.clone();
        out.push(Box::new(move || {
            for v in items.iter() {
                sketch.inner.add(v);
            }
            memory_dd_lib::<T>(&sketch)
        }) as Pass);
    }
    Ok(out)
}

pub fn insert_step_dd_lib<T>(
    params: &ParamSet,
    items: Rc<Vec<T>>,
    passes: usize,
) -> Result<Vec<StepPass>, BuildError>
where
    T: asap_sketchlib::common::numerical::NumericalValue + 'static,
{
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        let sketch: Shared<_> = Rc::new(RefCell::new(build_dd_lib::<T>(params)?));
        let (driven, read) = (sketch.clone(), sketch);
        let stream = items.clone();
        out.push(StepPass {
            steps: items.len(),
            step: Box::new(move |i| {
                let sketch = &mut *driven.borrow_mut();
                let v = &stream[i];
                sketch.inner.add(v);
            }),
            footprint: Box::new(move || memory_dd_lib::<T>(&read.borrow())),
        });
    }
    Ok(out)
}

pub fn query_dd_lib<T>(
    params: &ParamSet,
    items: Rc<Vec<T>>,
    probes: Rc<Vec<f64>>,
    passes: usize,
) -> Result<Vec<QueryPass<f64>>, BuildError>
where
    T: asap_sketchlib::common::numerical::NumericalValue + 'static,
{
    merge_query_dd_lib::<T>(params, items, probes, 1, passes)
}

/// The query, asked of the sketch a fold over `shards` shards leaves. One
/// shard is the plain query.
pub fn merge_query_dd_lib<T>(
    params: &ParamSet,
    items: Rc<Vec<T>>,
    probes: Rc<Vec<f64>>,
    shards: usize,
    passes: usize,
) -> Result<Vec<QueryPass<f64>>, BuildError>
where
    T: asap_sketchlib::common::numerical::NumericalValue + 'static,
{
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        // Built, fed and folded here: the closure below asks, and only asks.
        let (mut sketch, rest) = dd_lib_shards(params, &items, shards)?;
        for other in rest.iter() {
            sketch.inner.merge(&other.inner);
        }
        let probes = probes.clone();
        out.push(Box::new(move || {
            let mut answers = Vec::with_capacity(probes.len());
            for p in probes.iter() {
                answers.push(sketch.inner.get_value_at_quantile(*p).unwrap_or(f64::NAN));
            }
            let footprint = memory_dd_lib::<T>(&sketch);
            (answers, footprint)
        }) as QueryPass<f64>);
    }
    Ok(out)
}

pub fn merge_dd_lib<T>(
    params: &ParamSet,
    items: Rc<Vec<T>>,
    shards: usize,
    passes: usize,
) -> Result<Vec<Pass>, BuildError>
where
    T: asap_sketchlib::common::numerical::NumericalValue + 'static,
{
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        let (mut acc, rest) = dd_lib_shards(params, &items, shards)?;
        out.push(Box::new(move || {
            for other in rest.iter() {
                acc.inner.merge(&other.inner);
            }
            memory_dd_lib::<T>(&acc)
        }) as Pass);
    }
    Ok(out)
}

pub fn merge_step_dd_lib<T>(
    params: &ParamSet,
    items: Rc<Vec<T>>,
    shards: usize,
    passes: usize,
) -> Result<Vec<StepPass>, BuildError>
where
    T: asap_sketchlib::common::numerical::NumericalValue + 'static,
{
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        let (acc, rest) = dd_lib_shards(params, &items, shards)?;
        let acc: Shared<_> = Rc::new(RefCell::new(acc));
        let (driven, read) = (acc.clone(), acc);
        out.push(StepPass {
            steps: rest.len(),
            step: Box::new(move |i| {
                let acc = &mut *driven.borrow_mut();
                acc.inner.merge(&rest[i].inner);
            }),
            footprint: Box::new(move || memory_dd_lib::<T>(&read.borrow())),
        });
    }
    Ok(out)
}

#[allow(clippy::type_complexity)]
fn dd_lib_shards<T>(
    params: &ParamSet,
    items: &[T],
    shards: usize,
) -> Result<(DdLib<T>, Vec<DdLib<T>>), BuildError>
where
    T: asap_sketchlib::common::numerical::NumericalValue + 'static,
{
    let mut parts: Vec<DdLib<T>> = Vec::new();
    for shard in partition(items, shards) {
        let mut sketch = build_dd_lib::<T>(params)?;
        for v in shard {
            sketch.inner.add(v);
        }
        parts.push(sketch);
    }
    let rest = parts.split_off(1);
    Ok((
        parts.pop().expect("a split of the stream is never empty"),
        rest,
    ))
}
