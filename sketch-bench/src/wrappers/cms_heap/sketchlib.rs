//! The `asap_sketchlib::CMSHeap` implementation. `Vector2D` storage only,
//! `FastPath`/`RegularPath` hashing — no `FixedMatrix` or parallel-insert
//! variant, see the follow-up issue linked from #95.

use crate::params::{CmsHeapParams, ParamSet};
use crate::wrappers::{require_positive, BuildError, Pass, QueryPass, Shared, StepPass};
use std::cell::RefCell;
use std::rc::Rc;

use asap_sketchlib::{
    heap_item_to_sketch_input, CMSHeap, DataInput, FastPath, RegularPath, Vector2D,
};

/// The heap's fixed capacity, and the `k` every TopK-capability row grades
/// against. The two are the same constant on purpose (see #95's design-decision
/// comment): a `CMSHeap` cannot answer a `k` bigger than its own capacity, so
/// letting grading `k` and construction capacity diverge — whether by a
/// `--config` field or a second constant — would let them silently disagree.
/// Changing this means editing it and rebuilding, not a config sweep; the
/// follow-up issue tracks loosening that.
pub const CMS_HEAP_TOP_K: usize = 32;

/// One key read off the heap, paired with its estimated count. `TopkGT<i64>`
/// scores a `Vec` of these against the exact top-k.
pub type TopkAnswer = Vec<(i64, u64)>;

fn heap_key_to_i64(item: &DataInput) -> i64 {
    match item {
        DataInput::I64(v) => *v,
        other => panic!("cms-heap rows only ever insert DataInput::I64 keys, got {other:?}"),
    }
}

// ---------- asap_sketchlib: Vector2D + FastPath ----------
pub struct CmsHeapLibVector2dFast {
    inner: CMSHeap<Vector2D<i32>, FastPath>,
    rows: usize,
    cols: usize,
}

pub fn build_cms_heap_lib_vector2d_fast(
    config: &ParamSet,
) -> Result<CmsHeapLibVector2dFast, BuildError> {
    let p: CmsHeapParams = config.parse()?;
    require_positive("asap CMSHeap Vector2D FastPath", "rows", p.rows)?;
    require_positive("asap CMSHeap Vector2D FastPath", "cols", p.cols)?;
    Ok(CmsHeapLibVector2dFast {
        inner: CMSHeap::<Vector2D<i32>, FastPath>::new(p.rows, p.cols, CMS_HEAP_TOP_K),
        rows: p.rows,
        cols: p.cols,
    })
}

pub fn memory_cms_heap_lib_vector2d_fast(sketch: &CmsHeapLibVector2dFast) -> usize {
    // The heap holds at most `CMS_HEAP_TOP_K` items — negligible next to the
    // counter matrix, and not counted here, matching how the plain CMS rows
    // don't count their own bookkeeping fields either.
    sketch.rows * sketch.cols * std::mem::size_of::<i32>()
}

// ---------- asap_sketchlib: Vector2D + RegularPath ----------
pub struct CmsHeapLibVector2dRegular {
    inner: CMSHeap<Vector2D<i32>, RegularPath>,
    rows: usize,
    cols: usize,
}

pub fn build_cms_heap_lib_vector2d_regular(
    config: &ParamSet,
) -> Result<CmsHeapLibVector2dRegular, BuildError> {
    let p: CmsHeapParams = config.parse()?;
    require_positive("asap CMSHeap Vector2D RegularPath", "rows", p.rows)?;
    require_positive("asap CMSHeap Vector2D RegularPath", "cols", p.cols)?;
    Ok(CmsHeapLibVector2dRegular {
        inner: CMSHeap::<Vector2D<i32>, RegularPath>::new(p.rows, p.cols, CMS_HEAP_TOP_K),
        rows: p.rows,
        cols: p.cols,
    })
}

pub fn memory_cms_heap_lib_vector2d_regular(sketch: &CmsHeapLibVector2dRegular) -> usize {
    sketch.rows * sketch.cols * std::mem::size_of::<i32>()
}

// ---------- insert / insert_step (FastPath) ----------

