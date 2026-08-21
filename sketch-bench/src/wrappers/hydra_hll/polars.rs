//! The exact `polars` baseline the sketch above is scored against.
//!
//! Grouped under `wrappers/hydra_hll/` with the other implementations of this
//! algorithm; how each is driven lives beside it.

use super::*;
use crate::params::ParamSet;
use crate::wrappers::polars_shared::*;
use crate::wrappers::{BuildError, Pass, QueryPass, Shared, StepPass};
use ::polars::prelude::*;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

/// `hydra-hll/polars` — exact subpopulation cardinality.
#[derive(Default)]
pub struct PolarsSubpopCardinality {
    buf: Vec<(String, i64)>,
    distinct: HashMap<String, u64>,
}

pub fn build_polars_subpop_cardinality(
    config: &ParamSet,
) -> Result<PolarsSubpopCardinality, BuildError> {
    // Exact, so no knob here does anything. The config is still parsed
    // and discarded: this row is the baseline its sketch siblings are
    // scored against, and a config they refuse must not quietly produce
    // a number here.
    let _p: HydraHllParams = config.parse()?;
    Ok(PolarsSubpopCardinality::default())
}

impl PolarsSubpopCardinality {
    pub fn estimate_subpop_cardinality(&self, labels: &[&str]) -> f64 {
        self.distinct.get(&labels.join(";")).copied().unwrap_or(0) as f64
    }
}

pub fn memory_polars_subpop_cardinality(sketch: &PolarsSubpopCardinality) -> usize {
    sketch.buf.capacity() * std::mem::size_of::<(String, i64)>()
        + sketch.distinct.capacity() * (std::mem::size_of::<String>() + 8)
}

impl PolarsSubpopCardinality {
    /// The exact answer, over the stream as buffered. Its own step because the
    /// runner times it as `prepare`: this row's cost is the pass, not the ask.
    fn finalize(&mut self) {
        let (mut keys, mut values) = (Vec::new(), Vec::new());
        for r in &self.buf {
            fan_out(&r.0, r.1, &mut keys, &mut values);
        }
        if keys.is_empty() {
            return;
        }
        let df = DataFrame::new(vec![
            Column::new("g".into(), &keys),
            Column::new("v".into(), &values),
        ])
        .expect("DataFrame::new");
        let result = df
            .lazy()
            .group_by([col("g")])
            .agg([col("v").n_unique().alias("c")])
            .collect()
            .expect("polars group_by collect");

        let groups = result.column("g").expect("g column");
        let groups = groups.str().expect("str groups");
        let counts = result
            .column("c")
            .expect("c column")
            .cast(&DataType::UInt64)
            .expect("cast to u64");
        let counts = counts.u64().expect("u64 counts");

        self.distinct.reserve(groups.len());
        for (g, c) in groups.into_iter().zip(counts) {
            if let (Some(g), Some(c)) = (g, c) {
                self.distinct.insert(g.to_string(), c);
            }
        }
    }
}

pub fn insert_polars_subpop_cardinality(
    params: &ParamSet,
    items: Rc<Vec<(String, i64)>>,
    passes: usize,
) -> Result<Vec<Pass>, BuildError> {
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        // Built here, so calling the closure is the insert and nothing else.
        let mut sketch = build_polars_subpop_cardinality(params)?;
        let items = items.clone();
        out.push(Box::new(move || {
            for v in items.iter() {
                sketch.buf.push(v.clone());
            }
            memory_polars_subpop_cardinality(&sketch)
        }) as Pass);
    }
    Ok(out)
}

pub fn insert_step_polars_subpop_cardinality(
    params: &ParamSet,
    items: Rc<Vec<(String, i64)>>,
    passes: usize,
) -> Result<Vec<StepPass>, BuildError> {
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        let sketch: Shared<_> = Rc::new(RefCell::new(build_polars_subpop_cardinality(params)?));
        let (driven, read) = (sketch.clone(), sketch);
        let stream = items.clone();
        out.push(StepPass {
            steps: items.len(),
            step: Box::new(move |i| {
                let sketch = &mut *driven.borrow_mut();
                let v = &stream[i];
                sketch.buf.push(v.clone());
            }),
            footprint: Box::new(move || memory_polars_subpop_cardinality(&read.borrow())),
        });
    }
    Ok(out)
}

pub fn query_polars_subpop_cardinality(
    params: &ParamSet,
    items: Rc<Vec<(String, i64)>>,
    probes: Rc<Vec<Vec<String>>>,
    passes: usize,
) -> Result<Vec<QueryPass<f64>>, BuildError> {
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        // Built and fed here: the closure below asks, and only asks.
        let mut sketch = build_polars_subpop_cardinality(params)?;
        for v in items.iter() {
            sketch.buf.push(v.clone());
        }
        sketch.finalize();
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

pub fn prepare_polars_subpop_cardinality(
    params: &ParamSet,
    items: Rc<Vec<(String, i64)>>,
    passes: usize,
) -> Result<Vec<Pass>, BuildError> {
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        // Fed here: the closure below is the step that makes it ready to answer.
        let mut sketch = build_polars_subpop_cardinality(params)?;
        for v in items.iter() {
            sketch.buf.push(v.clone());
        }
        out.push(Box::new(move || {
            sketch.finalize();
            memory_polars_subpop_cardinality(&sketch)
        }) as Pass);
    }
    Ok(out)
}
