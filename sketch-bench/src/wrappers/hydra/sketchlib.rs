//! The `asap_sketchlib` implementations.
//!
//! Grouped under `wrappers/hydra/` with the other implementations of this
//! algorithm; how each is driven lives beside it.

use super::*;
use crate::params::ParamSet;
use crate::wrappers::partition;
use crate::wrappers::{BuildError, Pass, QueryPass, Shared, StepPass};
use asap_sketchlib::input::{HydraCounter, HydraQuery};
use asap_sketchlib::{CountMin, DataInput, FastPath, Hydra, HyperLogLog, Vector2D, KLL};
use std::cell::RefCell;
use std::rc::Rc;

pub struct HydraCms {
    pub(super) inner: Hydra,
    params: HydraCmsParams,
}

pub fn build_hydra_cms(config: &ParamSet) -> Result<HydraCms, BuildError> {
    let p: HydraCmsParams = config.parse()?;
    check_grid(p.rows, p.cols, "hydra-cms")?;
    for (name, v) in [("cell_rows", p.cell_rows), ("cell_cols", p.cell_cols)] {
        if v == 0 {
            return Err(BuildError(format!("hydra-cms: {name} must be > 0")));
        }
    }
    let cell = HydraCounter::CM(CountMin::<Vector2D<i32>, FastPath>::with_dimensions(
        p.cell_rows,
        p.cell_cols,
    ));
    Ok(HydraCms {
        inner: Hydra::with_dimensions(p.rows, p.cols, cell),
        params: p,
    })
}

impl HydraCms {
    #[inline]
    pub fn estimate_subpop_frequency(&self, labels: &[&str], value: &i64) -> f64 {
        self.inner
            .query_frequency(labels.to_vec(), &DataInput::I64(*value))
    }
}

/// The grid holds `rows * cols` cells and every cell is a full Count-Min of
/// `i32` counters, so the counter term is the product of both shapes.
pub fn memory_hydra_cms(sketch: &HydraCms) -> usize {
    let p = &sketch.params;
    p.rows * p.cols * p.cell_rows * p.cell_cols * std::mem::size_of::<i32>()
        + grid_overhead_bytes(p.rows, p.cols)
}

/// Hydra over HyperLogLog cells.
pub struct HydraHll {
    pub(super) inner: Hydra,
    params: HydraHllParams,
}

pub fn build_hydra_hll(config: &ParamSet) -> Result<HydraHll, BuildError> {
    let p: HydraHllParams = config.parse()?;
    check_grid(p.rows, p.cols, "hydra-hll")?;
    // Named through the `ErtlMLE` impl explicitly: `HyperLogLog` is a type
    // alias over the variant, so `new()` is ambiguous between the
    // estimators the alias can carry. The enum fixes this one.
    let cell = HydraCounter::HLL(HyperLogLog::<asap_sketchlib::ErtlMLE>::new());
    Ok(HydraHll {
        inner: Hydra::with_dimensions(p.rows, p.cols, cell),
        params: p,
    })
}

impl HydraHll {
    #[inline]
    pub fn estimate_subpop_cardinality(&self, labels: &[&str]) -> f64 {
        self.inner
            .query_key(labels.to_vec(), &HydraQuery::Cardinality)
    }
}

/// One byte per register per cell. The cell is fixed-shape, so unlike the
/// Count-Min row there is no cell parameter in this product.
pub fn memory_hydra_hll(sketch: &HydraHll) -> usize {
    let p = &sketch.params;
    p.rows * p.cols * HLL_CELL_REGISTERS + grid_overhead_bytes(p.rows, p.cols)
}

/// Hydra over KLL cells.
pub struct HydraKll {
    pub(super) inner: Hydra,
    params: HydraKllParams,
}

