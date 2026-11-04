use super::*;
use crate::wrappers::{
    coco as co, count_min_hll as cmh, count_sketch_l2_norm as csl, elastic as el,
    kll_dynamic as kd, nitro_count_min as ncm, nitro_count_sketch as ncs, uniform_sampling as us,
    univmon_q as uq, univmon_two_tier as utt,
};

fn string_only(req: &Requirement, got: Dtype) -> RunError {
    RunError::Sketch(format!(
        "{}/{} keys by String in the library's own API, so it builds at --dtype string only; --dtype {} is not it",
        req.variant,
        req.library,
        got.name(),
    ))
}

fn i64_only(req: &Requirement, got: Dtype) -> RunError {
    RunError::Sketch(format!(
        "{}/{} ingests i64 batches in the library's own API, so it builds at --dtype i64 only; --dtype {} is not it",
        req.variant,
        req.library,
        got.name(),
    ))
}

pub(crate) fn row_count_sketch_l2_norm_lib(
    req: &Requirement,
    description: &TableDescription,
    table: GeneratedTable,
    want: &[(Operation, Metric)],
) -> Result<Measurements, RunError> {
    macro_rules! at {
        ($t:ty) => {
            keyed_row::<$t, _>(
                req,
                description,
                table,
                want,
                KeyedL2NormGT::<$t>::over_columns(KEYED_KEY_COLUMN, value_column(description)),
                csl::insert_count_sketch_l2_norm_lib::<$t>,
                csl::insert_step_count_sketch_l2_norm_lib::<$t>,
                csl::query_count_sketch_l2_norm_lib::<$t>,
                Some((
                    csl::merge_count_sketch_l2_norm_lib::<$t>,
                    csl::merge_step_count_sketch_l2_norm_lib::<$t>,
                    csl::merge_query_count_sketch_l2_norm_lib::<$t>,
                )),
                None,
            )
        };
    }
    match req.width {
        Dtype::I64 => at!(i64),
        Dtype::U64 => at!(u64),
        Dtype::F64 => at!(f64),
        Dtype::Str => at!(String),
    }
}

pub(crate) fn row_count_min_hll_lib(
    req: &Requirement,
    description: &TableDescription,
    table: GeneratedTable,
    want: &[(Operation, Metric)],
) -> Result<Measurements, RunError> {
    macro_rules! at {
        ($t:ty) => {
            subpop_cardinality_row::<$t>(
                req,
                description,
                table,
                want,
                cmh::insert_count_min_hll_lib::<$t>,
                cmh::insert_step_count_min_hll_lib::<$t>,
                cmh::query_count_min_hll_lib::<$t>,
                Some((
                    cmh::merge_count_min_hll_lib::<$t>,
                    cmh::merge_step_count_min_hll_lib::<$t>,
                    cmh::merge_query_count_min_hll_lib::<$t>,
                )),
                None,
            )
        };
    }
    match req.width {
        Dtype::I64 => at!(i64),
        Dtype::U64 => at!(u64),
        Dtype::F64 => at!(f64),
        Dtype::Str => at!(String),
    }
}

pub(crate) fn row_elastic_lib(
    req: &Requirement,
    description: &TableDescription,
    table: GeneratedTable,
    want: &[(Operation, Metric)],
) -> Result<Measurements, RunError> {
    macro_rules! at {
        ($t:ty) => {
            frequency_row(
                req,
                description,
                table,
                want,
                el::insert_elastic_lib,
                el::insert_step_elastic_lib,
                el::query_elastic_lib_estimate,
                Some((
                    el::merge_elastic_lib,
                    el::merge_step_elastic_lib,
                    el::merge_query_elastic_lib_estimate,
                )),
                None,
            )
        };
    }
    match req.width {
        Dtype::Str => at!(String),
        other => Err(string_only(req, other)),
    }
}

pub(crate) fn row_elastic_topk_lib(
    req: &Requirement,
    description: &TableDescription,
    table: GeneratedTable,
    want: &[(Operation, Metric)],
) -> Result<Measurements, RunError> {
    macro_rules! at {
        ($t:ty) => {
            topk_row(
                req,
                description,
                table,
                want,
                el::insert_elastic_lib,
                el::insert_step_elastic_lib,
                el::query_elastic_lib_topk,
                Some((
                    el::merge_elastic_lib,
                    el::merge_step_elastic_lib,
                    el::merge_query_elastic_lib_topk,
                )),
            )
        };
    }
    match req.width {
        Dtype::Str => at!(String),
        other => Err(string_only(req, other)),
    }
}

