//! The `asap_sketchlib::CMSHeap` implementation. `Vector2D` storage only,
//! `FastPath`/`RegularPath` hashing — no `FixedMatrix` or parallel-insert
//! variant, see the follow-up issue linked from #95.

use crate::params::{CmsHeapParams, CsHeapParams, ParamSet};
use crate::wrappers::frequency_value::FrequencyValue;
use crate::wrappers::{require_positive, BuildError, Pass, QueryPass, Shared, StepPass};
use std::cell::RefCell;
use std::rc::Rc;

use asap_sketchlib::input::HHItem;
use asap_sketchlib::{heap_item_to_sketch_input, CMSHeap, FastPath, RegularPath, Vector2D};

/// The `k` a top-k row answers and is graded at when `--config` sets no
/// `topk_k`, and the heap's capacity when it sets no `heap`. A larger heap
/// (`heap=`) keeps more candidates, so a merge of `m` heaps of `m · k` loses
/// fewer of the true top `k`; the answer is still its heaviest `k`. A heap
/// smaller than `k` couldn't answer, and is refused at build.
pub const TOPK_K: usize = 32;

/// `(heap capacity, answered k)` for `heap` and `topk_k`: `k` defaults to
/// `TOPK_K`, the heap to `k`; a heap smaller than `k` is refused.
pub fn heap_capacity(
    name: &str,
    heap: Option<usize>,
    topk_k: Option<usize>,
) -> Result<(usize, usize), BuildError> {
    let k = topk_k.unwrap_or(TOPK_K);
    if k == 0 {
        return Err(BuildError(format!("{name}: topk_k must be positive")));
    }
    let heap = heap.unwrap_or(k);
    if heap < k {
        return Err(BuildError(format!(
            "{name}: heap={heap} is smaller than the k={k} it answers"
        )));
    }
    Ok((heap, k))
}

/// The `k` a top-k row with `params` answers: its `topk_k`, else `TOPK_K`.
/// Rows without the knob (UnivMon, exact) answer `TOPK_K`.
pub fn answered_k(params: &ParamSet) -> usize {
    params
        .parse::<CmsHeapParams>()
        .ok()
        .and_then(|p| p.topk_k)
        .or_else(|| params.parse::<CsHeapParams>().ok().and_then(|p| p.topk_k))
        .unwrap_or(TOPK_K)
}

/// A heap's contents, heaviest first, cut to the `k` a top-k query answers.
pub fn top_k<'a>(items: impl Iterator<Item = &'a HHItem>, k: usize) -> Vec<&'a HHItem> {
    let mut items: Vec<&HHItem> = items.collect();
    let heaviest = |item: &&HHItem| std::cmp::Reverse(item.count);
    // Select the heaviest k without sorting the rest: the query's cost is
    // what's benchmarked.
    if items.len() > k {
        items.select_nth_unstable_by_key(k, heaviest);
        items.truncate(k);
    }
    items.sort_unstable_by_key(heaviest);
    items
}

/// One key read off the heap, paired with its estimated count. `TopkGT<T>`
/// scores a `Vec` of these against the exact top-k.
///
/// The keys come back at the width they went in at: `HeapItem` carries the
/// variant the insert stored, so a row fed `u64` reads `u64` back. Widening
/// that translation is what [`FrequencyValue::from_data_input`] is for.
pub type TopkAnswer<T> = Vec<(T, u64)>;

// ---------- asap_sketchlib: Vector2D + FastPath ----------
pub struct CmsHeapLibVector2dFast {
    inner: CMSHeap<Vector2D<i32>, FastPath>,
    rows: usize,
    cols: usize,
    heap: usize,
    k: usize,
}

pub fn build_cms_heap_lib_vector2d_fast(
    config: &ParamSet,
) -> Result<CmsHeapLibVector2dFast, BuildError> {
    let p: CmsHeapParams = config.parse()?;
    require_positive("asap CMSHeap Vector2D FastPath", "rows", p.rows)?;
    require_positive("asap CMSHeap Vector2D FastPath", "cols", p.cols)?;
    let (heap, k) = heap_capacity("asap CMSHeap Vector2D FastPath", p.heap, p.topk_k)?;
    Ok(CmsHeapLibVector2dFast {
        inner: CMSHeap::<Vector2D<i32>, FastPath>::new(p.rows, p.cols, heap),
        rows: p.rows,
        cols: p.cols,
        heap,
        k,
    })
}

