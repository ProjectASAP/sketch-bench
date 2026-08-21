//! The `asap_sketchlib` implementation.
//!
//! Grouped under `wrappers/hydra_univmon/` with the other implementations of
//! this algorithm; how each is driven lives beside it.

use super::*;
use crate::params::ParamSet;
use crate::wrappers::hll::CardinalityValue;
use crate::wrappers::partition;
use crate::wrappers::{BuildError, Pass, QueryPass, Shared, StepPass};
use asap_sketchlib::input::{HHItem, HydraCounter, HydraQuery};
use asap_sketchlib::{Hydra, UnivMon};
use std::cell::RefCell;
use std::rc::Rc;

pub struct HydraUnivmon {
    pub(super) inner: Hydra,
    params: HydraUnivmonParams,
}

pub fn build_hydra_univmon(config: &ParamSet) -> Result<HydraUnivmon, BuildError> {
    let p: HydraUnivmonParams = config.parse()?;
    check_grid(p.rows, p.cols, "hydra-univmon")?;
    for (name, v) in [
        ("cell_heap_size", p.cell_heap_size),
        ("cell_sketch_row", p.cell_sketch_row),
        ("cell_sketch_col", p.cell_sketch_col),
        ("cell_layer_size", p.cell_layer_size),
    ] {
        if v == 0 {
            return Err(BuildError(format!("hydra-univmon: {name} must be > 0")));
        }
    }
    let cell = HydraCounter::UNIVERSAL(UnivMon::init_univmon(
        p.cell_heap_size,
        p.cell_sketch_row,
        p.cell_sketch_col,
        p.cell_layer_size,
    ));
    Ok(HydraUnivmon {
        inner: Hydra::with_dimensions(p.rows, p.cols, cell),
        params: p,
    })
}

impl HydraUnivmon {
    #[inline]
    pub fn estimate_subpop_cardinality(&self, labels: &[&str]) -> f64 {
        self.inner
            .query_key(labels.to_vec(), &HydraQuery::Cardinality)
    }

    #[inline]
    pub fn estimate_subpop_l1_norm(&self, labels: &[&str]) -> f64 {
        self.inner.query_key(labels.to_vec(), &HydraQuery::L1Norm)
    }

    #[inline]
    pub fn estimate_subpop_l2_norm(&self, labels: &[&str]) -> f64 {
        self.inner.query_key(labels.to_vec(), &HydraQuery::L2Norm)
    }

    #[inline]
    pub fn estimate_subpop_entropy(&self, labels: &[&str]) -> f64 {
        self.inner.query_key(labels.to_vec(), &HydraQuery::Entropy)
    }
}

pub fn memory_hydra_univmon(sketch: &HydraUnivmon) -> usize {
    let p = &sketch.params;
    let counters =
        (p.cell_sketch_row * p.cell_sketch_col + p.cell_sketch_row) * std::mem::size_of::<i64>();
    let heap = p.cell_heap_size * std::mem::size_of::<HHItem>();
    p.rows * p.cols * p.cell_layer_size * (counters + heap) + grid_overhead_bytes(p.rows, p.cols)
}

pub fn insert_hydra_univmon<V: CardinalityValue>(
    params: &ParamSet,
    items: Rc<Vec<(String, V)>>,
    passes: usize,
) -> Result<Vec<Pass>, BuildError> {
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        // Built here, so calling the closure is the insert and nothing else.
        let mut sketch = build_hydra_univmon(params)?;
        let items = items.clone();
        out.push(Box::new(move || {
            for v in items.iter() {
                sketch.inner.update(&v.0, &v.1.data_input(), None);
            }
            memory_hydra_univmon(&sketch)
        }) as Pass);
    }
    Ok(out)
}

pub fn insert_step_hydra_univmon<V: CardinalityValue>(
    params: &ParamSet,
    items: Rc<Vec<(String, V)>>,
    passes: usize,
) -> Result<Vec<StepPass>, BuildError> {
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        let sketch: Shared<_> = Rc::new(RefCell::new(build_hydra_univmon(params)?));
        let (driven, read) = (sketch.clone(), sketch);
        let stream = items.clone();
        out.push(StepPass {
            steps: items.len(),
            step: Box::new(move |i| {
                let sketch = &mut *driven.borrow_mut();
                let v = &stream[i];
                sketch.inner.update(&v.0, &v.1.data_input(), None);
            }),
            footprint: Box::new(move || memory_hydra_univmon(&read.borrow())),
        });
    }
    Ok(out)
}

