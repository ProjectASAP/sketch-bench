//! Which sketch a pair of strings names: the only place in the program where a
//! name is bound to code. [`registry`](crate::registry) declares what rows
//! exist; this binds each one to its wrapper.
//!
//! A row is entered once. It materialises the data at its own item type, draws
//! the exact answer and the probes once, and hands back one closure per
//! measurement asked for — so nothing typed has to cross back to the frontend.

use std::collections::BTreeMap;
use std::rc::Rc;

use crate::params::ParamSet;
use crate::request::{Dtype, Requirement};
use aqpbm_core::accuracy::cardinality::CardinalityGT;
use aqpbm_core::accuracy::frequency::FrequencyGT;
use aqpbm_core::accuracy::quantile::{RankErrorGT, ToF64};
use aqpbm_core::accuracy::subpopulation::{
    SubpopCardinalityGT, SubpopFrequencyGT, SubpopRankErrorGT,
};
use aqpbm_core::error::RunError;
use aqpbm_core::input_dataset::{BenchItem, InputDataSetData, InputDataSetDescription, Labeled};
use aqpbm_core::measurement::{
    bulk, insert_measurement, merge_measurement, per_item, prepare_measurement, query_measurement,
    questions, Insert, Measurement, Questions, Sketch,
};
use aqpbm_core::metrics::{Metric, Operation};

/// The label column every subpopulation comparator scores over.
const SCORED_LABEL_COLUMN: usize = 0;

/// One invocation's worth of work: what the data turned out to be, and one
/// closure per measurement, in the order they were asked for.
pub struct Measurements {
    pub dataset: InputDataSetDescription,
    pub bodies: Vec<((Operation, Metric), Measurement)>,
}

type Bodies = Vec<((Operation, Metric), Measurement)>;