pub fn insert_cms_heap_lib_vector2d_fast(
    params: &ParamSet,
    items: Rc<Vec<i64>>,
    passes: usize,
) -> Result<Vec<Pass>, BuildError> {
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        let mut sketch = build_cms_heap_lib_vector2d_fast(params)?;
        let items = items.clone();
        out.push(Box::new(move || {
            for v in items.iter() {
                sketch.inner.insert(&DataInput::I64(*v));
            }
            memory_cms_heap_lib_vector2d_fast(&sketch)
        }) as Pass);
    }
    Ok(out)
}

pub fn insert_step_cms_heap_lib_vector2d_fast(
    params: &ParamSet,
    items: Rc<Vec<i64>>,
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
                sketch.inner.insert(&DataInput::I64(*v));
            }),
            footprint: Box::new(move || memory_cms_heap_lib_vector2d_fast(&read.borrow())),
        });
    }
    Ok(out)
}

// ---------- insert / insert_step (RegularPath) ----------

pub fn insert_cms_heap_lib_vector2d_regular(
    params: &ParamSet,
    items: Rc<Vec<i64>>,
    passes: usize,
) -> Result<Vec<Pass>, BuildError> {
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        let mut sketch = build_cms_heap_lib_vector2d_regular(params)?;
        let items = items.clone();
        out.push(Box::new(move || {
            for v in items.iter() {
                sketch.inner.insert(&DataInput::I64(*v));
            }
            memory_cms_heap_lib_vector2d_regular(&sketch)
        }) as Pass);
    }
    Ok(out)
}

pub fn insert_step_cms_heap_lib_vector2d_regular(
    params: &ParamSet,
    items: Rc<Vec<i64>>,
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
                sketch.inner.insert(&DataInput::I64(*v));
            }),
            footprint: Box::new(move || memory_cms_heap_lib_vector2d_regular(&read.borrow())),
        });
    }
    Ok(out)
}

// ---------- query: per-key estimate (Frequency capability) ----------

pub fn query_cms_heap_lib_vector2d_fast_estimate(
    params: &ParamSet,
    items: Rc<Vec<i64>>,
    probes: Rc<Vec<i64>>,
    passes: usize,
) -> Result<Vec<QueryPass<u64>>, BuildError> {
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        let mut sketch = build_cms_heap_lib_vector2d_fast(params)?;
        for v in items.iter() {
            sketch.inner.insert(&DataInput::I64(*v));
        }
        let probes = probes.clone();
        out.push(Box::new(move || {
            let mut answers = Vec::with_capacity(probes.len());
            for p in probes.iter() {
                answers.push(sketch.inner.estimate(&DataInput::I64(*p)) as u64);
            }
            let footprint = memory_cms_heap_lib_vector2d_fast(&sketch);
            (answers, footprint)
        }) as QueryPass<u64>);
    }
    Ok(out)
}

pub fn query_cms_heap_lib_vector2d_regular_estimate(
    params: &ParamSet,
    items: Rc<Vec<i64>>,
    probes: Rc<Vec<i64>>,
    passes: usize,
) -> Result<Vec<QueryPass<u64>>, BuildError> {
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        let mut sketch = build_cms_heap_lib_vector2d_regular(params)?;
        for v in items.iter() {
            sketch.inner.insert(&DataInput::I64(*v));
        }
        let probes = probes.clone();
        out.push(Box::new(move || {
            let mut answers = Vec::with_capacity(probes.len());
            for p in probes.iter() {
                answers.push(sketch.inner.estimate(&DataInput::I64(*p)) as u64);
            }
            let footprint = memory_cms_heap_lib_vector2d_regular(&sketch);
            (answers, footprint)
        }) as QueryPass<u64>);
    }
    Ok(out)
}

// ---------- query: heap dump (TopK capability) ----------
//
// One `()` probe, one answer: the whole ranked heap, read off in one call.
// `TopkGT`'s `probes()` hands back `vec![()]`, so `probes` here is always
// length 1 — there is nothing per-item to loop over the way the estimate
// queries do.

