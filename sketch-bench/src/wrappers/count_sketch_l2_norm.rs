use crate::params::{CountSketchL2NormParams, ParamSet};
use crate::wrappers::frequency_value::FrequencyValue;
use crate::wrappers::{partition, require_positive, BuildError, Pass, QueryPass, Shared, StepPass};
use asap_sketchlib::{CountSketch, CountSketchWithL2Norm};
use std::cell::RefCell;
use std::rc::Rc;

pub struct CountSketchL2NormLib {
    inner: CountSketchWithL2Norm,
}

pub fn build_count_sketch_l2_norm_lib(
    config: &ParamSet,
) -> Result<CountSketchL2NormLib, BuildError> {
    let p: CountSketchL2NormParams = config.parse()?;
    require_positive("asap CountSketchWithL2Norm", "rows", p.rows)?;
    CountSketch::check_shape(p.rows, p.cols)
        .map_err(|e| BuildError(format!("asap CountSketchWithL2Norm: {e}")))?;
    Ok(CountSketchL2NormLib {
        inner: CountSketchWithL2Norm::with_dimensions(p.rows, p.cols),
    })
}

pub fn memory_count_sketch_l2_norm_lib(sketch: &CountSketchL2NormLib) -> usize {
    sketch.inner.memory_bytes()
}

pub fn insert_count_sketch_l2_norm_lib<K: FrequencyValue>(
    params: &ParamSet,
    items: Rc<Vec<(K, i64)>>,
    passes: usize,
) -> Result<Vec<Pass>, BuildError> {
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        let mut sketch = build_count_sketch_l2_norm_lib(params)?;
        let items = items.clone();
        out.push(Box::new(move || {
            for v in items.iter() {
                sketch.inner.update_weighted(&v.0.data_input(), v.1);
            }
            memory_count_sketch_l2_norm_lib(&sketch)
        }) as Pass);
    }
    Ok(out)
}

pub fn insert_step_count_sketch_l2_norm_lib<K: FrequencyValue>(
    params: &ParamSet,
    items: Rc<Vec<(K, i64)>>,
    passes: usize,
) -> Result<Vec<StepPass>, BuildError> {
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        let sketch: Shared<_> = Rc::new(RefCell::new(build_count_sketch_l2_norm_lib(params)?));
        let (driven, read) = (sketch.clone(), sketch);
        let stream = items.clone();
        out.push(StepPass {
            steps: items.len(),
            step: Box::new(move |i| {
                let sketch = &mut *driven.borrow_mut();
                let v = &stream[i];
                sketch.inner.update_weighted(&v.0.data_input(), v.1);
            }),
            footprint: Box::new(move || memory_count_sketch_l2_norm_lib(&read.borrow())),
        });
    }
    Ok(out)
}

pub fn query_count_sketch_l2_norm_lib<K: FrequencyValue>(
    params: &ParamSet,
    items: Rc<Vec<(K, i64)>>,
    probes: Rc<Vec<()>>,
    passes: usize,
) -> Result<Vec<QueryPass<f64>>, BuildError> {
    merge_query_count_sketch_l2_norm_lib(params, items, probes, 1, passes)
}

pub fn merge_query_count_sketch_l2_norm_lib<K: FrequencyValue>(
    params: &ParamSet,
    items: Rc<Vec<(K, i64)>>,
    probes: Rc<Vec<()>>,
    shards: usize,
    passes: usize,
) -> Result<Vec<QueryPass<f64>>, BuildError> {
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        let (mut sketch, rest) = count_sketch_l2_norm_lib_shards(params, &items, shards)?;
        for other in rest.iter() {
            sketch.inner.merge(&other.inner);
        }
        let probes = probes.clone();
        out.push(Box::new(move || {
            let mut answers = Vec::with_capacity(probes.len());
            for _p in probes.iter() {
                answers.push(sketch.inner.l2());
            }
            let footprint = memory_count_sketch_l2_norm_lib(&sketch);
            (answers, footprint)
        }) as QueryPass<f64>);
    }
    Ok(out)
}

pub fn merge_count_sketch_l2_norm_lib<K: FrequencyValue>(
    params: &ParamSet,
    items: Rc<Vec<(K, i64)>>,
    shards: usize,
    passes: usize,
) -> Result<Vec<Pass>, BuildError> {
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        let (mut acc, rest) = count_sketch_l2_norm_lib_shards(params, &items, shards)?;
        out.push(Box::new(move || {
            for other in rest.iter() {
                acc.inner.merge(&other.inner);
            }
            memory_count_sketch_l2_norm_lib(&acc)
        }) as Pass);
    }
    Ok(out)
}

pub fn merge_step_count_sketch_l2_norm_lib<K: FrequencyValue>(
    params: &ParamSet,
    items: Rc<Vec<(K, i64)>>,
    shards: usize,
    passes: usize,
) -> Result<Vec<StepPass>, BuildError> {
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        let (acc, rest) = count_sketch_l2_norm_lib_shards(params, &items, shards)?;
        let acc: Shared<_> = Rc::new(RefCell::new(acc));
        let (driven, read) = (acc.clone(), acc);
        out.push(StepPass {
            steps: rest.len(),
            step: Box::new(move |i| {
                let acc = &mut *driven.borrow_mut();
                let other = &rest[i];
                acc.inner.merge(&other.inner);
            }),
            footprint: Box::new(move || memory_count_sketch_l2_norm_lib(&read.borrow())),
        });
    }
    Ok(out)
}

#[allow(clippy::type_complexity)]
fn count_sketch_l2_norm_lib_shards<K: FrequencyValue>(
    params: &ParamSet,
    items: &[(K, i64)],
    shards: usize,
) -> Result<(CountSketchL2NormLib, Vec<CountSketchL2NormLib>), BuildError> {
    let mut parts: Vec<CountSketchL2NormLib> = Vec::new();
    for shard in partition(items, shards) {
        let mut sketch = build_count_sketch_l2_norm_lib(params)?;
        for v in shard {
            sketch.inner.update_weighted(&v.0.data_input(), v.1);
        }
        parts.push(sketch);
    }
    let rest = parts.split_off(1);
    Ok((
        parts.pop().expect("a split of the stream is never empty"),
        rest,
    ))
}
