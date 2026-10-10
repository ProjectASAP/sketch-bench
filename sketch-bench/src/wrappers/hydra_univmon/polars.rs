//! The exact `polars` baseline the sketches above are scored against.
//!
//! Grouped under `wrappers/hydra_univmon/` with the other implementations of
//! this algorithm; how each is driven lives beside it.

use super::*;
use crate::params::ParamSet;
use crate::wrappers::hll::CardinalityValue;
use crate::wrappers::polars_shared::*;
use crate::wrappers::{BuildError, Pass, QueryPass, Shared, StepPass};
use ::polars::prelude::*;
use aqpbm_core::accuracy::subpopulation::{entropy, l1_norm, l2_norm};
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

pub struct PolarsSubpopUnivmon<T: CardinalityValue = i64>(PolarsSubpopVectorCore<T>);

pub fn build_polars_subpop_cardinality_univmon<T: CardinalityValue>(
    config: &ParamSet,
) -> Result<PolarsSubpopCardinalityUnivmon<T>, BuildError> {
    // Exact, so no knob here does anything. The config is still parsed
    // and discarded: this row is the baseline its sketch siblings are
    // scored against, and a config they refuse must not quietly produce
    // a number here.
    let _p: HydraUnivmonParams = config.parse()?;
    Ok(PolarsSubpopCardinalityUnivmon::default())
}

pub struct PolarsSubpopCardinalityUnivmon<T: CardinalityValue = i64>(
    PolarsSubpopCardinalityCore<T>,
);

// Hand-written rather than derived: `derive` would bound `T: Default`, which
// the ingested widths have no reason to satisfy. An empty buffer is what
// "default" means here, and that needs nothing of `T`.
impl<T: CardinalityValue> Default for PolarsSubpopCardinalityUnivmon<T> {
    fn default() -> Self {
        Self(PolarsSubpopCardinalityCore::default())
    }
}

impl<T: CardinalityValue> PolarsSubpopCardinalityUnivmon<T> {
    pub fn estimate_subpop_cardinality(&self, labels: &[Option<&str>]) -> f64 {
        self.0.query(labels)
    }
}

pub fn memory_polars_subpop_cardinality_univmon<T: CardinalityValue>(
    sketch: &PolarsSubpopCardinalityUnivmon<T>,
) -> usize {
    sketch.0.memory_bytes()
}

fn build_polars_subpop_vector<T: CardinalityValue>(
    config: &ParamSet,
    fold: fn(&[u64]) -> f64,
) -> Result<PolarsSubpopUnivmon<T>, BuildError> {
    let _p: HydraUnivmonParams = config.parse()?;
    Ok(PolarsSubpopUnivmon(PolarsSubpopVectorCore::folding(fold)))
}

impl<T: CardinalityValue> PolarsSubpopUnivmon<T> {
    pub fn estimate_subpop_statistic(&self, labels: &[Option<&str>]) -> f64 {
        self.0.query(labels)
    }
}

pub fn memory_polars_subpop_univmon<T: CardinalityValue>(sketch: &PolarsSubpopUnivmon<T>) -> usize {
    sketch.0.memory_bytes()
}

pub fn insert_polars_subpop_cardinality_univmon<T: CardinalityValue>(
    params: &ParamSet,
    items: Rc<Vec<(String, T)>>,
    passes: usize,
) -> Result<Vec<Pass>, BuildError> {
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        // Built here, so calling the closure is the insert and nothing else.
        let mut sketch = build_polars_subpop_cardinality_univmon::<T>(params)?;
        let items = items.clone();
        out.push(Box::new(move || {
            for v in items.iter() {
                sketch.0.update(v);
            }
            memory_polars_subpop_cardinality_univmon(&sketch)
        }) as Pass);
    }
    Ok(out)
}

pub fn insert_step_polars_subpop_cardinality_univmon<T: CardinalityValue>(
    params: &ParamSet,
    items: Rc<Vec<(String, T)>>,
    passes: usize,
) -> Result<Vec<StepPass>, BuildError> {
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        let sketch: Shared<_> = Rc::new(RefCell::new(
            build_polars_subpop_cardinality_univmon::<T>(params)?,
        ));
        let (driven, read) = (sketch.clone(), sketch);
        let stream = items.clone();
        out.push(StepPass {
            steps: items.len(),
            step: Box::new(move |i| {
                let sketch = &mut *driven.borrow_mut();
                let v = &stream[i];
                sketch.0.update(v);
            }),
            footprint: Box::new(move || memory_polars_subpop_cardinality_univmon(&read.borrow())),
        });
    }
    Ok(out)
}

