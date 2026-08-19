//! Which sketch a pair of strings names: the only place in the program where a
//! name is bound to code. [`registry`](sketch_bench::registry) declares what rows
//! exist; this binds each one to its wrapper.

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
use aqpbm_core::accuracy::keyedcardinality::KeyedCardinalityGT;
use aqpbm_core::accuracy::keyedentropy::KeyedEntropyGT;
use aqpbm_core::accuracy::keyedl1norm::KeyedL1NormGT;
use aqpbm_core::accuracy::keyedl2norm::KeyedL2NormGT;
use aqpbm_core::accuracy::quantile::RankErrorGT;
use aqpbm_core::accuracy::subpopulation::{
    SubpopCardinalityGT, SubpopFrequencyGT, SubpopRankErrorGT,
};
use aqpbm_core::accuracy::Score;
use aqpbm_core::accuracy::{questions, GroundTruth};
use aqpbm_core::error::RunError;
use aqpbm_core::measure::{
    record_calls, runs_for, Measurement, Pass as CorePass, Report, RunOutcome, MIN_MERGE_SHARDS,
};
use aqpbm_core::metrics::{Metric, Operation};
use aqpbm_core::{ColumnItem, GeneratedTable, TableDescription};

use sketch_bench::wrappers::cms::{datasketches as cd, oxide as co, polars as cp, sketchlib as cl};
use sketch_bench::wrappers::cs::{oxide as so, polars as sp, sketchlib as sl};
use sketch_bench::wrappers::dd::{oxide as ddo, sketchlib as ddl};
use sketch_bench::wrappers::hll::{
    datasketches as hd, oxide as ho, polars as hpo, sketchlib as hl,
};
use sketch_bench::wrappers::hydra::{polars as hp, sketchlib as hs};
use sketch_bench::wrappers::kll::{oxide as ko, polars as kp, sketchlib as kl};
use sketch_bench::wrappers::univmon::{oxide as uo, sketchlib as ul};

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
    sketch_bench::registry::find(&req.algorithm, &req.impl_name).ok_or_else(|| {
        RunError::Sketch(format!(
            "{}/{} is not a registered row",
            req.algorithm, req.impl_name
        ))
    })?;
    let row = binding(&req.algorithm, &req.impl_name).ok_or_else(|| {
        RunError::Sketch(format!(
            "{}/{} is registered, but no row here is bound to it",
            req.algorithm, req.impl_name
        ))
    })?;
    row(req, description, table, want)
}

/// The pair a registry entry names, bound to the code that runs it.
fn binding(algorithm: &str, impl_name: &str) -> Option<RowBinding> {
    Some(match (algorithm, impl_name) {
        ("cms", "oxide") => row_cms_oxide,
        ("cms", "datasketches") => row_cms_datasketches,
        ("cms", "polars") => row_cms_polars,
        ("cms-fastpath-fixedmatrix", "lib") => row_cms_fastpath_fixedmatrix_lib,
        ("cms-fastpath-vector2d", "lib") => row_cms_fastpath_vector2d_lib,
        ("cms-regularpath-vector2d", "lib") => row_cms_regularpath_vector2d_lib,
        ("cms-fastpath-fixedmatrix-32k-parallel", "lib") => {
            row_cms_fastpath_fixedmatrix_32k_parallel_lib
        }
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
        ("hydra-hll", "lib") => row_hydra_hll_lib,
        ("hydra-hll", "polars") => row_hydra_hll_polars,
        ("hydra-kll", "lib") => row_hydra_kll_lib,
        ("hydra-kll", "polars") => row_hydra_kll_polars,
        ("univmon-cardinality", "lib") => row_univmon_cardinality_lib,
        ("univmon-l1-norm", "lib") => row_univmon_l1_norm_lib,
        ("univmon-l1-norm", "oxide") => row_univmon_l1_norm_oxide,
        ("univmon-l2-norm", "lib") => row_univmon_l2_norm_lib,
        ("univmon-l2-norm", "oxide") => row_univmon_l2_norm_oxide,
        ("univmon-entropy", "lib") => row_univmon_entropy_lib,
        ("univmon-entropy", "oxide") => row_univmon_entropy_oxide,
        _ => return None,
    })
}

// -------- CMS (frequency) --------

pub(crate) fn row_cms_oxide(
    req: &Requirement,
    description: &TableDescription,
    table: GeneratedTable,
    want: &[(Operation, Metric)],
) -> Result<Measurements, RunError> {
    frequency_row(
        req,
        description,
        table,
        want,
        co::insert_cms_oxide,
        co::insert_step_cms_oxide,
        co::query_cms_oxide,
        Some((co::merge_cms_oxide, co::merge_step_cms_oxide)),
        None,
    )
}

pub(crate) fn row_cms_datasketches(
    req: &Requirement,
    description: &TableDescription,
    table: GeneratedTable,
    want: &[(Operation, Metric)],
) -> Result<Measurements, RunError> {
    frequency_row(
        req,
        description,
        table,
        want,
        cd::insert_cms_datasketches,
        cd::insert_step_cms_datasketches,
        cd::query_cms_datasketches,
        Some((cd::merge_cms_datasketches, cd::merge_step_cms_datasketches)),
        None,
    )
}

pub(crate) fn row_cms_polars(
    req: &Requirement,
    description: &TableDescription,
    table: GeneratedTable,
    want: &[(Operation, Metric)],
) -> Result<Measurements, RunError> {
    frequency_row(
        req,
        description,
        table,
        want,
        cp::insert_polars_frequency_cms,
        cp::insert_step_polars_frequency_cms,
        cp::query_polars_frequency_cms,
        None,
        Some(cp::prepare_polars_frequency_cms),
    )
}

pub(crate) fn row_cms_fastpath_vector2d_lib(
    req: &Requirement,
    description: &TableDescription,
    table: GeneratedTable,
    want: &[(Operation, Metric)],
) -> Result<Measurements, RunError> {
    frequency_row(
        req,
        description,
        table,
        want,
        cl::insert_cms_lib_vector2d_fast,
        cl::insert_step_cms_lib_vector2d_fast,
        cl::query_cms_lib_vector2d_fast,
        Some((
            cl::merge_cms_lib_vector2d_fast,
            cl::merge_step_cms_lib_vector2d_fast,
        )),
        None,
    )
}

