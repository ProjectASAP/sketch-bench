use crate::params::{ParamSet, UniformSamplingParams};
use crate::wrappers::quantile_value::{QuantileValue, ToF64};
use crate::wrappers::{partition, BuildError, Pass, QueryPass, Shared, StepPass};
use asap_sketchlib::UniformSampling;
use std::cell::RefCell;
use std::rc::Rc;

pub struct UniformSamplingLib {
    inner: UniformSampling,
}

pub fn build_uniform_sampling_lib(config: &ParamSet) -> Result<UniformSamplingLib, BuildError> {
    let p: UniformSamplingParams = config.parse()?;
    if !(p.sample_rate > 0.0 && p.sample_rate <= 1.0) {
        return Err(BuildError(format!(
            "asap UniformSampling: sample_rate={} outside (0, 1]",
            p.sample_rate
        )));
    }
    Ok(UniformSamplingLib {
        inner: UniformSampling::with_seed(p.sample_rate, p.seed),
    })
}

pub fn memory_uniform_sampling_lib(sketch: &UniformSamplingLib) -> usize {
    sketch.inner.memory_bytes()
}

fn value_at(sorted: &[f64], q: f64) -> f64 {
    if sorted.is_empty() {
        return f64::NAN;
    }
    let rank = (q * sorted.len() as f64).ceil() as usize;
    sorted[rank.saturating_sub(1).min(sorted.len() - 1)]
}

pub fn insert_uniform_sampling_lib<T: QuantileValue + 'static>(
    params: &ParamSet,
    items: Rc<Vec<T>>,
    passes: usize,
) -> Result<Vec<Pass>, BuildError> {
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        let mut sketch = build_uniform_sampling_lib(params)?;
        let items = items.clone();
        out.push(Box::new(move || {
            for v in items.iter() {
                sketch.inner.update(ToF64::to_f64(*v));
            }
            memory_uniform_sampling_lib(&sketch)
        }) as Pass);
    }
    Ok(out)
}

pub fn insert_step_uniform_sampling_lib<T: QuantileValue + 'static>(
    params: &ParamSet,
    items: Rc<Vec<T>>,
    passes: usize,
) -> Result<Vec<StepPass>, BuildError> {
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        let sketch: Shared<_> = Rc::new(RefCell::new(build_uniform_sampling_lib(params)?));
        let (driven, read) = (sketch.clone(), sketch);
        let stream = items.clone();
        out.push(StepPass {
            steps: items.len(),
            step: Box::new(move |i| {
                let sketch = &mut *driven.borrow_mut();
                let v = &stream[i];
                sketch.inner.update(ToF64::to_f64(*v));
            }),
            footprint: Box::new(move || memory_uniform_sampling_lib(&read.borrow())),
        });
    }
    Ok(out)
}

pub fn query_uniform_sampling_lib<T: QuantileValue + 'static>(
    params: &ParamSet,
    items: Rc<Vec<T>>,
    probes: Rc<Vec<f64>>,
    passes: usize,
) -> Result<Vec<QueryPass<f64>>, BuildError> {
    merge_query_uniform_sampling_lib(params, items, probes, 1, passes)
}

pub fn merge_query_uniform_sampling_lib<T: QuantileValue + 'static>(
    params: &ParamSet,
    items: Rc<Vec<T>>,
    probes: Rc<Vec<f64>>,
    shards: usize,
    passes: usize,
) -> Result<Vec<QueryPass<f64>>, BuildError> {
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        let (mut sketch, rest) = uniform_sampling_lib_shards(params, &items, shards)?;
        for other in rest.iter() {
            sketch.inner.merge(&other.inner).unwrap_or_else(|e| {
                panic!("asap UniformSampling: shards built alike refused to merge: {e}")
            });
        }
        let probes = probes.clone();
        out.push(Box::new(move || {
            let mut sorted = sketch.inner.samples();
            sorted.sort_by(f64::total_cmp);
            let mut answers = Vec::with_capacity(probes.len());
            for p in probes.iter() {
                answers.push(value_at(&sorted, *p));
            }
            let footprint = memory_uniform_sampling_lib(&sketch);
            (answers, footprint)
        }) as QueryPass<f64>);
    }
    Ok(out)
}

pub fn merge_uniform_sampling_lib<T: QuantileValue + 'static>(
    params: &ParamSet,
    items: Rc<Vec<T>>,
    shards: usize,
    passes: usize,
) -> Result<Vec<Pass>, BuildError> {
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        let (mut acc, rest) = uniform_sampling_lib_shards(params, &items, shards)?;
        out.push(Box::new(move || {
            for other in rest.iter() {
                acc.inner.merge(&other.inner).unwrap_or_else(|e| {
                    panic!("asap UniformSampling: shards built alike refused to merge: {e}")
                });
            }
            memory_uniform_sampling_lib(&acc)
        }) as Pass);
    }
    Ok(out)
}

pub fn merge_step_uniform_sampling_lib<T: QuantileValue + 'static>(
    params: &ParamSet,
    items: Rc<Vec<T>>,
    shards: usize,
    passes: usize,
) -> Result<Vec<StepPass>, BuildError> {
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        let (acc, rest) = uniform_sampling_lib_shards(params, &items, shards)?;
        let acc: Shared<_> = Rc::new(RefCell::new(acc));
        let (driven, read) = (acc.clone(), acc);
        out.push(StepPass {
            steps: rest.len(),
            step: Box::new(move |i| {
                let acc = &mut *driven.borrow_mut();
                let other = &rest[i];
                acc.inner.merge(&other.inner).unwrap_or_else(|e| {
                    panic!("asap UniformSampling: shards built alike refused to merge: {e}")
                });
            }),
            footprint: Box::new(move || memory_uniform_sampling_lib(&read.borrow())),
        });
    }
    Ok(out)
}

#[allow(clippy::type_complexity)]
fn uniform_sampling_lib_shards<T: QuantileValue + 'static>(
    params: &ParamSet,
    items: &[T],
    shards: usize,
) -> Result<(UniformSamplingLib, Vec<UniformSamplingLib>), BuildError> {
    let mut parts: Vec<UniformSamplingLib> = Vec::new();
    for shard in partition(items, shards) {
        let mut sketch = build_uniform_sampling_lib(params)?;
        for v in shard {
            sketch.inner.update(ToF64::to_f64(*v));
        }
        parts.push(sketch);
    }
    let rest = parts.split_off(1);
    Ok((
        parts.pop().expect("a split of the stream is never empty"),
        rest,
    ))
}