pub(crate) fn row_coco_lib(
    req: &Requirement,
    description: &TableDescription,
    table: GeneratedTable,
    want: &[(Operation, Metric)],
) -> Result<Measurements, RunError> {
    macro_rules! at {
        ($t:ty) => {
            frequency_row(
                req,
                description,
                table,
                want,
                co::insert_coco_lib,
                co::insert_step_coco_lib,
                co::query_coco_lib,
                Some((
                    co::merge_coco_lib,
                    co::merge_step_coco_lib,
                    co::merge_query_coco_lib,
                )),
                None,
            )
        };
    }
    match req.width {
        Dtype::Str => at!(String),
        other => Err(string_only(req, other)),
    }
}

pub(crate) fn row_kll_dynamic_lib(
    req: &Requirement,
    description: &TableDescription,
    table: GeneratedTable,
    want: &[(Operation, Metric)],
) -> Result<Measurements, RunError> {
    macro_rules! at {
        ($t:ty) => {
            quantile_row::<$t>(
                req,
                description,
                table,
                want,
                kd::insert_kll_dynamic_lib::<$t>,
                kd::insert_step_kll_dynamic_lib::<$t>,
                kd::query_kll_dynamic_lib::<$t>,
                Some((
                    kd::merge_kll_dynamic_lib::<$t>,
                    kd::merge_step_kll_dynamic_lib::<$t>,
                    kd::merge_query_kll_dynamic_lib::<$t>,
                )),
                None,
            )
        };
    }
    match req.width {
        Dtype::I64 => at!(i64),
        Dtype::U64 => at!(u64),
        Dtype::F64 => at!(f64),
        other => Err(no_build_at(req, other)),
    }
}

pub(crate) fn row_uniform_sampling_lib(
    req: &Requirement,
    description: &TableDescription,
    table: GeneratedTable,
    want: &[(Operation, Metric)],
) -> Result<Measurements, RunError> {
    macro_rules! at {
        ($t:ty) => {
            quantile_row::<$t>(
                req,
                description,
                table,
                want,
                us::insert_uniform_sampling_lib::<$t>,
                us::insert_step_uniform_sampling_lib::<$t>,
                us::query_uniform_sampling_lib::<$t>,
                Some((
                    us::merge_uniform_sampling_lib::<$t>,
                    us::merge_step_uniform_sampling_lib::<$t>,
                    us::merge_query_uniform_sampling_lib::<$t>,
                )),
                None,
            )
        };
    }
    match req.width {
        Dtype::I64 => at!(i64),
        Dtype::U64 => at!(u64),
        Dtype::F64 => at!(f64),
        other => Err(no_build_at(req, other)),
    }
}

pub(crate) fn row_univmon_two_tier_cardinality_lib(
    req: &Requirement,
    description: &TableDescription,
    table: GeneratedTable,
    want: &[(Operation, Metric)],
) -> Result<Measurements, RunError> {
    macro_rules! at {
        ($t:ty) => {
            keyed_row::<$t, _>(
                req,
                description,
                table,
                want,
                KeyedCardinalityGT::<$t>::over_columns(KEYED_KEY_COLUMN, value_column(description)),
                utt::insert_univmon_two_tier_lib::<$t>,
                utt::insert_step_univmon_two_tier_lib::<$t>,
                utt::query_univmon_two_tier_lib_cardinality::<$t>,
                Some((
                    utt::merge_univmon_two_tier_lib::<$t>,
                    utt::merge_step_univmon_two_tier_lib::<$t>,
                    utt::merge_query_univmon_two_tier_lib_cardinality::<$t>,
                )),
                None,
            )
        };
    }
    match req.width {
        Dtype::I64 => at!(i64),
        Dtype::U64 => at!(u64),
        Dtype::F64 => at!(f64),
        Dtype::Str => at!(String),
    }
}

