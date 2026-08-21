//! The `datasketches` (Apache DataSketches) implementations.
//!
//! Grouped under `wrappers/cms/` with the other implementations of this
//! algorithm; how each is driven lives beside it.

use super::*;
use crate::params::ParamSet;
use crate::wrappers::{partition, require_range, require_resolved_shape};
use crate::wrappers::{BuildError, Pass, QueryPass, Shared, StepPass};
use std::cell::RefCell;
use std::rc::Rc;

use crate::wrappers::frequency_value::FrequencyValue;

/// asserts are in `countmin/sketch.rs::entries_for_config`; the row bound is
/// the `u8` the API takes.
const DS_CMS_ROWS: (usize, usize) = (1, u8::MAX as usize);

const DS_CMS_COLS: (usize, usize) = (3, u32::MAX as usize);

/// `num_hashes * num_buckets < MAX_TABLE_ENTRIES`. A product bound, so no
/// per-parameter range catches it.
const DS_CMS_MAX_ENTRIES: usize = 1 << 30;

pub struct CmsDatasketches {
    inner: ::datasketches::countmin::CountMinSketch,
    rows: usize,
    cols: usize,
}

pub fn build_cms_datasketches(config: &ParamSet) -> Result<CmsDatasketches, BuildError> {
    let p: CmsParams = config.parse()?;
    require_range(
        "datasketches CMS",
        "rows",
        p.rows,
        DS_CMS_ROWS.0,
        DS_CMS_ROWS.1,
    )?;
    require_range(
        "datasketches CMS",
        "cols",
        p.cols,
        DS_CMS_COLS.0,
        DS_CMS_COLS.1,
    )?;
    // Checked, because the point of the bound is that the product is what
    // overflows: `usize::MAX` rows-worth of columns must not wrap into a
    // small number that passes.
    let entries = p.rows.saturating_mul(p.cols);
    if entries >= DS_CMS_MAX_ENTRIES {
        return Err(BuildError(format!(
            "datasketches CMS: rows x cols = {entries} counters, and this library \
                 caps a table at {DS_CMS_MAX_ENTRIES}"
        )));
    }
    // The ranges above make the casts lossless; this proves it against the
    // built sketch rather than against that reasoning, so a library that
    // starts rounding its dimensions turns into a refusal here instead of a
    // silently different table. Same guard the oxide row uses.
    let inner = ::datasketches::countmin::CountMinSketch::new(p.rows as u8, p.cols as u32);
    require_resolved_shape(
        "datasketches CMS",
        (inner.num_hashes() as usize, inner.num_buckets() as usize),
        (p.rows, p.cols),
    )?;
    Ok(CmsDatasketches {
        inner,
        rows: p.rows,
        cols: p.cols,
    })
}

pub fn memory_cms_datasketches(sketch: &CmsDatasketches) -> usize {
    // Backing store is `counts: Vec<i64>`; spell the real type so the two
    // stay in step.
    sketch.rows * sketch.cols * std::mem::size_of::<i64>()
}

pub fn insert_cms_datasketches<T: FrequencyValue>(
    params: &ParamSet,
    items: Rc<Vec<T>>,
    passes: usize,
) -> Result<Vec<Pass>, BuildError> {
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        // Built here, so calling the closure is the insert and nothing else.
        let mut sketch = build_cms_datasketches(params)?;
        let items = items.clone();
        out.push(Box::new(move || {
            for v in items.iter() {
                sketch.inner.update(v.hash_key());
            }
            memory_cms_datasketches(&sketch)
        }) as Pass);
    }
    Ok(out)
}

pub fn insert_step_cms_datasketches<T: FrequencyValue>(
    params: &ParamSet,
    items: Rc<Vec<T>>,
    passes: usize,
) -> Result<Vec<StepPass>, BuildError> {
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        let sketch: Shared<_> = Rc::new(RefCell::new(build_cms_datasketches(params)?));
        let (driven, read) = (sketch.clone(), sketch);
        let stream = items.clone();
        out.push(StepPass {
            steps: items.len(),
            step: Box::new(move |i| {
                let sketch = &mut *driven.borrow_mut();
                let v = &stream[i];
                sketch.inner.update(v.hash_key());
            }),
            footprint: Box::new(move || memory_cms_datasketches(&read.borrow())),
        });
    }
    Ok(out)
}

pub fn query_cms_datasketches<T: FrequencyValue>(
    params: &ParamSet,
    items: Rc<Vec<T>>,
    probes: Rc<Vec<T>>,
    passes: usize,
) -> Result<Vec<QueryPass<u64>>, BuildError> {
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        // Built and fed here: the closure below asks, and only asks.
        let mut sketch = build_cms_datasketches(params)?;
        for v in items.iter() {
            sketch.inner.update(v.hash_key());
        }
        let probes = probes.clone();
        out.push(Box::new(move || {
            let mut answers = Vec::with_capacity(probes.len());
            for p in probes.iter() {
                answers.push(sketch.inner.estimate(p.hash_key()).max(0) as u64);
            }
            let footprint = memory_cms_datasketches(&sketch);
            (answers, footprint)
        }) as QueryPass<u64>);
    }
    Ok(out)
}

pub fn merge_cms_datasketches<T: FrequencyValue>(
    params: &ParamSet,
    items: Rc<Vec<T>>,
    shards: usize,
    passes: usize,
) -> Result<Vec<Pass>, BuildError> {
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        let (mut acc, rest) = cms_datasketches_shards(params, &items, shards)?;
        out.push(Box::new(move || {
            for other in rest.iter() {
                acc.inner.merge(&other.inner);
            }
            memory_cms_datasketches(&acc)
        }) as Pass);
    }
    Ok(out)
}

pub fn merge_step_cms_datasketches<T: FrequencyValue>(
    params: &ParamSet,
    items: Rc<Vec<T>>,
    shards: usize,
    passes: usize,
) -> Result<Vec<StepPass>, BuildError> {
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        let (acc, rest) = cms_datasketches_shards(params, &items, shards)?;
        let acc: Shared<_> = Rc::new(RefCell::new(acc));
        let (driven, read) = (acc.clone(), acc);
        out.push(StepPass {
            steps: rest.len(),
            step: Box::new(move |i| {
                let acc = &mut *driven.borrow_mut();
                let other = &rest[i];
                acc.inner.merge(&other.inner);
            }),
            footprint: Box::new(move || memory_cms_datasketches(&read.borrow())),
        });
    }
    Ok(out)
}

/// The shards a fold folds: the stream split `shards` ways, one sketch each,
/// all fed. The first is the accumulator, the rest are what it folds in.
#[allow(clippy::type_complexity)]
fn cms_datasketches_shards<T: FrequencyValue>(
    params: &ParamSet,
    items: &[T],
    shards: usize,
) -> Result<(CmsDatasketches, Vec<CmsDatasketches>), BuildError> {
    let mut parts: Vec<CmsDatasketches> = Vec::new();
    for shard in partition(items, shards) {
        let mut sketch = build_cms_datasketches(params)?;
        for v in shard {
            sketch.inner.update(v.hash_key());
        }
        parts.push(sketch);
    }
    let rest = parts.split_off(1);
    Ok((
        parts.pop().expect("a split of the stream is never empty"),
        rest,
    ))
}
