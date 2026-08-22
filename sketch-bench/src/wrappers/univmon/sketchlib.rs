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
    passes: usize,
    estimate: fn(&UnivMonLib) -> f64,
) -> Result<Vec<QueryPass<f64>>, BuildError> {
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        let mut sketch = build_univmon_lib(params)?;
        for record in items.iter() {
            sketch.feed(record);
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
    asked_univmon_lib(params, items, probes, passes, UnivMonLib::estimate_l1_norm)
}

pub fn query_univmon_lib_l2_norm<K: UnivMonKey>(
    params: &ParamSet,
    items: Rc<Vec<Record<K>>>,
    probes: Rc<Vec<()>>,
    passes: usize,
) -> Result<Vec<QueryPass<f64>>, BuildError> {
    asked_univmon_lib(params, items, probes, passes, UnivMonLib::estimate_l2_norm)
}

pub fn query_univmon_lib_entropy<K: UnivMonKey>(
    params: &ParamSet,
    items: Rc<Vec<Record<K>>>,
    probes: Rc<Vec<()>>,
    passes: usize,
) -> Result<Vec<QueryPass<f64>>, BuildError> {
    asked_univmon_lib(params, items, probes, passes, UnivMonLib::estimate_entropy)
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
