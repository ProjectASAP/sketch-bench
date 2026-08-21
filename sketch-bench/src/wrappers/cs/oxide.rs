//! The `sketch_oxide` implementations.
//!
//! Grouped under `wrappers/cs/` with the other implementations of this
//! algorithm; how each is driven lives beside it.

use super::*;
use crate::params::ParamSet;
use crate::wrappers::{partition, require_resolved_shape};
use crate::wrappers::{BuildError, Pass, QueryPass, Shared, StepPass};
use std::cell::RefCell;
use std::rc::Rc;

use crate::wrappers::frequency_value::FrequencyValue;

// No `rows` / `cols` field, as in `CmsOxide`: `init` proves the built table
// matches the request, so the sketch is the only place either is read from.
// One extra bound — the crate floors depth at 3, so `rows < 3` is refused.
pub struct CsOxide {
    inner: sketch_oxide::frequency::CountSketch,
}

pub fn build_cs_oxide(config: &ParamSet) -> Result<CsOxide, BuildError> {
    let p: CountSketchParams = config.parse()?;
    let (epsilon, delta) = dims_to_err(p.rows, p.cols);
    let inner = sketch_oxide::frequency::CountSketch::new(epsilon, delta).map_err(|e| {
        BuildError(format!(
            "oxide CountSketch rejected ε={epsilon} δ={delta}: {e:?}"
        ))
    })?;
    require_resolved_shape(
        "oxide CountSketch",
        (inner.depth(), inner.width()),
        (p.rows, p.cols),
    )?;
    Ok(CsOxide { inner })
}

pub fn memory_cs_oxide(sketch: &CsOxide) -> usize {
    // Off the built sketch: the crate rounds the width up to a power of two
    // and floors the depth at 3, so at `rows=2` a request-derived figure
    // under-reports by a third. Backing store is `table: Vec<i64>`.
    sketch.inner.depth() * sketch.inner.width() * std::mem::size_of::<i64>()
}

pub fn insert_cs_oxide<T: FrequencyValue>(
    params: &ParamSet,
    items: Rc<Vec<T>>,
    passes: usize,
) -> Result<Vec<Pass>, BuildError> {
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        // Built here, so calling the closure is the insert and nothing else.
        let mut sketch = build_cs_oxide(params)?;
        let items = items.clone();
        out.push(Box::new(move || {
            for v in items.iter() {
                sketch.inner.update(&v.hash_key(), 1);
            }
            memory_cs_oxide(&sketch)
        }) as Pass);
    }
    Ok(out)
}

pub fn insert_step_cs_oxide<T: FrequencyValue>(
    params: &ParamSet,
    items: Rc<Vec<T>>,
    passes: usize,
) -> Result<Vec<StepPass>, BuildError> {
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        let sketch: Shared<_> = Rc::new(RefCell::new(build_cs_oxide(params)?));
        let (driven, read) = (sketch.clone(), sketch);
        let stream = items.clone();
        out.push(StepPass {
            steps: items.len(),
            step: Box::new(move |i| {
                let sketch = &mut *driven.borrow_mut();
                let v = &stream[i];
                sketch.inner.update(&v.hash_key(), 1);
            }),
            footprint: Box::new(move || memory_cs_oxide(&read.borrow())),
        });
    }
    Ok(out)
}

pub fn query_cs_oxide<T: FrequencyValue>(
    params: &ParamSet,
    items: Rc<Vec<T>>,
    probes: Rc<Vec<T>>,
    passes: usize,
) -> Result<Vec<QueryPass<u64>>, BuildError> {
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        // Built and fed here: the closure below asks, and only asks.
        let mut sketch = build_cs_oxide(params)?;
        for v in items.iter() {
            sketch.inner.update(&v.hash_key(), 1);
        }
        let probes = probes.clone();
        out.push(Box::new(move || {
            let mut answers = Vec::with_capacity(probes.len());
            for p in probes.iter() {
                answers.push(sketch.inner.estimate(&p.hash_key()).max(0) as u64);
            }
            let footprint = memory_cs_oxide(&sketch);
            (answers, footprint)
        }) as QueryPass<u64>);
    }
    Ok(out)
}

pub fn merge_cs_oxide<T: FrequencyValue>(
    params: &ParamSet,
    items: Rc<Vec<T>>,
    shards: usize,
    passes: usize,
) -> Result<Vec<Pass>, BuildError> {
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        let (mut acc, rest) = cs_oxide_shards(params, &items, shards)?;
        out.push(Box::new(move || {
            for other in rest.iter() {
                acc.inner
                    .merge(&other.inner)
                    .expect("both operands built from one ParamSet, so rows/cols match");
            }
            memory_cs_oxide(&acc)
        }) as Pass);
    }
    Ok(out)
}

pub fn merge_step_cs_oxide<T: FrequencyValue>(
    params: &ParamSet,
    items: Rc<Vec<T>>,
    shards: usize,
    passes: usize,
) -> Result<Vec<StepPass>, BuildError> {
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        let (acc, rest) = cs_oxide_shards(params, &items, shards)?;
        let acc: Shared<_> = Rc::new(RefCell::new(acc));
        let (driven, read) = (acc.clone(), acc);
        out.push(StepPass {
            steps: rest.len(),
            step: Box::new(move |i| {
                let acc = &mut *driven.borrow_mut();
                let other = &rest[i];
                acc.inner
                    .merge(&other.inner)
                    .expect("both operands built from one ParamSet, so rows/cols match");
            }),
            footprint: Box::new(move || memory_cs_oxide(&read.borrow())),
        });
    }
    Ok(out)
}

/// The shards a fold folds: the stream split `shards` ways, one sketch each,
/// all fed. The first is the accumulator, the rest are what it folds in.
#[allow(clippy::type_complexity)]
fn cs_oxide_shards<T: FrequencyValue>(
    params: &ParamSet,
    items: &[T],
    shards: usize,
) -> Result<(CsOxide, Vec<CsOxide>), BuildError> {
    let mut parts: Vec<CsOxide> = Vec::new();
    for shard in partition(items, shards) {
        let mut sketch = build_cs_oxide(params)?;
        for v in shard {
            sketch.inner.update(&v.hash_key(), 1);
        }
        parts.push(sketch);
    }
    let rest = parts.split_off(1);
    Ok((
        parts.pop().expect("a split of the stream is never empty"),
        rest,
    ))
}
