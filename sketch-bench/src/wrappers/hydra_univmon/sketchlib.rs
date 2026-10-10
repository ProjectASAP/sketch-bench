//! The `asap_sketchlib` implementation.
//!
//! Grouped under `wrappers/hydra_univmon/` with the other implementations of
//! this algorithm; how each is driven lives beside it.

use super::*;
use crate::params::ParamSet;
use crate::wrappers::hll::CardinalityValue;
use crate::wrappers::hydra_shared::update_counted;
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

/// `label_columns`: the key columns every record carries.
pub fn build_hydra_univmon(
    config: &ParamSet,
    label_columns: usize,
) -> Result<HydraUnivmon, BuildError> {
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
        inner: new_hydra(p.rows, p.cols, label_columns, cell, "hydra-univmon")?,
        params: p,
    })
}

impl HydraUnivmon {
    #[inline]
    pub fn estimate_subpop_cardinality(&self, labels: &[Option<&str>]) -> f64 {
        query(
            &self.inner,
            labels,
            &HydraQuery::Cardinality,
            "hydra-univmon",
        )
    }

    #[inline]
    pub fn estimate_subpop_l1_norm(&self, labels: &[Option<&str>]) -> f64 {
        query(&self.inner, labels, &HydraQuery::L1Norm, "hydra-univmon")
    }

    #[inline]
    pub fn estimate_subpop_l2_norm(&self, labels: &[Option<&str>]) -> f64 {
        query(&self.inner, labels, &HydraQuery::L2Norm, "hydra-univmon")
    }

    #[inline]
    pub fn estimate_subpop_entropy(&self, labels: &[Option<&str>]) -> f64 {
        query(&self.inner, labels, &HydraQuery::Entropy, "hydra-univmon")
    }
}

pub fn memory_hydra_univmon(sketch: &HydraUnivmon) -> usize {
    let p = &sketch.params;
    let counters =
        (p.cell_sketch_row * p.cell_sketch_col + p.cell_sketch_row) * std::mem::size_of::<i64>();
    let heap = p.cell_heap_size * std::mem::size_of::<HHItem>();
    p.rows * p.cols * p.cell_layer_size * (counters + heap) + grid_overhead_bytes(p.rows, p.cols)
}

/// A value the `-sum` row inserts as its own count, the library's integer
/// weight: `None` for one an `i32` weight cannot carry, or a negative one.
pub trait WeightValue: CardinalityValue {
    fn weight(&self) -> Option<i32>;
}

impl WeightValue for i64 {
    fn weight(&self) -> Option<i32> {
        i32::try_from(*self).ok().filter(|w| *w >= 0)
    }
}

impl WeightValue for u64 {
    fn weight(&self) -> Option<i32> {
        i32::try_from(*self).ok()
    }
}

/// How one record enters the grid: its value as the key, counted once
/// ([`counted`]) or counted by itself ([`weighted`]).
type Feed<V> = fn(&mut HydraUnivmon, &(String, V));

fn counted<V: CardinalityValue>(sketch: &mut HydraUnivmon, r: &(String, V)) {
    update(&mut sketch.inner, &r.0, &r.1.data_input(), "hydra-univmon");
}

/// Weighted L1 is the group's sum. Every weight was checked by [`weights`]
/// before the grid was built.
fn weighted<V: WeightValue>(sketch: &mut HydraUnivmon, r: &(String, V)) {
    let count = r.1.weight().expect("weights() checked every value");
    update_counted(
        &mut sketch.inner,
        &r.0,
        &r.1.data_input(),
        Some(count),
        "hydra-univmon-sum",
    );
}

/// Refuses a stream with a value [`WeightValue::weight`] cannot carry, naming
/// it, so no sum is built over a truncated weight.
fn weights<V: WeightValue + std::fmt::Debug>(items: &[(String, V)]) -> Result<(), BuildError> {
    match items.iter().find(|r| r.1.weight().is_none()) {
        Some(r) => Err(BuildError(format!(
            "hydra-univmon-sum: value {:?} is not a count in [0, {}], the library's i32 weight",
            r.1,
            i32::MAX
        ))),
        None => Ok(()),
    }
}

pub fn insert_hydra_univmon<V: CardinalityValue>(
    params: &ParamSet,
    items: Rc<Vec<(String, V)>>,
    passes: usize,
) -> Result<Vec<Pass>, BuildError> {
    inserted(params, items, passes, counted::<V>)
}

pub fn insert_hydra_univmon_sum<V: WeightValue + std::fmt::Debug>(
    params: &ParamSet,
    items: Rc<Vec<(String, V)>>,
    passes: usize,
) -> Result<Vec<Pass>, BuildError> {
    weights(&items)?;
    inserted(params, items, passes, weighted::<V>)
}

