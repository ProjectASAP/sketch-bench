//! The exact `polars` baseline the sketch above is scored against.
//!
//! Grouped under `wrappers/hydra_hll/` with the other implementations of this
//! algorithm; how each is driven lives beside it.

use super::*;
use crate::params::ParamSet;
use crate::wrappers::hll::CardinalityValue;
use crate::wrappers::polars_shared::*;
use crate::wrappers::{BuildError, Pass, QueryPass, Shared, StepPass};
use std::cell::RefCell;
use std::rc::Rc;

pub struct PolarsSubpopCardinality<T: CardinalityValue = i64>(PolarsSubpopCardinalityCore<T>);

// Hand-written rather than derived: `derive` would bound `T: Default`, which
// the ingested widths have no reason to satisfy. An empty buffer is what
// "default" means here, and that needs nothing of `T`.
impl<T: CardinalityValue> Default for PolarsSubpopCardinality<T> {
    fn default() -> Self {
        Self(PolarsSubpopCardinalityCore::default())
    }
}

pub fn build_polars_subpop_cardinality<T: CardinalityValue>(
    config: &ParamSet,
) -> Result<PolarsSubpopCardinality<T>, BuildError> {
    // Exact, so no knob here does anything. The config is still parsed
    // and discarded: this row is the baseline its sketch siblings are
    // scored against, and a config they refuse must not quietly produce
    // a number here.
    let _p: HydraHllParams = config.parse()?;
    Ok(PolarsSubpopCardinality::default())
}

impl<T: CardinalityValue> PolarsSubpopCardinality<T> {
    pub fn estimate_subpop_cardinality(&self, labels: &[Option<&str>]) -> f64 {
        self.0.query(labels)
    }
}

pub fn memory_polars_subpop_cardinality<T: CardinalityValue>(
    sketch: &PolarsSubpopCardinality<T>,
) -> usize {
    sketch.0.memory_bytes()
}

pub fn insert_polars_subpop_cardinality<T: CardinalityValue>(
    params: &ParamSet,
    items: Rc<Vec<(String, T)>>,
    passes: usize,
) -> Result<Vec<Pass>, BuildError> {
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        // Built here, so calling the closure is the insert and nothing else.
        let mut sketch = build_polars_subpop_cardinality::<T>(params)?;
        let items = items.clone();
        out.push(Box::new(move || {
            for v in items.iter() {
                sketch.0.update(v);
            }
            memory_polars_subpop_cardinality(&sketch)
        }) as Pass);
    }
    Ok(out)
}

pub fn insert_step_polars_subpop_cardinality<T: CardinalityValue>(
    params: &ParamSet,
    items: Rc<Vec<(String, T)>>,
    passes: usize,
) -> Result<Vec<StepPass>, BuildError> {
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        let sketch: Shared<_> =
            Rc::new(RefCell::new(build_polars_subpop_cardinality::<T>(params)?));
        let (driven, read) = (sketch.clone(), sketch);
        let stream = items.clone();
        out.push(StepPass {
            steps: items.len(),
            step: Box::new(move |i| {
                let sketch = &mut *driven.borrow_mut();
                let v = &stream[i];
                sketch.0.update(v);
            }),
            footprint: Box::new(move || memory_polars_subpop_cardinality(&read.borrow())),
        });
    }
    Ok(out)
}

pub fn query_polars_subpop_cardinality<T: CardinalityValue>(
    params: &ParamSet,
    items: Rc<Vec<(String, T)>>,
    probes: Rc<Vec<Vec<Option<String>>>>,
    passes: usize,
) -> Result<Vec<QueryPass<f64>>, BuildError> {
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        // Built and fed here: the closure below asks, and only asks.
        let mut sketch = build_polars_subpop_cardinality::<T>(params)?;
        for v in items.iter() {
            sketch.0.update(v);
        }
        sketch.0.finalize();
        let probes = probes.clone();
        out.push(Box::new(move || {
            let mut answers = Vec::with_capacity(probes.len());
            for p in probes.iter() {
                answers.push(sketch.estimate_subpop_cardinality(&labels(p)));
            }
            let footprint = memory_polars_subpop_cardinality(&sketch);
            (answers, footprint)
        }) as QueryPass<f64>);
    }
    Ok(out)
}

pub fn prepare_polars_subpop_cardinality<T: CardinalityValue>(
    params: &ParamSet,
    items: Rc<Vec<(String, T)>>,
    passes: usize,
) -> Result<Vec<Pass>, BuildError> {
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        // Fed here: the closure below is the step that makes it ready to answer.
        let mut sketch = build_polars_subpop_cardinality::<T>(params)?;
        for v in items.iter() {
            sketch.0.update(v);
        }
        out.push(Box::new(move || {
            sketch.0.finalize();
            memory_polars_subpop_cardinality(&sketch)
        }) as Pass);
    }
    Ok(out)
}