pub fn query_cms_heap_lib_vector2d_fast_topk(
    params: &ParamSet,
    items: Rc<Vec<i64>>,
    _probes: Rc<Vec<()>>,
    passes: usize,
) -> Result<Vec<QueryPass<TopkAnswer>>, BuildError> {
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        let mut sketch = build_cms_heap_lib_vector2d_fast(params)?;
        for v in items.iter() {
            sketch.inner.insert(&DataInput::I64(*v));
        }
        out.push(Box::new(move || {
            let ranked: TopkAnswer = sketch
                .inner
                .heap()
                .heap()
                .iter()
                .map(|item| {
                    (
                        heap_key_to_i64(&heap_item_to_sketch_input(&item.key)),
                        item.count as u64,
                    )
                })
                .collect();
            let footprint = memory_cms_heap_lib_vector2d_fast(&sketch);
            (vec![ranked], footprint)
        }) as QueryPass<TopkAnswer>);
    }
    Ok(out)
}

pub fn query_cms_heap_lib_vector2d_regular_topk(
    params: &ParamSet,
    items: Rc<Vec<i64>>,
    _probes: Rc<Vec<()>>,
    passes: usize,
) -> Result<Vec<QueryPass<TopkAnswer>>, BuildError> {
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        let mut sketch = build_cms_heap_lib_vector2d_regular(params)?;
        for v in items.iter() {
            sketch.inner.insert(&DataInput::I64(*v));
        }
        out.push(Box::new(move || {
            let ranked: TopkAnswer = sketch
                .inner
                .heap()
                .heap()
                .iter()
                .map(|item| {
                    (
                        heap_key_to_i64(&heap_item_to_sketch_input(&item.key)),
                        item.count as u64,
                    )
                })
                .collect();
            let footprint = memory_cms_heap_lib_vector2d_regular(&sketch);
            (vec![ranked], footprint)
        }) as QueryPass<TopkAnswer>);
    }
    Ok(out)
}

// ---------- merge (shared by both capability rows per backend) ----------
//
// Heavier than plain CMS's merge: after folding the counter matrices,
// `CMSHeap::merge` re-estimates every heap candidate from both sides (up to
// 2 * CMS_HEAP_TOP_K `estimate()` calls) to rebuild the merged heap. Not a
// pure cell-fold the way plain CMS merge is — see the registry description.

pub fn merge_cms_heap_lib_vector2d_fast(
    params: &ParamSet,
    items: Rc<Vec<i64>>,
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

pub fn merge_step_cms_heap_lib_vector2d_fast(
    params: &ParamSet,
    items: Rc<Vec<i64>>,
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
fn cms_heap_lib_vector2d_fast_shards(
    params: &ParamSet,
    items: &[i64],
    shards: usize,
) -> Result<(CmsHeapLibVector2dFast, Vec<CmsHeapLibVector2dFast>), BuildError> {
    let mut parts: Vec<CmsHeapLibVector2dFast> = Vec::new();
    for shard in crate::wrappers::partition(items, shards) {
        let mut sketch = build_cms_heap_lib_vector2d_fast(params)?;
        for v in shard {
            sketch.inner.insert(&DataInput::I64(*v));
        }
        parts.push(sketch);
    }
    let rest = parts.split_off(1);
    Ok((
        parts.pop().expect("a split of the stream is never empty"),
        rest,
    ))
}

pub fn merge_cms_heap_lib_vector2d_regular(
    params: &ParamSet,
    items: Rc<Vec<i64>>,
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

pub fn merge_step_cms_heap_lib_vector2d_regular(
    params: &ParamSet,
    items: Rc<Vec<i64>>,
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
fn cms_heap_lib_vector2d_regular_shards(
    params: &ParamSet,
    items: &[i64],
    shards: usize,
) -> Result<(CmsHeapLibVector2dRegular, Vec<CmsHeapLibVector2dRegular>), BuildError> {
    let mut parts: Vec<CmsHeapLibVector2dRegular> = Vec::new();
    for shard in crate::wrappers::partition(items, shards) {
        let mut sketch = build_cms_heap_lib_vector2d_regular(params)?;
        for v in shard {
            sketch.inner.insert(&DataInput::I64(*v));
        }
        parts.push(sketch);
    }
    let rest = parts.split_off(1);
    Ok((
        parts.pop().expect("a split of the stream is never empty"),
        rest,
    ))
}
