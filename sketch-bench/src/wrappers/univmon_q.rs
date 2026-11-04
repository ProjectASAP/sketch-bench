use crate::params::{ParamSet, UnivMonQParams};
use crate::wrappers::cms_heap::sketchlib::{heap_capacity, TopkAnswer};
use crate::wrappers::quantile_value::QuantileValue;
use crate::wrappers::{partition, BuildError, Pass, QueryPass, Shared, StepPass};
use asap_sketchlib::common::numerical::NumericalValue;
use asap_sketchlib::{UnivMonQ, UnivMonQConfig};
use std::cell::RefCell;
use std::rc::Rc;

const NAME: &str = "asap UnivMonQ";

pub trait UnivMonQValue: NumericalValue + QuantileValue + 'static {
    fn from_f64(value: f64) -> Self;
}

impl UnivMonQValue for i64 {
    fn from_f64(value: f64) -> Self {
        value as i64
    }
}

impl UnivMonQValue for u64 {
    fn from_f64(value: f64) -> Self {
        value as u64
    }
}

impl UnivMonQValue for f64 {
    fn from_f64(value: f64) -> Self {
        value
    }
}

pub struct UnivMonQLib {
    inner: UnivMonQ,
}

pub fn build_univmon_q_lib(config: &ParamSet) -> Result<UnivMonQLib, BuildError> {
    let p: UnivMonQParams = config.parse()?;
    let inner = UnivMonQ::new(UnivMonQConfig {
        layers: p.layers,
        top_layer_cols: p.top_layer_cols,
        cols_halving_period: p.cols_halving_period,
        rows: p.rows,
        counter_bits: p.counter_bits,
        heap_size: p.heap_size,
        ordered_samples: p.ordered_samples,
        seed_index: p.seed_index,
    })
    .map_err(|e| BuildError(format!("{NAME}: {e:?}")))?;
    Ok(UnivMonQLib { inner })
}

pub fn memory_univmon_q_lib(sketch: &UnivMonQLib) -> usize {
    sketch.inner.memory_bytes()
}

pub fn answered_k_univmon_q_lib(config: &ParamSet) -> Result<usize, BuildError> {
    let p: UnivMonQParams = config.parse()?;
    Ok(heap_capacity(NAME, Some(p.heap_size), p.topk_k)?.1)
}

pub fn insert_univmon_q_lib<T: UnivMonQValue>(
    params: &ParamSet,
    items: Rc<Vec<T>>,
    passes: usize,
) -> Result<Vec<Pass>, BuildError> {
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        let mut sketch = build_univmon_q_lib(params)?;
        let items = items.clone();
        out.push(Box::new(move || {
            for v in items.iter() {
                sketch.inner.update(v);
            }
            memory_univmon_q_lib(&sketch)
        }) as Pass);
    }
    Ok(out)
}

pub fn insert_step_univmon_q_lib<T: UnivMonQValue>(
    params: &ParamSet,
    items: Rc<Vec<T>>,
    passes: usize,
) -> Result<Vec<StepPass>, BuildError> {
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        let sketch: Shared<_> = Rc::new(RefCell::new(build_univmon_q_lib(params)?));
        let (driven, read) = (sketch.clone(), sketch);
        let stream = items.clone();
        out.push(StepPass {
            steps: items.len(),
            step: Box::new(move |i| {
                let sketch = &mut *driven.borrow_mut();
                let v = &stream[i];
                sketch.inner.update(v);
            }),
            footprint: Box::new(move || memory_univmon_q_lib(&read.borrow())),
        });
    }
    Ok(out)
}

pub fn query_univmon_q_lib_quantile<T: UnivMonQValue>(
    params: &ParamSet,
    items: Rc<Vec<T>>,
    probes: Rc<Vec<f64>>,
    passes: usize,
) -> Result<Vec<QueryPass<f64>>, BuildError> {
    merge_query_univmon_q_lib_quantile(params, items, probes, 1, passes)
}

pub fn merge_query_univmon_q_lib_quantile<T: UnivMonQValue>(
    params: &ParamSet,
    items: Rc<Vec<T>>,
    probes: Rc<Vec<f64>>,
    shards: usize,
    passes: usize,
) -> Result<Vec<QueryPass<f64>>, BuildError> {
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        let (mut sketch, rest) = univmon_q_lib_shards(params, &items, shards)?;
        for other in rest.iter() {
            sketch
                .inner
                .merge(&other.inner)
                .unwrap_or_else(|e| panic!("{NAME}: shards built alike refused to merge: {e:?}"));
        }
        let probes = probes.clone();
        out.push(Box::new(move || {
            let mut answers = Vec::with_capacity(probes.len());
            for p in probes.iter() {
                answers.push(sketch.inner.quantile(*p).unwrap_or(f64::NAN));
            }
            let footprint = memory_univmon_q_lib(&sketch);
            (answers, footprint)
        }) as QueryPass<f64>);
    }
    Ok(out)
}

pub fn query_univmon_q_lib_cardinality<T: UnivMonQValue>(
    params: &ParamSet,
    items: Rc<Vec<T>>,
    probes: Rc<Vec<()>>,
    passes: usize,
) -> Result<Vec<QueryPass<f64>>, BuildError> {
    merge_query_univmon_q_lib_cardinality(params, items, probes, 1, passes)
}