pub fn query_polars_subpop_cardinality_univmon<T: CardinalityValue>(
    params: &ParamSet,
    items: Rc<Vec<(String, T)>>,
    probes: Rc<Vec<Vec<Option<String>>>>,
    passes: usize,
) -> Result<Vec<QueryPass<f64>>, BuildError> {
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        // Built and fed here: the closure below asks, and only asks.
        let mut sketch = build_polars_subpop_cardinality_univmon::<T>(params)?;
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
            let footprint = memory_polars_subpop_cardinality_univmon(&sketch);
            (answers, footprint)
        }) as QueryPass<f64>);
    }
    Ok(out)
}

pub fn prepare_polars_subpop_cardinality_univmon<T: CardinalityValue>(
    params: &ParamSet,
    items: Rc<Vec<(String, T)>>,
    passes: usize,
) -> Result<Vec<Pass>, BuildError> {
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        // Fed here: the closure below is the step that makes it ready to answer.
        let mut sketch = build_polars_subpop_cardinality_univmon::<T>(params)?;
        for v in items.iter() {
            sketch.0.update(v);
        }
        out.push(Box::new(move || {
            sketch.0.finalize();
            memory_polars_subpop_cardinality_univmon(&sketch)
        }) as Pass);
    }
    Ok(out)
}

fn insert_polars_subpop_vector<T: CardinalityValue>(
    params: &ParamSet,
    items: Rc<Vec<(String, T)>>,
    passes: usize,
    fold: fn(&[u64]) -> f64,
) -> Result<Vec<Pass>, BuildError> {
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        // Built here, so calling the closure is the insert and nothing else.
        let mut sketch = build_polars_subpop_vector::<T>(params, fold)?;
        let items = items.clone();
        out.push(Box::new(move || {
            for v in items.iter() {
                sketch.0.update(v);
            }
            memory_polars_subpop_univmon(&sketch)
        }) as Pass);
    }
    Ok(out)
}

fn insert_step_polars_subpop_vector<T: CardinalityValue>(
    params: &ParamSet,
    items: Rc<Vec<(String, T)>>,
    passes: usize,
    fold: fn(&[u64]) -> f64,
) -> Result<Vec<StepPass>, BuildError> {
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        let sketch: Shared<_> =
            Rc::new(RefCell::new(build_polars_subpop_vector::<T>(params, fold)?));
        let (driven, read) = (sketch.clone(), sketch);
        let stream = items.clone();
        out.push(StepPass {
            steps: items.len(),
            step: Box::new(move |i| {
                let sketch = &mut *driven.borrow_mut();
                let v = &stream[i];
                sketch.0.update(v);
            }),
            footprint: Box::new(move || memory_polars_subpop_univmon(&read.borrow())),
        });
    }
    Ok(out)
}

fn query_polars_subpop_vector<T: CardinalityValue>(
    params: &ParamSet,
    items: Rc<Vec<(String, T)>>,
    probes: Rc<Vec<Vec<Option<String>>>>,
    passes: usize,
    fold: fn(&[u64]) -> f64,
) -> Result<Vec<QueryPass<f64>>, BuildError> {
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        // Built and fed here: the closure below asks, and only asks.
        let mut sketch = build_polars_subpop_vector::<T>(params, fold)?;
        for v in items.iter() {
            sketch.0.update(v);
        }
        sketch.0.finalize();
        let probes = probes.clone();
        out.push(Box::new(move || {
            let mut answers = Vec::with_capacity(probes.len());
            for p in probes.iter() {
                answers.push(sketch.estimate_subpop_statistic(&labels(p)));
            }
            let footprint = memory_polars_subpop_univmon(&sketch);
            (answers, footprint)
        }) as QueryPass<f64>);
    }
    Ok(out)
}

fn prepare_polars_subpop_vector<T: CardinalityValue>(
    params: &ParamSet,
    items: Rc<Vec<(String, T)>>,
    passes: usize,
    fold: fn(&[u64]) -> f64,
) -> Result<Vec<Pass>, BuildError> {
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        // Fed here: the closure below is the step that makes it ready to answer.
        let mut sketch = build_polars_subpop_vector::<T>(params, fold)?;
        for v in items.iter() {
            sketch.0.update(v);
        }
        out.push(Box::new(move || {
            sketch.0.finalize();
            memory_polars_subpop_univmon(&sketch)
        }) as Pass);
    }
    Ok(out)
}