pub(crate) fn row_cms_regularpath_vector2d_lib(
    req: &Requirement,
    description: &TableDescription,
    table: GeneratedTable,
    want: &[(Operation, Metric)],
) -> Result<Measurements, RunError> {
    frequency_row(
        req,
        description,
        table,
        want,
        cl::insert_cms_lib_vector2d_regular,
        cl::insert_step_cms_lib_vector2d_regular,
        cl::query_cms_lib_vector2d_regular,
        Some((
            cl::merge_cms_lib_vector2d_regular,
            cl::merge_step_cms_lib_vector2d_regular,
        )),
        None,
    )
}

pub(crate) fn row_cms_fastpath_fixedmatrix_32k_parallel_lib(
    req: &Requirement,
    description: &TableDescription,
    table: GeneratedTable,
    want: &[(Operation, Metric)],
) -> Result<Measurements, RunError> {
    timed_row(
        req,
        description,
        table,
        want,
        cl::insert_parallel_cms_fast_path,
    )
}

// -------- CountSketch (frequency) --------

pub(crate) fn row_countsketch_oxide(
    req: &Requirement,
    description: &TableDescription,
    table: GeneratedTable,
    want: &[(Operation, Metric)],
) -> Result<Measurements, RunError> {
    frequency_row(
        req,
        description,
        table,
        want,
        so::insert_cs_oxide,
        so::insert_step_cs_oxide,
        so::query_cs_oxide,
        Some((so::merge_cs_oxide, so::merge_step_cs_oxide)),
        None,
    )
}

pub(crate) fn row_countsketch_polars(
    req: &Requirement,
    description: &TableDescription,
    table: GeneratedTable,
    want: &[(Operation, Metric)],
) -> Result<Measurements, RunError> {
    frequency_row(
        req,
        description,
        table,
        want,
        sp::insert_polars_frequency_cs,
        sp::insert_step_polars_frequency_cs,
        sp::query_polars_frequency_cs,
        None,
        Some(sp::prepare_polars_frequency_cs),
    )
}

pub(crate) fn row_countsketch_fastpath_vector2d_lib(
    req: &Requirement,
    description: &TableDescription,
    table: GeneratedTable,
    want: &[(Operation, Metric)],
) -> Result<Measurements, RunError> {
    frequency_row(
        req,
        description,
        table,
        want,
        sl::insert_cs_lib_vector2d_fast,
        sl::insert_step_cs_lib_vector2d_fast,
        sl::query_cs_lib_vector2d_fast,
        Some((
            sl::merge_cs_lib_vector2d_fast,
            sl::merge_step_cs_lib_vector2d_fast,
        )),
        None,
    )
}

pub(crate) fn row_countsketch_regularpath_vector2d_lib(
    req: &Requirement,
    description: &TableDescription,
    table: GeneratedTable,
    want: &[(Operation, Metric)],
) -> Result<Measurements, RunError> {
    frequency_row(
        req,
        description,
        table,
        want,
        sl::insert_cs_lib_vector2d_regular,
        sl::insert_step_cs_lib_vector2d_regular,
        sl::query_cs_lib_vector2d_regular,
        Some((
            sl::merge_cs_lib_vector2d_regular,
            sl::merge_step_cs_lib_vector2d_regular,
        )),
        None,
    )
}

pub(crate) fn row_countsketch_fastpath_fixedmatrix_32k_parallel_lib(
    req: &Requirement,
    description: &TableDescription,
    table: GeneratedTable,
    want: &[(Operation, Metric)],
) -> Result<Measurements, RunError> {
    timed_row(
        req,
        description,
        table,
        want,
        sl::insert_parallel_cs_fast_path,
    )
}

// -------- HLL (cardinality) --------

pub(crate) fn row_hll_oxide(
    req: &Requirement,
    description: &TableDescription,
    table: GeneratedTable,
    want: &[(Operation, Metric)],
) -> Result<Measurements, RunError> {
    cardinality_row(
        req,
        description,
        table,
        want,
        ho::insert_hll_oxide,
        ho::insert_step_hll_oxide,
        ho::query_hll_oxide,
        Some((ho::merge_hll_oxide, ho::merge_step_hll_oxide)),
        None,
    )
}

pub(crate) fn row_hll_datasketches(
    req: &Requirement,
    description: &TableDescription,
    table: GeneratedTable,
    want: &[(Operation, Metric)],
) -> Result<Measurements, RunError> {
    cardinality_row(
        req,
        description,
        table,
        want,
        hd::insert_hll_datasketches,
        hd::insert_step_hll_datasketches,
        hd::query_hll_datasketches,
        Some((hd::merge_hll_datasketches, hd::merge_step_hll_datasketches)),
        None,
    )
}

pub(crate) fn row_hll_polars(
    req: &Requirement,
    description: &TableDescription,
    table: GeneratedTable,
    want: &[(Operation, Metric)],
) -> Result<Measurements, RunError> {
    cardinality_row(
        req,
        description,
        table,
        want,
        hpo::insert_polars_cardinality,
        hpo::insert_step_polars_cardinality,
        hpo::query_polars_cardinality,
        None,
        Some(hpo::prepare_polars_cardinality),
    )
}

pub(crate) fn row_hll_fastpath_parallel_lib(
    req: &Requirement,
    description: &TableDescription,
    table: GeneratedTable,
    want: &[(Operation, Metric)],
) -> Result<Measurements, RunError> {
    timed_row(
        req,
        description,
        table,
        want,
        hl::insert_parallel_hll_fast_path,
    )
}

// -------- KLL (quantile) --------
//
// The only rows `--dtype` selects anything for: their library is generic over
// the value type, so a width picks a monomorphisation rather than being
// refused. Each arm is its own instantiation.

