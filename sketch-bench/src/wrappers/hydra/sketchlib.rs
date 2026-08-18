//! The `asap_sketchlib` implementations.
//!
//! Grouped under `wrappers/hydra/` with the other implementations of this
//! algorithm; how each is driven lives beside it.

use super::*;
use crate::params::ParamSet;
use aqpbm_core::input_dataset::Labeled;
use aqpbm_core::RunError;
use asap_sketchlib::input::{HydraCounter, HydraQuery};
use asap_sketchlib::{CountMin, DataInput, FastPath, Hydra, HyperLogLog, Vector2D, KLL};

pub struct HydraCms {
    inner: Hydra,
    params: HydraCmsParams,
}

pub fn build_hydra_cms(config: &ParamSet) -> Result<HydraCms, RunError> {
    let p: HydraCmsParams = config.parse()?;
    check_grid(p.rows, p.cols, "hydra-cms")?;
    for (name, v) in [("cell_rows", p.cell_rows), ("cell_cols", p.cell_cols)] {
        if v == 0 {
            return Err(RunError::Sketch(format!("hydra-cms: {name} must be > 0")));
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
    inner: Hydra,
    params: HydraHllParams,
}

pub fn build_hydra_hll(config: &ParamSet) -> Result<HydraHll, RunError> {
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
    inner: Hydra,
    params: HydraKllParams,
}

pub fn build_hydra_kll(config: &ParamSet) -> Result<HydraKll, RunError> {
    let p: HydraKllParams = config.parse()?;
    check_grid(p.rows, p.cols, "hydra-kll")?;
    // The cell is the same `asap_sketchlib::KLL` the `kll-*` rows hold, and it
    // clamps `k` to its own range without saying so. Refuse for the same reason
    // those rows do: the grid would be built at a `cell_k` the record misnames.
    if !(crate::wrappers::kll::LIB_K_MIN..=crate::wrappers::kll::LIB_K_MAX).contains(&p.cell_k) {
        return Err(RunError::Sketch(format!(
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

pub fn insert_hydra_cms(sketch: &mut HydraCms, r: &Labeled<i64>) {
    sketch.inner.update(&r.key, &DataInput::I64(r.value), None);
}

pub fn merge_hydra_cms(into: &mut HydraCms, from: &HydraCms) {
    into.inner
        .merge(&from.inner)
        .expect("both operands built from one ParamSet, so grid and cell shapes match");
}

pub fn insert_hydra_hll(sketch: &mut HydraHll, r: &Labeled<i64>) {
    sketch.inner.update(&r.key, &DataInput::I64(r.value), None);
}

pub fn merge_hydra_hll(into: &mut HydraHll, from: &HydraHll) {
    into.inner
        .merge(&from.inner)
        .expect("both operands built from one ParamSet, so grid and cell shapes match");
}

pub fn insert_hydra_kll(sketch: &mut HydraKll, r: &Labeled<f64>) {
    sketch.inner.update(&r.key, &DataInput::F64(r.value), None);
}

pub fn merge_hydra_kll(into: &mut HydraKll, from: &HydraKll) {
    into.inner
        .merge(&from.inner)
        .expect("both operands built from one ParamSet, so grid and cell shapes match");
}

pub fn query_hydra_cms(sketch: &mut HydraCms, probe: &(Vec<String>, i64)) -> f64 {
    sketch.estimate_subpop_frequency(&labels(&probe.0), &probe.1)
}

// `&Vec<String>` and not `&[String]`: this is an `ask` closure, so its
// parameter type is `&<SubpopCardinalityGT as GroundTruth>::Probe`, and that
// associated type is `Vec<String>`. A slice here does not satisfy the bound.
#[allow(clippy::ptr_arg)]
pub fn query_hydra_hll(sketch: &mut HydraHll, probe: &Vec<String>) -> f64 {
    sketch.estimate_subpop_cardinality(&labels(probe))
}

pub fn query_hydra_kll(sketch: &mut HydraKll, probe: &(Vec<String>, f64)) -> f64 {
    sketch.estimate_subpop_quantile(&labels(&probe.0), probe.1)
}