pub(crate) fn row_univmon_two_tier_l1_norm_lib(
    req: &Requirement,
    description: &TableDescription,
    table: GeneratedTable,
    want: &[(Operation, Metric)],
) -> Result<Measurements, RunError> {
    macro_rules! at {
        ($t:ty) => {
            keyed_row::<$t, _>(
                req,
                description,
                table,
                want,
                KeyedL1NormGT::<$t>::over_columns(KEYED_KEY_COLUMN, value_column(description)),
                utt::insert_univmon_two_tier_lib::<$t>,
                utt::insert_step_univmon_two_tier_lib::<$t>,
                utt::query_univmon_two_tier_lib_l1_norm::<$t>,
                Some((
                    utt::merge_univmon_two_tier_lib::<$t>,
                    utt::merge_step_univmon_two_tier_lib::<$t>,
                    utt::merge_query_univmon_two_tier_lib_l1_norm::<$t>,
                )),
                None,
            )
        };
    }
    match req.width {
        Dtype::I64 => at!(i64),
        Dtype::U64 => at!(u64),
        Dtype::F64 => at!(f64),
        Dtype::Str => at!(String),
    }
}

pub(crate) fn row_univmon_two_tier_l2_norm_lib(
    req: &Requirement,
    description: &TableDescription,
    table: GeneratedTable,
    want: &[(Operation, Metric)],
) -> Result<Measurements, RunError> {
    macro_rules! at {
        ($t:ty) => {
            keyed_row::<$t, _>(
                req,
                description,
                table,
                want,
                KeyedL2NormGT::<$t>::over_columns(KEYED_KEY_COLUMN, value_column(description)),
                utt::insert_univmon_two_tier_lib::<$t>,
                utt::insert_step_univmon_two_tier_lib::<$t>,
                utt::query_univmon_two_tier_lib_l2_norm::<$t>,
                Some((
                    utt::merge_univmon_two_tier_lib::<$t>,
                    utt::merge_step_univmon_two_tier_lib::<$t>,
                    utt::merge_query_univmon_two_tier_lib_l2_norm::<$t>,
                )),
                None,
            )
        };
    }
    match req.width {
        Dtype::I64 => at!(i64),
        Dtype::U64 => at!(u64),
        Dtype::F64 => at!(f64),
        Dtype::Str => at!(String),
    }
}

pub(crate) fn row_univmon_two_tier_entropy_lib(
    req: &Requirement,
    description: &TableDescription,
    table: GeneratedTable,
    want: &[(Operation, Metric)],
) -> Result<Measurements, RunError> {
    macro_rules! at {
        ($t:ty) => {
            keyed_row::<$t, _>(
                req,
                description,
                table,
                want,
                KeyedEntropyGT::<$t>::over_columns(KEYED_KEY_COLUMN, value_column(description)),
                utt::insert_univmon_two_tier_lib::<$t>,
                utt::insert_step_univmon_two_tier_lib::<$t>,
                utt::query_univmon_two_tier_lib_entropy::<$t>,
                Some((
                    utt::merge_univmon_two_tier_lib::<$t>,
                    utt::merge_step_univmon_two_tier_lib::<$t>,
                    utt::merge_query_univmon_two_tier_lib_entropy::<$t>,
                )),
                None,
            )
        };
    }
    match req.width {
        Dtype::I64 => at!(i64),
        Dtype::U64 => at!(u64),
        Dtype::F64 => at!(f64),
        Dtype::Str => at!(String),
    }
}

pub(crate) fn row_univmon_q_quantile_lib(
    req: &Requirement,
    description: &TableDescription,
    table: GeneratedTable,
    want: &[(Operation, Metric)],
) -> Result<Measurements, RunError> {
    macro_rules! at {
        ($t:ty) => {
            quantile_row::<$t>(
                req,
                description,
                table,
                want,
                uq::insert_univmon_q_lib::<$t>,
                uq::insert_step_univmon_q_lib::<$t>,
                uq::query_univmon_q_lib_quantile::<$t>,
                Some((
                    uq::merge_univmon_q_lib::<$t>,
                    uq::merge_step_univmon_q_lib::<$t>,
                    uq::merge_query_univmon_q_lib_quantile::<$t>,
                )),
                None,
            )
        };
    }
    match req.width {
        Dtype::I64 => at!(i64),
        Dtype::U64 => at!(u64),
        Dtype::F64 => at!(f64),
        other => Err(no_build_at(req, other)),
    }
}