/// The counter matrix plus a full heap of `heap` `HHItem`s, counted the way
/// UnivMon counts its heaps so the TopK rows compare like for like.
pub fn memory_cms_heap_lib_vector2d_fast(sketch: &CmsHeapLibVector2dFast) -> usize {
    sketch.rows * sketch.cols * std::mem::size_of::<i32>()
        + crate::wrappers::hh_heap_footprint(sketch.heap)
}

// ---------- asap_sketchlib: Vector2D + RegularPath ----------
pub struct CmsHeapLibVector2dRegular {
    inner: CMSHeap<Vector2D<i32>, RegularPath>,
    rows: usize,
    cols: usize,
    heap: usize,
    k: usize,
}

pub fn build_cms_heap_lib_vector2d_regular(
    config: &ParamSet,
) -> Result<CmsHeapLibVector2dRegular, BuildError> {
    let p: CmsHeapParams = config.parse()?;
    require_positive("asap CMSHeap Vector2D RegularPath", "rows", p.rows)?;
    require_positive("asap CMSHeap Vector2D RegularPath", "cols", p.cols)?;
    let (heap, k) = heap_capacity("asap CMSHeap Vector2D RegularPath", p.heap, p.topk_k)?;
    Ok(CmsHeapLibVector2dRegular {
        inner: CMSHeap::<Vector2D<i32>, RegularPath>::new(p.rows, p.cols, heap),
        rows: p.rows,
        cols: p.cols,
        heap,
        k,
    })
}

/// As [`memory_cms_heap_lib_vector2d_fast`].
pub fn memory_cms_heap_lib_vector2d_regular(sketch: &CmsHeapLibVector2dRegular) -> usize {
    sketch.rows * sketch.cols * std::mem::size_of::<i32>()
        + crate::wrappers::hh_heap_footprint(sketch.heap)
}

// ---------- insert / insert_step (FastPath) ----------

pub fn insert_cms_heap_lib_vector2d_fast<T: FrequencyValue>(
    params: &ParamSet,
    items: Rc<Vec<T>>,
    passes: usize,
) -> Result<Vec<Pass>, BuildError> {
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        let mut sketch = build_cms_heap_lib_vector2d_fast(params)?;
        let items = items.clone();
        out.push(Box::new(move || {
            for v in items.iter() {
                sketch.inner.insert(&v.data_input());
            }
            memory_cms_heap_lib_vector2d_fast(&sketch)
        }) as Pass);
    }
    Ok(out)
}

pub fn insert_step_cms_heap_lib_vector2d_fast<T: FrequencyValue>(
    params: &ParamSet,
    items: Rc<Vec<T>>,
    passes: usize,
) -> Result<Vec<StepPass>, BuildError> {
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        let sketch: Shared<_> = Rc::new(RefCell::new(build_cms_heap_lib_vector2d_fast(params)?));
        let (driven, read) = (sketch.clone(), sketch);
        let stream = items.clone();
        out.push(StepPass {
            steps: items.len(),
            step: Box::new(move |i| {
                let sketch = &mut *driven.borrow_mut();
                let v = &stream[i];
                sketch.inner.insert(&v.data_input());
            }),
            footprint: Box::new(move || memory_cms_heap_lib_vector2d_fast(&read.borrow())),
        });
    }
    Ok(out)
}

// ---------- insert / insert_step (RegularPath) ----------

pub fn insert_cms_heap_lib_vector2d_regular<T: FrequencyValue>(
    params: &ParamSet,
    items: Rc<Vec<T>>,
    passes: usize,
) -> Result<Vec<Pass>, BuildError> {
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        let mut sketch = build_cms_heap_lib_vector2d_regular(params)?;
        let items = items.clone();
        out.push(Box::new(move || {
            for v in items.iter() {
                sketch.inner.insert(&v.data_input());
            }
            memory_cms_heap_lib_vector2d_regular(&sketch)
        }) as Pass);
    }
    Ok(out)
}

