use super::*;
use crate::params::ParamSet;
use crate::wrappers::partition;
use crate::wrappers::{BuildError, Pass, QueryPass, Shared, StepPass};
use asap_sketchlib::input::HHItem;
use asap_sketchlib::UnivMon;
use std::cell::RefCell;
use std::rc::Rc;

pub struct UnivMonLib {
    pub(super) inner: UnivMon,
    params: UnivMonParams,
}

pub fn build_univmon_lib(config: &ParamSet) -> Result<UnivMonLib, BuildError> {
    let p: UnivMonParams = config.parse()?;
    check_shape(&p, "univmon")?;
    Ok(UnivMonLib {
        inner: UnivMon::init_univmon(p.heap_size, p.sketch_row, p.sketch_col, p.layer_size),
        params: p,
    })
}

impl UnivMonLib {
    #[inline]
    pub(super) fn feed<K: UnivMonKey>(&mut self, record: &Record<K>) {
        self.inner.insert(&record.0.data_input(), record.1);
    }

    #[inline]
    pub fn estimate_cardinality(&self) -> f64 {
        self.inner.calc_card()
    }

    #[inline]
    pub fn estimate_l1_norm(&self) -> f64 {
        self.inner.calc_l1()
    }

    #[inline]
    pub fn estimate_l2_norm(&self) -> f64 {
        self.inner.calc_l2()
    }

    #[inline]
    pub fn estimate_entropy(&self) -> f64 {
        self.inner.calc_entropy()
    }
}

pub fn memory_univmon_lib(sketch: &UnivMonLib) -> usize {
    let p = &sketch.params;
    let counters = (p.sketch_row * p.sketch_col + p.sketch_row) * std::mem::size_of::<i64>();
    let heap = p.heap_size * std::mem::size_of::<HHItem>();
    p.layer_size * (counters + heap)
}

pub fn insert_univmon_lib<K: UnivMonKey>(
    params: &ParamSet,
    items: Rc<Vec<Record<K>>>,
    passes: usize,
) -> Result<Vec<Pass>, BuildError> {
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        let mut sketch = build_univmon_lib(params)?;
        let items = items.clone();
        out.push(Box::new(move || {
            for record in items.iter() {
                sketch.feed(record);
            }
            memory_univmon_lib(&sketch)
        }) as Pass);
    }
    Ok(out)
}

pub fn insert_step_univmon_lib<K: UnivMonKey>(
    params: &ParamSet,
    items: Rc<Vec<Record<K>>>,
    passes: usize,
) -> Result<Vec<StepPass>, BuildError> {
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        let sketch: Shared<_> = Rc::new(RefCell::new(build_univmon_lib(params)?));
        let (driven, read) = (sketch.clone(), sketch);
        let stream = items.clone();
        out.push(StepPass {
            steps: items.len(),
            step: Box::new(move |i| {
                driven.borrow_mut().feed(&stream[i]);
            }),
            footprint: Box::new(move || memory_univmon_lib(&read.borrow())),
        });
    }
    Ok(out)
}

fn asked_univmon_lib<K: UnivMonKey>(
    params: &ParamSet,
    items: Rc<Vec<Record<K>>>,
    probes: Rc<Vec<()>>,
    shards: usize,
    passes: usize,
    estimate: fn(&UnivMonLib) -> f64,
) -> Result<Vec<QueryPass<f64>>, BuildError> {
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        // Built, fed and folded here: the closure below asks, and only asks.
        let (mut sketch, rest) = univmon_lib_shards(params, &items, shards)?;
        for other in rest.iter() {
            sketch.inner.merge(&other.inner);
        }
        let probes = probes.clone();
        out.push(Box::new(move || {
            let answers: Vec<f64> = probes.iter().map(|()| estimate(&sketch)).collect();
            let footprint = memory_univmon_lib(&sketch);
            (answers, footprint)
        }) as QueryPass<f64>);
    }
    Ok(out)
}

pub fn query_univmon_lib_cardinality<K: UnivMonKey>(
    params: &ParamSet,
    items: Rc<Vec<Record<K>>>,
    probes: Rc<Vec<()>>,
    passes: usize,
) -> Result<Vec<QueryPass<f64>>, BuildError> {
    asked_univmon_lib(
        params,
        items,
        probes,
        1,
        passes,
        UnivMonLib::estimate_cardinality,
    )
}

/// The query, asked of the sketch a fold over `shards` shards leaves. One
/// shard is the plain query.
pub fn merge_query_univmon_lib_cardinality<K: UnivMonKey>(
    params: &ParamSet,
    items: Rc<Vec<Record<K>>>,
    probes: Rc<Vec<()>>,
    shards: usize,
    passes: usize,
) -> Result<Vec<QueryPass<f64>>, BuildError> {
    asked_univmon_lib(
        params,
        items,
        probes,
        shards,
        passes,
        UnivMonLib::estimate_cardinality,
    )
}