pub fn build_hydra_kll(config: &ParamSet) -> Result<HydraKll, BuildError> {
    let p: HydraKllParams = config.parse()?;
    check_grid(p.rows, p.cols, "hydra-kll")?;
    // The cell is the same `asap_sketchlib::KLL` the `kll-*` rows hold, and it
    // clamps `k` to its own range without saying so. Refuse for the same reason
    // those rows do: the grid would be built at a `cell_k` the record misnames.
    if !(crate::wrappers::kll::LIB_K_MIN..=crate::wrappers::kll::LIB_K_MAX).contains(&p.cell_k) {
        return Err(BuildError(format!(
            "hydra-kll: cell_k={} outside [{}, {}]; the library clamps to that range",
            p.cell_k,
            crate::wrappers::kll::LIB_K_MIN,
            crate::wrappers::kll::LIB_K_MAX
        )));
    }
    let cell = HydraCounter::KLL(KLL::init_kll(p.cell_k as i32));
    Ok(HydraKll {
        inner: Hydra::with_dimensions(p.rows, p.cols, cell),
        params: p,
    })
}

impl HydraKll {
    /// `HydraQuery::Quantile` and not `Cdf`: the comparator asks for the value
    /// at a rank, which is what rank error is defined over. The `Cdf` variant
    /// answers the inverse question.
    #[inline]
    pub fn estimate_subpop_quantile(&self, labels: &[&str], phi: f64) -> f64 {
        self.inner
            .query_key(labels.to_vec(), &HydraQuery::Quantile(phi))
    }
}

/// Retained slots per cell times the grid area, plus the level index every
/// cell carries. Analytic because the cell allocates once — see
/// `kll_cell_slots` in this crate's `hydra` module for the per-cell count.
pub fn memory_hydra_kll(sketch: &HydraKll) -> usize {
    let p = &sketch.params;
    let per_cell = kll_cell_slots(p.cell_k) * std::mem::size_of::<f64>()
        + (KLL_MAX_LEVELS + 1) * std::mem::size_of::<usize>();
    p.rows * p.cols * per_cell + grid_overhead_bytes(p.rows, p.cols)
}

pub fn insert_hydra_cms(
    params: &ParamSet,
    items: Rc<Vec<(String, i64)>>,
    passes: usize,
) -> Result<Vec<Pass>, BuildError> {
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        // Built here, so calling the closure is the insert and nothing else.
        let mut sketch = build_hydra_cms(params)?;
        let items = items.clone();
        out.push(Box::new(move || {
            for v in items.iter() {
                sketch.inner.update(&v.0, &DataInput::I64(v.1), None);
            }
            memory_hydra_cms(&sketch)
        }) as Pass);
    }
    Ok(out)
}

pub fn insert_step_hydra_cms(
    params: &ParamSet,
    items: Rc<Vec<(String, i64)>>,
    passes: usize,
) -> Result<Vec<StepPass>, BuildError> {
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        let sketch: Shared<_> = Rc::new(RefCell::new(build_hydra_cms(params)?));
        let (driven, read) = (sketch.clone(), sketch);
        let stream = items.clone();
        out.push(StepPass {
            steps: items.len(),
            step: Box::new(move |i| {
                let sketch = &mut *driven.borrow_mut();
                let v = &stream[i];
                sketch.inner.update(&v.0, &DataInput::I64(v.1), None);
            }),
            footprint: Box::new(move || memory_hydra_cms(&read.borrow())),
        });
    }
    Ok(out)
}

pub fn query_hydra_cms(
    params: &ParamSet,
    items: Rc<Vec<(String, i64)>>,
    probes: Rc<Vec<(Vec<String>, i64)>>,
    passes: usize,
) -> Result<Vec<QueryPass<f64>>, BuildError> {
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        // Built and fed here: the closure below asks, and only asks.
        let mut sketch = build_hydra_cms(params)?;
        for v in items.iter() {
            sketch.inner.update(&v.0, &DataInput::I64(v.1), None);
        }
        let probes = probes.clone();
        out.push(Box::new(move || {
            let mut answers = Vec::with_capacity(probes.len());
            for p in probes.iter() {
                answers.push(sketch.estimate_subpop_frequency(&labels(&p.0), &p.1));
            }
            let footprint = memory_hydra_cms(&sketch);
            (answers, footprint)
        }) as QueryPass<f64>);
    }
    Ok(out)
}

