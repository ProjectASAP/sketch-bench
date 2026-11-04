use crate::params::{CocoParams, ParamSet};
use crate::wrappers::{partition, require_positive, BuildError, Pass, QueryPass, Shared, StepPass};
use asap_sketchlib::CocoSketch;
use std::cell::RefCell;
use std::rc::Rc;

pub struct CocoLib {
    inner: CocoSketch,
}

pub fn build_coco_lib(config: &ParamSet) -> Result<CocoLib, BuildError> {
    let p: CocoParams = config.parse()?;
    require_positive("asap CocoSketch", "rows", p.rows)?;
    require_positive("asap CocoSketch", "cols", p.cols)?;
    Ok(CocoLib {
        inner: match p.seed {
            Some(seed) => CocoSketch::with_dimensions_and_seed(p.rows, p.cols, seed),
            None => CocoSketch::with_dimensions(p.rows, p.cols),
        },
    })
}

pub fn memory_coco_lib(sketch: &CocoLib) -> usize {
    sketch.inner.memory_bytes()
}

pub fn insert_coco_lib(
    params: &ParamSet,
    items: Rc<Vec<String>>,
    passes: usize,
) -> Result<Vec<Pass>, BuildError> {
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        let mut sketch = build_coco_lib(params)?;
        let items = items.clone();
        out.push(Box::new(move || {
            for v in items.iter() {
                sketch.inner.update_weighted(v, 1);
            }
            memory_coco_lib(&sketch)
        }) as Pass);
    }
    Ok(out)
}

pub fn insert_step_coco_lib(
    params: &ParamSet,
    items: Rc<Vec<String>>,
    passes: usize,
) -> Result<Vec<StepPass>, BuildError> {
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        let sketch: Shared<_> = Rc::new(RefCell::new(build_coco_lib(params)?));
        let (driven, read) = (sketch.clone(), sketch);
        let stream = items.clone();
        out.push(StepPass {
            steps: items.len(),
            step: Box::new(move |i| {
                let sketch = &mut *driven.borrow_mut();
                let v = &stream[i];
                sketch.inner.update_weighted(v, 1);
            }),
            footprint: Box::new(move || memory_coco_lib(&read.borrow())),
        });
    }
    Ok(out)
}

pub fn query_coco_lib(
    params: &ParamSet,
    items: Rc<Vec<String>>,
    probes: Rc<Vec<String>>,
    passes: usize,
) -> Result<Vec<QueryPass<u64>>, BuildError> {
    merge_query_coco_lib(params, items, probes, 1, passes)
}

pub fn merge_query_coco_lib(
    params: &ParamSet,
    items: Rc<Vec<String>>,
    probes: Rc<Vec<String>>,
    shards: usize,
    passes: usize,
) -> Result<Vec<QueryPass<u64>>, BuildError> {
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        let (mut sketch, rest) = coco_lib_shards(params, &items, shards)?;
        for other in rest.iter() {
            sketch.inner.merge(&other.inner);
        }
        let probes = probes.clone();
        out.push(Box::new(move || {
            let mut answers = Vec::with_capacity(probes.len());
            for p in probes.iter() {
                answers.push(sketch.inner.estimate(p));
            }
            let footprint = memory_coco_lib(&sketch);
            (answers, footprint)
        }) as QueryPass<u64>);
    }
    Ok(out)
}

pub fn merge_coco_lib(
    params: &ParamSet,
    items: Rc<Vec<String>>,
    shards: usize,
    passes: usize,
) -> Result<Vec<Pass>, BuildError> {
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        let (mut acc, rest) = coco_lib_shards(params, &items, shards)?;
        out.push(Box::new(move || {
            for other in rest.iter() {
                acc.inner.merge(&other.inner);
            }
            memory_coco_lib(&acc)
        }) as Pass);
    }
    Ok(out)
}

pub fn merge_step_coco_lib(
    params: &ParamSet,
    items: Rc<Vec<String>>,
    shards: usize,
    passes: usize,
) -> Result<Vec<StepPass>, BuildError> {
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        let (acc, rest) = coco_lib_shards(params, &items, shards)?;
        let acc: Shared<_> = Rc::new(RefCell::new(acc));
        let (driven, read) = (acc.clone(), acc);
        out.push(StepPass {
            steps: rest.len(),
            step: Box::new(move |i| {
                let acc = &mut *driven.borrow_mut();
                let other = &rest[i];
                acc.inner.merge(&other.inner);
            }),
            footprint: Box::new(move || memory_coco_lib(&read.borrow())),
        });
    }
    Ok(out)
}

#[allow(clippy::type_complexity)]
fn coco_lib_shards(
    params: &ParamSet,
    items: &[String],
    shards: usize,
) -> Result<(CocoLib, Vec<CocoLib>), BuildError> {
    let mut parts: Vec<CocoLib> = Vec::new();
    for shard in partition(items, shards) {
        let mut sketch = build_coco_lib(params)?;
        for v in shard {
            sketch.inner.update_weighted(v, 1);
        }
        parts.push(sketch);
    }
    let rest = parts.split_off(1);
    Ok((
        parts.pop().expect("a split of the stream is never empty"),
        rest,
    ))
}