pub(crate) fn row_kll_percall_oxide(
    req: &Requirement,
    description: &TableDescription,
    table: GeneratedTable,
    want: &[(Operation, Metric)],
) -> Result<Measurements, RunError> {
    match req.width {
        Dtype::I64 => quantile_row::<i64>(
            req,
            description,
            table,
            want,
            ko::insert_kll_oxide_per_call::<i64>,
            ko::insert_step_kll_oxide_per_call::<i64>,
            ko::query_kll_oxide_per_call::<i64>,
            Some((
                ko::merge_kll_oxide_per_call::<i64>,
                ko::merge_step_kll_oxide_per_call::<i64>,
            )),
            None,
        ),
        Dtype::F64 => quantile_row::<f64>(
            req,
            description,
            table,
            want,
            ko::insert_kll_oxide_per_call::<f64>,
            ko::insert_step_kll_oxide_per_call::<f64>,
            ko::query_kll_oxide_per_call::<f64>,
            Some((
                ko::merge_kll_oxide_per_call::<f64>,
                ko::merge_step_kll_oxide_per_call::<f64>,
            )),
            None,
        ),
        other => Err(no_build_at(req, other)),
    }
}

pub(crate) fn row_kll_percall_lib(
    req: &Requirement,
    description: &TableDescription,
    table: GeneratedTable,
    want: &[(Operation, Metric)],
) -> Result<Measurements, RunError> {
    match req.width {
        Dtype::I64 => quantile_row::<i64>(
            req,
            description,
            table,
            want,
            kl::insert_kll_lib_per_call::<i64>,
            kl::insert_step_kll_lib_per_call::<i64>,
            kl::query_kll_lib_per_call::<i64>,
            Some((
                kl::merge_kll_lib_per_call::<i64>,
                kl::merge_step_kll_lib_per_call::<i64>,
            )),
            None,
        ),
        Dtype::F64 => quantile_row::<f64>(
            req,
            description,
            table,
            want,
            kl::insert_kll_lib_per_call::<f64>,
            kl::insert_step_kll_lib_per_call::<f64>,
            kl::query_kll_lib_per_call::<f64>,
            Some((
                kl::merge_kll_lib_per_call::<f64>,
                kl::merge_step_kll_lib_per_call::<f64>,
            )),
            None,
        ),
        other => Err(no_build_at(req, other)),
    }
}

pub(crate) fn row_kll_cdf_oxide(
    req: &Requirement,
    description: &TableDescription,
    table: GeneratedTable,
    want: &[(Operation, Metric)],
) -> Result<Measurements, RunError> {
    match req.width {
        Dtype::I64 => quantile_row::<i64>(
            req,
            description,
            table,
            want,
            ko::insert_kll_oxide_cdf::<i64>,
            ko::insert_step_kll_oxide_cdf::<i64>,
            ko::query_kll_oxide_cdf::<i64>,
            Some((
                ko::merge_kll_oxide_cdf::<i64>,
                ko::merge_step_kll_oxide_cdf::<i64>,
            )),
            Some(ko::prepare_kll_oxide_cdf::<i64>),
        ),
        Dtype::F64 => quantile_row::<f64>(
            req,
            description,
            table,
            want,
            ko::insert_kll_oxide_cdf::<f64>,
            ko::insert_step_kll_oxide_cdf::<f64>,
            ko::query_kll_oxide_cdf::<f64>,
            Some((
                ko::merge_kll_oxide_cdf::<f64>,
                ko::merge_step_kll_oxide_cdf::<f64>,
            )),
            Some(ko::prepare_kll_oxide_cdf::<f64>),
        ),
        other => Err(no_build_at(req, other)),
    }
}

pub(crate) fn row_kll_cdf_lib(
    req: &Requirement,
    description: &TableDescription,
    table: GeneratedTable,
    want: &[(Operation, Metric)],
) -> Result<Measurements, RunError> {
    match req.width {
        Dtype::I64 => quantile_row::<i64>(
            req,
            description,
            table,
            want,
            kl::insert_kll_lib_cdf::<i64>,
            kl::insert_step_kll_lib_cdf::<i64>,
            kl::query_kll_lib_cdf::<i64>,
            Some((
                kl::merge_kll_lib_cdf::<i64>,
                kl::merge_step_kll_lib_cdf::<i64>,
            )),
            Some(kl::prepare_kll_lib_cdf::<i64>),
        ),
        Dtype::F64 => quantile_row::<f64>(
            req,
            description,
            table,
            want,
            kl::insert_kll_lib_cdf::<f64>,
            kl::insert_step_kll_lib_cdf::<f64>,
            kl::query_kll_lib_cdf::<f64>,
            Some((
                kl::merge_kll_lib_cdf::<f64>,
                kl::merge_step_kll_lib_cdf::<f64>,
            )),
            Some(kl::prepare_kll_lib_cdf::<f64>),
        ),
        other => Err(no_build_at(req, other)),
    }
}

// The exact baseline is i64 only, unlike the four sketch rows above it: its
// grid is built from a sorted i64 column.
pub(crate) fn row_kll_cdf_polars(
    req: &Requirement,
    description: &TableDescription,
    table: GeneratedTable,
    want: &[(Operation, Metric)],
) -> Result<Measurements, RunError> {
    quantile_row::<i64>(
        req,
        description,
        table,
        want,
        kp::insert_polars_quantile_kll,
        kp::insert_step_polars_quantile_kll,
        kp::query_polars_quantile_kll,
        None,
        Some(kp::prepare_polars_quantile_kll),
    )
}

// -------- DDSketch (quantile) --------

pub(crate) fn row_dd_lib(
    req: &Requirement,
    description: &TableDescription,
    table: GeneratedTable,
    want: &[(Operation, Metric)],
) -> Result<Measurements, RunError> {
    match req.width {
        Dtype::I64 => quantile_row::<i64>(
            req,
            description,
            table,
            want,
            ddl::insert_dd_lib::<i64>,
            ddl::insert_step_dd_lib::<i64>,
            ddl::query_dd_lib::<i64>,
            Some((ddl::merge_dd_lib::<i64>, ddl::merge_step_dd_lib::<i64>)),
            None,
        ),
        Dtype::F64 => quantile_row::<f64>(
            req,
            description,
            table,
            want,
            ddl::insert_dd_lib::<f64>,
            ddl::insert_step_dd_lib::<f64>,
            ddl::query_dd_lib::<f64>,
            Some((ddl::merge_dd_lib::<f64>, ddl::merge_step_dd_lib::<f64>)),
            None,
        ),
        other => Err(no_build_at(req, other)),
    }
}