pub fn insert_step_cms_heap_lib_vector2d_regular<T: FrequencyValue>(
    params: &ParamSet,
    items: Rc<Vec<T>>,
    passes: usize,
) -> Result<Vec<StepPass>, BuildError> {
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        let sketch: Shared<_> = Rc::new(RefCell::new(build_cms_heap_lib_vector2d_regular(params)?));
        let (driven, read) = (sketch.clone(), sketch);
        let stream = items.clone();
        out.push(StepPass {
            steps: items.len(),
            step: Box::new(move |i| {
                let sketch = &mut *driven.borrow_mut();
                let v = &stream[i];
                sketch.inner.insert(&v.data_input());
            }),
            footprint: Box::new(move || memory_cms_heap_lib_vector2d_regular(&read.borrow())),
        });
    }
    Ok(out)
}

// ---------- query: per-key estimate (Frequency capability) ----------

pub fn query_cms_heap_lib_vector2d_fast_estimate<T: FrequencyValue>(
    params: &ParamSet,
    items: Rc<Vec<T>>,
    probes: Rc<Vec<T>>,
    passes: usize,
) -> Result<Vec<QueryPass<u64>>, BuildError> {
    merge_query_cms_heap_lib_vector2d_fast_estimate(params, items, probes, 1, passes)
}

/// The query, asked of the sketch a fold over `shards` shards leaves. One
/// shard is the plain query.
pub fn merge_query_cms_heap_lib_vector2d_fast_estimate<T: FrequencyValue>(
    params: &ParamSet,
    items: Rc<Vec<T>>,
    probes: Rc<Vec<T>>,
    shards: usize,
    passes: usize,
) -> Result<Vec<QueryPass<u64>>, BuildError> {
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        // Built, fed and folded here: the closure below asks, and only asks.
        let (mut sketch, rest) = cms_heap_lib_vector2d_fast_shards(params, &items, shards)?;
        for other in rest.iter() {
            sketch.inner.merge(&other.inner);
        }
        let probes = probes.clone();
        out.push(Box::new(move || {
            let mut answers = Vec::with_capacity(probes.len());
            for p in probes.iter() {
                answers.push(sketch.inner.estimate(&p.data_input()) as u64);
            }
            let footprint = memory_cms_heap_lib_vector2d_fast(&sketch);
            (answers, footprint)
        }) as QueryPass<u64>);
    }
    Ok(out)
}

pub fn query_cms_heap_lib_vector2d_regular_estimate<T: FrequencyValue>(
    params: &ParamSet,
    items: Rc<Vec<T>>,
    probes: Rc<Vec<T>>,
    passes: usize,
) -> Result<Vec<QueryPass<u64>>, BuildError> {
    merge_query_cms_heap_lib_vector2d_regular_estimate(params, items, probes, 1, passes)
}

/// The query, asked of the sketch a fold over `shards` shards leaves. One
/// shard is the plain query.
pub fn merge_query_cms_heap_lib_vector2d_regular_estimate<T: FrequencyValue>(
    params: &ParamSet,
    items: Rc<Vec<T>>,
    probes: Rc<Vec<T>>,
    shards: usize,
    passes: usize,
) -> Result<Vec<QueryPass<u64>>, BuildError> {
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        // Built, fed and folded here: the closure below asks, and only asks.
        let (mut sketch, rest) = cms_heap_lib_vector2d_regular_shards(params, &items, shards)?;
        for other in rest.iter() {
            sketch.inner.merge(&other.inner);
        }
        let probes = probes.clone();
        out.push(Box::new(move || {
            let mut answers = Vec::with_capacity(probes.len());
            for p in probes.iter() {
                answers.push(sketch.inner.estimate(&p.data_input()) as u64);
            }
            let footprint = memory_cms_heap_lib_vector2d_regular(&sketch);
            (answers, footprint)
        }) as QueryPass<u64>);
    }
    Ok(out)
}