/// Bind a row to its wrapper and hand back the closures. Each arm names only
/// the statistic and the row's `build` / `insert` / `query`. `None` means the
/// pair has no arm yet.
pub fn measurements(
    req: &Requirement,
    data: InputDataSetData,
    want: &[(Operation, Metric)],
) -> Option<Result<Measurements, RunError>> {
    use crate::wrappers::cms::{datasketches as cd, oxide as co, polars as cp, sketchlib as cl};
    use crate::wrappers::cs::{oxide as so, polars as sp, sketchlib as sl};
    use crate::wrappers::hll::{datasketches as hd, oxide as ho, polars as hpo, sketchlib as hl};
    use crate::wrappers::hydra::{polars as hp, sketchlib as hs};
    use crate::wrappers::kll::{oxide as ko, polars as kp, sketchlib as kl};

    Some(match (req.algorithm.as_str(), req.impl_name.as_str()) {
        // -------- CMS (frequency) --------
        ("cms", "oxide") => frequency_row(
            req,
            data,
            want,
            co::build_cms_oxide,
            co::memory_cms_oxide,
            co::insert_cms_oxide,
            co::query_cms_oxide,
            Some(co::merge_cms_oxide),
            None,
        ),
        ("cms", "datasketches") => frequency_row(
            req,
            data,
            want,
            cd::build_cms_datasketches,
            cd::memory_cms_datasketches,
            cd::insert_cms_datasketches,
            cd::query_cms_datasketches,
            Some(cd::merge_cms_datasketches),
            None,
        ),
        ("cms", "polars") => frequency_row(
            req,
            data,
            want,
            cp::build_polars_frequency_cms,
            cp::memory_polars_frequency_cms,
            cp::insert_polars_frequency_cms,
            cp::query_polars_frequency_cms,
            None,
            Some(cp::prepare_polars_frequency_cms),
        ),
        ("cms-fastpath-fixedmatrix", "lib") => fixed_matrix_cms(req, data, want),
        ("cms-fastpath-vector2d", "lib") => frequency_row(
            req,
            data,
            want,
            cl::build_cms_lib_vector2d_fast,
            cl::memory_cms_lib_vector2d_fast,
            cl::insert_cms_lib_vector2d_fast,
            cl::query_cms_lib_vector2d_fast,
            Some(cl::merge_cms_lib_vector2d_fast),
            None,
        ),
        ("cms-regularpath-vector2d", "lib") => frequency_row(
            req,
            data,
            want,
            cl::build_cms_lib_vector2d_regular,
            cl::memory_cms_lib_vector2d_regular,
            cl::insert_cms_lib_vector2d_regular,
            cl::query_cms_lib_vector2d_regular,
            Some(cl::merge_cms_lib_vector2d_regular),
            None,
        ),
        ("cms-fastpath-fixedmatrix-32k-parallel", "lib") => timed_row(
            req,
            data,
            want,
            cl::build_parallel_cms_fast_path,
            cl::memory_parallel_cms_fast_path,
            cl::insert_parallel_cms_fast_path,
            None,
            None,
        ),

        // -------- CountSketch (frequency) --------
        ("countsketch", "oxide") => frequency_row(
            req,
            data,
            want,
            so::build_cs_oxide,
            so::memory_cs_oxide,
            so::insert_cs_oxide,
            so::query_cs_oxide,
            Some(so::merge_cs_oxide),
            None,
        ),
        ("countsketch", "polars") => frequency_row(
            req,
            data,
            want,
            sp::build_polars_frequency_cs,
            sp::memory_polars_frequency_cs,
            sp::insert_polars_frequency_cs,
            sp::query_polars_frequency_cs,
            None,
            Some(sp::prepare_polars_frequency_cs),
        ),
        ("countsketch-fastpath-fixedmatrix", "lib") => fixed_matrix_cs(req, data, want),
        ("countsketch-fastpath-vector2d", "lib") => frequency_row(
            req,
            data,
            want,
            sl::build_cs_lib_vector2d_fast,
            sl::memory_cs_lib_vector2d_fast,
            sl::insert_cs_lib_vector2d_fast,
            sl::query_cs_lib_vector2d_fast,
            Some(sl::merge_cs_lib_vector2d_fast),
            None,
        ),
        ("countsketch-regularpath-vector2d", "lib") => frequency_row(
            req,
            data,
            want,
            sl::build_cs_lib_vector2d_regular,
            sl::memory_cs_lib_vector2d_regular,
            sl::insert_cs_lib_vector2d_regular,
            sl::query_cs_lib_vector2d_regular,
            Some(sl::merge_cs_lib_vector2d_regular),
            None,
        ),
        ("countsketch-fastpath-fixedmatrix-32k-parallel", "lib") => timed_row(
            req,
            data,
            want,
            sl::build_parallel_cs_fast_path,
            sl::memory_parallel_cs_fast_path,
            sl::insert_parallel_cs_fast_path,
            None,
            None,
        ),

        // -------- HLL (cardinality) --------
        ("hll", "oxide") => cardinality_row(
            req,
            data,
            want,
            ho::build_hll_oxide,
            ho::memory_hll_oxide,
            ho::insert_hll_oxide,
            ho::query_hll_oxide,
            Some(ho::merge_hll_oxide),
            None,
        ),
        ("hll", "datasketches") => cardinality_row(
            req,
            data,
            want,
            hd::build_hll_datasketches,
            hd::memory_hll_datasketches,
            hd::insert_hll_datasketches,
            hd::query_hll_datasketches,
            Some(hd::merge_hll_datasketches),
            None,
        ),
        ("hll", "lib") => hll_lib(req, data, want),
        ("hll", "polars") => cardinality_row(
            req,
            data,
            want,
            hpo::build_polars_cardinality,
            hpo::memory_polars_cardinality,
            hpo::insert_polars_cardinality,
            hpo::query_polars_cardinality,
            None,
            Some(hpo::prepare_polars_cardinality),
        ),
        ("hll-hip", "lib") => hll_lib_hip(req, data, want),
        ("hll-fastpath-parallel", "lib") => timed_row(
            req,
            data,
            want,
            hl::build_parallel_hll_fast_path,
            hl::memory_parallel_hll_fast_path,
            hl::insert_parallel_hll_fast_path,
            None,
            None,
        ),

        // -------- KLL (quantile) --------
        // The only rows `--dtype` selects anything for: their library is generic
        // over the value type, so a width picks a monomorphisation rather than
        // being refused. Each arm is its own instantiation.
        ("kll-percall", "oxide") => match req.width {
            Dtype::I64 => quantile_row::<i64, _, _, _, _, _>(
                req,
                data,
                want,
                ko::build_kll_oxide_per_call::<i64>,
                ko::memory_kll_oxide_per_call::<i64>,
                ko::insert_kll_oxide_per_call::<i64>,
                ko::query_kll_oxide_per_call::<i64>,
                Some(ko::merge_kll_oxide_per_call::<i64>),
                None,
            ),
            Dtype::F64 => quantile_row::<f64, _, _, _, _, _>(
                req,
                data,
                want,
                ko::build_kll_oxide_per_call::<f64>,
                ko::memory_kll_oxide_per_call::<f64>,
                ko::insert_kll_oxide_per_call::<f64>,
                ko::query_kll_oxide_per_call::<f64>,
                Some(ko::merge_kll_oxide_per_call::<f64>),
                None,
            ),
            other => Err(no_build_at(req, other)),
        },
        ("kll-percall", "lib") => match req.width {
            Dtype::I64 => quantile_row::<i64, _, _, _, _, _>(
                req,
                data,
                want,
                kl::build_kll_lib_per_call::<i64>,
                kl::memory_kll_lib_per_call::<i64>,
                kl::insert_kll_lib_per_call::<i64>,
                kl::query_kll_lib_per_call::<i64>,
                Some(kl::merge_kll_lib_per_call::<i64>),
                None,
            ),
            Dtype::F64 => quantile_row::<f64, _, _, _, _, _>(
                req,
                data,
                want,
                kl::build_kll_lib_per_call::<f64>,
                kl::memory_kll_lib_per_call::<f64>,
                kl::insert_kll_lib_per_call::<f64>,
                kl::query_kll_lib_per_call::<f64>,
                Some(kl::merge_kll_lib_per_call::<f64>),
                None,
            ),
            other => Err(no_build_at(req, other)),
        },
        ("kll-cdf", "oxide") => match req.width {
            Dtype::I64 => quantile_row::<i64, _, _, _, _, _>(
                req,
                data,
                want,
                ko::build_kll_oxide_cdf::<i64>,
                ko::memory_kll_oxide_cdf::<i64>,
                ko::insert_kll_oxide_cdf::<i64>,
                ko::query_kll_oxide_cdf::<i64>,
                Some(ko::merge_kll_oxide_cdf::<i64>),
                Some(ko::prepare_kll_oxide_cdf::<i64>),
            ),
            Dtype::F64 => quantile_row::<f64, _, _, _, _, _>(
                req,
                data,
                want,
                ko::build_kll_oxide_cdf::<f64>,
                ko::memory_kll_oxide_cdf::<f64>,
                ko::insert_kll_oxide_cdf::<f64>,
                ko::query_kll_oxide_cdf::<f64>,
                Some(ko::merge_kll_oxide_cdf::<f64>),
                Some(ko::prepare_kll_oxide_cdf::<f64>),
            ),
            other => Err(no_build_at(req, other)),
        },
        ("kll-cdf", "lib") => match req.width {
            Dtype::I64 => quantile_row::<i64, _, _, _, _, _>(
                req,
                data,
                want,
                kl::build_kll_lib_cdf::<i64>,
                kl::memory_kll_lib_cdf::<i64>,
                kl::insert_kll_lib_cdf::<i64>,
                kl::query_kll_lib_cdf::<i64>,
                Some(kl::merge_kll_lib_cdf::<i64>),
                Some(kl::prepare_kll_lib_cdf::<i64>),
            ),
            Dtype::F64 => quantile_row::<f64, _, _, _, _, _>(
                req,
                data,
                want,
                kl::build_kll_lib_cdf::<f64>,
                kl::memory_kll_lib_cdf::<f64>,
                kl::insert_kll_lib_cdf::<f64>,
                kl::query_kll_lib_cdf::<f64>,
                Some(kl::merge_kll_lib_cdf::<f64>),
                Some(kl::prepare_kll_lib_cdf::<f64>),
            ),
            other => Err(no_build_at(req, other)),
        },
        // The exact baseline is i64 only, unlike the four sketch rows above it:
        // its grid is built from a sorted i64 column.
        ("kll-cdf", "polars") => quantile_row::<i64, _, _, _, _, _>(
            req,
            data,
            want,
            kp::build_polars_quantile_kll,
            kp::memory_polars_quantile_kll,
            kp::insert_polars_quantile_kll,
            kp::query_polars_quantile_kll,
            None,
            Some(kp::prepare_polars_quantile_kll),
        ),

        // -------- Hydra (per-subpopulation statistics over labelled records) --------
        ("hydra-cms", "lib") => subpop_frequency_row(
            req,
            data,
            want,
            hs::build_hydra_cms,
            hs::memory_hydra_cms,
            hs::insert_hydra_cms,
            hs::query_hydra_cms,
            Some(hs::merge_hydra_cms),
            None,
        ),
        ("hydra-cms", "polars") => subpop_frequency_row(
            req,
            data,
            want,
            hp::build_polars_subpop_frequency,
            hp::memory_polars_subpop_frequency,
            hp::insert_polars_subpop_frequency,
            hp::query_polars_subpop_frequency,
            None,
            Some(hp::prepare_polars_subpop_frequency),
        ),
        ("hydra-hll", "lib") => subpop_cardinality_row(
            req,
            data,
            want,
            hs::build_hydra_hll,
            hs::memory_hydra_hll,
            hs::insert_hydra_hll,
            hs::query_hydra_hll,
            Some(hs::merge_hydra_hll),
            None,
        ),
        ("hydra-hll", "polars") => subpop_cardinality_row(
            req,
            data,
            want,
            hp::build_polars_subpop_cardinality,
            hp::memory_polars_subpop_cardinality,
            hp::insert_polars_subpop_cardinality,
            hp::query_polars_subpop_cardinality,
            None,
            Some(hp::prepare_polars_subpop_cardinality),
        ),
        ("hydra-kll", "lib") => subpop_quantile_row(
            req,
            data,
            want,
            hs::build_hydra_kll,
            hs::memory_hydra_kll,
            hs::insert_hydra_kll,
            hs::query_hydra_kll,
            Some(hs::merge_hydra_kll),
            None,
        ),
        ("hydra-kll", "polars") => subpop_quantile_row(
            req,
            data,
            want,
            hp::build_polars_subpop_quantile,
            hp::memory_polars_subpop_quantile,
            hp::insert_polars_subpop_quantile,
            hp::query_polars_subpop_quantile,
            None,
            Some(hp::prepare_polars_subpop_quantile),
        ),

        _ => return None,
    })
}