pub(crate) fn row_dd_oxide(
    req: &Requirement,
    description: &TableDescription,
    table: GeneratedTable,
    want: &[(Operation, Metric)],
) -> Result<Measurements, RunError> {
    match req.width {
        Dtype::I64 => quantile_row::<i64>(
            req,
            description,
            table,
            want,
            ddo::insert_dd_oxide::<i64>,
            ddo::insert_step_dd_oxide::<i64>,
            ddo::query_dd_oxide::<i64>,
            Some((ddo::merge_dd_oxide::<i64>, ddo::merge_step_dd_oxide::<i64>)),
            None,
        ),
        Dtype::F64 => quantile_row::<f64>(
            req,
            description,
            table,
            want,
            ddo::insert_dd_oxide::<f64>,
            ddo::insert_step_dd_oxide::<f64>,
            ddo::query_dd_oxide::<f64>,
            Some((ddo::merge_dd_oxide::<f64>, ddo::merge_step_dd_oxide::<f64>)),
            None,
        ),
        other => Err(no_build_at(req, other)),
    }
}

// -------- Hydra (per-subpopulation statistics over labelled records) --------

pub(crate) fn row_hydra_cms_lib(
    req: &Requirement,
    description: &TableDescription,
    table: GeneratedTable,
    want: &[(Operation, Metric)],
) -> Result<Measurements, RunError> {
    subpop_frequency_row(
        req,
        description,
        table,
        want,
        hs::insert_hydra_cms,
        hs::insert_step_hydra_cms,
        hs::query_hydra_cms,
        Some((hs::merge_hydra_cms, hs::merge_step_hydra_cms)),
        None,
    )
}

pub(crate) fn row_hydra_cms_polars(
    req: &Requirement,
    description: &TableDescription,
    table: GeneratedTable,
    want: &[(Operation, Metric)],
) -> Result<Measurements, RunError> {
    subpop_frequency_row(
        req,
        description,
        table,
        want,
        hp::insert_polars_subpop_frequency,
        hp::insert_step_polars_subpop_frequency,
        hp::query_polars_subpop_frequency,
        None,
        Some(hp::prepare_polars_subpop_frequency),
    )
}

pub(crate) fn row_hydra_hll_lib(
    req: &Requirement,
    description: &TableDescription,
    table: GeneratedTable,
    want: &[(Operation, Metric)],
) -> Result<Measurements, RunError> {
    subpop_cardinality_row(
        req,
        description,
        table,
        want,
        hs::insert_hydra_hll,
        hs::insert_step_hydra_hll,
        hs::query_hydra_hll,
        Some((hs::merge_hydra_hll, hs::merge_step_hydra_hll)),
        None,
    )
}

pub(crate) fn row_hydra_hll_polars(
    req: &Requirement,
    description: &TableDescription,
    table: GeneratedTable,
    want: &[(Operation, Metric)],
) -> Result<Measurements, RunError> {
    subpop_cardinality_row(
        req,
        description,
        table,
        want,
        hp::insert_polars_subpop_cardinality,
        hp::insert_step_polars_subpop_cardinality,
        hp::query_polars_subpop_cardinality,
        None,
        Some(hp::prepare_polars_subpop_cardinality),
    )
}

pub(crate) fn row_hydra_kll_lib(
    req: &Requirement,
    description: &TableDescription,
    table: GeneratedTable,
    want: &[(Operation, Metric)],
) -> Result<Measurements, RunError> {
    subpop_quantile_row(
        req,
        description,
        table,
        want,
        hs::insert_hydra_kll,
        hs::insert_step_hydra_kll,
        hs::query_hydra_kll,
        Some((hs::merge_hydra_kll, hs::merge_step_hydra_kll)),
        None,
    )
}

pub(crate) fn row_hydra_kll_polars(
    req: &Requirement,
    description: &TableDescription,
    table: GeneratedTable,
    want: &[(Operation, Metric)],
) -> Result<Measurements, RunError> {
    subpop_quantile_row(
        req,
        description,
        table,
        want,
        hp::insert_polars_subpop_quantile,
        hp::insert_step_polars_subpop_quantile,
        hp::query_polars_subpop_quantile,
        None,
        Some(hp::prepare_polars_subpop_quantile),
    )
}

pub(crate) fn row_univmon_cardinality_lib(
    req: &Requirement,
    description: &TableDescription,
    table: GeneratedTable,
    want: &[(Operation, Metric)],
) -> Result<Measurements, RunError> {
    keyed_row(
        req,
        description,
        table,
        want,
        KeyedCardinalityGT {
            key_column: KEYED_KEY_COLUMN,
            value_column: value_column(description),
        },
        ul::insert_univmon_lib,
        ul::insert_step_univmon_lib,
        ul::query_univmon_lib_cardinality,
        Some((ul::merge_univmon_lib, ul::merge_step_univmon_lib)),
        None,
    )
}

pub(crate) fn row_univmon_l1_norm_lib(
    req: &Requirement,
    description: &TableDescription,
    table: GeneratedTable,
    want: &[(Operation, Metric)],
) -> Result<Measurements, RunError> {
    keyed_row(
        req,
        description,
        table,
        want,
        KeyedL1NormGT {
            key_column: KEYED_KEY_COLUMN,
            value_column: value_column(description),
        },
        ul::insert_univmon_lib,
        ul::insert_step_univmon_lib,
        ul::query_univmon_lib_l1_norm,
        Some((ul::merge_univmon_lib, ul::merge_step_univmon_lib)),
        None,
    )
}

pub(crate) fn row_univmon_l1_norm_oxide(
    req: &Requirement,
    description: &TableDescription,
    table: GeneratedTable,
    want: &[(Operation, Metric)],
) -> Result<Measurements, RunError> {
    keyed_row(
        req,
        description,
        table,
        want,
        KeyedL1NormGT {
            key_column: KEYED_KEY_COLUMN,
            value_column: value_column(description),
        },
        uo::insert_univmon_oxide,
        uo::insert_step_univmon_oxide,
        uo::query_univmon_oxide_l1_norm,
        Some((uo::merge_univmon_oxide, uo::merge_step_univmon_oxide)),
        None,
    )
}