// ---------- query: heap dump (TopK capability) ----------
//
// One answer per `()` probe: the whole ranked heap, read off in one call.
// Every probe is the same question; `TopkGT` repeats it so a pass's timed
// region sits well above the timer floor, as `CardinalityGT` does for HLL.

pub fn query_cms_heap_lib_vector2d_fast_topk<T: FrequencyValue>(
    params: &ParamSet,
    items: Rc<Vec<T>>,
    probes: Rc<Vec<()>>,
    passes: usize,
) -> Result<Vec<QueryPass<TopkAnswer<T>>>, BuildError> {
    merge_query_cms_heap_lib_vector2d_fast_topk(params, items, probes, 1, passes)
}

/// The query, asked of the sketch a fold over `shards` shards leaves. One
/// shard is the plain query.
pub fn merge_query_cms_heap_lib_vector2d_fast_topk<T: FrequencyValue>(
    params: &ParamSet,
    items: Rc<Vec<T>>,
    probes: Rc<Vec<()>>,
    shards: usize,
    passes: usize,
) -> Result<Vec<QueryPass<TopkAnswer<T>>>, BuildError> {
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        // Built, fed and folded here: the closure below asks, and only asks.
        let (mut sketch, rest) = cms_heap_lib_vector2d_fast_shards(params, &items, shards)?;
        for other in rest.iter() {
            sketch.inner.merge(&other.inner);
        }
        let probes = probes.clone();
        out.push(Box::new(move || {
            let mut answers = Vec::with_capacity(probes.len());
            for _p in probes.iter() {
                let ranked: TopkAnswer<T> = top_k(sketch.inner.heap().heap().iter(), sketch.k)
                    .into_iter()
                    .map(|item| {
                        (
                            T::from_data_input(&heap_item_to_sketch_input(&item.key)),
                            item.count as u64,
                        )
                    })
                    .collect();
                answers.push(ranked);
            }
            let footprint = memory_cms_heap_lib_vector2d_fast(&sketch);
            (answers, footprint)
        }) as QueryPass<TopkAnswer<T>>);
    }
    Ok(out)
}

pub fn query_cms_heap_lib_vector2d_regular_topk<T: FrequencyValue>(
    params: &ParamSet,
    items: Rc<Vec<T>>,
    probes: Rc<Vec<()>>,
    passes: usize,
) -> Result<Vec<QueryPass<TopkAnswer<T>>>, BuildError> {
    merge_query_cms_heap_lib_vector2d_regular_topk(params, items, probes, 1, passes)
}

/// The query, asked of the sketch a fold over `shards` shards leaves. One
/// shard is the plain query.
pub fn merge_query_cms_heap_lib_vector2d_regular_topk<T: FrequencyValue>(
    params: &ParamSet,
    items: Rc<Vec<T>>,
    probes: Rc<Vec<()>>,
    shards: usize,
    passes: usize,
) -> Result<Vec<QueryPass<TopkAnswer<T>>>, BuildError> {
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        // Built, fed and folded here: the closure below asks, and only asks.
        let (mut sketch, rest) = cms_heap_lib_vector2d_regular_shards(params, &items, shards)?;
        for other in rest.iter() {
            sketch.inner.merge(&other.inner);
        }
        let probes = probes.clone();
        out.push(Box::new(move || {
            let mut answers = Vec::with_capacity(probes.len());
            for _p in probes.iter() {
                let ranked: TopkAnswer<T> = top_k(sketch.inner.heap().heap().iter(), sketch.k)
                    .into_iter()
                    .map(|item| {
                        (
                            T::from_data_input(&heap_item_to_sketch_input(&item.key)),
                            item.count as u64,
                        )
                    })
                    .collect();
                answers.push(ranked);
            }
            let footprint = memory_cms_heap_lib_vector2d_regular(&sketch);
            (answers, footprint)
        }) as QueryPass<TopkAnswer<T>>);
    }
    Ok(out)
}

// ---------- merge (shared by both capability rows per backend) ----------
//
// Heavier than plain CMS's merge: after folding the counter matrices,
// `CMSHeap::merge` re-estimates every heap candidate from both sides (up to
// 2 * heap `estimate()` calls) to rebuild the merged heap. Not a
// pure cell-fold the way plain CMS merge is — see the registry description.

