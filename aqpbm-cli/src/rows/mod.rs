//! Which sketch a pair of strings names: the only place in the program where a
//! name is bound to code. [`registry`](sketch_bench::registry) declares what rows
//! exist; this binds each one to its wrapper.

use std::marker::PhantomData;
use std::rc::Rc;

use sketch_bench::request::{Dtype, Requirement};
use sketch_bench::wrappers::{
    BuildError, Folds, InsertBody, InsertStepBody, ParallelInsertBody, PrepareBody, QueryBody,
    QueryPass, StepPass,
};

/// A row that cannot be built at the requested config says so in its own words;
/// the framework carries them.
fn cannot_build(e: BuildError) -> RunError {
    RunError::Sketch(e.0)
}
use aqpbm_core::accuracy::cardinality::CardinalityGT;
use aqpbm_core::accuracy::frequency::FrequencyGT;
use aqpbm_core::accuracy::keyed::{
    KeyedCardinalityGT, KeyedEntropyGT, KeyedL1NormGT, KeyedL2NormGT,
};
use aqpbm_core::accuracy::quantile::RankErrorGT;
use aqpbm_core::accuracy::subpopulation::{
    SubpopCardinalityGT, SubpopEntropyGT, SubpopFrequencyGT, SubpopL1NormGT, SubpopL2NormGT,
    SubpopRankErrorGT,
};
use aqpbm_core::accuracy::topk::TopkGT;
use aqpbm_core::accuracy::Score;
use aqpbm_core::accuracy::{questions, CountedValue, GroundTruth};
use aqpbm_core::error::RunError;
use aqpbm_core::measure::{
    record_calls, runs_for, Measurement, Pass as CorePass, Report, RunOutcome, MIN_MERGE_SHARDS,
};
use aqpbm_core::metrics::{Metric, Operation};
use aqpbm_core::{ColumnItem, GeneratedTable, TableDescription};
use sketch_bench::wrappers::frequency_value::FrequencyValue;

/// The label column every subpopulation comparator scores over.
const SCORED_LABEL_COLUMN: usize = 0;

const KEYED_KEY_COLUMN: usize = 0;

fn value_column(description: &TableDescription) -> usize {
    description.column_spec.len().saturating_sub(1)
}
/// One invocation's worth of work: one closure per measurement, in the order
/// they were asked for. The provenance is not here — the frontend generated the
/// data and already holds the description that names it.
pub type Measurements = Vec<((Operation, Metric), Measurement)>;

/// What a registry entry names to bind itself to code: one row, entered with a
/// request and the data, handing back the closures.
pub type RowBinding = fn(
    &Requirement,
    &TableDescription,
    GeneratedTable,
    &[(Operation, Metric)],
) -> Result<Measurements, RunError>;

type Materialise<I> = fn(&TableDescription, GeneratedTable) -> Result<Rc<Vec<I>>, RunError>;

/// Bind a row to its wrapper and hand back the closures. The registry declares
/// the pair; [`binding`] is what names the code, so a pair the registry lists
/// and this table does not is a disagreement, reported as one.
pub fn measurements(
    req: &Requirement,
    description: &TableDescription,
    table: GeneratedTable,
    want: &[(Operation, Metric)],
) -> Result<Measurements, RunError> {
    sketch_bench::registry::find(&req.variant, &req.library).ok_or_else(|| {
        RunError::Sketch(format!(
            "{}/{} is not a registered row",
            req.variant, req.library
        ))
    })?;
    let row = binding(&req.variant, &req.library).ok_or_else(|| {
        RunError::Sketch(format!(
            "{}/{} is registered, but no row here is bound to it",
            req.variant, req.library
        ))
    })?;
    row(req, description, table, want)
}