// ---------- the statistic a row answers ----------

/// A row answering **frequency**: how often a key occurs in the stream.
#[allow(clippy::too_many_arguments)]
fn frequency_row<S, Bf, M, Per, Q>(
    req: &Requirement,
    data: InputDataSetData,
    want: &[(Operation, Metric)],
    build: Bf,
    footprint: M,
    insert: Per,
    query: Q,
    merge: Option<fn(&mut S, &S)>,
    prepare: Option<fn(&mut S)>,
) -> Result<Measurements, RunError>
where
    S: 'static,
    Bf: Fn(&ParamSet) -> Result<S, RunError> + Copy + 'static,
    M: Fn(&S) -> usize + Copy + 'static,
    Per: Fn(&mut S, &i64) + Copy + 'static,
    Q: Fn(&mut S, &i64) -> u64 + Copy + 'static,
{
    let (dataset, items) = peel::<i64>(data)?;
    let questions = questions(FrequencyGT, &items);
    scored(
        req,
        want,
        sketch(req, build, footprint),
        per_item(insert),
        items,
        merge,
        prepare,
        query,
        questions,
    )
    .map(|bodies| Measurements { dataset, bodies })
}

/// A row answering **cardinality**: how many distinct keys the stream carried.
/// The probe is `()` — there is one question, asked repeatedly.
#[allow(clippy::too_many_arguments)]
fn cardinality_row<S, Bf, M, Per, Q>(
    req: &Requirement,
    data: InputDataSetData,
    want: &[(Operation, Metric)],
    build: Bf,
    footprint: M,
    insert: Per,
    query: Q,
    merge: Option<fn(&mut S, &S)>,
    prepare: Option<fn(&mut S)>,
) -> Result<Measurements, RunError>
where
    S: 'static,
    Bf: Fn(&ParamSet) -> Result<S, RunError> + Copy + 'static,
    M: Fn(&S) -> usize + Copy + 'static,
    Per: Fn(&mut S, &i64) + Copy + 'static,
    Q: Fn(&mut S, &()) -> f64 + Copy + 'static,
{
    let (dataset, items) = peel::<i64>(data)?;
    let questions = questions(CardinalityGT, &items);
    scored(
        req,
        want,
        sketch(req, build, footprint),
        per_item(insert),
        items,
        merge,
        prepare,
        query,
        questions,
    )
    .map(|bodies| Measurements { dataset, bodies })
}

