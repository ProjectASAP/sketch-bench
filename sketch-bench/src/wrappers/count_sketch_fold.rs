use crate::params::{CountSketchFoldParams, ParamSet};
use crate::wrappers::cms_heap::sketchlib::{heap_capacity, top_k, TopkAnswer};
use crate::wrappers::frequency_value::FrequencyValue;
use crate::wrappers::{partition, require_positive, BuildError, Pass, QueryPass, Shared, StepPass};
use asap_sketchlib::CountSketchFold;
use std::cell::RefCell;
use std::rc::Rc;

const NAME: &str = "asap CountSketchFold";

pub struct CountSketchFoldLib {
    inner: CountSketchFold,
}

pub fn build_count_sketch_fold_lib(config: &ParamSet) -> Result<CountSketchFoldLib, BuildError> {
    let p: CountSketchFoldParams = config.parse()?;
    require_positive(NAME, "rows", p.rows)?;
    require_positive(NAME, "heap_size", p.heap_size)?;
    if !p.full_cols.is_power_of_two() || p.full_cols > 65_536 {
        return Err(BuildError(format!(
            "{NAME}: full_cols={} must be a power of two at most 65536",
            p.full_cols
        )));
    }
    if p.fold_level > p.full_cols.trailing_zeros() {
        return Err(BuildError(format!(
            "{NAME}: fold_level={} folds full_cols={} below one column",
            p.fold_level, p.full_cols
        )));
    }
    Ok(CountSketchFoldLib {
        inner: CountSketchFold::new(p.rows, p.full_cols, p.fold_level, p.heap_size),
    })
}

pub fn memory_count_sketch_fold_lib(sketch: &CountSketchFoldLib) -> usize {
    sketch.inner.memory_bytes()
}

pub fn answered_k_count_sketch_fold_lib(config: &ParamSet) -> Result<usize, BuildError> {
    let p: CountSketchFoldParams = config.parse()?;
    Ok(heap_capacity(NAME, Some(p.heap_size), p.topk_k)?.1)
}

pub fn insert_count_sketch_fold_lib<T: FrequencyValue>(
    params: &ParamSet,
    items: Rc<Vec<T>>,
    passes: usize,
) -> Result<Vec<Pass>, BuildError> {
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        let mut sketch = build_count_sketch_fold_lib(params)?;
        let items = items.clone();
        out.push(Box::new(move || {
            for v in items.iter() {
                sketch.inner.update(&v.data_input());
            }
            memory_count_sketch_fold_lib(&sketch)
        }) as Pass);
    }
    Ok(out)
}

pub fn insert_step_count_sketch_fold_lib<T: FrequencyValue>(
    params: &ParamSet,
    items: Rc<Vec<T>>,
    passes: usize,
) -> Result<Vec<StepPass>, BuildError> {
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        let sketch: Shared<_> = Rc::new(RefCell::new(build_count_sketch_fold_lib(params)?));
        let (driven, read) = (sketch.clone(), sketch);
        let stream = items.clone();
        out.push(StepPass {
            steps: items.len(),
            step: Box::new(move |i| {
                let sketch = &mut *driven.borrow_mut();
                let v = &stream[i];
                sketch.inner.update(&v.data_input());
            }),
            footprint: Box::new(move || memory_count_sketch_fold_lib(&read.borrow())),
        });
    }
    Ok(out)
}

pub fn query_count_sketch_fold_lib_estimate<T: FrequencyValue>(
    params: &ParamSet,
    items: Rc<Vec<T>>,
    probes: Rc<Vec<T>>,
    passes: usize,
) -> Result<Vec<QueryPass<u64>>, BuildError> {
    merge_query_count_sketch_fold_lib_estimate(params, items, probes, 1, passes)
}

