use crate::params::{KmvParams, ParamSet};
use crate::wrappers::hll::CardinalityValue;
use crate::wrappers::{partition, require_positive, BuildError, Pass, QueryPass, Shared, StepPass};
use asap_sketchlib::Kmv;
use std::cell::RefCell;
use std::rc::Rc;

pub struct KmvLib {
    inner: Kmv,
}

pub fn build_kmv_lib(config: &ParamSet) -> Result<KmvLib, BuildError> {
    let p: KmvParams = config.parse()?;
    require_positive("asap Kmv", "k", p.k)?;
    Ok(KmvLib {
        inner: Kmv::new(p.k),
    })
}

pub fn memory_kmv_lib(sketch: &KmvLib) -> usize {
    sketch.inner.memory_bytes()
}

pub fn insert_kmv_lib<T: CardinalityValue>(
    params: &ParamSet,
    items: Rc<Vec<T>>,
    passes: usize,
) -> Result<Vec<Pass>, BuildError> {
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        let mut sketch = build_kmv_lib(params)?;
        let items = items.clone();
        out.push(Box::new(move || {
            for v in items.iter() {
                sketch.inner.update(&v.data_input());
            }
            memory_kmv_lib(&sketch)
        }) as Pass);
    }
    Ok(out)
}

pub fn insert_step_kmv_lib<T: CardinalityValue>(
    params: &ParamSet,
    items: Rc<Vec<T>>,
    passes: usize,
) -> Result<Vec<StepPass>, BuildError> {
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        let sketch: Shared<_> = Rc::new(RefCell::new(build_kmv_lib(params)?));
        let (driven, read) = (sketch.clone(), sketch);
        let stream = items.clone();
        out.push(StepPass {
            steps: items.len(),
            step: Box::new(move |i| {
                let sketch = &mut *driven.borrow_mut();
                let v = &stream[i];
                sketch.inner.update(&v.data_input());
            }),
            footprint: Box::new(move || memory_kmv_lib(&read.borrow())),
        });
    }
    Ok(out)
}

pub fn query_kmv_lib<T: CardinalityValue>(
    params: &ParamSet,
    items: Rc<Vec<T>>,
    probes: Rc<Vec<()>>,
    passes: usize,
) -> Result<Vec<QueryPass<f64>>, BuildError> {
    merge_query_kmv_lib(params, items, probes, 1, passes)
}

pub fn merge_query_kmv_lib<T: CardinalityValue>(
    params: &ParamSet,
    items: Rc<Vec<T>>,
    probes: Rc<Vec<()>>,
    shards: usize,
    passes: usize,
) -> Result<Vec<QueryPass<f64>>, BuildError> {
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        let (mut sketch, rest) = kmv_lib_shards(params, &items, shards)?;
        for other in rest.iter() {
            sketch.inner.merge(&other.inner);
        }
        let probes = probes.clone();
        out.push(Box::new(move || {
            let mut answers = Vec::with_capacity(probes.len());
            for p in probes.iter() {
                answers.push({
                    let _ = p;
                    sketch.inner.estimate()
                });
            }
            let footprint = memory_kmv_lib(&sketch);
            (answers, footprint)
        }) as QueryPass<f64>);
    }
    Ok(out)
}

pub fn merge_kmv_lib<T: CardinalityValue>(
    params: &ParamSet,
    items: Rc<Vec<T>>,
    shards: usize,
    passes: usize,
) -> Result<Vec<Pass>, BuildError> {
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        let (mut acc, rest) = kmv_lib_shards(params, &items, shards)?;
        out.push(Box::new(move || {
            for other in rest.iter() {
                acc.inner.merge(&other.inner);
            }
            memory_kmv_lib(&acc)
        }) as Pass);
    }
    Ok(out)
}

pub fn merge_step_kmv_lib<T: CardinalityValue>(
    params: &ParamSet,
    items: Rc<Vec<T>>,
    shards: usize,
    passes: usize,
) -> Result<Vec<StepPass>, BuildError> {
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        let (acc, rest) = kmv_lib_shards(params, &items, shards)?;
        let acc: Shared<_> = Rc::new(RefCell::new(acc));
        let (driven, read) = (acc.clone(), acc);
        out.push(StepPass {
            steps: rest.len(),
            step: Box::new(move |i| {
                let acc = &mut *driven.borrow_mut();
                let other = &rest[i];
                acc.inner.merge(&other.inner);
            }),
            footprint: Box::new(move || memory_kmv_lib(&read.borrow())),
        });
    }
    Ok(out)
}

#[allow(clippy::type_complexity)]
fn kmv_lib_shards<T: CardinalityValue>(
    params: &ParamSet,
    items: &[T],
    shards: usize,
) -> Result<(KmvLib, Vec<KmvLib>), BuildError> {
    let mut parts: Vec<KmvLib> = Vec::new();
    for shard in partition(items, shards) {
        let mut sketch = build_kmv_lib(params)?;
        for v in shard {
            sketch.inner.update(&v.data_input());
        }
        parts.push(sketch);
    }
    let rest = parts.split_off(1);
    Ok((
        parts.pop().expect("a split of the stream is never empty"),
        rest,
    ))
}
