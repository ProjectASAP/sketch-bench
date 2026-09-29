//! The `sketch_oxide` implementations.
//!
//! Grouped under `wrappers/cms/` with the other implementations of this
//! algorithm; how each is driven lives beside it.

use super::*;
use crate::params::ParamSet;
use crate::wrappers::{partition, require_resolved_shape};
use crate::wrappers::{BuildError, Pass, QueryPass, Shared, StepPass};
use std::cell::RefCell;
use std::rc::Rc;

use crate::wrappers::frequency_value::FrequencyValue;

// No `rows` / `cols` field: `init` has already proven the built table matches
// the request, so the sketch itself is the one place either figure is read
// from, and a library upgrade that changed the rounding cannot slip past.
pub struct CmsOxide {
    inner: sketch_oxide::frequency::CountMinSketch,
}

pub fn build_cms_oxide(config: &ParamSet) -> Result<CmsOxide, BuildError> {
    let p: CmsParams = config.parse()?;
    // Native API takes an error bound, not raw dimensions — translate.
    let (epsilon, delta) = dims_to_err(p.rows, p.cols);
    let inner = sketch_oxide::frequency::CountMinSketch::new(epsilon, delta)
        .map_err(|e| BuildError(format!("oxide CMS rejected ε={epsilon} δ={delta}: {e:?}")))?;
    require_resolved_shape(
        "oxide CMS",
        (inner.depth(), inner.width()),
        (p.rows, p.cols),
    )?;
    Ok(CmsOxide { inner })
}

pub fn memory_cms_oxide(sketch: &CmsOxide) -> usize {
    // Read off the built sketch, not the requested `(rows, cols)`: the crate
    // rounds width up to a power of two, so `cols = 3000` allocates 4096. The
    // counters are `table: Vec<u64>`, not 32-bit — size them as `u64`.
    sketch.inner.depth() * sketch.inner.width() * std::mem::size_of::<u64>()
}

pub fn insert_cms_oxide<T: FrequencyValue>(
    params: &ParamSet,
    items: Rc<Vec<T>>,
    passes: usize,
) -> Result<Vec<Pass>, BuildError> {
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        // Built here, so calling the closure is the insert and nothing else.
        let mut sketch = build_cms_oxide(params)?;
        let items = items.clone();
        out.push(Box::new(move || {
            for v in items.iter() {
                sketch.inner.update(&v.hash_key());
            }
            memory_cms_oxide(&sketch)
        }) as Pass);
    }
    Ok(out)
}

pub fn insert_step_cms_oxide<T: FrequencyValue>(
    params: &ParamSet,
    items: Rc<Vec<T>>,
    passes: usize,
) -> Result<Vec<StepPass>, BuildError> {
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        let sketch: Shared<_> = Rc::new(RefCell::new(build_cms_oxide(params)?));
        let (driven, read) = (sketch.clone(), sketch);
        let stream = items.clone();
        out.push(StepPass {
            steps: items.len(),
            step: Box::new(move |i| {
                let sketch = &mut *driven.borrow_mut();
                let v = &stream[i];
                sketch.inner.update(&v.hash_key());
            }),
            footprint: Box::new(move || memory_cms_oxide(&read.borrow())),
        });
    }
    Ok(out)
}

pub fn query_cms_oxide<T: FrequencyValue>(
    params: &ParamSet,
    items: Rc<Vec<T>>,
    probes: Rc<Vec<T>>,
    passes: usize,
) -> Result<Vec<QueryPass<u64>>, BuildError> {
    merge_query_cms_oxide(params, items, probes, 1, passes)
}

/// The query, asked of the sketch a fold over `shards` shards leaves. One
/// shard is the plain query.
pub fn merge_query_cms_oxide<T: FrequencyValue>(
    params: &ParamSet,
    items: Rc<Vec<T>>,
    probes: Rc<Vec<T>>,
    shards: usize,
    passes: usize,
) -> Result<Vec<QueryPass<u64>>, BuildError> {
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        // Built, fed and folded here: the closure below asks, and only asks.
        let (mut sketch, rest) = cms_oxide_shards(params, &items, shards)?;
        for other in rest.iter() {
            sketch
                .inner
                .merge(&other.inner)
                .expect("both operands built from one ParamSet, so rows/cols match");
        }
        let probes = probes.clone();
        out.push(Box::new(move || {
            let mut answers = Vec::with_capacity(probes.len());
            for p in probes.iter() {
                answers.push(sketch.inner.estimate(&p.hash_key()));
            }
            let footprint = memory_cms_oxide(&sketch);
            (answers, footprint)
        }) as QueryPass<u64>);
    }
    Ok(out)
}

pub fn merge_cms_oxide<T: FrequencyValue>(
    params: &ParamSet,
    items: Rc<Vec<T>>,
    shards: usize,
    passes: usize,
) -> Result<Vec<Pass>, BuildError> {
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        let (mut acc, rest) = cms_oxide_shards(params, &items, shards)?;
        out.push(Box::new(move || {
            for other in rest.iter() {
                acc.inner
                    .merge(&other.inner)
                    .expect("both operands built from one ParamSet, so rows/cols match");
            }
            memory_cms_oxide(&acc)
        }) as Pass);
    }
    Ok(out)
}

pub fn merge_step_cms_oxide<T: FrequencyValue>(
    params: &ParamSet,
    items: Rc<Vec<T>>,
    shards: usize,
    passes: usize,
) -> Result<Vec<StepPass>, BuildError> {
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        let (acc, rest) = cms_oxide_shards(params, &items, shards)?;
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
            footprint: Box::new(move || memory_cms_oxide(&read.borrow())),
        });
    }
    Ok(out)
}

/// The shards a fold folds: the stream split `shards` ways, one sketch each,
/// all fed. The first is the accumulator, the rest are what it folds in.
#[allow(clippy::type_complexity)]
fn cms_oxide_shards<T: FrequencyValue>(
    params: &ParamSet,
    items: &[T],
    shards: usize,
) -> Result<(CmsOxide, Vec<CmsOxide>), BuildError> {
    let mut parts: Vec<CmsOxide> = Vec::new();
    for shard in partition(items, shards) {
        let mut sketch = build_cms_oxide(params)?;
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