/// A row answering **quantile**, scored in rank error: the value at a fraction
/// of the sorted stream. The one statistic whose rows build at either width.
#[allow(clippy::too_many_arguments)]
fn quantile_row<T, S, Bf, M, Per, Q>(
    req: &Requirement,
    data: InputDataSetData,
    want: &[(Operation, Metric)],
    build: Bf,
    footprint: M,
    insert: Per,
    query: Q,
    merge: Option<fn(&mut S, &S)>,
    prepare: Option<fn(&mut S)>,
) -> Result<Measurements, RunError>
where
    T: BenchItem + Clone + PartialOrd + ToF64 + 'static,
    S: 'static,
    Bf: Fn(&ParamSet) -> Result<S, RunError> + Copy + 'static,
    M: Fn(&S) -> usize + Copy + 'static,
    Per: Fn(&mut S, &T) + Copy + 'static,
    Q: Fn(&mut S, &f64) -> f64 + Copy + 'static,
{
    let (dataset, items) = peel::<T>(data)?;
    let questions = questions(RankErrorGT, &items);
    scored(
        req,
        want,
        sketch(req, build, footprint),
        per_item(insert),
        items,
        merge,
        prepare,
        query,
        questions,
    )
    .map(|bodies| Measurements { dataset, bodies })
}