pub(crate) fn row_univmon_l2_norm_lib(
    req: &Requirement,
    description: &TableDescription,
    table: GeneratedTable,
    want: &[(Operation, Metric)],
) -> Result<Measurements, RunError> {
    keyed_row(
        req,
        description,
        table,
        want,
        KeyedL2NormGT {
            key_column: KEYED_KEY_COLUMN,
            value_column: value_column(description),
        },
        ul::insert_univmon_lib,
        ul::insert_step_univmon_lib,
        ul::query_univmon_lib_l2_norm,
        Some((ul::merge_univmon_lib, ul::merge_step_univmon_lib)),
        None,
    )
}

pub(crate) fn row_univmon_l2_norm_oxide(
    req: &Requirement,
    description: &TableDescription,
    table: GeneratedTable,
    want: &[(Operation, Metric)],
) -> Result<Measurements, RunError> {
    keyed_row(
        req,
        description,
        table,
        want,
        KeyedL2NormGT {
            key_column: KEYED_KEY_COLUMN,
            value_column: value_column(description),
        },
        uo::insert_univmon_oxide,
        uo::insert_step_univmon_oxide,
        uo::query_univmon_oxide_l2_norm,
        Some((uo::merge_univmon_oxide, uo::merge_step_univmon_oxide)),
        None,
    )
}

pub(crate) fn row_univmon_entropy_lib(
    req: &Requirement,
    description: &TableDescription,
    table: GeneratedTable,
    want: &[(Operation, Metric)],
) -> Result<Measurements, RunError> {
    keyed_row(
        req,
        description,
        table,
        want,
        KeyedEntropyGT {
            key_column: KEYED_KEY_COLUMN,
            value_column: value_column(description),
        },
        ul::insert_univmon_lib,
        ul::insert_step_univmon_lib,
        ul::query_univmon_lib_entropy,
        Some((ul::merge_univmon_lib, ul::merge_step_univmon_lib)),
        None,
    )
}

pub(crate) fn row_univmon_entropy_oxide(
    req: &Requirement,
    description: &TableDescription,
    table: GeneratedTable,
    want: &[(Operation, Metric)],
) -> Result<Measurements, RunError> {
    keyed_row(
        req,
        description,
        table,
        want,
        KeyedEntropyGT {
            key_column: KEYED_KEY_COLUMN,
            value_column: value_column(description),
        },
        uo::insert_univmon_oxide,
        uo::insert_step_univmon_oxide,
        uo::query_univmon_oxide_entropy,
        Some((uo::merge_univmon_oxide, uo::merge_step_univmon_oxide)),
        None,
    )
}

// ---------- the statistic a row answers ----------

/// What an operation a row does not have would be: the registry declares the
/// operations each row supports and the frontend checks a request against it,
/// so reaching one here is a table disagreeing with itself.
const SUPPORTED: &str = "the registry declares the operations this row supports";

/// How many primed closures one measurement needs: its measured runs plus the
/// warm-ups thrown away before them. The run count follows the metric, which
/// [`runs_for`] is the one statement of.
fn passes(req: &Requirement, metric: Metric) -> usize {
    req.warmup_runs + runs_for(metric, req.runs)
}

/// The shard count a fold runs at, floored where a fold stops being one.
fn shards(req: &Requirement) -> usize {
    req.merge_shards.max(MIN_MERGE_SHARDS)
}

/// A row answering **frequency**: how often a key occurs in the stream.
#[allow(clippy::too_many_arguments)]
fn frequency_row(
    req: &Requirement,
    description: &TableDescription,
    table: GeneratedTable,
    want: &[(Operation, Metric)],
    insert: InsertBody<i64>,
    insert_step: InsertStepBody<i64>,
    query: QueryBody<i64, i64, u64>,
    merge: Option<Folds<i64>>,
    prepare: Option<PrepareBody<i64>>,
) -> Result<Measurements, RunError> {
    scored_row(
        req,
        description,
        table,
        want,
        FrequencyGT::<i64>::over_column(value_column(description)),
        peel::<i64>,
        insert,
        insert_step,
        query,
        merge,
        prepare,
    )
}

/// A row answering **cardinality**: how many distinct keys the stream carried.
/// The probe is `()` — there is one question, asked repeatedly.
#[allow(clippy::too_many_arguments)]
fn cardinality_row(
    req: &Requirement,
    description: &TableDescription,
    table: GeneratedTable,
    want: &[(Operation, Metric)],
    insert: InsertBody<i64>,
    insert_step: InsertStepBody<i64>,
    query: QueryBody<i64, (), f64>,
    merge: Option<Folds<i64>>,
    prepare: Option<PrepareBody<i64>>,
) -> Result<Measurements, RunError> {
    scored_row(
        req,
        description,
        table,
        want,
        CardinalityGT {
            column: value_column(description),
        },
        peel::<i64>,
        insert,
        insert_step,
        query,
        merge,
        prepare,
    )
}

/// A row answering **subpopulation frequency**: how often a value occurs inside
/// a group.
#[allow(clippy::too_many_arguments)]
fn subpop_frequency_row(
    req: &Requirement,
    description: &TableDescription,
    table: GeneratedTable,
    want: &[(Operation, Metric)],
    insert: InsertBody<(String, i64)>,
    insert_step: InsertStepBody<(String, i64)>,
    query: QueryBody<(String, i64), (Vec<String>, i64), f64>,
    merge: Option<Folds<(String, i64)>>,
    prepare: Option<PrepareBody<(String, i64)>>,
) -> Result<Measurements, RunError> {
    scored_row(
        req,
        description,
        table,
        want,
        SubpopFrequencyGT::<i64>::over_columns(
            vec![SCORED_LABEL_COLUMN],
            value_column(description),
        ),
        peel_labeled::<i64>,
        insert,
        insert_step,
        query,
        merge,
        prepare,
    )
}

/// A row answering **subpopulation cardinality**: how many distinct values a
/// group holds. The statistic a Count-Min cell structurally cannot reach.
#[allow(clippy::too_many_arguments)]
fn subpop_cardinality_row(
    req: &Requirement,
    description: &TableDescription,
    table: GeneratedTable,
    want: &[(Operation, Metric)],
    insert: InsertBody<(String, i64)>,
    insert_step: InsertStepBody<(String, i64)>,
    query: QueryBody<(String, i64), Vec<String>, f64>,
    merge: Option<Folds<(String, i64)>>,
    prepare: Option<PrepareBody<(String, i64)>>,
) -> Result<Measurements, RunError> {
    scored_row(
        req,
        description,
        table,
        want,
        SubpopCardinalityGT {
            group_columns: vec![SCORED_LABEL_COLUMN],
            value_column: value_column(description),
        },
        peel_labeled::<i64>,
        insert,
        insert_step,
        query,
        merge,
        prepare,
    )
}

