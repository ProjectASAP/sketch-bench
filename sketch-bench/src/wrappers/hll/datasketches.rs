//! The `datasketches` (Apache DataSketches) implementations.
//!
//! Grouped under `wrappers/hll/` with the other implementations of this
//! algorithm; how each is driven lives beside it.

use super::*;
use crate::params::ParamSet;
use crate::wrappers::{partition, require_range};
use crate::wrappers::{BuildError, Pass, QueryPass, Shared, StepPass};
use std::cell::RefCell;
use std::rc::Rc;

// `merge` lives on sketch_oxide's `Mergeable`, not on its `Accumulator`.

// ---------- datasketches HLL ----------
pub struct HllDatasketches {
    inner: ::datasketches::hll::HllSketch,
    lg_k: u8,
    hll_type: ::datasketches::hll::HllType,
}

/// What `::datasketches::hll::HllSketch::new` accepts. It asserts rather than
/// returning an error, so the bound has to be stated on this side: the twin of
/// [`LIB_PRECISIONS`], for the library that expresses its domain as a range
/// instead of a set of types.
pub const DS_LG_K: (u8, u8) = (4, 21);

pub fn build_hll_datasketches(config: &ParamSet) -> Result<HllDatasketches, BuildError> {
    let p: HllParams = config.parse()?;
    require_range("datasketches HLL", "lg_k", p.lg_k, DS_LG_K.0, DS_LG_K.1)?;
    let hll_type = ::datasketches::hll::HllType::Hll8;
    Ok(HllDatasketches {
        inner: ::datasketches::hll::HllSketch::new(p.lg_k, hll_type),
        lg_k: p.lg_k,
        hll_type,
    })
}

pub fn memory_hll_datasketches(sketch: &HllDatasketches) -> usize {
    // Apache datasketches packs registers per HllType:
    // Hll4 → 0.5 B, Hll6 → 0.75 B, Hll8 → 1 B.
    let m = 1usize << sketch.lg_k;
    match sketch.hll_type {
        ::datasketches::hll::HllType::Hll4 => m / 2,
        ::datasketches::hll::HllType::Hll6 => (m * 6).div_ceil(8),
        ::datasketches::hll::HllType::Hll8 => m,
    }
}

pub fn insert_hll_datasketches(
    params: &ParamSet,
    items: Rc<Vec<i64>>,
    passes: usize,
) -> Result<Vec<Pass>, BuildError> {
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        // Built here, so calling the closure is the insert and nothing else.
        let mut sketch = build_hll_datasketches(params)?;
        let items = items.clone();
        out.push(Box::new(move || {
            for v in items.iter() {
                sketch.inner.update(*v);
            }
            memory_hll_datasketches(&sketch)
        }) as Pass);
    }
    Ok(out)
}

pub fn insert_step_hll_datasketches(
    params: &ParamSet,
    items: Rc<Vec<i64>>,
    passes: usize,
) -> Result<Vec<StepPass>, BuildError> {
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        let sketch: Shared<_> = Rc::new(RefCell::new(build_hll_datasketches(params)?));
        let (driven, read) = (sketch.clone(), sketch);
        let stream = items.clone();
        out.push(StepPass {
            steps: items.len(),
            step: Box::new(move |i| {
                let sketch = &mut *driven.borrow_mut();
                let v = &stream[i];
                sketch.inner.update(*v);
            }),
            footprint: Box::new(move || memory_hll_datasketches(&read.borrow())),
        });
    }
    Ok(out)
}

pub fn query_hll_datasketches(
    params: &ParamSet,
    items: Rc<Vec<i64>>,
    probes: Rc<Vec<()>>,
    passes: usize,
) -> Result<Vec<QueryPass<f64>>, BuildError> {
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        // Built and fed here: the closure below asks, and only asks.
        let mut sketch = build_hll_datasketches(params)?;
        for v in items.iter() {
            sketch.inner.update(*v);
        }
        let probes = probes.clone();
        out.push(Box::new(move || {
            let mut answers = Vec::with_capacity(probes.len());
            for _p in probes.iter() {
                answers.push(sketch.inner.estimate());
            }
            let footprint = memory_hll_datasketches(&sketch);
            (answers, footprint)
        }) as QueryPass<f64>);
    }
    Ok(out)
}

pub fn merge_hll_datasketches(
    params: &ParamSet,
    items: Rc<Vec<i64>>,
    shards: usize,
    passes: usize,
) -> Result<Vec<Pass>, BuildError> {
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        let (mut acc, rest) = hll_datasketches_shards(params, &items, shards)?;
        out.push(Box::new(move || {
            for other in rest.iter() {
                let mut union = ::datasketches::hll::HllUnion::new(acc.lg_k);
                union.update(&acc.inner);
                union.update(&other.inner);
                acc.inner = union.get_result(acc.hll_type);
            }
            memory_hll_datasketches(&acc)
        }) as Pass);
    }
    Ok(out)
}

pub fn merge_step_hll_datasketches(
    params: &ParamSet,
    items: Rc<Vec<i64>>,
    shards: usize,
    passes: usize,
) -> Result<Vec<StepPass>, BuildError> {
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        let (acc, rest) = hll_datasketches_shards(params, &items, shards)?;
        let acc: Shared<_> = Rc::new(RefCell::new(acc));
        let (driven, read) = (acc.clone(), acc);
        out.push(StepPass {
            steps: rest.len(),
            step: Box::new(move |i| {
                let acc = &mut *driven.borrow_mut();
                let other = &rest[i];
                let mut union = ::datasketches::hll::HllUnion::new(acc.lg_k);
                union.update(&acc.inner);
                union.update(&other.inner);
                acc.inner = union.get_result(acc.hll_type);
            }),
            footprint: Box::new(move || memory_hll_datasketches(&read.borrow())),
        });
    }
    Ok(out)
}

/// The shards a fold folds: the stream split `shards` ways, one sketch each,
/// all fed. The first is the accumulator, the rest are what it folds in.
#[allow(clippy::type_complexity)]
fn hll_datasketches_shards(
    params: &ParamSet,
    items: &[i64],
    shards: usize,
) -> Result<(HllDatasketches, Vec<HllDatasketches>), BuildError> {
    let mut parts: Vec<HllDatasketches> = Vec::new();
    for shard in partition(items, shards) {
        let mut sketch = build_hll_datasketches(params)?;
        for v in shard {
            sketch.inner.update(*v);
        }
        parts.push(sketch);
    }
    let rest = parts.split_off(1);
    Ok((
        parts.pop().expect("a split of the stream is never empty"),
        rest,
    ))
}