/// A row answering **subpopulation frequency**: how often a value occurs inside
/// a group.
#[allow(clippy::too_many_arguments)]
fn subpop_frequency_row<S, Bf, M, Per, Q>(
    req: &Requirement,
    data: InputDataSetData,
    want: &[(Operation, Metric)],
    build: Bf,
    footprint: M,
    insert: Per,
    query: Q,
    merge: Option<fn(&mut S, &S)>,
    prepare: Option<fn(&mut S)>,
) -> Result<Measurements, RunError>
where
    S: 'static,
    Bf: Fn(&ParamSet) -> Result<S, RunError> + Copy + 'static,
    M: Fn(&S) -> usize + Copy + 'static,
    Per: Fn(&mut S, &Labeled<i64>) + Copy + 'static,
    Q: Fn(&mut S, &(String, i64)) -> f64 + Copy + 'static,
{
    let (dataset, items) = peel::<Labeled<i64>>(data)?;
    let questions = questions(
        SubpopFrequencyGT {
            label_column: SCORED_LABEL_COLUMN,
        },
        &items,
    );
    scored(
        req,
        want,
        sketch(req, build, footprint),
        per_item(insert),
        items,
        merge,
        prepare,
        query,
        questions,
    )
    .map(|bodies| Measurements { dataset, bodies })
}

/// A row answering **subpopulation cardinality**: how many distinct values a
/// group holds. The statistic a Count-Min cell structurally cannot reach.
#[allow(clippy::too_many_arguments)]
fn subpop_cardinality_row<S, Bf, M, Per, Q>(
    req: &Requirement,
    data: InputDataSetData,
    want: &[(Operation, Metric)],
    build: Bf,
    footprint: M,
    insert: Per,
    query: Q,
    merge: Option<fn(&mut S, &S)>,
    prepare: Option<fn(&mut S)>,
) -> Result<Measurements, RunError>
where
    S: 'static,
    Bf: Fn(&ParamSet) -> Result<S, RunError> + Copy + 'static,
    M: Fn(&S) -> usize + Copy + 'static,
    Per: Fn(&mut S, &Labeled<i64>) + Copy + 'static,
    Q: Fn(&mut S, &String) -> f64 + Copy + 'static,
{
    let (dataset, items) = peel::<Labeled<i64>>(data)?;
    let questions = questions(
        SubpopCardinalityGT {
            label_column: SCORED_LABEL_COLUMN,
        },
        &items,
    );
    scored(
        req,
        want,
        sketch(req, build, footprint),
        per_item(insert),
        items,
        merge,
        prepare,
        query,
        questions,
    )
    .map(|bodies| Measurements { dataset, bodies })
}

/// A row answering **subpopulation quantile**: the ordered statistic inside a
/// group, scored in rank error.
#[allow(clippy::too_many_arguments)]
fn subpop_quantile_row<S, Bf, M, Per, Q>(
    req: &Requirement,
    data: InputDataSetData,
    want: &[(Operation, Metric)],
    build: Bf,
    footprint: M,
    insert: Per,
    query: Q,
    merge: Option<fn(&mut S, &S)>,
    prepare: Option<fn(&mut S)>,
) -> Result<Measurements, RunError>
where
    S: 'static,
    Bf: Fn(&ParamSet) -> Result<S, RunError> + Copy + 'static,
    M: Fn(&S) -> usize + Copy + 'static,
    Per: Fn(&mut S, &Labeled<f64>) + Copy + 'static,
    Q: Fn(&mut S, &(String, f64)) -> f64 + Copy + 'static,
{
    let (dataset, items) = peel::<Labeled<f64>>(data)?;
    let questions = questions(
        SubpopRankErrorGT {
            label_column: SCORED_LABEL_COLUMN,
        },
        &items,
    );
    scored(
        req,
        want,
        sketch(req, build, footprint),
        per_item(insert),
        items,
        merge,
        prepare,
        query,
        questions,
    )
    .map(|bodies| Measurements { dataset, bodies })
}