/// A row answering **subpopulation quantile**: the ordered statistic inside a
/// group, scored in rank error.
#[allow(clippy::too_many_arguments)]
fn subpop_quantile_row(
    req: &Requirement,
    description: &TableDescription,
    table: GeneratedTable,
    want: &[(Operation, Metric)],
    insert: InsertBody<(String, f64)>,
    insert_step: InsertStepBody<(String, f64)>,
    query: QueryBody<(String, f64), (Vec<String>, f64), f64>,
    merge: Option<Folds<(String, f64)>>,
    prepare: Option<PrepareBody<(String, f64)>>,
) -> Result<Measurements, RunError> {
    scored_row(
        req,
        description,
        table,
        want,
        SubpopRankErrorGT {
            group_columns: vec![SCORED_LABEL_COLUMN],
            value_column: value_column(description),
        },
        peel_labeled::<f64>,
        insert,
        insert_step,
        query,
        merge,
        prepare,
    )
}

#[allow(clippy::too_many_arguments)]
fn keyed_row<G>(
    req: &Requirement,
    description: &TableDescription,
    table: GeneratedTable,
    want: &[(Operation, Metric)],
    ground_truth: G,
    insert: InsertBody<(u64, i64)>,
    insert_step: InsertStepBody<(u64, i64)>,
    query: QueryBody<(u64, i64), (), f64>,
    merge: Option<Folds<(u64, i64)>>,
    prepare: Option<PrepareBody<(u64, i64)>>,
) -> Result<Measurements, RunError>
where
    G: GroundTruth<Probe = (), Answer = f64> + 'static,
    G::Truth: 'static,
{
    scored_row(
        req,
        description,
        table,
        want,
        ground_truth,
        peel_keyed,
        insert,
        insert_step,
        query,
        merge,
        prepare,
    )
}

/// A row answering **quantile**, scored in rank error: the value at a fraction
/// of the sorted stream. The one statistic whose rows build at either width.
#[allow(clippy::too_many_arguments)]
fn quantile_row<T: ColumnItem>(
    req: &Requirement,
    description: &TableDescription,
    table: GeneratedTable,
    want: &[(Operation, Metric)],
    insert: InsertBody<T>,
    insert_step: InsertStepBody<T>,
    query: QueryBody<T, f64, f64>,
    merge: Option<Folds<T>>,
    prepare: Option<PrepareBody<T>>,
) -> Result<Measurements, RunError> {
    scored_row(
        req,
        description,
        table,
        want,
        RankErrorGT {
            column: value_column(description),
        },
        peel::<T>,
        insert,
        insert_step,
        query,
        merge,
        prepare,
    )
}

#[allow(clippy::too_many_arguments)]
fn scored_row<G, I>(
    req: &Requirement,
    description: &TableDescription,
    table: GeneratedTable,
    want: &[(Operation, Metric)],
    ground_truth: G,
    materialise: Materialise<I>,
    insert: InsertBody<I>,
    insert_step: InsertStepBody<I>,
    query: QueryBody<I, G::Probe, G::Answer>,
    merge: Option<Folds<I>>,
    prepare: Option<PrepareBody<I>>,
) -> Result<Measurements, RunError>
where
    G: GroundTruth + 'static,
    G::Truth: 'static,
    G::Probe: 'static,
    G::Answer: 'static,
{
    let (probes, score) = questions(ground_truth, &table)?;
    let items = materialise(description, table)?;
    let mut bodies = Vec::with_capacity(want.len());
    for &(operation, metric) in want {
        let n = passes(req, metric);
        let body = match (operation, metric) {
            (Operation::Insert, Metric::Latency) => {
                stepped(insert_step(&req.params, items.clone(), n).map_err(cannot_build)?)
            }
            (Operation::Insert, _) => timed(
                insert(&req.params, items.clone(), n).map_err(cannot_build)?,
                items.len() as u64,
            ),
            (Operation::Query, _) => answered(
                query(&req.params, items.clone(), probes.clone(), n).map_err(cannot_build)?,
                score.clone(),
                metric,
            ),
            (Operation::Merge, Metric::Latency) => stepped(
                merge.expect(SUPPORTED).1(&req.params, items.clone(), shards(req), n)
                    .map_err(cannot_build)?,
            ),
            (Operation::Merge, _) => timed(
                merge.expect(SUPPORTED).0(&req.params, items.clone(), shards(req), n)
                    .map_err(cannot_build)?,
                (shards(req) - 1) as u64,
            ),
            (Operation::Prepare, _) => timed(
                prepare.expect(SUPPORTED)(&req.params, items.clone(), n).map_err(cannot_build)?,
                items.len() as u64,
            ),
        };
        bodies.push(((operation, metric), body));
    }
    Ok(bodies)
}

/// A row that answers **nothing**: measured but not scored. The parallel-insert
/// rows, whose worker sketches are dropped rather than asked, and whose ingest
/// is one call over the whole stream rather than a loop over it.
fn timed_row(
    req: &Requirement,
    description: &TableDescription,
    table: GeneratedTable,
    want: &[(Operation, Metric)],
    insert: ParallelInsertBody,
) -> Result<Measurements, RunError> {
    let items = peel::<i64>(description, table)?;
    let workers = req.workers.max(1);
    let mut bodies = Vec::with_capacity(want.len());
    for &(operation, metric) in want {
        let body =
            match operation {
                Operation::Insert => timed(
                    insert(&req.params, workers, items.clone(), passes(req, metric))
                        .map_err(cannot_build)?,
                    items.len() as u64,
                ),
                _ => return Err(RunError::Sketch(
                    "answers no statistic and folds nothing, so insert is all it is measured over"
                        .to_string(),
                )),
            };
        bodies.push(((operation, metric), body));
    }
    Ok(bodies)
}

// ---------- the row's closures, in the framework's terms ----------
//
// A wrapper hands back closures over a sketch and nothing else: how many units
// of work one pass covers, and what it answered, are read here.