pub fn merge_hydra_cms(
    params: &ParamSet,
    items: Rc<Vec<(String, i64)>>,
    shards: usize,
    passes: usize,
) -> Result<Vec<Pass>, BuildError> {
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        let (mut acc, rest) = hydra_cms_shards(params, &items, shards)?;
        out.push(Box::new(move || {
            for other in rest.iter() {
                acc.inner
                    .merge(&other.inner)
                    .expect("both operands built from one ParamSet, so grid and cell shapes match");
            }
            memory_hydra_cms(&acc)
        }) as Pass);
    }
    Ok(out)
}

pub fn merge_step_hydra_cms(
    params: &ParamSet,
    items: Rc<Vec<(String, i64)>>,
    shards: usize,
    passes: usize,
) -> Result<Vec<StepPass>, BuildError> {
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        let (acc, rest) = hydra_cms_shards(params, &items, shards)?;
        let acc: Shared<_> = Rc::new(RefCell::new(acc));
        let (driven, read) = (acc.clone(), acc);
        out.push(StepPass {
            steps: rest.len(),
            step: Box::new(move |i| {
                let acc = &mut *driven.borrow_mut();
                let other = &rest[i];
                acc.inner
                    .merge(&other.inner)
                    .expect("both operands built from one ParamSet, so grid and cell shapes match");
            }),
            footprint: Box::new(move || memory_hydra_cms(&read.borrow())),
        });
    }
    Ok(out)
}

/// The shards a fold folds: the stream split `shards` ways, one sketch each,
/// all fed. The first is the accumulator, the rest are what it folds in.
#[allow(clippy::type_complexity)]
fn hydra_cms_shards(
    params: &ParamSet,
    items: &[(String, i64)],
    shards: usize,
) -> Result<(HydraCms, Vec<HydraCms>), BuildError> {
    let mut parts: Vec<HydraCms> = Vec::new();
    for shard in partition(items, shards) {
        let mut sketch = build_hydra_cms(params)?;
        for v in shard {
            sketch.inner.update(&v.0, &DataInput::I64(v.1), None);
        }
        parts.push(sketch);
    }
    let rest = parts.split_off(1);
    Ok((
        parts.pop().expect("a split of the stream is never empty"),
        rest,
    ))
}

pub fn insert_hydra_hll(
    params: &ParamSet,
    items: Rc<Vec<(String, i64)>>,
    passes: usize,
) -> Result<Vec<Pass>, BuildError> {
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        // Built here, so calling the closure is the insert and nothing else.
        let mut sketch = build_hydra_hll(params)?;
        let items = items.clone();
        out.push(Box::new(move || {
            for v in items.iter() {
                sketch.inner.update(&v.0, &DataInput::I64(v.1), None);
            }
            memory_hydra_hll(&sketch)
        }) as Pass);
    }
    Ok(out)
}

pub fn insert_step_hydra_hll(
    params: &ParamSet,
    items: Rc<Vec<(String, i64)>>,
    passes: usize,
) -> Result<Vec<StepPass>, BuildError> {
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        let sketch: Shared<_> = Rc::new(RefCell::new(build_hydra_hll(params)?));
        let (driven, read) = (sketch.clone(), sketch);
        let stream = items.clone();
        out.push(StepPass {
            steps: items.len(),
            step: Box::new(move |i| {
                let sketch = &mut *driven.borrow_mut();
                let v = &stream[i];
                sketch.inner.update(&v.0, &DataInput::I64(v.1), None);
            }),
            footprint: Box::new(move || memory_hydra_hll(&read.borrow())),
        });
    }
    Ok(out)
}