/// A row that answers **nothing**: measured but not scored. The parallel-insert
/// rows, whose worker sketches are dropped rather than asked, and whose ingest
/// is one call over the whole stream rather than a loop over it.
#[allow(clippy::too_many_arguments)]
fn timed_row<S, Bf, M>(
    req: &Requirement,
    data: InputDataSetData,
    want: &[(Operation, Metric)],
    build: Bf,
    footprint: M,
    ingest: fn(&mut S, &[i64]),
    merge: Option<fn(&mut S, &S)>,
    prepare: Option<fn(&mut S)>,
) -> Result<Measurements, RunError>
where
    S: 'static,
    Bf: Fn(&ParamSet, usize) -> Result<S, RunError> + Copy + 'static,
    M: Fn(&S) -> usize + Copy + 'static,
{
    let (dataset, items) = peel::<i64>(data)?;
    timed(
        req,
        want,
        parallel_sketch(req, build, footprint),
        bulk(ingest),
        items,
        merge,
        prepare,
    )
    .map(|bodies| Measurements { dataset, bodies })
}

// ---------- what every row does with what it named ----------

/// Materialise at the item type this row ingests, and take the stream away from
/// the dataset rather than copying it.
fn peel<T: BenchItem>(
    data: InputDataSetData,
) -> Result<(InputDataSetDescription, Rc<Vec<T>>), RunError> {
    let (description, items) = T::materialise(data)?;
    Ok((description, Rc::new(items)))
}

/// The row's construction vocabulary, captured. Past here nothing reads a
/// [`ParamSet`], which is what keeps the framework out of the sketch's dialect.
fn sketch<S, Bf, M>(
    req: &Requirement,
    build: Bf,
    footprint: M,
) -> Sketch<impl Fn() -> Result<S, RunError> + Clone + 'static, M>
where
    Bf: Fn(&ParamSet) -> Result<S, RunError> + Copy + 'static,
    M: Fn(&S) -> usize + Copy + 'static,
{
    let params = req.params.clone();
    Sketch {
        build: move || build(&params),
        footprint,
    }
}

/// The same, for the rows that spread their ingest over worker threads — the
/// only ones that read the thread count, so the only ones it reaches. A run of
/// no threads is not a run, so it is floored here rather than trusted from the
/// request.
fn parallel_sketch<S, Bf, M>(
    req: &Requirement,
    build: Bf,
    footprint: M,
) -> Sketch<impl Fn() -> Result<S, RunError> + Clone + 'static, M>
where
    Bf: Fn(&ParamSet, usize) -> Result<S, RunError> + Copy + 'static,
    M: Fn(&S) -> usize + Copy + 'static,
{
    let (params, workers) = (req.params.clone(), req.workers.max(1));
    Sketch {
        build: move || build(&params, workers),
        footprint,
    }
}