/// A pass that only does work: its unit count is the work it covers, which the
/// row knows because it handed the stream over.
fn timed(passes: Vec<sketch_bench::wrappers::Pass>, work: u64) -> Measurement {
    passes
        .into_iter()
        .map(|pass| {
            Box::new(move || {
                let footprint = pass();
                Box::new(move || RunOutcome {
                    work,
                    memory_bytes: Some(footprint as u64),
                    ..Default::default()
                }) as Report
            }) as CorePass
        })
        .collect()
}

/// A pass driven one unit at a time, so the clock can be read per call.
fn stepped(passes: Vec<StepPass>) -> Measurement {
    passes
        .into_iter()
        .map(|pass| {
            let StepPass {
                steps,
                step,
                footprint,
            } = pass;
            Box::new(move || {
                let latency_ns = Some(record_calls(steps, step));
                Box::new(move || RunOutcome {
                    work: steps as u64,
                    memory_bytes: Some(footprint() as u64),
                    latency_ns,
                    ..Default::default()
                }) as Report
            }) as CorePass
        })
        .collect()
}

/// A pass that answers: the comparator scores what it said, here, once the
/// clock has stopped.
fn answered<A: 'static>(passes: Vec<QueryPass<A>>, score: Score<A>, metric: Metric) -> Measurement {
    passes
        .into_iter()
        .map(|pass| {
            let score = score.clone();
            Box::new(move || {
                let (answers, footprint) = pass();
                Box::new(move || RunOutcome {
                    work: answers.len() as u64,
                    memory_bytes: Some(footprint as u64),
                    scores: if metric == Metric::Accuracy {
                        score(&answers)
                    } else {
                        Default::default()
                    },
                    ..Default::default()
                }) as Report
            }) as CorePass
        })
        .collect()
}

// ---------- what every row does with what it named ----------

/// Materialise at the item type this row ingests: the value column, read at
/// the row's own width. The stream is moved into an `Rc` rather than copied:
/// every measurement of this row reads the same one.
fn peel<T: ColumnItem>(
    description: &TableDescription,
    table: GeneratedTable,
) -> Result<Rc<Vec<T>>, RunError> {
    let column = table.into_column(value_column(description))?;
    Ok(Rc::new(T::from_column(column)?))
}

/// The same, for the grouped rows: the columns before the value column joined
/// with `;` into one key, paired with the value. The join happens here rather
/// than on the insert path, so a wrapper feeding a library that takes `"a;b"`
/// pays nothing for it per item.
fn peel_labeled<V: ColumnItem>(
    description: &TableDescription,
    table: GeneratedTable,
) -> Result<Rc<Vec<(String, V)>>, RunError> {
    let value_column = value_column(description);
    if value_column == 0 {
        return Err(RunError::Sketch(format!(
            "this row ingests labelled records, so it needs at least one label \
             column before the value column; the description has {} column(s)",
            description.column_spec.len(),
        )));
    }
    let titles = table.column_title.clone();
    let mut columns = table.into_columns();
    if value_column >= columns.len() {
        return Err(RunError::Sketch(format!(
            "column {value_column} was asked for, but the table holds {}",
            columns.len(),
        )));
    }
    let values = V::from_column(columns.remove(value_column)).map_err(|e| {
        RunError::Sketch(format!(
            "value column '{}': {e}. The value column's data_type has to be the \
             row's item type",
            titles[value_column],
        ))
    })?;
    columns.truncate(value_column);
    let labels: Vec<Vec<String>> = columns
        .into_iter()
        .enumerate()
        .map(|(i, c)| {
            String::from_column(c).map_err(|e| {
                RunError::Sketch(format!(
                    "label column '{}': {e}. Label columns are rendered as text, \
                     so their data_type has to be `string`",
                    titles[i],
                ))
            })
        })
        .collect::<Result<_, RunError>>()?;

    let mut items = Vec::with_capacity(values.len());
    for (row, value) in values.into_iter().enumerate() {
        let mut key = String::new();
        for (column, labels) in labels.iter().enumerate() {
            if column > 0 {
                key.push(';');
            }
            key.push_str(&labels[row]);
        }
        items.push((key, value));
    }
    Ok(Rc::new(items))
}

fn peel_keyed(
    description: &TableDescription,
    table: GeneratedTable,
) -> Result<Rc<Vec<(u64, i64)>>, RunError> {
    let value_column = value_column(description);
    if value_column == 0 {
        return Err(RunError::Sketch(format!(
            "this row ingests keyed records, so it needs a key column before the \
             value column; the description has {} column(s)",
            description.column_spec.len(),
        )));
    }
    let titles = table.column_title.clone();
    let mut columns = table.into_columns();
    if value_column >= columns.len() {
        return Err(RunError::Sketch(format!(
            "column {value_column} was asked for, but the table holds {}",
            columns.len(),
        )));
    }
    let values = i64::from_column(columns.remove(value_column)).map_err(|e| {
        RunError::Sketch(format!(
            "value column '{}': {e}. A keyed row weights its keys with `i64`",
            titles[value_column],
        ))
    })?;
    let keys = u64::from_column(columns.remove(KEYED_KEY_COLUMN)).map_err(|e| {
        RunError::Sketch(format!(
            "key column '{}': {e}. A keyed row hashes its keys as `u64`",
            titles[KEYED_KEY_COLUMN],
        ))
    })?;
    Ok(Rc::new(keys.into_iter().zip(values).collect()))
}

/// The ordered rows build at either numeric width and at neither of the other
/// two: a KLL cell stores what it can compare. Nothing else reads `--dtype` —
/// a row is handed the data the frontend generated, and materialising it at the
/// row's own item type is what catches a stream it cannot ingest.
fn no_build_at(req: &Requirement, got: Dtype) -> RunError {
    RunError::Sketch(format!(
        "{}/{} builds at i64 or f64; --dtype {} is neither",
        req.algorithm,
        req.impl_name,
        got.name(),
    ))
}

// ---------- the rows whose shape or precision selects a type ----------
//
// Three runtime values that are really type choices. Each match below turns one
// back into a monomorphisation; past it, nothing sees a runtime value again.

