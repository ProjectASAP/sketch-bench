//! The `asap_sketchlib` implementation.
//!
//! Grouped under `wrappers/hydra_hll/` with the other implementations of this
//! algorithm; how each is driven lives beside it.

use super::*;
use crate::params::ParamSet;
use crate::wrappers::hll::CardinalityValue;
use crate::wrappers::partition;
use crate::wrappers::{BuildError, Pass, QueryPass, Shared, StepPass};
use asap_sketchlib::input::{HydraCounter, HydraQuery};
use asap_sketchlib::{Hydra, HyperLogLog};
use std::cell::RefCell;
use std::rc::Rc;

/// Hydra over HyperLogLog cells.
pub struct HydraHll {
    pub(super) inner: Hydra,
    params: HydraHllParams,
}

pub fn build_hydra_hll(config: &ParamSet) -> Result<HydraHll, BuildError> {
    let p: HydraHllParams = config.parse()?;
    check_grid(p.rows, p.cols, "hydra-hll")?;
    // Named through the `ErtlMLE` impl explicitly: `HyperLogLog` is a type
    // alias over the variant, so `new()` is ambiguous between the
    // estimators the alias can carry. The enum fixes this one.
    let cell = HydraCounter::HLL(HyperLogLog::<asap_sketchlib::ErtlMLE>::new());
    Ok(HydraHll {
        inner: Hydra::with_dimensions(p.rows, p.cols, cell),
        params: p,
    })
}

impl HydraHll {
    #[inline]
    pub fn estimate_subpop_cardinality(&self, labels: &[&str]) -> f64 {
        self.inner
            .query_key(labels.to_vec(), &HydraQuery::Cardinality)
    }
}

/// One byte per register per cell. The cell is fixed-shape, so unlike the
/// Count-Min row there is no cell parameter in this product.
pub fn memory_hydra_hll(sketch: &HydraHll) -> usize {
    let p = &sketch.params;
    p.rows * p.cols * HLL_CELL_REGISTERS + grid_overhead_bytes(p.rows, p.cols)
}

pub fn insert_hydra_hll<V: CardinalityValue>(
    params: &ParamSet,
    items: Rc<Vec<(String, V)>>,
    passes: usize,
) -> Result<Vec<Pass>, BuildError> {
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        // Built here, so calling the closure is the insert and nothing else.
        let mut sketch = build_hydra_hll(params)?;
        let items = items.clone();
        out.push(Box::new(move || {
            for v in items.iter() {
                sketch.inner.update(&v.0, &v.1.data_input(), None);
            }
            memory_hydra_hll(&sketch)
        }) as Pass);
    }
    Ok(out)
}

pub fn insert_step_hydra_hll<V: CardinalityValue>(
    params: &ParamSet,
    items: Rc<Vec<(String, V)>>,
    passes: usize,
) -> Result<Vec<StepPass>, BuildError> {
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        let sketch: Shared<_> = Rc::new(RefCell::new(build_hydra_hll(params)?));
        let (driven, read) = (sketch.clone(), sketch);
        let stream = items.clone();
        out.push(StepPass {
            steps: items.len(),
            step: Box::new(move |i| {
                let sketch = &mut *driven.borrow_mut();
                let v = &stream[i];
                sketch.inner.update(&v.0, &v.1.data_input(), None);
            }),
            footprint: Box::new(move || memory_hydra_hll(&read.borrow())),
        });
    }
    Ok(out)
}

pub fn query_hydra_hll<V: CardinalityValue>(
    params: &ParamSet,
    items: Rc<Vec<(String, V)>>,
    probes: Rc<Vec<Vec<String>>>,
    passes: usize,
) -> Result<Vec<QueryPass<f64>>, BuildError> {
    merge_query_hydra_hll(params, items, probes, 1, passes)
}

/// The query, asked of the sketch a fold over `shards` shards leaves. One
/// shard is the plain query.
pub fn merge_query_hydra_hll<V: CardinalityValue>(
    params: &ParamSet,
    items: Rc<Vec<(String, V)>>,
    probes: Rc<Vec<Vec<String>>>,
    shards: usize,
    passes: usize,
) -> Result<Vec<QueryPass<f64>>, BuildError> {
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        // Built, fed and folded here: the closure below asks, and only asks.
        let (mut sketch, rest) = hydra_hll_shards(params, &items, shards)?;
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
                answers.push(sketch.estimate_subpop_cardinality(&labels(p)));
            }
            let footprint = memory_hydra_hll(&sketch);
            (answers, footprint)
        }) as QueryPass<f64>);
    }
    Ok(out)
}

pub fn merge_hydra_hll<V: CardinalityValue>(
    params: &ParamSet,
    items: Rc<Vec<(String, V)>>,
    shards: usize,
    passes: usize,
) -> Result<Vec<Pass>, BuildError> {
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        let (mut acc, rest) = hydra_hll_shards(params, &items, shards)?;
        out.push(Box::new(move || {
            for other in rest.iter() {
                acc.inner
                    .merge(&other.inner)
                    .expect("both operands built from one ParamSet, so grid and cell shapes match");
            }
            memory_hydra_hll(&acc)
        }) as Pass);
    }
    Ok(out)
}

pub fn merge_step_hydra_hll<V: CardinalityValue>(
    params: &ParamSet,
    items: Rc<Vec<(String, V)>>,
    shards: usize,
    passes: usize,
) -> Result<Vec<StepPass>, BuildError> {
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        let (acc, rest) = hydra_hll_shards(params, &items, shards)?;
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
            footprint: Box::new(move || memory_hydra_hll(&read.borrow())),
        });
    }
    Ok(out)
}

/// The shards a fold folds: the stream split `shards` ways, one sketch each,
/// all fed. The first is the accumulator, the rest are what it folds in.
#[allow(clippy::type_complexity)]
fn hydra_hll_shards<V: CardinalityValue>(
    params: &ParamSet,
    items: &[(String, V)],
    shards: usize,
) -> Result<(HydraHll, Vec<HydraHll>), BuildError> {
    let mut parts: Vec<HydraHll> = Vec::new();
    for shard in partition(items, shards) {
        let mut sketch = build_hydra_hll(params)?;
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