/// The measurements a row with a comparator answers.
#[allow(clippy::too_many_arguments)]
fn scored<S, P, A, I, B, M, Per, Q, Sc>(
    req: &Requirement,
    want: &[(Operation, Metric)],
    sketch: Sketch<B, M>,
    insert: Insert<Per, S, I>,
    items: Rc<Vec<I>>,
    merge: Option<fn(&mut S, &S)>,
    prepare: Option<fn(&mut S)>,
    query: Q,
    questions: Questions<P, Sc>,
) -> Result<Bodies, RunError>
where
    S: 'static,
    I: 'static,
    P: 'static,
    A: 'static,
    B: Fn() -> Result<S, RunError> + Clone + 'static,
    M: Fn(&S) -> usize + Copy + 'static,
    Per: Fn(&mut S, &I) + Copy + 'static,
    Q: Fn(&mut S, &P) -> A + Copy + 'static,
    Sc: Fn(&[A]) -> BTreeMap<String, f64> + Clone + 'static,
{
    // Construction is proved here, once, so a config the row cannot satisfy
    // fails before any measurement ships — which is what lets every body treat
    // its own build as infallible.
    (sketch.build)()?;
    let mut bodies = Vec::with_capacity(want.len());
    for &(operation, metric) in want {
        let body = match operation {
            Operation::Insert => insert_measurement(metric, sketch.clone(), insert, items.clone())?,
            Operation::Query => query_measurement(
                metric,
                sketch.clone(),
                insert,
                items.clone(),
                prepare,
                query,
                questions.clone(),
            )?,
            Operation::Merge => merge_measurement(
                metric,
                sketch.clone(),
                insert,
                items.clone(),
                merge.ok_or_else(no_merge)?,
                req.merge_shards,
            )?,
            Operation::Prepare => prepare_measurement(
                metric,
                sketch.clone(),
                insert,
                items.clone(),
                prepare.ok_or_else(no_prepare)?,
            )?,
        };
        bodies.push(((operation, metric), body));
    }
    Ok(bodies)
}