pub fn query_hydra_hll(
    params: &ParamSet,
    items: Rc<Vec<(String, i64)>>,
    probes: Rc<Vec<Vec<String>>>,
    passes: usize,
) -> Result<Vec<QueryPass<f64>>, BuildError> {
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        // Built and fed here: the closure below asks, and only asks.
        let mut sketch = build_hydra_hll(params)?;
        for v in items.iter() {
            sketch.inner.update(&v.0, &DataInput::I64(v.1), None);
        }
        let probes = probes.clone();
        out.push(Box::new(move || {
            let mut answers = Vec::with_capacity(probes.len());
            for p in probes.iter() {
                answers.push(sketch.estimate_subpop_cardinality(&labels(p)));
            }
            let footprint = memory_hydra_hll(&sketch);
            (answers, footprint)
        }) as QueryPass<f64>);
    }
    Ok(out)
}

pub fn merge_hydra_hll(
    params: &ParamSet,
    items: Rc<Vec<(String, i64)>>,
    shards: usize,
    passes: usize,
) -> Result<Vec<Pass>, BuildError> {
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        let (mut acc, rest) = hydra_hll_shards(params, &items, shards)?;
        out.push(Box::new(move || {
            for other in rest.iter() {
                acc.inner
                    .merge(&other.inner)
                    .expect("both operands built from one ParamSet, so grid and cell shapes match");
            }
            memory_hydra_hll(&acc)
        }) as Pass);
    }
    Ok(out)
}

pub fn merge_step_hydra_hll(
    params: &ParamSet,
    items: Rc<Vec<(String, i64)>>,
    shards: usize,
    passes: usize,
) -> Result<Vec<StepPass>, BuildError> {
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        let (acc, rest) = hydra_hll_shards(params, &items, shards)?;
        let acc: Shared<_> = Rc::new(RefCell::new(acc));
        let (driven, read) = (acc.clone(), acc);
        out.push(StepPass {
            steps: rest.len(),
            step: Box::new(move |i| {
                let acc = &mut *driven.borrow_mut();
                let other = &rest[i];
                acc.inner
                    .merge(&other.inner)
                    .expect("both operands built from one ParamSet, so grid and cell shapes match");
            }),
            footprint: Box::new(move || memory_hydra_hll(&read.borrow())),
        });
    }
    Ok(out)
}

/// The shards a fold folds: the stream split `shards` ways, one sketch each,
/// all fed. The first is the accumulator, the rest are what it folds in.
#[allow(clippy::type_complexity)]
fn hydra_hll_shards(
    params: &ParamSet,
    items: &[(String, i64)],
    shards: usize,
) -> Result<(HydraHll, Vec<HydraHll>), BuildError> {
    let mut parts: Vec<HydraHll> = Vec::new();
    for shard in partition(items, shards) {
        let mut sketch = build_hydra_hll(params)?;
        for v in shard {
            sketch.inner.update(&v.0, &DataInput::I64(v.1), None);
        }
        parts.push(sketch);
    }
    let rest = parts.split_off(1);
    Ok((
        parts.pop().expect("a split of the stream is never empty"),
        rest,
    ))
}

pub fn insert_hydra_kll(
    params: &ParamSet,
    items: Rc<Vec<(String, f64)>>,
    passes: usize,
) -> Result<Vec<Pass>, BuildError> {
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        // Built here, so calling the closure is the insert and nothing else.
        let mut sketch = build_hydra_kll(params)?;
        let items = items.clone();
        out.push(Box::new(move || {
            for v in items.iter() {
                sketch.inner.update(&v.0, &DataInput::F64(v.1), None);
            }
            memory_hydra_kll(&sketch)
        }) as Pass);
    }
    Ok(out)
}