fn asked_hydra_univmon<V: CardinalityValue>(
    params: &ParamSet,
    items: Rc<Vec<(String, V)>>,
    probes: Rc<Vec<Vec<String>>>,
    passes: usize,
    estimate: fn(&HydraUnivmon, &[&str]) -> f64,
) -> Result<Vec<QueryPass<f64>>, BuildError> {
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        // Built and fed here: the closure below asks, and only asks.
        let mut sketch = build_hydra_univmon(params)?;
        for v in items.iter() {
            sketch.inner.update(&v.0, &v.1.data_input(), None);
        }
        let probes = probes.clone();
        out.push(Box::new(move || {
            let mut answers = Vec::with_capacity(probes.len());
            for p in probes.iter() {
                answers.push(estimate(&sketch, &labels(p)));
            }
            let footprint = memory_hydra_univmon(&sketch);
            (answers, footprint)
        }) as QueryPass<f64>);
    }
    Ok(out)
}

pub fn query_hydra_univmon_cardinality<V: CardinalityValue>(
    params: &ParamSet,
    items: Rc<Vec<(String, V)>>,
    probes: Rc<Vec<Vec<String>>>,
    passes: usize,
) -> Result<Vec<QueryPass<f64>>, BuildError> {
    asked_hydra_univmon(
        params,
        items,
        probes,
        passes,
        HydraUnivmon::estimate_subpop_cardinality,
    )
}

pub fn query_hydra_univmon_l1_norm<V: CardinalityValue>(
    params: &ParamSet,
    items: Rc<Vec<(String, V)>>,
    probes: Rc<Vec<Vec<String>>>,
    passes: usize,
) -> Result<Vec<QueryPass<f64>>, BuildError> {
    asked_hydra_univmon(
        params,
        items,
        probes,
        passes,
        HydraUnivmon::estimate_subpop_l1_norm,
    )
}

pub fn query_hydra_univmon_l2_norm<V: CardinalityValue>(
    params: &ParamSet,
    items: Rc<Vec<(String, V)>>,
    probes: Rc<Vec<Vec<String>>>,
    passes: usize,
) -> Result<Vec<QueryPass<f64>>, BuildError> {
    asked_hydra_univmon(
        params,
        items,
        probes,
        passes,
        HydraUnivmon::estimate_subpop_l2_norm,
    )
}

pub fn query_hydra_univmon_entropy<V: CardinalityValue>(
    params: &ParamSet,
    items: Rc<Vec<(String, V)>>,
    probes: Rc<Vec<Vec<String>>>,
    passes: usize,
) -> Result<Vec<QueryPass<f64>>, BuildError> {
    asked_hydra_univmon(
        params,
        items,
        probes,
        passes,
        HydraUnivmon::estimate_subpop_entropy,
    )
}

pub fn merge_hydra_univmon<V: CardinalityValue>(
    params: &ParamSet,
    items: Rc<Vec<(String, V)>>,
    shards: usize,
    passes: usize,
) -> Result<Vec<Pass>, BuildError> {
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        let (mut acc, rest) = hydra_univmon_shards(params, &items, shards)?;
        out.push(Box::new(move || {
            for other in rest.iter() {
                acc.inner
                    .merge(&other.inner)
                    .expect("both operands built from one ParamSet, so grid and cell shapes match");
            }
            memory_hydra_univmon(&acc)
        }) as Pass);
    }
    Ok(out)
}

pub fn merge_step_hydra_univmon<V: CardinalityValue>(
    params: &ParamSet,
    items: Rc<Vec<(String, V)>>,
    shards: usize,
    passes: usize,
) -> Result<Vec<StepPass>, BuildError> {
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        let (acc, rest) = hydra_univmon_shards(params, &items, shards)?;
        let acc: Shared<_> = Rc::new(RefCell::new(acc));
        let (driven, read) = (acc.clone(), acc);
        out.push(StepPass {
            steps: rest.len(),
            step: Box::new(move |i| {
                let acc = &mut *driven.borrow_mut();
                let other = &rest[i];
                acc.inner
                    .merge(&other.inner)
                    .expect("both operands built from one ParamSet, so grid and cell shapes match");
            }),
            footprint: Box::new(move || memory_hydra_univmon(&read.borrow())),
        });
    }
    Ok(out)
}

/// The shards a fold folds: the stream split `shards` ways, one sketch each,
/// all fed. The first is the accumulator, the rest are what it folds in.
#[allow(clippy::type_complexity)]
fn hydra_univmon_shards<V: CardinalityValue>(
    params: &ParamSet,
    items: &[(String, V)],
    shards: usize,
) -> Result<(HydraUnivmon, Vec<HydraUnivmon>), BuildError> {
    let mut parts: Vec<HydraUnivmon> = Vec::new();
    for shard in partition(items, shards) {
        let mut sketch = build_hydra_univmon(params)?;
        for v in shard {
            sketch.inner.update(&v.0, &v.1.data_input(), None);
        }
        parts.push(sketch);
    }
    let rest = parts.split_off(1);
    Ok((
        parts.pop().expect("a split of the stream is never empty"),
        rest,
    ))
}