/// The same, for a row nothing scores: insert, prepare and merge only, no
/// comparator and no probes.
#[allow(clippy::too_many_arguments)]
fn timed<S, I, B, M, Per>(
    req: &Requirement,
    want: &[(Operation, Metric)],
    sketch: Sketch<B, M>,
    insert: Insert<Per, S, I>,
    items: Rc<Vec<I>>,
    merge: Option<fn(&mut S, &S)>,
    prepare: Option<fn(&mut S)>,
) -> Result<Bodies, RunError>
where
    S: 'static,
    I: 'static,
    B: Fn() -> Result<S, RunError> + Clone + 'static,
    M: Fn(&S) -> usize + Copy + 'static,
    Per: Fn(&mut S, &I) + Copy + 'static,
{
    (sketch.build)()?;
    let mut bodies = Vec::with_capacity(want.len());
    for &(operation, metric) in want {
        let body = match operation {
            Operation::Insert => insert_measurement(metric, sketch.clone(), insert, items.clone())?,
            Operation::Query => {
                return Err(RunError::Sketch(
                    "answers no statistic, so there is nothing to ask it".to_string(),
                ))
            }
            Operation::Merge => merge_measurement(
                metric,
                sketch.clone(),
                insert,
                items.clone(),
                merge.ok_or_else(no_merge)?,
                req.merge_shards,
            )?,
            Operation::Prepare => prepare_measurement(
                metric,
                sketch.clone(),
                insert,
                items.clone(),
                prepare.ok_or_else(no_prepare)?,
            )?,
        };
        bodies.push(((operation, metric), body));
    }
    Ok(bodies)
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

fn no_merge() -> RunError {
    RunError::Sketch("provides no merge".to_string())
}

fn no_prepare() -> RunError {
    RunError::Sketch("provides no prepare".to_string())
}

// ---------- the rows whose shape or precision selects a type ----------
//
// Three runtime values that are really type choices. Each match below turns one
// back into a monomorphisation; past it, nothing sees a runtime value again.

/// `lg_k` selects a register-storage type, because `asap_sketchlib` puts the
/// register count in the type rather than in a field. A value outside the three
/// is refused by name rather than run at 14 under its own label.
fn hll_lib(
    req: &Requirement,
    data: InputDataSetData,
    want: &[(Operation, Metric)],
) -> Result<Measurements, RunError> {
    use crate::wrappers::hll::sketchlib as hl;
    use crate::wrappers::{HllBucketListP12, HllBucketListP14, HllBucketListP16};
    macro_rules! at {
        ($r:ty) => {
            cardinality_row(
                req,
                data,
                want,
                hl::build_hll_lib::<$r>,
                hl::memory_hll_lib::<$r>,
                hl::insert_hll_lib::<$r>,
                hl::query_hll_lib::<$r>,
                Some(hl::merge_hll_lib::<$r>),
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
fn hll_lib_hip(
    req: &Requirement,
    data: InputDataSetData,
    want: &[(Operation, Metric)],
) -> Result<Measurements, RunError> {
    use crate::wrappers::hll::sketchlib as hl;
    use crate::wrappers::{HllBucketListP12, HllBucketListP14, HllBucketListP16};
    macro_rules! at {
        ($r:ty) => {
            cardinality_row(
                req,
                data,
                want,
                hl::build_hll_lib_hip::<$r>,
                hl::memory_hll_lib_hip::<$r>,
                hl::insert_hll_lib_hip::<$r>,
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
    let p: crate::params::HllParams = req
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
/// with two runtime integers, and the batch is what it hands back: an erased
/// list, so nothing in the return type mentions the shape it was built at.
macro_rules! fixed_matrix_row {
    ($fname:ident, $params:ty, $algo:literal, $module:ident, $build:ident, $memory:ident,
     $insert:ident, $query:ident, $merge:ident) => {
        fn $fname(
            req: &Requirement,
            data: InputDataSetData,
            want: &[(Operation, Metric)],
        ) -> Result<Measurements, RunError> {
            use crate::wrappers::fixed_matrix::{
                unsupported_shape, with_fixed_matrix, FixedMatrixVisitor,
            };
            use crate::wrappers::$module::sketchlib as w;
            use crate::wrappers::{DefaultXxHasher, FastPathHasher, MatrixStorage};

            let p: $params = req
                .params
                .parse()
                .map_err(|e: aqpbm_core::DataGenError| RunError::Sketch(e.to_string()))?;
            // Every shape in the table ingests i64, so the data is materialised
            // and the exact answer drawn before the shape is resolved.
            let (dataset, items) = peel::<i64>(data)?;

            struct V<'a> {
                req: &'a Requirement,
                want: &'a [(Operation, Metric)],
                dataset: InputDataSetDescription,
                items: Rc<Vec<i64>>,
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
                    let questions = questions(FrequencyGT, &self.items);
                    scored(
                        self.req,
                        self.want,
                        sketch(self.req, w::$build::<M>, w::$memory::<M>),
                        per_item(w::$insert::<M>),
                        self.items,
                        Some(w::$merge::<M>),
                        None,
                        w::$query::<M>,
                        questions,
                    )
                    .map(|bodies| Measurements {
                        dataset: self.dataset,
                        bodies,
                    })
                }
            }

            with_fixed_matrix(
                p.rows,
                p.cols,
                V {
                    req,
                    want,
                    dataset,
                    items,
                },
            )
            .unwrap_or_else(|| Err(RunError::Sketch(unsupported_shape($algo, p.rows, p.cols))))
        }
    };
}

fixed_matrix_row!(
    fixed_matrix_cms,
    crate::params::CmsParams,
    "cms-fastpath-fixedmatrix",
    cms,
    build_cms_lib_fixedmatrix,
    memory_cms_lib_fixedmatrix,
    insert_cms_lib_fixedmatrix,
    query_cms_lib_fixedmatrix,
    merge_cms_lib_fixedmatrix
);

fixed_matrix_row!(
    fixed_matrix_cs,
    crate::params::CountSketchParams,
    "countsketch-fastpath-fixedmatrix",
    cs,
    build_cs_lib_fixedmatrix,
    memory_cs_lib_fixedmatrix,
    insert_cs_lib_fixedmatrix,
    query_cs_lib_fixedmatrix,
    merge_cs_lib_fixedmatrix
);

#[cfg(test)]
mod tests {
    use super::*;

    /// Every registered pair can actually be run. `registry::check` and the
    /// match in [`measurements`] live in two tables that have to agree — a pair
    /// that passes the check and finds no code reads as a broken tool.
    #[test]
    fn every_registry_entry_has_an_arm() {
        // A parameterless request over an unreadable file reaches the arm and
        // stops at the first thing it needs, which is enough to prove an arm
        // exists: what is being checked is that the pair is not `None`, and
        // `Some(Err(_))` answers that as well as `Some(Ok(_))` does.
        let req = |e: &crate::registry::SketchId| Requirement {
            algorithm: e.algorithm.to_string(),
            impl_name: e.impl_name.to_string(),
            params: ParamSet::empty(e.algorithm),
            width: Dtype::I64,
            workers: 1,
            merge_shards: 2,
            comparator: None,
        };
        let data = || InputDataSetData::File {
            path: "unreadable on purpose: only the dispatch is under test".to_string(),
        };
        let missing: Vec<String> = crate::registry::REGISTRY
            .iter()
            .filter(|e| measurements(&req(e), data(), &[]).is_none())
            .map(|e| format!("{}/{}", e.algorithm, e.impl_name))
            .collect();
        assert!(
            missing.is_empty(),
            "registered but unrunnable: {}",
            missing.join(", ")
        );
    }
}
