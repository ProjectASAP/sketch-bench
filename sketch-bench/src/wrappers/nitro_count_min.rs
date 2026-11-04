use crate::params::{NitroParams, ParamSet};
use crate::wrappers::{partition, require_positive, BuildError, Pass, QueryPass, Shared, StepPass};
use asap_sketchlib::{CountMinSketch, DataInput, FastPath, NitroBatch, Vector2D};
use std::cell::RefCell;
use std::rc::Rc;

const NAME: &str = "asap NitroBatch<CountMinSketch>";

pub struct NitroCountMinLib {
    inner: NitroBatch<CountMinSketch<Vector2D<i32>, FastPath>>,
}

pub fn build_nitro_count_min_lib(config: &ParamSet) -> Result<NitroCountMinLib, BuildError> {
    let p: NitroParams = config.parse()?;
    if !(p.sample_rate > 0.0 && p.sample_rate <= 1.0) {
        return Err(BuildError(format!(
            "{NAME}: sample_rate={} outside (0, 1]",
            p.sample_rate
        )));
    }
    require_positive(NAME, "rows", p.rows)?;
    CountMinSketch::check_shape(p.rows, p.cols).map_err(|e| BuildError(format!("{NAME}: {e}")))?;
    Ok(NitroCountMinLib {
        inner: NitroBatch::with_target_and_seed(
            p.sample_rate,
            CountMinSketch::<Vector2D<i32>, FastPath>::with_dimensions(p.rows, p.cols),
            p.seed,
        ),
    })
}

pub fn memory_nitro_count_min_lib(sketch: &NitroCountMinLib) -> usize {
    sketch.inner.memory_bytes()
}

pub fn insert_nitro_count_min_lib(
    params: &ParamSet,
    items: Rc<Vec<i64>>,
    passes: usize,
) -> Result<Vec<Pass>, BuildError> {
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        let mut sketch = build_nitro_count_min_lib(params)?;
        let items = items.clone();
        out.push(Box::new(move || {
            sketch.inner.update_batch(&items);
            memory_nitro_count_min_lib(&sketch)
        }) as Pass);
    }
    Ok(out)
}

pub fn insert_step_nitro_count_min_lib(
    params: &ParamSet,
    items: Rc<Vec<i64>>,
    passes: usize,
) -> Result<Vec<StepPass>, BuildError> {
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        let sketch: Shared<_> = Rc::new(RefCell::new(build_nitro_count_min_lib(params)?));
        let (driven, read) = (sketch.clone(), sketch);
        let stream = items.clone();
        out.push(StepPass {
            steps: items.len(),
            step: Box::new(move |i| {
                let sketch = &mut *driven.borrow_mut();
                let v = &stream[i];
                sketch.inner.update_batch(std::slice::from_ref(v));
            }),
            footprint: Box::new(move || memory_nitro_count_min_lib(&read.borrow())),
        });
    }
    Ok(out)
}

pub fn query_nitro_count_min_lib(
    params: &ParamSet,
    items: Rc<Vec<i64>>,
    probes: Rc<Vec<i64>>,
    passes: usize,
) -> Result<Vec<QueryPass<u64>>, BuildError> {
    merge_query_nitro_count_min_lib(params, items, probes, 1, passes)
}

pub fn merge_query_nitro_count_min_lib(
    params: &ParamSet,
    items: Rc<Vec<i64>>,
    probes: Rc<Vec<i64>>,
    shards: usize,
    passes: usize,
) -> Result<Vec<QueryPass<u64>>, BuildError> {
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        let (mut sketch, rest) = nitro_count_min_lib_shards(params, &items, shards)?;
        for other in rest.iter() {
            sketch.inner.merge(&other.inner);
        }
        let probes = probes.clone();
        out.push(Box::new(move || {
            let mut answers = Vec::with_capacity(probes.len());
            for p in probes.iter() {
                answers.push(
                    sketch
                        .inner
                        .estimate_median(&DataInput::I64(*p))
                        .round()
                        .max(0.0) as u64,
                );
            }
            let footprint = memory_nitro_count_min_lib(&sketch);
            (answers, footprint)
        }) as QueryPass<u64>);
    }
    Ok(out)
}

pub fn merge_nitro_count_min_lib(
    params: &ParamSet,
    items: Rc<Vec<i64>>,
    shards: usize,
    passes: usize,
) -> Result<Vec<Pass>, BuildError> {
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        let (mut acc, rest) = nitro_count_min_lib_shards(params, &items, shards)?;
        out.push(Box::new(move || {
            for other in rest.iter() {
                acc.inner.merge(&other.inner);
            }
            memory_nitro_count_min_lib(&acc)
        }) as Pass);
    }
    Ok(out)
}

pub fn merge_step_nitro_count_min_lib(
    params: &ParamSet,
    items: Rc<Vec<i64>>,
    shards: usize,
    passes: usize,
) -> Result<Vec<StepPass>, BuildError> {
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        let (acc, rest) = nitro_count_min_lib_shards(params, &items, shards)?;
        let acc: Shared<_> = Rc::new(RefCell::new(acc));
        let (driven, read) = (acc.clone(), acc);
        out.push(StepPass {
            steps: rest.len(),
            step: Box::new(move |i| {
                let acc = &mut *driven.borrow_mut();
                let other = &rest[i];
                acc.inner.merge(&other.inner);
            }),
            footprint: Box::new(move || memory_nitro_count_min_lib(&read.borrow())),
        });
    }
    Ok(out)
}

#[allow(clippy::type_complexity)]
fn nitro_count_min_lib_shards(
    params: &ParamSet,
    items: &[i64],
    shards: usize,
) -> Result<(NitroCountMinLib, Vec<NitroCountMinLib>), BuildError> {
    let mut parts: Vec<NitroCountMinLib> = Vec::new();
    for shard in partition(items, shards) {
        let mut sketch = build_nitro_count_min_lib(params)?;
        sketch.inner.update_batch(shard);
        parts.push(sketch);
    }
    let rest = parts.split_off(1);
    Ok((
        parts.pop().expect("a split of the stream is never empty"),
        rest,
    ))
}
