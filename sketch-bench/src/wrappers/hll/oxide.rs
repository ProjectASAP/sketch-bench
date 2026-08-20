//! The `sketch_oxide` implementations.
//!
//! Grouped under `wrappers/hll/` with the other implementations of this
//! algorithm; how each is driven lives beside it.

use super::*;
use crate::params::ParamSet;
use crate::wrappers::partition;
use crate::wrappers::{BuildError, Pass, QueryPass, Shared, StepPass};
use std::cell::RefCell;
use std::rc::Rc;

use sketch_oxide::Sketch as OxideSketch;

// `merge` lives on sketch_oxide's `Mergeable`, not on its `Accumulator`.

// ---------- sketch_oxide HLL ----------
pub struct HllOxide {
    inner: sketch_oxide::cardinality::HyperLogLog,
    lg_k: u8,
}

pub fn build_hll_oxide(config: &ParamSet) -> Result<HllOxide, BuildError> {
    let p: HllParams = config.parse()?;
    let inner = sketch_oxide::cardinality::HyperLogLog::new(p.lg_k)
        .map_err(|e| BuildError(format!("oxide HLL rejected lg_k={}: {e:?}", p.lg_k)))?;
    Ok(HllOxide {
        inner,
        lg_k: p.lg_k,
    })
}

pub fn memory_hll_oxide(sketch: &HllOxide) -> usize {
    // sketch_oxide stores registers as Vec<u8>: 1 byte/register.
    1usize << sketch.lg_k
}

pub fn insert_hll_oxide<T: CardinalityValue>(
    params: &ParamSet,
    items: Rc<Vec<T>>,
    passes: usize,
) -> Result<Vec<Pass>, BuildError> {
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        // Built here, so calling the closure is the insert and nothing else.
        let mut sketch = build_hll_oxide(params)?;
        let items = items.clone();
        out.push(Box::new(move || {
            for v in items.iter() {
                sketch.inner.update(&v.hash_key());
            }
            memory_hll_oxide(&sketch)
        }) as Pass);
    }
    Ok(out)
}

pub fn insert_step_hll_oxide<T: CardinalityValue>(
    params: &ParamSet,
    items: Rc<Vec<T>>,
    passes: usize,
) -> Result<Vec<StepPass>, BuildError> {
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        let sketch: Shared<_> = Rc::new(RefCell::new(build_hll_oxide(params)?));
        let (driven, read) = (sketch.clone(), sketch);
        let stream = items.clone();
        out.push(StepPass {
            steps: items.len(),
            step: Box::new(move |i| {
                let sketch = &mut *driven.borrow_mut();
                let v = &stream[i];
                sketch.inner.update(&v.hash_key());
            }),
            footprint: Box::new(move || memory_hll_oxide(&read.borrow())),
        });
    }
    Ok(out)
}

pub fn query_hll_oxide<T: CardinalityValue>(
    params: &ParamSet,
    items: Rc<Vec<T>>,
    probes: Rc<Vec<()>>,
    passes: usize,
) -> Result<Vec<QueryPass<f64>>, BuildError> {
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        // Built and fed here: the closure below asks, and only asks.
        let mut sketch = build_hll_oxide(params)?;
        for v in items.iter() {
            sketch.inner.update(&v.hash_key());
        }
        let probes = probes.clone();
        out.push(Box::new(move || {
            let mut answers = Vec::with_capacity(probes.len());
            for _p in probes.iter() {
                answers.push(sketch.inner.estimate());
            }
            let footprint = memory_hll_oxide(&sketch);
            (answers, footprint)
        }) as QueryPass<f64>);
    }
    Ok(out)
}

pub fn merge_hll_oxide<T: CardinalityValue>(
    params: &ParamSet,
    items: Rc<Vec<T>>,
    shards: usize,
    passes: usize,
) -> Result<Vec<Pass>, BuildError> {
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        let (mut acc, rest) = hll_oxide_shards(params, &items, shards)?;
        out.push(Box::new(move || {
            for other in rest.iter() {
                acc.inner
                    .merge(&other.inner)
                    .expect("both operands built from one ParamSet, so lg_k matches");
            }
            memory_hll_oxide(&acc)
        }) as Pass);
    }
    Ok(out)
}

pub fn merge_step_hll_oxide<T: CardinalityValue>(
    params: &ParamSet,
    items: Rc<Vec<T>>,
    shards: usize,
    passes: usize,
) -> Result<Vec<StepPass>, BuildError> {
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        let (acc, rest) = hll_oxide_shards(params, &items, shards)?;
        let acc: Shared<_> = Rc::new(RefCell::new(acc));
        let (driven, read) = (acc.clone(), acc);
        out.push(StepPass {
            steps: rest.len(),
            step: Box::new(move |i| {
                let acc = &mut *driven.borrow_mut();
                let other = &rest[i];
                acc.inner
                    .merge(&other.inner)
                    .expect("both operands built from one ParamSet, so lg_k matches");
            }),
            footprint: Box::new(move || memory_hll_oxide(&read.borrow())),
        });
    }
    Ok(out)
}

/// The shards a fold folds: the stream split `shards` ways, one sketch each,
/// all fed. The first is the accumulator, the rest are what it folds in.
#[allow(clippy::type_complexity)]
fn hll_oxide_shards<T: CardinalityValue>(
    params: &ParamSet,
    items: &[T],
    shards: usize,
) -> Result<(HllOxide, Vec<HllOxide>), BuildError> {
    let mut parts: Vec<HllOxide> = Vec::new();
    for shard in partition(items, shards) {
        let mut sketch = build_hll_oxide(params)?;
        for v in shard {
            sketch.inner.update(&v.hash_key());
        }
        parts.push(sketch);
    }
    let rest = parts.split_off(1);
    Ok((
        parts.pop().expect("a split of the stream is never empty"),
        rest,
    ))
}