pub fn query_univmon_lib_l1_norm<K: UnivMonKey>(
    params: &ParamSet,
    items: Rc<Vec<Record<K>>>,
    probes: Rc<Vec<()>>,
    passes: usize,
) -> Result<Vec<QueryPass<f64>>, BuildError> {
    asked_univmon_lib(
        params,
        items,
        probes,
        1,
        passes,
        UnivMonLib::estimate_l1_norm,
    )
}

/// The query, asked of the sketch a fold over `shards` shards leaves. One
/// shard is the plain query.
pub fn merge_query_univmon_lib_l1_norm<K: UnivMonKey>(
    params: &ParamSet,
    items: Rc<Vec<Record<K>>>,
    probes: Rc<Vec<()>>,
    shards: usize,
    passes: usize,
) -> Result<Vec<QueryPass<f64>>, BuildError> {
    asked_univmon_lib(
        params,
        items,
        probes,
        shards,
        passes,
        UnivMonLib::estimate_l1_norm,
    )
}

pub fn query_univmon_lib_l2_norm<K: UnivMonKey>(
    params: &ParamSet,
    items: Rc<Vec<Record<K>>>,
    probes: Rc<Vec<()>>,
    passes: usize,
) -> Result<Vec<QueryPass<f64>>, BuildError> {
    asked_univmon_lib(
        params,
        items,
        probes,
        1,
        passes,
        UnivMonLib::estimate_l2_norm,
    )
}

/// The query, asked of the sketch a fold over `shards` shards leaves. One
/// shard is the plain query.
pub fn merge_query_univmon_lib_l2_norm<K: UnivMonKey>(
    params: &ParamSet,
    items: Rc<Vec<Record<K>>>,
    probes: Rc<Vec<()>>,
    shards: usize,
    passes: usize,
) -> Result<Vec<QueryPass<f64>>, BuildError> {
    asked_univmon_lib(
        params,
        items,
        probes,
        shards,
        passes,
        UnivMonLib::estimate_l2_norm,
    )
}

pub fn query_univmon_lib_entropy<K: UnivMonKey>(
    params: &ParamSet,
    items: Rc<Vec<Record<K>>>,
    probes: Rc<Vec<()>>,
    passes: usize,
) -> Result<Vec<QueryPass<f64>>, BuildError> {
    asked_univmon_lib(
        params,
        items,
        probes,
        1,
        passes,
        UnivMonLib::estimate_entropy,
    )
}

/// The query, asked of the sketch a fold over `shards` shards leaves. One
/// shard is the plain query.
pub fn merge_query_univmon_lib_entropy<K: UnivMonKey>(
    params: &ParamSet,
    items: Rc<Vec<Record<K>>>,
    probes: Rc<Vec<()>>,
    shards: usize,
    passes: usize,
) -> Result<Vec<QueryPass<f64>>, BuildError> {
    asked_univmon_lib(
        params,
        items,
        probes,
        shards,
        passes,
        UnivMonLib::estimate_entropy,
    )
}

pub fn merge_univmon_lib<K: UnivMonKey>(
    params: &ParamSet,
    items: Rc<Vec<Record<K>>>,
    shards: usize,
    passes: usize,
) -> Result<Vec<Pass>, BuildError> {
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        let (mut acc, rest) = univmon_lib_shards(params, &items, shards)?;
        out.push(Box::new(move || {
            for other in rest.iter() {
                acc.inner.merge(&other.inner);
            }
            memory_univmon_lib(&acc)
        }) as Pass);
    }
    Ok(out)
}

pub fn merge_step_univmon_lib<K: UnivMonKey>(
    params: &ParamSet,
    items: Rc<Vec<Record<K>>>,
    shards: usize,
    passes: usize,
) -> Result<Vec<StepPass>, BuildError> {
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        let (acc, rest) = univmon_lib_shards(params, &items, shards)?;
        let acc: Shared<_> = Rc::new(RefCell::new(acc));
        let (driven, read) = (acc.clone(), acc);
        out.push(StepPass {
            steps: rest.len(),
            step: Box::new(move |i| {
                driven.borrow_mut().inner.merge(&rest[i].inner);
            }),
            footprint: Box::new(move || memory_univmon_lib(&read.borrow())),
        });
    }
    Ok(out)
}

#[allow(clippy::type_complexity)]
fn univmon_lib_shards<K: UnivMonKey>(
    params: &ParamSet,
    items: &[Record<K>],
    shards: usize,
) -> Result<(UnivMonLib, Vec<UnivMonLib>), BuildError> {
    let mut parts: Vec<UnivMonLib> = Vec::new();
    for shard in partition(items, shards) {
        let mut sketch = build_univmon_lib(params)?;
        for record in shard {
            sketch.feed(record);
        }
        parts.push(sketch);
    }
    let rest = parts.split_off(1);
    Ok((
        parts.pop().expect("a split of the stream is never empty"),
        rest,
    ))
}

// ---------- top-k (single value column, each item weight 1) ----------
//
// Fed the same one-column stream the CMS/CS heap rows are, so `TopkGT` grades
// all three alike. The answer is layer 0's heap -- the layer every item
// reaches -- cut to the `CMS_HEAP_TOP_K` heaviest.

