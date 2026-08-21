//! The exact `polars` baseline the sketch above is scored against.
//!
//! Grouped under `wrappers/hydra_kll/` with the other implementations of this
//! algorithm; how each is driven lives beside it.

use super::*;
use crate::params::ParamSet;
use crate::wrappers::polars_shared::*;
use crate::wrappers::{BuildError, Pass, QueryPass, Shared, StepPass};
use ::polars::prelude::*;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

/// `hydra-kll/polars` — exact subpopulation quantile.
#[derive(Default)]
pub struct PolarsSubpopQuantile {
    buf: Vec<(String, f64)>,
    sorted: HashMap<String, Vec<f64>>,
}

pub fn build_polars_subpop_quantile(config: &ParamSet) -> Result<PolarsSubpopQuantile, BuildError> {
    // Exact, so no knob here does anything. The config is still parsed
    // and discarded: this row is the baseline its sketch siblings are
    // scored against, and a config they refuse must not quietly produce
    // a number here.
    let _p: HydraKllParams = config.parse()?;
    Ok(PolarsSubpopQuantile::default())
}

impl PolarsSubpopQuantile {
    /// `floor(phi * n)`, clamped to the last index. That is the index whose
    /// rank interval contains `phi * n`, which is what the rank-error
    /// comparator scores against, so an exact answer scores zero.
    pub fn estimate_subpop_quantile(&self, labels: &[&str], phi: f64) -> f64 {
        let Some(values) = self.sorted.get(&labels.join(";")) else {
            return f64::NAN;
        };
        if values.is_empty() {
            return f64::NAN;
        }
        let idx = ((phi * values.len() as f64).floor() as usize).min(values.len() - 1);
        values[idx]
    }
}

pub fn memory_polars_subpop_quantile(sketch: &PolarsSubpopQuantile) -> usize {
    sketch.buf.capacity() * std::mem::size_of::<(String, f64)>()
        + sketch
            .sorted
            .values()
            .map(|v| v.capacity() * std::mem::size_of::<f64>())
            .sum::<usize>()
}

impl PolarsSubpopQuantile {
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
            .sort(["g", "v"], Default::default())
            .collect()
            .expect("polars sort collect");

        let groups = result.column("g").expect("g column");
        let groups = groups.str().expect("str groups");
        let vals = result.column("v").expect("v column");
        let vals = vals.f64().expect("f64 values");

        for (g, v) in groups.into_iter().zip(vals) {
            if let (Some(g), Some(v)) = (g, v) {
                // Already ascending within a group, so push keeps it sorted.
                self.sorted.entry(g.to_string()).or_default().push(v);
            }
        }
    }
}

pub fn insert_polars_subpop_quantile(
    params: &ParamSet,
    items: Rc<Vec<(String, f64)>>,
    passes: usize,
) -> Result<Vec<Pass>, BuildError> {
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        // Built here, so calling the closure is the insert and nothing else.
        let mut sketch = build_polars_subpop_quantile(params)?;
        let items = items.clone();
        out.push(Box::new(move || {
            for v in items.iter() {
                sketch.buf.push(v.clone());
            }
            memory_polars_subpop_quantile(&sketch)
        }) as Pass);
    }
    Ok(out)
}

pub fn insert_step_polars_subpop_quantile(
    params: &ParamSet,
    items: Rc<Vec<(String, f64)>>,
    passes: usize,
) -> Result<Vec<StepPass>, BuildError> {
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        let sketch: Shared<_> = Rc::new(RefCell::new(build_polars_subpop_quantile(params)?));
        let (driven, read) = (sketch.clone(), sketch);
        let stream = items.clone();
        out.push(StepPass {
            steps: items.len(),
            step: Box::new(move |i| {
                let sketch = &mut *driven.borrow_mut();
                let v = &stream[i];
                sketch.buf.push(v.clone());
            }),
            footprint: Box::new(move || memory_polars_subpop_quantile(&read.borrow())),
        });
    }
    Ok(out)
}

pub fn query_polars_subpop_quantile(
    params: &ParamSet,
    items: Rc<Vec<(String, f64)>>,
    probes: Rc<Vec<(Vec<String>, f64)>>,
    passes: usize,
) -> Result<Vec<QueryPass<f64>>, BuildError> {
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        // Built and fed here: the closure below asks, and only asks.
        let mut sketch = build_polars_subpop_quantile(params)?;
        for v in items.iter() {
            sketch.buf.push(v.clone());
        }
        sketch.finalize();
        let probes = probes.clone();
        out.push(Box::new(move || {
            let mut answers = Vec::with_capacity(probes.len());
            for p in probes.iter() {
                answers.push(sketch.estimate_subpop_quantile(&labels(&p.0), p.1));
            }
            let footprint = memory_polars_subpop_quantile(&sketch);
            (answers, footprint)
        }) as QueryPass<f64>);
    }
    Ok(out)
}

pub fn prepare_polars_subpop_quantile(
    params: &ParamSet,
    items: Rc<Vec<(String, f64)>>,
    passes: usize,
) -> Result<Vec<Pass>, BuildError> {
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        // Fed here: the closure below is the step that makes it ready to answer.
        let mut sketch = build_polars_subpop_quantile(params)?;
        for v in items.iter() {
            sketch.buf.push(v.clone());
        }
        out.push(Box::new(move || {
            sketch.finalize();
            memory_polars_subpop_quantile(&sketch)
        }) as Pass);
    }
    Ok(out)
}
