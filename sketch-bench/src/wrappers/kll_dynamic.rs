use crate::params::{KllParams, ParamSet};
use crate::wrappers::kll::{LIB_K_MAX, LIB_K_MIN};
use crate::wrappers::quantile_value::QuantileValue;
use crate::wrappers::{partition, require_range, BuildError, Pass, QueryPass, Shared, StepPass};
use asap_sketchlib::common::numerical::NumericalValue;
use asap_sketchlib::KllDynamic;
use std::cell::RefCell;
use std::rc::Rc;

pub struct KllDynamicLib<T: NumericalValue = i64> {
    inner: KllDynamic<T>,
}

pub fn build_kll_dynamic_lib<T: NumericalValue>(
    config: &ParamSet,
) -> Result<KllDynamicLib<T>, BuildError> {
    let p: KllParams = config.parse()?;
    require_range("asap KllDynamic", "k", p.k, LIB_K_MIN, LIB_K_MAX)?;
    Ok(KllDynamicLib {
        inner: match p.seed {
            Some(seed) => KllDynamic::<T>::with_k_and_seed(p.k as i32, seed),
            None => KllDynamic::<T>::with_k(p.k as i32),
        },
    })
}

pub fn memory_kll_dynamic_lib<T: NumericalValue>(sketch: &KllDynamicLib<T>) -> usize {
    sketch.inner.memory_bytes()
}

pub fn insert_kll_dynamic_lib<T: NumericalValue + QuantileValue + 'static>(
    params: &ParamSet,
    items: Rc<Vec<T>>,
    passes: usize,
) -> Result<Vec<Pass>, BuildError> {
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        let mut sketch = build_kll_dynamic_lib(params)?;
        let items = items.clone();
        out.push(Box::new(move || {
            for v in items.iter() {
                sketch.inner.update(v);
            }
            memory_kll_dynamic_lib(&sketch)
        }) as Pass);
    }
    Ok(out)
}

pub fn insert_step_kll_dynamic_lib<T: NumericalValue + QuantileValue + 'static>(
    params: &ParamSet,
    items: Rc<Vec<T>>,
    passes: usize,
) -> Result<Vec<StepPass>, BuildError> {
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        let sketch: Shared<_> = Rc::new(RefCell::new(build_kll_dynamic_lib(params)?));
        let (driven, read) = (sketch.clone(), sketch);
        let stream = items.clone();
        out.push(StepPass {
            steps: items.len(),
            step: Box::new(move |i| {
                let sketch = &mut *driven.borrow_mut();
                let v = &stream[i];
                sketch.inner.update(v);
            }),
            footprint: Box::new(move || memory_kll_dynamic_lib(&read.borrow())),
        });
    }
    Ok(out)
}

pub fn query_kll_dynamic_lib<T: NumericalValue + QuantileValue + 'static>(
    params: &ParamSet,
    items: Rc<Vec<T>>,
    probes: Rc<Vec<f64>>,
    passes: usize,
) -> Result<Vec<QueryPass<f64>>, BuildError> {
    merge_query_kll_dynamic_lib(params, items, probes, 1, passes)
}

pub fn merge_query_kll_dynamic_lib<T: NumericalValue + QuantileValue + 'static>(
    params: &ParamSet,
    items: Rc<Vec<T>>,
    probes: Rc<Vec<f64>>,
    shards: usize,
    passes: usize,
) -> Result<Vec<QueryPass<f64>>, BuildError> {
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        let (mut sketch, rest) = kll_dynamic_lib_shards(params, &items, shards)?;
        for other in rest.iter() {
            sketch.inner.merge(&other.inner);
        }
        let probes = probes.clone();
        out.push(Box::new(move || {
            let mut answers = Vec::with_capacity(probes.len());
            for p in probes.iter() {
                answers.push(sketch.inner.quantile(*p));
            }
            let footprint = memory_kll_dynamic_lib(&sketch);
            (answers, footprint)
        }) as QueryPass<f64>);
    }
    Ok(out)
}

pub fn merge_kll_dynamic_lib<T: NumericalValue + QuantileValue + 'static>(
    params: &ParamSet,
    items: Rc<Vec<T>>,
    shards: usize,
    passes: usize,
) -> Result<Vec<Pass>, BuildError> {
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        let (mut acc, rest) = kll_dynamic_lib_shards(params, &items, shards)?;
        out.push(Box::new(move || {
            for other in rest.iter() {
                acc.inner.merge(&other.inner);
            }
            memory_kll_dynamic_lib(&acc)
        }) as Pass);
    }
    Ok(out)
}

pub fn merge_step_kll_dynamic_lib<T: NumericalValue + QuantileValue + 'static>(
    params: &ParamSet,
    items: Rc<Vec<T>>,
    shards: usize,
    passes: usize,
) -> Result<Vec<StepPass>, BuildError> {
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        let (acc, rest) = kll_dynamic_lib_shards(params, &items, shards)?;
        let acc: Shared<_> = Rc::new(RefCell::new(acc));
        let (driven, read) = (acc.clone(), acc);
        out.push(StepPass {
            steps: rest.len(),
            step: Box::new(move |i| {
                let acc = &mut *driven.borrow_mut();
                let other = &rest[i];
                acc.inner.merge(&other.inner);
            }),
            footprint: Box::new(move || memory_kll_dynamic_lib(&read.borrow())),
        });
    }
    Ok(out)
}

#[allow(clippy::type_complexity)]
fn kll_dynamic_lib_shards<T: NumericalValue + QuantileValue + 'static>(
    params: &ParamSet,
    items: &[T],
    shards: usize,
) -> Result<(KllDynamicLib<T>, Vec<KllDynamicLib<T>>), BuildError> {
    let mut parts: Vec<KllDynamicLib<T>> = Vec::new();
    for shard in partition(items, shards) {
        let mut sketch = build_kll_dynamic_lib(params)?;
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