pub fn insert_step_hydra_kll(
    params: &ParamSet,
    items: Rc<Vec<(String, f64)>>,
    passes: usize,
) -> Result<Vec<StepPass>, BuildError> {
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        let sketch: Shared<_> = Rc::new(RefCell::new(build_hydra_kll(params)?));
        let (driven, read) = (sketch.clone(), sketch);
        let stream = items.clone();
        out.push(StepPass {
            steps: items.len(),
            step: Box::new(move |i| {
                let sketch = &mut *driven.borrow_mut();
                let v = &stream[i];
                sketch.inner.update(&v.0, &DataInput::F64(v.1), None);
            }),
            footprint: Box::new(move || memory_hydra_kll(&read.borrow())),
        });
    }
    Ok(out)
}

pub fn query_hydra_kll(
    params: &ParamSet,
    items: Rc<Vec<(String, f64)>>,
    probes: Rc<Vec<(Vec<String>, f64)>>,
    passes: usize,
) -> Result<Vec<QueryPass<f64>>, BuildError> {
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        // Built and fed here: the closure below asks, and only asks.
        let mut sketch = build_hydra_kll(params)?;
        for v in items.iter() {
            sketch.inner.update(&v.0, &DataInput::F64(v.1), None);
        }
        let probes = probes.clone();
        out.push(Box::new(move || {
            let mut answers = Vec::with_capacity(probes.len());
            for p in probes.iter() {
                answers.push(sketch.estimate_subpop_quantile(&labels(&p.0), p.1));
            }
            let footprint = memory_hydra_kll(&sketch);
            (answers, footprint)
        }) as QueryPass<f64>);
    }
    Ok(out)
}

pub fn merge_hydra_kll(
    params: &ParamSet,
    items: Rc<Vec<(String, f64)>>,
    shards: usize,
    passes: usize,
) -> Result<Vec<Pass>, BuildError> {
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        let (mut acc, rest) = hydra_kll_shards(params, &items, shards)?;
        out.push(Box::new(move || {
            for other in rest.iter() {
                acc.inner
                    .merge(&other.inner)
                    .expect("both operands built from one ParamSet, so grid and cell shapes match");
            }
            memory_hydra_kll(&acc)
        }) as Pass);
    }
    Ok(out)
}

pub fn merge_step_hydra_kll(
    params: &ParamSet,
    items: Rc<Vec<(String, f64)>>,
    shards: usize,
    passes: usize,
) -> Result<Vec<StepPass>, BuildError> {
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        let (acc, rest) = hydra_kll_shards(params, &items, shards)?;
        let acc: Shared<_> = Rc::new(RefCell::new(acc));
        let (driven, read) = (acc.clone(), acc);
        out.push(StepPass {
            steps: rest.len(),
            step: Box::new(move |i| {
                let acc = &mut *driven.borrow_mut();
                let other = &rest[i];
                acc.inner
                    .merge(&other.inner)
                    .expect("both operands built from one ParamSet, so grid and cell shapes match");
            }),
            footprint: Box::new(move || memory_hydra_kll(&read.borrow())),
        });
    }
    Ok(out)
}

/// The shards a fold folds: the stream split `shards` ways, one sketch each,
/// all fed. The first is the accumulator, the rest are what it folds in.
#[allow(clippy::type_complexity)]
fn hydra_kll_shards(
    params: &ParamSet,
    items: &[(String, f64)],
    shards: usize,
) -> Result<(HydraKll, Vec<HydraKll>), BuildError> {
    let mut parts: Vec<HydraKll> = Vec::new();
    for shard in partition(items, shards) {
        let mut sketch = build_hydra_kll(params)?;
        for v in shard {
            sketch.inner.update(&v.0, &DataInput::F64(v.1), None);
        }
        parts.push(sketch);
    }
    let rest = parts.split_off(1);
    Ok((
        parts.pop().expect("a split of the stream is never empty"),
        rest,
    ))
}