/// The pair a registry entry names, bound to the code that runs it.
fn binding(variant: &str, library: &str) -> Option<RowBinding> {
    Some(match (variant, library) {
        ("cms", "oxide") => row_cms_oxide,
        ("cms", "datasketches") => row_cms_datasketches,
        ("cms", "polars") => row_cms_polars,
        ("cms-fastpath-fixedmatrix", "lib") => row_cms_fastpath_fixedmatrix_lib,
        ("cms-fastpath-vector2d", "lib") => row_cms_fastpath_vector2d_lib,
        ("cms-regularpath-vector2d", "lib") => row_cms_regularpath_vector2d_lib,
        ("cms-fastpath-fixedmatrix-32k-parallel", "lib") => {
            row_cms_fastpath_fixedmatrix_32k_parallel_lib
        }
        ("cms-heap-fastpath-vector2d", "lib") => row_cms_heap_fastpath_vector2d_lib,
        ("cms-heap-regularpath-vector2d", "lib") => row_cms_heap_regularpath_vector2d_lib,
        ("cms-heap-topk-fastpath-vector2d", "lib") => row_cms_heap_topk_fastpath_vector2d_lib,
        ("cms-heap-topk-regularpath-vector2d", "lib") => row_cms_heap_topk_regularpath_vector2d_lib,
        ("countsketch", "oxide") => row_countsketch_oxide,
        ("countsketch", "polars") => row_countsketch_polars,
        ("countsketch-fastpath-fixedmatrix", "lib") => row_countsketch_fastpath_fixedmatrix_lib,
        ("countsketch-fastpath-vector2d", "lib") => row_countsketch_fastpath_vector2d_lib,
        ("countsketch-regularpath-vector2d", "lib") => row_countsketch_regularpath_vector2d_lib,
        ("countsketch-fastpath-fixedmatrix-32k-parallel", "lib") => {
            row_countsketch_fastpath_fixedmatrix_32k_parallel_lib
        }
        ("hll", "oxide") => row_hll_oxide,
        ("hll", "datasketches") => row_hll_datasketches,
        ("hll", "lib") => row_hll_lib,
        ("hll", "polars") => row_hll_polars,
        ("hll-hip", "lib") => row_hll_hip_lib,
        ("hll-fastpath-parallel", "lib") => row_hll_fastpath_parallel_lib,
        ("kll-percall", "oxide") => row_kll_percall_oxide,
        ("kll-percall", "lib") => row_kll_percall_lib,
        ("kll-cdf", "oxide") => row_kll_cdf_oxide,
        ("kll-cdf", "lib") => row_kll_cdf_lib,
        ("kll-cdf", "polars") => row_kll_cdf_polars,
        ("dd", "lib") => row_dd_lib,
        ("dd", "oxide") => row_dd_oxide,
        ("hydra-cms", "lib") => row_hydra_cms_lib,
        ("hydra-cms", "polars") => row_hydra_cms_polars,
        ("hydra-cs", "lib") => row_hydra_cs_lib,
        ("hydra-cs", "polars") => row_hydra_cs_polars,
        ("hydra-hll", "lib") => row_hydra_hll_lib,
        ("hydra-hll", "polars") => row_hydra_hll_polars,
        ("hydra-kll", "lib") => row_hydra_kll_lib,
        ("hydra-kll", "polars") => row_hydra_kll_polars,
        ("hydra-univmon-cardinality", "lib") => row_hydra_univmon_cardinality_lib,
        ("hydra-univmon-cardinality", "polars") => row_hydra_univmon_cardinality_polars,
        ("hydra-univmon-l1-norm", "lib") => row_hydra_univmon_l1_norm_lib,
        ("hydra-univmon-l1-norm", "polars") => row_hydra_univmon_l1_norm_polars,
        ("hydra-univmon-l2-norm", "lib") => row_hydra_univmon_l2_norm_lib,
        ("hydra-univmon-l2-norm", "polars") => row_hydra_univmon_l2_norm_polars,
        ("hydra-univmon-entropy", "lib") => row_hydra_univmon_entropy_lib,
        ("hydra-univmon-entropy", "polars") => row_hydra_univmon_entropy_polars,
        ("univmon-cardinality", "lib") => row_univmon_cardinality_lib,
        ("univmon-l1-norm", "lib") => row_univmon_l1_norm_lib,
        ("univmon-l1-norm", "oxide") => row_univmon_l1_norm_oxide,
        ("univmon-l2-norm", "lib") => row_univmon_l2_norm_lib,
        ("univmon-l2-norm", "oxide") => row_univmon_l2_norm_oxide,
        ("univmon-entropy", "lib") => row_univmon_entropy_lib,
        ("univmon-entropy", "oxide") => row_univmon_entropy_oxide,
        ("univmon-topk", "lib") => row_univmon_topk_lib,
        ("countsketch-heap-topk-fastpath-vector2d", "lib") => {
            row_countsketch_heap_topk_fastpath_vector2d_lib
        }
        ("exact-sum", "exact") => row_exact_sum,
        ("exact-min", "exact") => row_exact_min,
        ("exact-max", "exact") => row_exact_max,
        ("exact-increase", "exact") => row_exact_increase,
        _ => return None,
    })
}

mod cms;
mod cms_heap;
mod cs;
mod dd;
mod exact;
mod fixed_matrix;
mod hll;
mod hydra;
mod hydra_univmon;
mod kll;
mod materialise;
mod measurement;
mod statistic;
mod univmon;

use cms::*;
use cms_heap::*;
use cs::*;
use dd::*;
use exact::*;
use fixed_matrix::*;
use hll::*;
use hydra::*;
use hydra_univmon::*;
use kll::*;
use materialise::*;
use measurement::*;
use statistic::*;
use univmon::*;

#[cfg(test)]
mod tests {
    use super::binding;
    use sketch_bench::registry::REGISTRY;

    #[test]
    fn every_registered_pair_is_bound() {
        for e in REGISTRY {
            assert!(
                binding(e.variant, e.library).is_some(),
                "{}/{}",
                e.variant,
                e.library
            );
        }
    }
}
