use super::*;
use crate::params::ParamSet;
use crate::wrappers::cs::dims_to_err;
use crate::wrappers::{partition, require_resolved_shape};
use crate::wrappers::{BuildError, Pass, QueryPass, Shared, StepPass};
use sketch_oxide::common::Mergeable;
use sketch_oxide::universal::UnivMon;
use std::cell::RefCell;
use std::rc::Rc;

const LAYER_HEAP: usize = 1000;

pub struct UnivMonOxide {
    pub(super) inner: UnivMon,
}

pub fn build_univmon_oxide(config: &ParamSet) -> Result<UnivMonOxide, BuildError> {
    let p: UnivMonParams = config.parse()?;
    check_shape(&p, "univmon")?;
    if p.heap_size != LAYER_HEAP {
        return Err(BuildError(format!(
            "oxide UnivMon sizes layer i's heavy-hitter heap at max(ceil({LAYER_HEAP}/(i+1)), 10) \
             and takes no heap parameter, so it builds at heap_size={LAYER_HEAP} only, \
             not {}",
            p.heap_size
        )));
    }
    let max_stream_size = 1u64.checked_shl(p.layer_size as u32).ok_or_else(|| {
        BuildError(format!(
            "oxide UnivMon derives its layer count from a stream bound of 2^layer_size, \
             which layer_size={} overflows",
            p.layer_size
        ))
    })?;
    let (epsilon, delta) = dims_to_err(p.sketch_row, p.sketch_col);
    let inner = UnivMon::new(max_stream_size, epsilon, delta).map_err(|e| {
        BuildError(format!(
            "oxide UnivMon rejected ε={epsilon} δ={delta}: {e:?}"
        ))
    })?;
    if inner.num_layers() != p.layer_size {
        return Err(BuildError(format!(
            "oxide UnivMon floors its layer count at 3 and takes it as ceil(log2(max_stream)), \
             so layer_size={} builds {} layers",
            p.layer_size,
            inner.num_layers()
        )));
    }
    let layer = sketch_oxide::frequency::CountSketch::new(epsilon, delta).map_err(|e| {
        BuildError(format!(
            "oxide CountSketch rejected ε={epsilon} δ={delta}: {e:?}"
        ))
    })?;
    require_resolved_shape(
        "oxide UnivMon layer",
        (layer.depth(), layer.width()),
        (p.sketch_row, p.sketch_col),
    )?;
    Ok(UnivMonOxide { inner })
}

impl UnivMonOxide {
    #[inline]
    pub(super) fn feed<K: UnivMonKey>(&mut self, record: &Record<K>) {
        self.inner
            .update(record.0.key_bytes().as_ref(), record.1 as f64)
            .expect("the stream was proved non-negative before this pass was primed");
    }

    #[inline]
    pub fn estimate_l1_norm(&self) -> f64 {
        self.inner.estimate_l1()
    }

    #[inline]
    pub fn estimate_l2_norm(&self) -> f64 {
        self.inner.estimate_l2()
    }

    #[inline]
    pub fn estimate_entropy(&self) -> f64 {
        self.inner.estimate_entropy()
    }
}

pub fn memory_univmon_oxide(sketch: &UnivMonOxide) -> usize {
    sketch.inner.stats().total_memory as usize
}

fn require_non_negative<K: UnivMonKey>(items: &[Record<K>]) -> Result<(), BuildError> {
    match items.iter().find(|(_, value)| *value < 0) {
        Some((key, value)) => Err(BuildError(format!(
            "oxide UnivMon refuses a negative weight, and key {key:?} carries {value}"
        ))),
        None => Ok(()),
    }
}

pub fn insert_univmon_oxide<K: UnivMonKey>(
    params: &ParamSet,
    items: Rc<Vec<Record<K>>>,
    passes: usize,
) -> Result<Vec<Pass>, BuildError> {
    require_non_negative(&items)?;
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        let mut sketch = build_univmon_oxide(params)?;
        let items = items.clone();
        out.push(Box::new(move || {
            for record in items.iter() {
                sketch.feed(record);
            }
            memory_univmon_oxide(&sketch)
        }) as Pass);
    }
    Ok(out)
}

pub fn insert_step_univmon_oxide<K: UnivMonKey>(
    params: &ParamSet,
    items: Rc<Vec<Record<K>>>,
    passes: usize,
) -> Result<Vec<StepPass>, BuildError> {
    require_non_negative(&items)?;
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        let sketch: Shared<_> = Rc::new(RefCell::new(build_univmon_oxide(params)?));
        let (driven, read) = (sketch.clone(), sketch);
        let stream = items.clone();
        out.push(StepPass {
            steps: items.len(),
            step: Box::new(move |i| {
                driven.borrow_mut().feed(&stream[i]);
            }),
            footprint: Box::new(move || memory_univmon_oxide(&read.borrow())),
        });
    }
    Ok(out)
}

fn asked_univmon_oxide<K: UnivMonKey>(
    params: &ParamSet,
    items: Rc<Vec<Record<K>>>,
    probes: Rc<Vec<()>>,
    shards: usize,
    passes: usize,
    estimate: fn(&UnivMonOxide) -> f64,
) -> Result<Vec<QueryPass<f64>>, BuildError> {
    require_non_negative(&items)?;
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        // Built, fed and folded here: the closure below asks, and only asks.
        let (mut sketch, rest) = univmon_oxide_shards(params, &items, shards)?;
        for other in rest.iter() {
            sketch
                .inner
                .merge(&other.inner)
                .expect("both operands built from one ParamSet, so their layers match");
        }
        let probes = probes.clone();
        out.push(Box::new(move || {
            let answers: Vec<f64> = probes.iter().map(|()| estimate(&sketch)).collect();
            let footprint = memory_univmon_oxide(&sketch);
            (answers, footprint)
        }) as QueryPass<f64>);
    }
    Ok(out)
}