pub fn insert_polars_subpop_l1_norm<T: CardinalityValue>(
    params: &ParamSet,
    items: Rc<Vec<(String, T)>>,
    passes: usize,
) -> Result<Vec<Pass>, BuildError> {
    insert_polars_subpop_vector(params, items, passes, l1_norm)
}

pub fn insert_step_polars_subpop_l1_norm<T: CardinalityValue>(
    params: &ParamSet,
    items: Rc<Vec<(String, T)>>,
    passes: usize,
) -> Result<Vec<StepPass>, BuildError> {
    insert_step_polars_subpop_vector(params, items, passes, l1_norm)
}

pub fn query_polars_subpop_l1_norm<T: CardinalityValue>(
    params: &ParamSet,
    items: Rc<Vec<(String, T)>>,
    probes: Rc<Vec<Vec<Option<String>>>>,
    passes: usize,
) -> Result<Vec<QueryPass<f64>>, BuildError> {
    query_polars_subpop_vector(params, items, probes, passes, l1_norm)
}

pub fn prepare_polars_subpop_l1_norm<T: CardinalityValue>(
    params: &ParamSet,
    items: Rc<Vec<(String, T)>>,
    passes: usize,
) -> Result<Vec<Pass>, BuildError> {
    prepare_polars_subpop_vector(params, items, passes, l1_norm)
}

pub fn insert_polars_subpop_l2_norm<T: CardinalityValue>(
    params: &ParamSet,
    items: Rc<Vec<(String, T)>>,
    passes: usize,
) -> Result<Vec<Pass>, BuildError> {
    insert_polars_subpop_vector(params, items, passes, l2_norm)
}

pub fn insert_step_polars_subpop_l2_norm<T: CardinalityValue>(
    params: &ParamSet,
    items: Rc<Vec<(String, T)>>,
    passes: usize,
) -> Result<Vec<StepPass>, BuildError> {
    insert_step_polars_subpop_vector(params, items, passes, l2_norm)
}

pub fn query_polars_subpop_l2_norm<T: CardinalityValue>(
    params: &ParamSet,
    items: Rc<Vec<(String, T)>>,
    probes: Rc<Vec<Vec<Option<String>>>>,
    passes: usize,
) -> Result<Vec<QueryPass<f64>>, BuildError> {
    query_polars_subpop_vector(params, items, probes, passes, l2_norm)
}

pub fn prepare_polars_subpop_l2_norm<T: CardinalityValue>(
    params: &ParamSet,
    items: Rc<Vec<(String, T)>>,
    passes: usize,
) -> Result<Vec<Pass>, BuildError> {
    prepare_polars_subpop_vector(params, items, passes, l2_norm)
}

pub fn insert_polars_subpop_entropy<T: CardinalityValue>(
    params: &ParamSet,
    items: Rc<Vec<(String, T)>>,
    passes: usize,
) -> Result<Vec<Pass>, BuildError> {
    insert_polars_subpop_vector(params, items, passes, entropy)
}

pub fn insert_step_polars_subpop_entropy<T: CardinalityValue>(
    params: &ParamSet,
    items: Rc<Vec<(String, T)>>,
    passes: usize,
) -> Result<Vec<StepPass>, BuildError> {
    insert_step_polars_subpop_vector(params, items, passes, entropy)
}

pub fn query_polars_subpop_entropy<T: CardinalityValue>(
    params: &ParamSet,
    items: Rc<Vec<(String, T)>>,
    probes: Rc<Vec<Vec<Option<String>>>>,
    passes: usize,
) -> Result<Vec<QueryPass<f64>>, BuildError> {
    query_polars_subpop_vector(params, items, probes, passes, entropy)
}

pub fn prepare_polars_subpop_entropy<T: CardinalityValue>(
    params: &ParamSet,
    items: Rc<Vec<(String, T)>>,
    passes: usize,
) -> Result<Vec<Pass>, BuildError> {
    prepare_polars_subpop_vector(params, items, passes, entropy)
}

/// `hydra-univmon-sum/polars` — the exact sum of each label subset's values.
pub struct PolarsSubpopSum<T: CardinalityValue = i64> {
    buf: Vec<(String, T)>,
    exact: HashMap<String, f64>,
}

// Hand-written rather than derived, for the reason the cardinality baseline's is.
impl<T: CardinalityValue> Default for PolarsSubpopSum<T> {
    fn default() -> Self {
        Self {
            buf: Vec::new(),
            exact: HashMap::new(),
        }
    }
}