fn inserted<V: CardinalityValue>(
    params: &ParamSet,
    items: Rc<Vec<(String, V)>>,
    passes: usize,
    feed: Feed<V>,
) -> Result<Vec<Pass>, BuildError> {
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        // Built here, so calling the closure is the insert and nothing else.
        let mut sketch = build_hydra_univmon(params, label_columns(&items, "hydra-univmon")?)?;
        let items = items.clone();
        out.push(Box::new(move || {
            for v in items.iter() {
                feed(&mut sketch, v);
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
    inserted_step(params, items, passes, counted::<V>)
}

pub fn insert_step_hydra_univmon_sum<V: WeightValue + std::fmt::Debug>(
    params: &ParamSet,
    items: Rc<Vec<(String, V)>>,
    passes: usize,
) -> Result<Vec<StepPass>, BuildError> {
    weights(&items)?;
    inserted_step(params, items, passes, weighted::<V>)
}

fn inserted_step<V: CardinalityValue>(
    params: &ParamSet,
    items: Rc<Vec<(String, V)>>,
    passes: usize,
    feed: Feed<V>,
) -> Result<Vec<StepPass>, BuildError> {
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        let sketch: Shared<_> = Rc::new(RefCell::new(build_hydra_univmon(
            params,
            label_columns(&items, "hydra-univmon")?,
        )?));
        let (driven, read) = (sketch.clone(), sketch);
        let stream = items.clone();
        out.push(StepPass {
            steps: items.len(),
            step: Box::new(move |i| {
                feed(&mut driven.borrow_mut(), &stream[i]);
            }),
            footprint: Box::new(move || memory_hydra_univmon(&read.borrow())),
        });
    }
    Ok(out)
}

fn asked_hydra_univmon<V: CardinalityValue>(
    params: &ParamSet,
    items: Rc<Vec<(String, V)>>,
    probes: Rc<Vec<Vec<Option<String>>>>,
    shards: usize,
    passes: usize,
    feed: Feed<V>,
    estimate: fn(&HydraUnivmon, &[Option<&str>]) -> f64,
) -> Result<Vec<QueryPass<f64>>, BuildError> {
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        // Built, fed and folded here: the closure below asks, and only asks.
        let (mut sketch, rest) = hydra_univmon_shards(params, &items, shards, feed)?;
        for other in rest.iter() {
            sketch
                .inner
                .merge(&other.inner)
                .expect("both operands built from one ParamSet, so grid and cell shapes match");
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
    probes: Rc<Vec<Vec<Option<String>>>>,
    passes: usize,
) -> Result<Vec<QueryPass<f64>>, BuildError> {
    asked_hydra_univmon(
        params,
        items,
        probes,
        1,
        passes,
        counted::<V>,
        HydraUnivmon::estimate_subpop_cardinality,
    )
}

/// The query, asked of the sketch a fold over `shards` shards leaves. One
/// shard is the plain query.
pub fn merge_query_hydra_univmon_cardinality<V: CardinalityValue>(
    params: &ParamSet,
    items: Rc<Vec<(String, V)>>,
    probes: Rc<Vec<Vec<Option<String>>>>,
    shards: usize,
    passes: usize,
) -> Result<Vec<QueryPass<f64>>, BuildError> {
    asked_hydra_univmon(
        params,
        items,
        probes,
        shards,
        passes,
        counted::<V>,
        HydraUnivmon::estimate_subpop_cardinality,
    )
}

pub fn query_hydra_univmon_l1_norm<V: CardinalityValue>(
    params: &ParamSet,
    items: Rc<Vec<(String, V)>>,
    probes: Rc<Vec<Vec<Option<String>>>>,
    passes: usize,
) -> Result<Vec<QueryPass<f64>>, BuildError> {
    asked_hydra_univmon(
        params,
        items,
        probes,
        1,
        passes,
        counted::<V>,
        HydraUnivmon::estimate_subpop_l1_norm,
    )
}

/// The query, asked of the sketch a fold over `shards` shards leaves. One
/// shard is the plain query.
pub fn merge_query_hydra_univmon_l1_norm<V: CardinalityValue>(
    params: &ParamSet,
    items: Rc<Vec<(String, V)>>,
    probes: Rc<Vec<Vec<Option<String>>>>,
    shards: usize,
    passes: usize,
) -> Result<Vec<QueryPass<f64>>, BuildError> {
    asked_hydra_univmon(
        params,
        items,
        probes,
        shards,
        passes,
        counted::<V>,
        HydraUnivmon::estimate_subpop_l1_norm,
    )
}

pub fn query_hydra_univmon_l2_norm<V: CardinalityValue>(
    params: &ParamSet,
    items: Rc<Vec<(String, V)>>,
    probes: Rc<Vec<Vec<Option<String>>>>,
    passes: usize,
) -> Result<Vec<QueryPass<f64>>, BuildError> {
    asked_hydra_univmon(
        params,
        items,
        probes,
        1,
        passes,
        counted::<V>,
        HydraUnivmon::estimate_subpop_l2_norm,
    )
}

/// The query, asked of the sketch a fold over `shards` shards leaves. One
/// shard is the plain query.
pub fn merge_query_hydra_univmon_l2_norm<V: CardinalityValue>(
    params: &ParamSet,
    items: Rc<Vec<(String, V)>>,
    probes: Rc<Vec<Vec<Option<String>>>>,
    shards: usize,
    passes: usize,
) -> Result<Vec<QueryPass<f64>>, BuildError> {
    asked_hydra_univmon(
        params,
        items,
        probes,
        shards,
        passes,
        counted::<V>,
        HydraUnivmon::estimate_subpop_l2_norm,
    )
}

pub fn query_hydra_univmon_entropy<V: CardinalityValue>(
    params: &ParamSet,
    items: Rc<Vec<(String, V)>>,
    probes: Rc<Vec<Vec<Option<String>>>>,
    passes: usize,
) -> Result<Vec<QueryPass<f64>>, BuildError> {
    asked_hydra_univmon(
        params,
        items,
        probes,
        1,
        passes,
        counted::<V>,
        HydraUnivmon::estimate_subpop_entropy,
    )
}

/// The query, asked of the sketch a fold over `shards` shards leaves. One
/// shard is the plain query.
pub fn merge_query_hydra_univmon_entropy<V: CardinalityValue>(
    params: &ParamSet,
    items: Rc<Vec<(String, V)>>,
    probes: Rc<Vec<Vec<Option<String>>>>,
    shards: usize,
    passes: usize,
) -> Result<Vec<QueryPass<f64>>, BuildError> {
    asked_hydra_univmon(
        params,
        items,
        probes,
        shards,
        passes,
        counted::<V>,
        HydraUnivmon::estimate_subpop_entropy,
    )
}

/// The group's sum: the L1 norm of a grid whose records were each counted by
/// their own value.
pub fn query_hydra_univmon_sum<V: WeightValue + std::fmt::Debug>(
    params: &ParamSet,
    items: Rc<Vec<(String, V)>>,
    probes: Rc<Vec<Vec<Option<String>>>>,
    passes: usize,
) -> Result<Vec<QueryPass<f64>>, BuildError> {
    merge_query_hydra_univmon_sum(params, items, probes, 1, passes)
}

/// The query, asked of the sketch a fold over `shards` shards leaves. One
/// shard is the plain query.
pub fn merge_query_hydra_univmon_sum<V: WeightValue + std::fmt::Debug>(
    params: &ParamSet,
    items: Rc<Vec<(String, V)>>,
    probes: Rc<Vec<Vec<Option<String>>>>,
    shards: usize,
    passes: usize,
) -> Result<Vec<QueryPass<f64>>, BuildError> {
    weights(&items)?;
    asked_hydra_univmon(
        params,
        items,
        probes,
        shards,
        passes,
        weighted::<V>,
        HydraUnivmon::estimate_subpop_l1_norm,
    )
}

pub fn merge_hydra_univmon<V: CardinalityValue>(
    params: &ParamSet,
    items: Rc<Vec<(String, V)>>,
    shards: usize,
    passes: usize,
) -> Result<Vec<Pass>, BuildError> {
    merged(params, items, shards, passes, counted::<V>)
}

pub fn merge_hydra_univmon_sum<V: WeightValue + std::fmt::Debug>(
    params: &ParamSet,
    items: Rc<Vec<(String, V)>>,
    shards: usize,
    passes: usize,
) -> Result<Vec<Pass>, BuildError> {
    weights(&items)?;
    merged(params, items, shards, passes, weighted::<V>)
}

fn merged<V: CardinalityValue>(
    params: &ParamSet,
    items: Rc<Vec<(String, V)>>,
    shards: usize,
    passes: usize,
    feed: Feed<V>,
) -> Result<Vec<Pass>, BuildError> {
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        let (mut acc, rest) = hydra_univmon_shards(params, &items, shards, feed)?;
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
    merged_step(params, items, shards, passes, counted::<V>)
}

pub fn merge_step_hydra_univmon_sum<V: WeightValue + std::fmt::Debug>(
    params: &ParamSet,
    items: Rc<Vec<(String, V)>>,
    shards: usize,
    passes: usize,
) -> Result<Vec<StepPass>, BuildError> {
    weights(&items)?;
    merged_step(params, items, shards, passes, weighted::<V>)
}

fn merged_step<V: CardinalityValue>(
    params: &ParamSet,
    items: Rc<Vec<(String, V)>>,
    shards: usize,
    passes: usize,
    feed: Feed<V>,
) -> Result<Vec<StepPass>, BuildError> {
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        let (acc, rest) = hydra_univmon_shards(params, &items, shards, feed)?;
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
    feed: Feed<V>,
) -> Result<(HydraUnivmon, Vec<HydraUnivmon>), BuildError> {
    let mut parts: Vec<HydraUnivmon> = Vec::new();
    for shard in partition(items, shards) {
        let mut sketch = build_hydra_univmon(params, label_columns(items, "hydra-univmon")?)?;
        for v in shard {
            feed(&mut sketch, v);
        }
        parts.push(sketch);
    }
    let rest = parts.split_off(1);
    Ok((
        parts.pop().expect("a split of the stream is never empty"),
        rest,
    ))
}
