//! The exact `polars` baseline the sketches above are scored against.
//!
//! Grouped under `wrappers/hll/` with the other implementations of this
//! algorithm; how each is driven lives beside it.

use super::*;
use crate::params::ParamSet;
use crate::wrappers::{BuildError, Pass, QueryPass, Shared, StepPass};
use ::polars::prelude::*;
use std::cell::RefCell;
use std::rc::Rc;

// `merge` lives on sketch_oxide's `Mergeable`, not on its `Accumulator`.

/// `hll/polars` — distinct count via `n_unique`.
pub struct PolarsCardinality<T: CardinalityValue = i64> {
    buf: Vec<T>,
    buffered_heap_bytes: usize,
    estimate: f64,
}

/// Polars computes the exact answer and has no `(rows, cols)` to tune, so it
/// ignores the `ParamSet` and builds unconditionally. Its record carries
/// whatever config the run was given.
pub fn build_polars_cardinality<T: CardinalityValue>(
    config: &ParamSet,
) -> Result<PolarsCardinality<T>, BuildError> {
    // Exact, so no knob here does anything. The config is still parsed
    // and discarded: this row is the baseline its sketch siblings are
    // scored against, and a config they refuse must not quietly produce
    // a number here.
    let _p: HllParams = config.parse()?;
    Ok(PolarsCardinality {
        buf: Vec::new(),
        buffered_heap_bytes: 0,
        estimate: 0.0,
    })
}

pub fn memory_polars_cardinality<T: CardinalityValue>(sketch: &PolarsCardinality<T>) -> usize {
    sketch.buf.capacity() * std::mem::size_of::<T>() + sketch.buffered_heap_bytes
}

impl<T: CardinalityValue> PolarsCardinality<T> {
    /// The exact answer, over the stream as buffered. Its own step because the
    /// runner times it as `prepare` — this row's cost is the pass, not the ask.
    fn finalize(&mut self) {
        let series = T::polars_column(&self.buf);
        let df = DataFrame::new(vec![series]).expect("DataFrame::new");
        let result = df
            .lazy()
            .select([col("v").n_unique().alias("c")])
            .collect()
            .expect("polars n_unique collect");
        let c = result
            .column("c")
            .expect("c column")
            .cast(&DataType::Float64)
            .expect("cast to f64");
        self.estimate = c.f64().expect("f64 chunked").get(0).unwrap_or(0.0);
    }
}

pub fn insert_polars_cardinality<T: CardinalityValue>(
    params: &ParamSet,
    items: Rc<Vec<T>>,
    passes: usize,
) -> Result<Vec<Pass>, BuildError> {
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        // Built here, so calling the closure is the insert and nothing else.
        let mut sketch = build_polars_cardinality::<T>(params)?;
        let items = items.clone();
        out.push(Box::new(move || {
            for v in items.iter() {
                sketch.buffered_heap_bytes += v.heap_bytes();
                sketch.buf.push(v.clone());
            }
            memory_polars_cardinality(&sketch)
        }) as Pass);
    }
    Ok(out)
}

pub fn insert_step_polars_cardinality<T: CardinalityValue>(
    params: &ParamSet,
    items: Rc<Vec<T>>,
    passes: usize,
) -> Result<Vec<StepPass>, BuildError> {
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        let sketch: Shared<_> = Rc::new(RefCell::new(build_polars_cardinality::<T>(params)?));
        let (driven, read) = (sketch.clone(), sketch);
        let stream = items.clone();
        out.push(StepPass {
            steps: items.len(),
            step: Box::new(move |i| {
                let sketch = &mut *driven.borrow_mut();
                let v = &stream[i];
                sketch.buffered_heap_bytes += v.heap_bytes();
                sketch.buf.push(v.clone());
            }),
            footprint: Box::new(move || memory_polars_cardinality(&read.borrow())),
        });
    }
    Ok(out)
}

pub fn query_polars_cardinality<T: CardinalityValue>(
    params: &ParamSet,
    items: Rc<Vec<T>>,
    probes: Rc<Vec<()>>,
    passes: usize,
) -> Result<Vec<QueryPass<f64>>, BuildError> {
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        // Built and fed here: the closure below asks, and only asks.
        let mut sketch = build_polars_cardinality::<T>(params)?;
        for v in items.iter() {
            sketch.buffered_heap_bytes += v.heap_bytes();
            sketch.buf.push(v.clone());
        }
        sketch.finalize();
        let probes = probes.clone();
        out.push(Box::new(move || {
            let mut answers = Vec::with_capacity(probes.len());
            for _p in probes.iter() {
                answers.push(sketch.estimate);
            }
            let footprint = memory_polars_cardinality(&sketch);
            (answers, footprint)
        }) as QueryPass<f64>);
    }
    Ok(out)
}

pub fn prepare_polars_cardinality<T: CardinalityValue>(
    params: &ParamSet,
    items: Rc<Vec<T>>,
    passes: usize,
) -> Result<Vec<Pass>, BuildError> {
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        // Fed here: the closure below is the step that makes it ready to answer.
        let mut sketch = build_polars_cardinality::<T>(params)?;
        for v in items.iter() {
            sketch.buffered_heap_bytes += v.heap_bytes();
            sketch.buf.push(v.clone());
        }
        out.push(Box::new(move || {
            sketch.finalize();
            memory_polars_cardinality(&sketch)
        }) as Pass);
    }
    Ok(out)
}