pub(crate) fn row_univmon_q_cardinality_lib(
    req: &Requirement,
    description: &TableDescription,
    table: GeneratedTable,
    want: &[(Operation, Metric)],
) -> Result<Measurements, RunError> {
    macro_rules! at {
        ($t:ty) => {
            cardinality_row::<$t>(
                req,
                description,
                table,
                want,
                uq::insert_univmon_q_lib::<$t>,
                uq::insert_step_univmon_q_lib::<$t>,
                uq::query_univmon_q_lib_cardinality::<$t>,
                Some((
                    uq::merge_univmon_q_lib::<$t>,
                    uq::merge_step_univmon_q_lib::<$t>,
                    uq::merge_query_univmon_q_lib_cardinality::<$t>,
                )),
                None,
            )
        };
    }
    match req.width {
        Dtype::I64 => at!(i64),
        Dtype::U64 => at!(u64),
        Dtype::F64 => at!(f64),
        other => Err(no_build_at(req, other)),
    }
}

pub(crate) fn row_univmon_q_frequency_lib(
    req: &Requirement,
    description: &TableDescription,
    table: GeneratedTable,
    want: &[(Operation, Metric)],
) -> Result<Measurements, RunError> {
    macro_rules! at {
        ($t:ty) => {
            frequency_row::<$t>(
                req,
                description,
                table,
                want,
                uq::insert_univmon_q_lib::<$t>,
                uq::insert_step_univmon_q_lib::<$t>,
                uq::query_univmon_q_lib_frequency::<$t>,
                Some((
                    uq::merge_univmon_q_lib::<$t>,
                    uq::merge_step_univmon_q_lib::<$t>,
                    uq::merge_query_univmon_q_lib_frequency::<$t>,
                )),
                None,
            )
        };
    }
    match req.width {
        Dtype::I64 => at!(i64),
        Dtype::U64 => at!(u64),
        Dtype::F64 => at!(f64),
        other => Err(no_build_at(req, other)),
    }
}

pub(crate) fn row_univmon_q_topk_lib(
    req: &Requirement,
    description: &TableDescription,
    table: GeneratedTable,
    want: &[(Operation, Metric)],
) -> Result<Measurements, RunError> {
    macro_rules! at {
        ($t:ty) => {
            topk_row::<$t>(
                req,
                description,
                table,
                want,
                uq::insert_univmon_q_lib::<$t>,
                uq::insert_step_univmon_q_lib::<$t>,
                uq::query_univmon_q_lib_topk::<$t>,
                Some((
                    uq::merge_univmon_q_lib::<$t>,
                    uq::merge_step_univmon_q_lib::<$t>,
                    uq::merge_query_univmon_q_lib_topk::<$t>,
                )),
            )
        };
    }
    match req.width {
        Dtype::I64 => at!(i64),
        Dtype::U64 => at!(u64),
        Dtype::F64 => at!(f64),
        other => Err(no_build_at(req, other)),
    }
}

pub(crate) fn row_nitro_count_min_lib(
    req: &Requirement,
    description: &TableDescription,
    table: GeneratedTable,
    want: &[(Operation, Metric)],
) -> Result<Measurements, RunError> {
    macro_rules! at {
        ($t:ty) => {
            frequency_row(
                req,
                description,
                table,
                want,
                ncm::insert_nitro_count_min_lib,
                ncm::insert_step_nitro_count_min_lib,
                ncm::query_nitro_count_min_lib,
                Some((
                    ncm::merge_nitro_count_min_lib,
                    ncm::merge_step_nitro_count_min_lib,
                    ncm::merge_query_nitro_count_min_lib,
                )),
                None,
            )
        };
    }
    match req.width {
        Dtype::I64 => at!(i64),
        other => Err(i64_only(req, other)),
    }
}

pub(crate) fn row_nitro_count_sketch_lib(
    req: &Requirement,
    description: &TableDescription,
    table: GeneratedTable,
    want: &[(Operation, Metric)],
) -> Result<Measurements, RunError> {
    macro_rules! at {
        ($t:ty) => {
            frequency_row(
                req,
                description,
                table,
                want,
                ncs::insert_nitro_count_sketch_lib,
                ncs::insert_step_nitro_count_sketch_lib,
                ncs::query_nitro_count_sketch_lib,
                Some((
                    ncs::merge_nitro_count_sketch_lib,
                    ncs::merge_step_nitro_count_sketch_lib,
                    ncs::merge_query_nitro_count_sketch_lib,
                )),
                None,
            )
        };
    }
    match req.width {
        Dtype::I64 => at!(i64),
        other => Err(i64_only(req, other)),
    }
}
