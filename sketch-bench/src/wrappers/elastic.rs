use crate::params::{ElasticParams, ParamSet};
use crate::wrappers::cms_heap::sketchlib::{heap_capacity, TopkAnswer};
use crate::wrappers::{partition, require_positive, BuildError, Pass, QueryPass, Shared, StepPass};
use asap_sketchlib::{CountMinSketch, ElasticSketch};
use std::cell::RefCell;
use std::rc::Rc;

const NAME: &str = "asap ElasticSketch";

pub struct ElasticLib {
    inner: ElasticSketch,
}

pub fn build_elastic_lib(config: &ParamSet) -> Result<ElasticLib, BuildError> {
    let p: ElasticParams = config.parse()?;
    require_positive(NAME, "heavy_buckets", p.heavy_buckets)?;
    let buckets = i32::try_from(p.heavy_buckets).map_err(|_| {
        BuildError(format!(
            "{NAME}: heavy_buckets={} exceeds the i32 its API takes",
            p.heavy_buckets
        ))
    })?;
    CountMinSketch::check_shape(p.light_rows, p.light_cols)
        .map_err(|e| BuildError(format!("{NAME} light layer: {e}")))?;
    Ok(ElasticLib {
        inner: ElasticSketch::with_heavy_and_light(buckets, p.light_rows, p.light_cols),
    })
}

pub fn memory_elastic_lib(sketch: &ElasticLib) -> usize {
    sketch.inner.memory_bytes()
}

pub fn answered_k_elastic_lib(config: &ParamSet) -> Result<usize, BuildError> {
    let p: ElasticParams = config.parse()?;
    Ok(heap_capacity(NAME, Some(p.heavy_buckets), p.topk_k)?.1)
}

pub fn insert_elastic_lib(
    params: &ParamSet,
    items: Rc<Vec<String>>,
    passes: usize,
) -> Result<Vec<Pass>, BuildError> {
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        let mut sketch = build_elastic_lib(params)?;
        let items = items.clone();
        out.push(Box::new(move || {
            for v in items.iter() {
                sketch.inner.update(v.clone());
            }
            memory_elastic_lib(&sketch)
        }) as Pass);
    }
    Ok(out)
}

pub fn insert_step_elastic_lib(
    params: &ParamSet,
    items: Rc<Vec<String>>,
    passes: usize,
) -> Result<Vec<StepPass>, BuildError> {
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        let sketch: Shared<_> = Rc::new(RefCell::new(build_elastic_lib(params)?));
        let (driven, read) = (sketch.clone(), sketch);
        let stream = items.clone();
        out.push(StepPass {
            steps: items.len(),
            step: Box::new(move |i| {
                let sketch = &mut *driven.borrow_mut();
                let v = &stream[i];
                sketch.inner.update(v.clone());
            }),
            footprint: Box::new(move || memory_elastic_lib(&read.borrow())),
        });
    }
    Ok(out)
}

pub fn query_elastic_lib_estimate(
    params: &ParamSet,
    items: Rc<Vec<String>>,
    probes: Rc<Vec<String>>,
    passes: usize,
) -> Result<Vec<QueryPass<u64>>, BuildError> {
    merge_query_elastic_lib_estimate(params, items, probes, 1, passes)
}

pub fn merge_query_elastic_lib_estimate(
    params: &ParamSet,
    items: Rc<Vec<String>>,
    probes: Rc<Vec<String>>,
    shards: usize,
    passes: usize,
) -> Result<Vec<QueryPass<u64>>, BuildError> {
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        let (mut sketch, rest) = elastic_lib_shards(params, &items, shards)?;
        for other in rest.iter() {
            sketch.inner.merge(&other.inner);
        }
        let probes = probes.clone();
        out.push(Box::new(move || {
            let mut answers = Vec::with_capacity(probes.len());
            for p in probes.iter() {
                answers.push(sketch.inner.estimate(p.clone()).max(0) as u64);
            }
            let footprint = memory_elastic_lib(&sketch);
            (answers, footprint)
        }) as QueryPass<u64>);
    }
    Ok(out)
}

pub fn query_elastic_lib_topk(
    params: &ParamSet,
    items: Rc<Vec<String>>,
    probes: Rc<Vec<()>>,
    passes: usize,
) -> Result<Vec<QueryPass<TopkAnswer<String>>>, BuildError> {
    merge_query_elastic_lib_topk(params, items, probes, 1, passes)
}

pub fn merge_query_elastic_lib_topk(
    params: &ParamSet,
    items: Rc<Vec<String>>,
    probes: Rc<Vec<()>>,
    shards: usize,
    passes: usize,
) -> Result<Vec<QueryPass<TopkAnswer<String>>>, BuildError> {
    let k = answered_k_elastic_lib(params)?;
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        let (mut sketch, rest) = elastic_lib_shards(params, &items, shards)?;
        for other in rest.iter() {
            sketch.inner.merge(&other.inner);
        }
        let probes = probes.clone();
        out.push(Box::new(move || {
            let mut answers = Vec::with_capacity(probes.len());
            for _p in probes.iter() {
                answers.push(
                    sketch
                        .inner
                        .top_k(k)
                        .into_iter()
                        .map(|(key, count)| (key, count.max(0) as u64))
                        .collect(),
                );
            }
            let footprint = memory_elastic_lib(&sketch);
            (answers, footprint)
        }) as QueryPass<TopkAnswer<String>>);
    }
    Ok(out)
}

pub fn merge_elastic_lib(
    params: &ParamSet,
    items: Rc<Vec<String>>,
    shards: usize,
    passes: usize,
) -> Result<Vec<Pass>, BuildError> {
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        let (mut acc, rest) = elastic_lib_shards(params, &items, shards)?;
        out.push(Box::new(move || {
            for other in rest.iter() {
                acc.inner.merge(&other.inner);
            }
            memory_elastic_lib(&acc)
        }) as Pass);
    }
    Ok(out)
}

pub fn merge_step_elastic_lib(
    params: &ParamSet,
    items: Rc<Vec<String>>,
    shards: usize,
    passes: usize,
) -> Result<Vec<StepPass>, BuildError> {
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        let (acc, rest) = elastic_lib_shards(params, &items, shards)?;
        let acc: Shared<_> = Rc::new(RefCell::new(acc));
        let (driven, read) = (acc.clone(), acc);
        out.push(StepPass {
            steps: rest.len(),
            step: Box::new(move |i| {
                let acc = &mut *driven.borrow_mut();
                let other = &rest[i];
                acc.inner.merge(&other.inner);
            }),
            footprint: Box::new(move || memory_elastic_lib(&read.borrow())),
        });
    }
    Ok(out)
}

#[allow(clippy::type_complexity)]
fn elastic_lib_shards(
    params: &ParamSet,
    items: &[String],
    shards: usize,
) -> Result<(ElasticLib, Vec<ElasticLib>), BuildError> {
    let mut parts: Vec<ElasticLib> = Vec::new();
    for shard in partition(items, shards) {
        let mut sketch = build_elastic_lib(params)?;
        for v in shard {
            sketch.inner.update(v.clone());
        }
        parts.push(sketch);
    }
    let rest = parts.split_off(1);
    Ok((
        parts.pop().expect("a split of the stream is never empty"),
        rest,
    ))
}
