//! `asap_sketchlib::CSHeap` — a Count Sketch paired with a top-k heap. The
//! CountSketch counterpart of `cms_heap`'s top-k row, and graded the same way:
//! heap capacity is `CMS_HEAP_TOP_K`, the `k` `TopkGT` grades against.
// ponytail: Vector2D + FastPath top-k only, the one the optimizer prices; add
// RegularPath / per-key estimate rows when something asks for them.

use crate::params::{CountSketchParams, ParamSet};
use crate::wrappers::cms_heap::sketchlib::{TopkAnswer, CMS_HEAP_TOP_K};
use crate::wrappers::frequency_value::FrequencyValue;
use crate::wrappers::{require_positive, BuildError, Pass, QueryPass, Shared, StepPass};
use asap_sketchlib::input::HHItem;
use asap_sketchlib::{heap_item_to_sketch_input, CSHeap, FastPath, Vector2D};
use std::cell::RefCell;
use std::rc::Rc;

pub struct CsHeapLibVector2dFast {
    inner: CSHeap<Vector2D<i32>, FastPath>,
    rows: usize,
    cols: usize,
}

pub fn build_cs_heap_lib_vector2d_fast(
    config: &ParamSet,
) -> Result<CsHeapLibVector2dFast, BuildError> {
    let p: CountSketchParams = config.parse()?;
    require_positive("asap CSHeap Vector2D FastPath", "rows", p.rows)?;
    require_positive("asap CSHeap Vector2D FastPath", "cols", p.cols)?;
    Ok(CsHeapLibVector2dFast {
        inner: CSHeap::<Vector2D<i32>, FastPath>::new(p.rows, p.cols, CMS_HEAP_TOP_K),
        rows: p.rows,
        cols: p.cols,
    })
}

/// Counter matrix plus a full heap, as `cms_heap` counts it.
pub fn memory_cs_heap_lib_vector2d_fast(sketch: &CsHeapLibVector2dFast) -> usize {
    sketch.rows * sketch.cols * std::mem::size_of::<i32>()
        + CMS_HEAP_TOP_K * std::mem::size_of::<HHItem>()
}

pub fn insert_cs_heap_lib_vector2d_fast<T: FrequencyValue>(
    params: &ParamSet,
    items: Rc<Vec<T>>,
    passes: usize,
) -> Result<Vec<Pass>, BuildError> {
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        let mut sketch = build_cs_heap_lib_vector2d_fast(params)?;
        let items = items.clone();
        out.push(Box::new(move || {
            for v in items.iter() {
                sketch.inner.insert(&v.data_input());
            }
            memory_cs_heap_lib_vector2d_fast(&sketch)
        }) as Pass);
    }
    Ok(out)
}

pub fn insert_step_cs_heap_lib_vector2d_fast<T: FrequencyValue>(
    params: &ParamSet,
    items: Rc<Vec<T>>,
    passes: usize,
) -> Result<Vec<StepPass>, BuildError> {
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        let sketch: Shared<_> = Rc::new(RefCell::new(build_cs_heap_lib_vector2d_fast(params)?));
        let (driven, read) = (sketch.clone(), sketch);
        let stream = items.clone();
        out.push(StepPass {
            steps: items.len(),
            step: Box::new(move |i| driven.borrow_mut().inner.insert(&stream[i].data_input())),
            footprint: Box::new(move || memory_cs_heap_lib_vector2d_fast(&read.borrow())),
        });
    }
    Ok(out)
}

pub fn query_cs_heap_lib_vector2d_fast_topk<T: FrequencyValue>(
    params: &ParamSet,
    items: Rc<Vec<T>>,
    probes: Rc<Vec<()>>,
    passes: usize,
) -> Result<Vec<QueryPass<TopkAnswer<T>>>, BuildError> {
    merge_query_cs_heap_lib_vector2d_fast_topk(params, items, probes, 1, passes)
}

pub fn merge_query_cs_heap_lib_vector2d_fast_topk<T: FrequencyValue>(
    params: &ParamSet,
    items: Rc<Vec<T>>,
    probes: Rc<Vec<()>>,
    shards: usize,
    passes: usize,
) -> Result<Vec<QueryPass<TopkAnswer<T>>>, BuildError> {
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        let (mut sketch, rest) = cs_heap_lib_vector2d_fast_shards(params, &items, shards)?;
        for other in rest.iter() {
            sketch.inner.merge(&other.inner);
        }
        let probes = probes.clone();
        out.push(Box::new(move || {
            let mut answers = Vec::with_capacity(probes.len());
            for _p in probes.iter() {
                let ranked: TopkAnswer<T> = sketch
                    .inner
                    .heap()
                    .heap()
                    .iter()
                    .map(|item| {
                        (
                            T::from_data_input(&heap_item_to_sketch_input(&item.key)),
                            item.count.max(0) as u64,
                        )
                    })
                    .collect();
                answers.push(ranked);
            }
            let footprint = memory_cs_heap_lib_vector2d_fast(&sketch);
            (answers, footprint)
        }) as QueryPass<TopkAnswer<T>>);
    }
    Ok(out)
}

pub fn merge_cs_heap_lib_vector2d_fast<T: FrequencyValue>(
    params: &ParamSet,
    items: Rc<Vec<T>>,
    shards: usize,
    passes: usize,
) -> Result<Vec<Pass>, BuildError> {
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        let (mut acc, rest) = cs_heap_lib_vector2d_fast_shards(params, &items, shards)?;
        out.push(Box::new(move || {
            for other in rest.iter() {
                acc.inner.merge(&other.inner);
            }
            memory_cs_heap_lib_vector2d_fast(&acc)
        }) as Pass);
    }
    Ok(out)
}

pub fn merge_step_cs_heap_lib_vector2d_fast<T: FrequencyValue>(
    params: &ParamSet,
    items: Rc<Vec<T>>,
    shards: usize,
    passes: usize,
) -> Result<Vec<StepPass>, BuildError> {
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        let (acc, rest) = cs_heap_lib_vector2d_fast_shards(params, &items, shards)?;
        let acc: Shared<_> = Rc::new(RefCell::new(acc));
        let (driven, read) = (acc.clone(), acc);
        out.push(StepPass {
            steps: rest.len(),
            step: Box::new(move |i| driven.borrow_mut().inner.merge(&rest[i].inner)),
            footprint: Box::new(move || memory_cs_heap_lib_vector2d_fast(&read.borrow())),
        });
    }
    Ok(out)
}

#[allow(clippy::type_complexity)]
fn cs_heap_lib_vector2d_fast_shards<T: FrequencyValue>(
    params: &ParamSet,
    items: &[T],
    shards: usize,
) -> Result<(CsHeapLibVector2dFast, Vec<CsHeapLibVector2dFast>), BuildError> {
    let mut parts = Vec::new();
    for shard in crate::wrappers::partition(items, shards) {
        let mut sketch = build_cs_heap_lib_vector2d_fast(params)?;
        for v in shard {
            sketch.inner.insert(&v.data_input());
        }
        parts.push(sketch);
    }
    let rest = parts.split_off(1);
    Ok((
        parts.pop().expect("a split of the stream is never empty"),
        rest,
    ))
}
