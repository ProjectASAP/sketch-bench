//! The `datasketches` (Apache DataSketches) implementations.
//!
//! Grouped under `wrappers/cms/` with the other implementations of this
//! algorithm; how each is driven lives beside it.

use super::*;
use crate::build_error::BuildError;
use crate::wrappers::{require_range, require_resolved_shape};
use aqpbm_core::config::ParamSet;

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

pub fn build_cms_datasketches(
    config: &ParamSet,
    _workers: usize,
) -> Result<CmsDatasketches, BuildError> {
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
    let entries = p.rows.checked_mul(p.cols).unwrap_or(usize::MAX);
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

impl CmsDatasketches {
    pub fn estimate_frequency(&self, key: &i64) -> u64 {
        self.inner.estimate(*key).max(0) as u64
    }
}

pub fn insert_cms_datasketches(sketch: &mut CmsDatasketches, v: &i64) {
    sketch.inner.update(*v);
}

pub fn merge_cms_datasketches(into: &mut CmsDatasketches, from: &CmsDatasketches) {
    into.inner.merge(&from.inner);
}

pub fn ask_cms_datasketches(sketch: &mut CmsDatasketches, key: &i64) -> u64 {
    sketch.estimate_frequency(key)
}