/// `lg_k` selects a register-storage type, because `asap_sketchlib` puts the
/// register count in the type rather than in a field. A value outside the three
/// is refused by name rather than run at 14 under its own label.
pub(crate) fn row_hll_lib(
    req: &Requirement,
    description: &TableDescription,
    table: GeneratedTable,
    want: &[(Operation, Metric)],
) -> Result<Measurements, RunError> {
    use sketch_bench::wrappers::hll::sketchlib as hl;
    use sketch_bench::wrappers::{HllBucketListP12, HllBucketListP14, HllBucketListP16};
    macro_rules! at {
        ($r:ty) => {
            cardinality_row(
                req,
                description,
                table,
                want,
                hl::insert_hll_lib::<$r>,
                hl::insert_step_hll_lib::<$r>,
                hl::query_hll_lib::<$r>,
                Some((hl::merge_hll_lib::<$r>, hl::merge_step_hll_lib::<$r>)),
                None,
            )
        };
    }
    match lg_k(req)? {
        12 => at!(HllBucketListP12),
        14 => at!(HllBucketListP14),
        16 => at!(HllBucketListP16),
        other => Err(unsupported_precision(other)),
    }
}

/// The HIP variant: same precision dispatch, but no merge — it maintains its
/// estimate on the insert path and the library supplies no fold.
pub(crate) fn row_hll_hip_lib(
    req: &Requirement,
    description: &TableDescription,
    table: GeneratedTable,
    want: &[(Operation, Metric)],
) -> Result<Measurements, RunError> {
    use sketch_bench::wrappers::hll::sketchlib as hl;
    use sketch_bench::wrappers::{HllBucketListP12, HllBucketListP14, HllBucketListP16};
    macro_rules! at {
        ($r:ty) => {
            cardinality_row(
                req,
                description,
                table,
                want,
                hl::insert_hll_lib_hip::<$r>,
                hl::insert_step_hll_lib_hip::<$r>,
                hl::query_hll_lib_hip::<$r>,
                None,
                None,
            )
        };
    }
    match lg_k(req)? {
        12 => at!(HllBucketListP12),
        14 => at!(HllBucketListP14),
        16 => at!(HllBucketListP16),
        other => Err(unsupported_precision(other)),
    }
}

fn lg_k(req: &Requirement) -> Result<u8, RunError> {
    let p: sketch_bench::params::HllParams = req
        .params
        .parse()
        .map_err(|e: aqpbm_core::DataGenError| RunError::Sketch(e.to_string()))?;
    Ok(p.lg_k)
}

fn unsupported_precision(lg_k: u8) -> RunError {
    RunError::Sketch(format!(
        "asap_sketchlib HLL is compiled at lg_k 12, 14 and 16; requested {lg_k}"
    ))
}

/// `(rows, cols)` selects a matrix storage type, because `impl_fixed_matrix!`
/// bakes the shape in — which is the thing these rows exist to price. The visitor
/// is the only way to hand a monomorphisation back to a caller that picked it
/// with two runtime integers, and the measurements are what it hands back: an
/// erased list, so nothing in the return type mentions the shape it ran at.
macro_rules! fixed_matrix_row {
    ($fname:ident, $params:ty, $algo:literal, $module:ident, $insert:ident, $insert_step:ident,
     $query:ident, $merge:ident, $merge_step:ident) => {
        pub(crate) fn $fname(
            req: &Requirement,
            description: &TableDescription,
            table: GeneratedTable,
            want: &[(Operation, Metric)],
        ) -> Result<Measurements, RunError> {
            use sketch_bench::wrappers::fixed_matrix::{
                unsupported_shape, with_fixed_matrix, FixedMatrixVisitor,
            };
            use sketch_bench::wrappers::$module::sketchlib as w;
            use sketch_bench::wrappers::{DefaultXxHasher, FastPathHasher, MatrixStorage};

            let p: $params = req
                .params
                .parse()
                .map_err(|e: aqpbm_core::DataGenError| RunError::Sketch(e.to_string()))?;

            struct V<'a> {
                req: &'a Requirement,
                description: &'a TableDescription,
                table: GeneratedTable,
                want: &'a [(Operation, Metric)],
            }
            impl FixedMatrixVisitor for V<'_> {
                type Out = Result<Measurements, RunError>;
                fn visit<M>(self) -> Self::Out
                where
                    M: MatrixStorage<Counter = i32>
                        + FastPathHasher<DefaultXxHasher>
                        + Default
                        + Clone
                        + 'static,
                {
                    frequency_row(
                        self.req,
                        self.description,
                        self.table,
                        self.want,
                        w::$insert::<M>,
                        w::$insert_step::<M>,
                        w::$query::<M>,
                        Some((w::$merge::<M>, w::$merge_step::<M>)),
                        None,
                    )
                }
            }

            with_fixed_matrix(
                p.rows,
                p.cols,
                V {
                    req,
                    description,
                    table,
                    want,
                },
            )
            .unwrap_or_else(|| Err(RunError::Sketch(unsupported_shape($algo, p.rows, p.cols))))
        }
    };
}

fixed_matrix_row!(
    row_cms_fastpath_fixedmatrix_lib,
    sketch_bench::params::CmsParams,
    "cms-fastpath-fixedmatrix",
    cms,
    insert_cms_lib_fixedmatrix,
    insert_step_cms_lib_fixedmatrix,
    query_cms_lib_fixedmatrix,
    merge_cms_lib_fixedmatrix,
    merge_step_cms_lib_fixedmatrix
);

fixed_matrix_row!(
    row_countsketch_fastpath_fixedmatrix_lib,
    sketch_bench::params::CountSketchParams,
    "countsketch-fastpath-fixedmatrix",
    cs,
    insert_cs_lib_fixedmatrix,
    insert_step_cs_lib_fixedmatrix,
    query_cs_lib_fixedmatrix,
    merge_cs_lib_fixedmatrix,
    merge_step_cs_lib_fixedmatrix
);

#[cfg(test)]
mod tests {
    use super::binding;
    use sketch_bench::registry::REGISTRY;

    #[test]
    fn every_registered_pair_is_bound() {
        for e in REGISTRY {
            assert!(
                binding(e.algorithm, e.impl_name).is_some(),
                "{}/{}",
                e.algorithm,
                e.impl_name
            );
        }
    }
}
