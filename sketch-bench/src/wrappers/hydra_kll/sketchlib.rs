//! The `asap_sketchlib` implementation.
//!
//! Grouped under `wrappers/hydra_kll/` with the other implementations of this
//! algorithm; how each is driven lives beside it.

use super::*;
use crate::params::ParamSet;
use crate::wrappers::partition;
use crate::wrappers::quantile_value::QuantileValue;
use crate::wrappers::{BuildError, Pass, QueryPass, Shared, StepPass};
use asap_sketchlib::input::{HydraCounter, HydraQuery};
use asap_sketchlib::{Hydra, KLL};
use std::cell::RefCell;
use std::rc::Rc;

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
        let key = labels.join(&SERIES_SEPARATOR.to_string());
        self.inner.query_key(vec![&key], &HydraQuery::Quantile(phi))
    }
}

/// Retained slots per cell times the grid area, plus the level index every
/// cell carries. Analytic because the cell allocates once — see
/// `kll_cell_slots` in this crate's `hydra_kll` module for the per-cell count.
pub fn memory_hydra_kll(sketch: &HydraKll) -> usize {
    let p = &sketch.params;
    let per_cell = kll_cell_slots(p.cell_k) * std::mem::size_of::<f64>()
        + (KLL_MAX_LEVELS + 1) * std::mem::size_of::<usize>();
    p.rows * p.cols * per_cell + grid_overhead_bytes(p.rows, p.cols)
}

pub fn insert_hydra_kll<V: QuantileValue + 'static>(
    params: &ParamSet,
    items: Rc<Vec<(String, V)>>,
    passes: usize,
) -> Result<Vec<Pass>, BuildError> {
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        // Built here, so calling the closure is the insert and nothing else.
        let mut sketch = build_hydra_kll(params)?;
        let items = items.clone();
        out.push(Box::new(move || {
            for v in items.iter() {
                sketch.inner.update(&v.0, &v.1.data_input(), None);
            }
            memory_hydra_kll(&sketch)
        }) as Pass);
    }
    Ok(out)
}

pub fn insert_step_hydra_kll<V: QuantileValue + 'static>(
    params: &ParamSet,
    items: Rc<Vec<(String, V)>>,
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
                sketch.inner.update(&v.0, &v.1.data_input(), None);
            }),
            footprint: Box::new(move || memory_hydra_kll(&read.borrow())),
        });
    }
    Ok(out)
}

pub fn query_hydra_kll<V: QuantileValue + 'static>(
    params: &ParamSet,
    items: Rc<Vec<(String, V)>>,
    probes: Rc<Vec<(Vec<String>, f64)>>,
    passes: usize,
) -> Result<Vec<QueryPass<f64>>, BuildError> {
    merge_query_hydra_kll(params, items, probes, 1, passes)
}

/// The query, asked of the sketch a fold over `shards` shards leaves. One
/// shard is the plain query.
pub fn merge_query_hydra_kll<V: QuantileValue + 'static>(
    params: &ParamSet,
    items: Rc<Vec<(String, V)>>,
    probes: Rc<Vec<(Vec<String>, f64)>>,
    shards: usize,
    passes: usize,
) -> Result<Vec<QueryPass<f64>>, BuildError> {
    let mut out = Vec::with_capacity(passes);
    for _ in 0..passes {
        // Built, fed and folded here: the closure below asks, and only asks.
        let (mut sketch, rest) = hydra_kll_shards(params, &items, shards)?;
        for other in rest.iter() {
            sketch
                .inner
                .merge(&other.inner)
                .expect("both operands built from one ParamSet, so grid and cell shapes match");
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

pub fn merge_hydra_kll<V: QuantileValue + 'static>(
    params: &ParamSet,
    items: Rc<Vec<(String, V)>>,
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

pub fn merge_step_hydra_kll<V: QuantileValue + 'static>(
    params: &ParamSet,
    items: Rc<Vec<(String, V)>>,
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
fn hydra_kll_shards<V: QuantileValue>(
    params: &ParamSet,
    items: &[(String, V)],
    shards: usize,
) -> Result<(HydraKll, Vec<HydraKll>), BuildError> {
    let mut parts: Vec<HydraKll> = Vec::new();
    for shard in partition(items, shards) {
        let mut sketch = build_hydra_kll(params)?;
        for v in shard {
            sketch.inner.update(&v.0, &v.1.data_input(), None);
        }
        parts.push(sketch);
    }
    let rest = parts.split_off(1);
    Ok((
        parts.pop().expect("a split of the stream is never empty"),
        rest,
    ))
}