pub fn build_polars_subpop_sum<T: CardinalityValue>(
    config: &ParamSet,
) -> Result<PolarsSubpopSum<T>, BuildError> {
    // Parsed and discarded, as the other baselines here do.
    let _p: HydraUnivmonParams = config.parse()?;
    Ok(PolarsSubpopSum::default())
}

impl<T: CardinalityValue> PolarsSubpopSum<T> {
    /// The exact answer, over the stream as buffered. Its own step because the
    /// runner times it as `prepare`: this row's cost is the pass, not the ask.
    fn finalize(&mut self) {
        let Some(result) = grouped_values(&self.buf, |lazy| {
            lazy.group_by([col("g")]).agg([col("v").sum().alias("sum")])
        }) else {
            return;
        };
        let groups = result.column("g").expect("g column");
        let groups = groups.str().expect("str groups");
        let sums = result
            .column("sum")
            .expect("sum column")
            .cast(&DataType::Float64)
            .expect("cast to f64");
        let sums = sums.f64().expect("f64 sums");
        self.exact = groups
            .into_iter()
            .zip(sums)
            .filter_map(|(g, s)| Some((g?.to_string(), s?)))
            .collect();
    }

    pub fn estimate_subpop_sum(&self, labels: &[Option<&str>]) -> f64 {
        self.exact.get(&group_key(labels)).copied().unwrap_or(0.0)
    }
}

pub fn memory_polars_subpop_sum<T: CardinalityValue>(sketch: &PolarsSubpopSum<T>) -> usize {
    let held: usize = sketch.buf.iter().map(|(g, _)| g.capacity()).sum();
    sketch.buf.capacity() * std::mem::size_of::<(String, T)>()
        + held
        + sketch.exact.capacity() * (std::mem::size_of::<String>() + std::mem::size_of::<f64>())
}

pub fn insert_polars_subpop_sum<T: CardinalityValue>(
    params: &ParamSet,
    items: Rc<Vec<(String, T)>>,
    passes: usize,
) -> Result<Vec<Pass>, BuildError> {
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        // Built here, so calling the closure is the insert and nothing else.
        let mut sketch = build_polars_subpop_sum::<T>(params)?;
        let items = items.clone();
        out.push(Box::new(move || {
            for v in items.iter() {
                sketch.buf.push(v.clone());
            }
            memory_polars_subpop_sum(&sketch)
        }) as Pass);
    }
    Ok(out)
}

pub fn insert_step_polars_subpop_sum<T: CardinalityValue>(
    params: &ParamSet,
    items: Rc<Vec<(String, T)>>,
    passes: usize,
) -> Result<Vec<StepPass>, BuildError> {
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        let sketch: Shared<_> = Rc::new(RefCell::new(build_polars_subpop_sum::<T>(params)?));
        let (driven, read) = (sketch.clone(), sketch);
        let stream = items.clone();
        out.push(StepPass {
            steps: items.len(),
            step: Box::new(move |i| driven.borrow_mut().buf.push(stream[i].clone())),
            footprint: Box::new(move || memory_polars_subpop_sum(&read.borrow())),
        });
    }
    Ok(out)
}

pub fn query_polars_subpop_sum<T: CardinalityValue>(
    params: &ParamSet,
    items: Rc<Vec<(String, T)>>,
    probes: Rc<Vec<Vec<Option<String>>>>,
    passes: usize,
) -> Result<Vec<QueryPass<f64>>, BuildError> {
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        // Built and fed here: the closure below asks, and only asks.
        let mut sketch = build_polars_subpop_sum::<T>(params)?;
        sketch.buf.extend(items.iter().cloned());
        sketch.finalize();
        let probes = probes.clone();
        out.push(Box::new(move || {
            let mut answers = Vec::with_capacity(probes.len());
            for p in probes.iter() {
                answers.push(sketch.estimate_subpop_sum(&labels(p)));
            }
            let footprint = memory_polars_subpop_sum(&sketch);
            (answers, footprint)
        }) as QueryPass<f64>);
    }
    Ok(out)
}

pub fn prepare_polars_subpop_sum<T: CardinalityValue>(
    params: &ParamSet,
    items: Rc<Vec<(String, T)>>,
    passes: usize,
) -> Result<Vec<Pass>, BuildError> {
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        // Fed here: the closure below is the step that makes it ready to answer.
        let mut sketch = build_polars_subpop_sum::<T>(params)?;
        sketch.buf.extend(items.iter().cloned());
        out.push(Box::new(move || {
            sketch.finalize();
            memory_polars_subpop_sum(&sketch)
        }) as Pass);
    }
    Ok(out)
}