pub fn query_univmon_oxide_l1_norm<K: UnivMonKey>(
    params: &ParamSet,
    items: Rc<Vec<Record<K>>>,
    probes: Rc<Vec<()>>,
    passes: usize,
) -> Result<Vec<QueryPass<f64>>, BuildError> {
    asked_univmon_oxide(
        params,
        items,
        probes,
        1,
        passes,
        UnivMonOxide::estimate_l1_norm,
    )
}

/// The query, asked of the sketch a fold over `shards` shards leaves. One
/// shard is the plain query.
pub fn merge_query_univmon_oxide_l1_norm<K: UnivMonKey>(
    params: &ParamSet,
    items: Rc<Vec<Record<K>>>,
    probes: Rc<Vec<()>>,
    shards: usize,
    passes: usize,
) -> Result<Vec<QueryPass<f64>>, BuildError> {
    asked_univmon_oxide(
        params,
        items,
        probes,
        shards,
        passes,
        UnivMonOxide::estimate_l1_norm,
    )
}

pub fn query_univmon_oxide_l2_norm<K: UnivMonKey>(
    params: &ParamSet,
    items: Rc<Vec<Record<K>>>,
    probes: Rc<Vec<()>>,
    passes: usize,
) -> Result<Vec<QueryPass<f64>>, BuildError> {
    asked_univmon_oxide(
        params,
        items,
        probes,
        1,
        passes,
        UnivMonOxide::estimate_l2_norm,
    )
}

/// The query, asked of the sketch a fold over `shards` shards leaves. One
/// shard is the plain query.
pub fn merge_query_univmon_oxide_l2_norm<K: UnivMonKey>(
    params: &ParamSet,
    items: Rc<Vec<Record<K>>>,
    probes: Rc<Vec<()>>,
    shards: usize,
    passes: usize,
) -> Result<Vec<QueryPass<f64>>, BuildError> {
    asked_univmon_oxide(
        params,
        items,
        probes,
        shards,
        passes,
        UnivMonOxide::estimate_l2_norm,
    )
}

pub fn query_univmon_oxide_entropy<K: UnivMonKey>(
    params: &ParamSet,
    items: Rc<Vec<Record<K>>>,
    probes: Rc<Vec<()>>,
    passes: usize,
) -> Result<Vec<QueryPass<f64>>, BuildError> {
    asked_univmon_oxide(
        params,
        items,
        probes,
        1,
        passes,
        UnivMonOxide::estimate_entropy,
    )
}

/// The query, asked of the sketch a fold over `shards` shards leaves. One
/// shard is the plain query.
pub fn merge_query_univmon_oxide_entropy<K: UnivMonKey>(
    params: &ParamSet,
    items: Rc<Vec<Record<K>>>,
    probes: Rc<Vec<()>>,
    shards: usize,
    passes: usize,
) -> Result<Vec<QueryPass<f64>>, BuildError> {
    asked_univmon_oxide(
        params,
        items,
        probes,
        shards,
        passes,
        UnivMonOxide::estimate_entropy,
    )
}

pub fn merge_univmon_oxide<K: UnivMonKey>(
    params: &ParamSet,
    items: Rc<Vec<Record<K>>>,
    shards: usize,
    passes: usize,
) -> Result<Vec<Pass>, BuildError> {
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        let (mut acc, rest) = univmon_oxide_shards(params, &items, shards)?;
        out.push(Box::new(move || {
            for other in rest.iter() {
                acc.inner
                    .merge(&other.inner)
                    .expect("both operands built from one ParamSet, so their layers match");
            }
            memory_univmon_oxide(&acc)
        }) as Pass);
    }
    Ok(out)
}

pub fn merge_step_univmon_oxide<K: UnivMonKey>(
    params: &ParamSet,
    items: Rc<Vec<Record<K>>>,
    shards: usize,
    passes: usize,
) -> Result<Vec<StepPass>, BuildError> {
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        let (acc, rest) = univmon_oxide_shards(params, &items, shards)?;
        let acc: Shared<_> = Rc::new(RefCell::new(acc));
        let (driven, read) = (acc.clone(), acc);
        out.push(StepPass {
            steps: rest.len(),
            step: Box::new(move |i| {
                driven
                    .borrow_mut()
                    .inner
                    .merge(&rest[i].inner)
                    .expect("both operands built from one ParamSet, so their layers match");
            }),
            footprint: Box::new(move || memory_univmon_oxide(&read.borrow())),
        });
    }
    Ok(out)
}

#[allow(clippy::type_complexity)]
fn univmon_oxide_shards<K: UnivMonKey>(
    params: &ParamSet,
    items: &[Record<K>],
    shards: usize,
) -> Result<(UnivMonOxide, Vec<UnivMonOxide>), BuildError> {
    require_non_negative(items)?;
    let mut parts: Vec<UnivMonOxide> = Vec::new();
    for shard in partition(items, shards) {
        let mut sketch = build_univmon_oxide(params)?;
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