pub fn merge_query_count_sketch_fold_lib_estimate<T: FrequencyValue>(
    params: &ParamSet,
    items: Rc<Vec<T>>,
    probes: Rc<Vec<T>>,
    shards: usize,
    passes: usize,
) -> Result<Vec<QueryPass<u64>>, BuildError> {
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        let (mut sketch, rest) = count_sketch_fold_lib_shards(params, &items, shards)?;
        for other in rest.iter() {
            sketch.inner.merge_same_level(&other.inner);
        }
        let probes = probes.clone();
        out.push(Box::new(move || {
            let mut answers = Vec::with_capacity(probes.len());
            for p in probes.iter() {
                answers.push(sketch.inner.estimate(&p.data_input()).max(0) as u64);
            }
            let footprint = memory_count_sketch_fold_lib(&sketch);
            (answers, footprint)
        }) as QueryPass<u64>);
    }
    Ok(out)
}

pub fn query_count_sketch_fold_lib_topk<T: FrequencyValue>(
    params: &ParamSet,
    items: Rc<Vec<T>>,
    probes: Rc<Vec<()>>,
    passes: usize,
) -> Result<Vec<QueryPass<TopkAnswer<T>>>, BuildError> {
    merge_query_count_sketch_fold_lib_topk(params, items, probes, 1, passes)
}

pub fn merge_query_count_sketch_fold_lib_topk<T: FrequencyValue>(
    params: &ParamSet,
    items: Rc<Vec<T>>,
    probes: Rc<Vec<()>>,
    shards: usize,
    passes: usize,
) -> Result<Vec<QueryPass<TopkAnswer<T>>>, BuildError> {
    let k = answered_k_count_sketch_fold_lib(params)?;
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        let (mut sketch, rest) = count_sketch_fold_lib_shards(params, &items, shards)?;
        for other in rest.iter() {
            sketch.inner.merge_same_level(&other.inner);
        }
        let probes = probes.clone();
        out.push(Box::new(move || {
            let mut answers = Vec::with_capacity(probes.len());
            for p in probes.iter() {
                answers.push({
                    let _ = p;
                    top_k(sketch.inner.heap().heap().iter(), k)
                        .into_iter()
                        .map(|e| {
                            (
                                T::from_data_input(&e.key.as_data_input()),
                                e.count.max(0) as u64,
                            )
                        })
                        .collect()
                });
            }
            let footprint = memory_count_sketch_fold_lib(&sketch);
            (answers, footprint)
        }) as QueryPass<TopkAnswer<T>>);
    }
    Ok(out)
}

pub fn merge_count_sketch_fold_lib<T: FrequencyValue>(
    params: &ParamSet,
    items: Rc<Vec<T>>,
    shards: usize,
    passes: usize,
) -> Result<Vec<Pass>, BuildError> {
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        let (mut acc, rest) = count_sketch_fold_lib_shards(params, &items, shards)?;
        out.push(Box::new(move || {
            for other in rest.iter() {
                acc.inner.merge_same_level(&other.inner);
            }
            memory_count_sketch_fold_lib(&acc)
        }) as Pass);
    }
    Ok(out)
}

pub fn merge_step_count_sketch_fold_lib<T: FrequencyValue>(
    params: &ParamSet,
    items: Rc<Vec<T>>,
    shards: usize,
    passes: usize,
) -> Result<Vec<StepPass>, BuildError> {
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        let (acc, rest) = count_sketch_fold_lib_shards(params, &items, shards)?;
        let acc: Shared<_> = Rc::new(RefCell::new(acc));
        let (driven, read) = (acc.clone(), acc);
        out.push(StepPass {
            steps: rest.len(),
            step: Box::new(move |i| {
                let acc = &mut *driven.borrow_mut();
                let other = &rest[i];
                acc.inner.merge_same_level(&other.inner);
            }),
            footprint: Box::new(move || memory_count_sketch_fold_lib(&read.borrow())),
        });
    }
    Ok(out)
}

#[allow(clippy::type_complexity)]
fn count_sketch_fold_lib_shards<T: FrequencyValue>(
    params: &ParamSet,
    items: &[T],
    shards: usize,
) -> Result<(CountSketchFoldLib, Vec<CountSketchFoldLib>), BuildError> {
    let mut parts: Vec<CountSketchFoldLib> = Vec::new();
    for shard in partition(items, shards) {
        let mut sketch = build_count_sketch_fold_lib(params)?;
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