pub fn merge_cms_heap_lib_vector2d_fast<T: FrequencyValue>(
    params: &ParamSet,
    items: Rc<Vec<T>>,
    shards: usize,
    passes: usize,
) -> Result<Vec<Pass>, BuildError> {
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        let (mut acc, rest) = cms_heap_lib_vector2d_fast_shards(params, &items, shards)?;
        out.push(Box::new(move || {
            for other in rest.iter() {
                acc.inner.merge(&other.inner);
            }
            memory_cms_heap_lib_vector2d_fast(&acc)
        }) as Pass);
    }
    Ok(out)
}

pub fn merge_step_cms_heap_lib_vector2d_fast<T: FrequencyValue>(
    params: &ParamSet,
    items: Rc<Vec<T>>,
    shards: usize,
    passes: usize,
) -> Result<Vec<StepPass>, BuildError> {
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        let (acc, rest) = cms_heap_lib_vector2d_fast_shards(params, &items, shards)?;
        let acc: Shared<_> = Rc::new(RefCell::new(acc));
        let (driven, read) = (acc.clone(), acc);
        out.push(StepPass {
            steps: rest.len(),
            step: Box::new(move |i| {
                let acc = &mut *driven.borrow_mut();
                let other = &rest[i];
                acc.inner.merge(&other.inner);
            }),
            footprint: Box::new(move || memory_cms_heap_lib_vector2d_fast(&read.borrow())),
        });
    }
    Ok(out)
}

#[allow(clippy::type_complexity)]
fn cms_heap_lib_vector2d_fast_shards<T: FrequencyValue>(
    params: &ParamSet,
    items: &[T],
    shards: usize,
) -> Result<(CmsHeapLibVector2dFast, Vec<CmsHeapLibVector2dFast>), BuildError> {
    let mut parts: Vec<CmsHeapLibVector2dFast> = Vec::new();
    for shard in crate::wrappers::partition(items, shards) {
        let mut sketch = build_cms_heap_lib_vector2d_fast(params)?;
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

pub fn merge_cms_heap_lib_vector2d_regular<T: FrequencyValue>(
    params: &ParamSet,
    items: Rc<Vec<T>>,
    shards: usize,
    passes: usize,
) -> Result<Vec<Pass>, BuildError> {
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        let (mut acc, rest) = cms_heap_lib_vector2d_regular_shards(params, &items, shards)?;
        out.push(Box::new(move || {
            for other in rest.iter() {
                acc.inner.merge(&other.inner);
            }
            memory_cms_heap_lib_vector2d_regular(&acc)
        }) as Pass);
    }
    Ok(out)
}

pub fn merge_step_cms_heap_lib_vector2d_regular<T: FrequencyValue>(
    params: &ParamSet,
    items: Rc<Vec<T>>,
    shards: usize,
    passes: usize,
) -> Result<Vec<StepPass>, BuildError> {
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        let (acc, rest) = cms_heap_lib_vector2d_regular_shards(params, &items, shards)?;
        let acc: Shared<_> = Rc::new(RefCell::new(acc));
        let (driven, read) = (acc.clone(), acc);
        out.push(StepPass {
            steps: rest.len(),
            step: Box::new(move |i| {
                let acc = &mut *driven.borrow_mut();
                let other = &rest[i];
                acc.inner.merge(&other.inner);
            }),
            footprint: Box::new(move || memory_cms_heap_lib_vector2d_regular(&read.borrow())),
        });
    }
    Ok(out)
}

#[allow(clippy::type_complexity)]
fn cms_heap_lib_vector2d_regular_shards<T: FrequencyValue>(
    params: &ParamSet,
    items: &[T],
    shards: usize,
) -> Result<(CmsHeapLibVector2dRegular, Vec<CmsHeapLibVector2dRegular>), BuildError> {
    let mut parts: Vec<CmsHeapLibVector2dRegular> = Vec::new();
    for shard in crate::wrappers::partition(items, shards) {
        let mut sketch = build_cms_heap_lib_vector2d_regular(params)?;
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