pub fn merge_query_univmon_q_lib_cardinality<T: UnivMonQValue>(
    params: &ParamSet,
    items: Rc<Vec<T>>,
    probes: Rc<Vec<()>>,
    shards: usize,
    passes: usize,
) -> Result<Vec<QueryPass<f64>>, BuildError> {
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        let (mut sketch, rest) = univmon_q_lib_shards(params, &items, shards)?;
        for other in rest.iter() {
            sketch
                .inner
                .merge(&other.inner)
                .unwrap_or_else(|e| panic!("{NAME}: shards built alike refused to merge: {e:?}"));
        }
        let probes = probes.clone();
        out.push(Box::new(move || {
            let mut answers = Vec::with_capacity(probes.len());
            for _p in probes.iter() {
                answers.push(sketch.inner.estimate_cardinality());
            }
            let footprint = memory_univmon_q_lib(&sketch);
            (answers, footprint)
        }) as QueryPass<f64>);
    }
    Ok(out)
}

pub fn query_univmon_q_lib_frequency<T: UnivMonQValue>(
    params: &ParamSet,
    items: Rc<Vec<T>>,
    probes: Rc<Vec<T>>,
    passes: usize,
) -> Result<Vec<QueryPass<u64>>, BuildError> {
    merge_query_univmon_q_lib_frequency(params, items, probes, 1, passes)
}

pub fn merge_query_univmon_q_lib_frequency<T: UnivMonQValue>(
    params: &ParamSet,
    items: Rc<Vec<T>>,
    probes: Rc<Vec<T>>,
    shards: usize,
    passes: usize,
) -> Result<Vec<QueryPass<u64>>, BuildError> {
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        let (mut sketch, rest) = univmon_q_lib_shards(params, &items, shards)?;
        for other in rest.iter() {
            sketch
                .inner
                .merge(&other.inner)
                .unwrap_or_else(|e| panic!("{NAME}: shards built alike refused to merge: {e:?}"));
        }
        let probes = probes.clone();
        out.push(Box::new(move || {
            let mut answers = Vec::with_capacity(probes.len());
            for p in probes.iter() {
                answers.push(sketch.inner.estimate_frequency(NumericalValue::to_f64(*p)));
            }
            let footprint = memory_univmon_q_lib(&sketch);
            (answers, footprint)
        }) as QueryPass<u64>);
    }
    Ok(out)
}

pub fn query_univmon_q_lib_topk<T: UnivMonQValue>(
    params: &ParamSet,
    items: Rc<Vec<T>>,
    probes: Rc<Vec<()>>,
    passes: usize,
) -> Result<Vec<QueryPass<TopkAnswer<T>>>, BuildError> {
    merge_query_univmon_q_lib_topk(params, items, probes, 1, passes)
}

pub fn merge_query_univmon_q_lib_topk<T: UnivMonQValue>(
    params: &ParamSet,
    items: Rc<Vec<T>>,
    probes: Rc<Vec<()>>,
    shards: usize,
    passes: usize,
) -> Result<Vec<QueryPass<TopkAnswer<T>>>, BuildError> {
    let k = answered_k_univmon_q_lib(params)?;
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        let (mut sketch, rest) = univmon_q_lib_shards(params, &items, shards)?;
        for other in rest.iter() {
            sketch
                .inner
                .merge(&other.inner)
                .unwrap_or_else(|e| panic!("{NAME}: shards built alike refused to merge: {e:?}"));
        }
        let probes = probes.clone();
        out.push(Box::new(move || {
            let mut answers = Vec::with_capacity(probes.len());
            for _p in probes.iter() {
                answers.push(
                    sketch
                        .inner
                        .heavy_hitters(k)
                        .into_iter()
                        .map(|(value, count)| (T::from_f64(value), count))
                        .collect(),
                );
            }
            let footprint = memory_univmon_q_lib(&sketch);
            (answers, footprint)
        }) as QueryPass<TopkAnswer<T>>);
    }
    Ok(out)
}

pub fn merge_univmon_q_lib<T: UnivMonQValue>(
    params: &ParamSet,
    items: Rc<Vec<T>>,
    shards: usize,
    passes: usize,
) -> Result<Vec<Pass>, BuildError> {
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        let (mut acc, rest) = univmon_q_lib_shards(params, &items, shards)?;
        out.push(Box::new(move || {
            for other in rest.iter() {
                acc.inner.merge(&other.inner).unwrap_or_else(|e| {
                    panic!("{NAME}: shards built alike refused to merge: {e:?}")
                });
            }
            memory_univmon_q_lib(&acc)
        }) as Pass);
    }
    Ok(out)
}

pub fn merge_step_univmon_q_lib<T: UnivMonQValue>(
    params: &ParamSet,
    items: Rc<Vec<T>>,
    shards: usize,
    passes: usize,
) -> Result<Vec<StepPass>, BuildError> {
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        let (acc, rest) = univmon_q_lib_shards(params, &items, shards)?;
        let acc: Shared<_> = Rc::new(RefCell::new(acc));
        let (driven, read) = (acc.clone(), acc);
        out.push(StepPass {
            steps: rest.len(),
            step: Box::new(move |i| {
                let acc = &mut *driven.borrow_mut();
                let other = &rest[i];
                acc.inner.merge(&other.inner).unwrap_or_else(|e| {
                    panic!("{NAME}: shards built alike refused to merge: {e:?}")
                });
            }),
            footprint: Box::new(move || memory_univmon_q_lib(&read.borrow())),
        });
    }
    Ok(out)
}

#[allow(clippy::type_complexity)]
fn univmon_q_lib_shards<T: UnivMonQValue>(
    params: &ParamSet,
    items: &[T],
    shards: usize,
) -> Result<(UnivMonQLib, Vec<UnivMonQLib>), BuildError> {
    let mut parts: Vec<UnivMonQLib> = Vec::new();
    for shard in partition(items, shards) {
        let mut sketch = build_univmon_q_lib(params)?;
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