use crate::wrappers::cms_heap::sketchlib::{TopkAnswer, CMS_HEAP_TOP_K};
use asap_sketchlib::heap_item_to_sketch_input;

fn univmon_lib_item_shards<T: FrequencyValue>(
    params: &ParamSet,
    items: &[T],
    shards: usize,
) -> Result<(UnivMonLib, Vec<UnivMonLib>), BuildError> {
    let mut parts = Vec::new();
    for shard in partition(items, shards) {
        let mut sketch = build_univmon_lib(params)?;
        for v in shard {
            sketch.inner.insert(&v.data_input(), 1);
        }
        parts.push(sketch);
    }
    let rest = parts.split_off(1);
    Ok((
        parts.pop().expect("a split of the stream is never empty"),
        rest,
    ))
}

pub fn insert_univmon_lib_items<T: FrequencyValue>(
    params: &ParamSet,
    items: Rc<Vec<T>>,
    passes: usize,
) -> Result<Vec<Pass>, BuildError> {
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        let mut sketch = build_univmon_lib(params)?;
        let items = items.clone();
        out.push(Box::new(move || {
            for v in items.iter() {
                sketch.inner.insert(&v.data_input(), 1);
            }
            memory_univmon_lib(&sketch)
        }) as Pass);
    }
    Ok(out)
}

pub fn insert_step_univmon_lib_items<T: FrequencyValue>(
    params: &ParamSet,
    items: Rc<Vec<T>>,
    passes: usize,
) -> Result<Vec<StepPass>, BuildError> {
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        let sketch: Shared<_> = Rc::new(RefCell::new(build_univmon_lib(params)?));
        let (driven, read) = (sketch.clone(), sketch);
        let stream = items.clone();
        out.push(StepPass {
            steps: items.len(),
            step: Box::new(move |i| driven.borrow_mut().inner.insert(&stream[i].data_input(), 1)),
            footprint: Box::new(move || memory_univmon_lib(&read.borrow())),
        });
    }
    Ok(out)
}

pub fn query_univmon_lib_topk<T: FrequencyValue>(
    params: &ParamSet,
    items: Rc<Vec<T>>,
    probes: Rc<Vec<()>>,
    passes: usize,
) -> Result<Vec<QueryPass<TopkAnswer<T>>>, BuildError> {
    merge_query_univmon_lib_topk(params, items, probes, 1, passes)
}

pub fn merge_query_univmon_lib_topk<T: FrequencyValue>(
    params: &ParamSet,
    items: Rc<Vec<T>>,
    probes: Rc<Vec<()>>,
    shards: usize,
    passes: usize,
) -> Result<Vec<QueryPass<TopkAnswer<T>>>, BuildError> {
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        let (mut sketch, rest) = univmon_lib_item_shards(params, &items, shards)?;
        for other in rest.iter() {
            sketch.inner.merge(&other.inner);
        }
        let probes = probes.clone();
        out.push(Box::new(move || {
            let mut answers = Vec::with_capacity(probes.len());
            for _p in probes.iter() {
                let mut heap: Vec<_> = sketch.inner.hh_layers[0].heap().iter().collect();
                heap.sort_unstable_by_key(|item| std::cmp::Reverse(item.count));
                let ranked: TopkAnswer<T> = heap
                    .into_iter()
                    .take(CMS_HEAP_TOP_K)
                    .map(|item| {
                        (
                            T::from_data_input(&heap_item_to_sketch_input(&item.key)),
                            item.count.max(0) as u64,
                        )
                    })
                    .collect();
                answers.push(ranked);
            }
            let footprint = memory_univmon_lib(&sketch);
            (answers, footprint)
        }) as QueryPass<TopkAnswer<T>>);
    }
    Ok(out)
}

pub fn merge_univmon_lib_items<T: FrequencyValue>(
    params: &ParamSet,
    items: Rc<Vec<T>>,
    shards: usize,
    passes: usize,
) -> Result<Vec<Pass>, BuildError> {
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        let (mut acc, rest) = univmon_lib_item_shards(params, &items, shards)?;
        out.push(Box::new(move || {
            for other in rest.iter() {
                acc.inner.merge(&other.inner);
            }
            memory_univmon_lib(&acc)
        }) as Pass);
    }
    Ok(out)
}

pub fn merge_step_univmon_lib_items<T: FrequencyValue>(
    params: &ParamSet,
    items: Rc<Vec<T>>,
    shards: usize,
    passes: usize,
) -> Result<Vec<StepPass>, BuildError> {
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        let (acc, rest) = univmon_lib_item_shards(params, &items, shards)?;
        let acc: Shared<_> = Rc::new(RefCell::new(acc));
        let (driven, read) = (acc.clone(), acc);
        out.push(StepPass {
            steps: rest.len(),
            step: Box::new(move |i| driven.borrow_mut().inner.merge(&rest[i].inner)),
            footprint: Box::new(move || memory_univmon_lib(&read.borrow())),
        });
    }
    Ok(out)
}
