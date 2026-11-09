use super::*;
use crate::wrappers::univmon::{oxide as uo, sketchlib as ul};
use crate::wrappers::{univmon_q as uq, univmon_two_tier as utt};

pub(crate) fn row_univmon_cardinality_lib(
    req: &Requirement,
    value_column: &ValueColumn,
    table: GeneratedTable,
    want: &[(Operation, Metric)],
) -> Result<Measurements, RunError> {
    macro_rules! at {
        ($k:ty) => {
            keyed_row::<$k, _>(
                req,
                value_column,
                table,
                want,
                KeyedCardinalityGT::<$k>::over_columns(KEYED_KEY_COLUMN, value_column.index),
                ul::insert_univmon_lib::<$k>,
                ul::insert_step_univmon_lib::<$k>,
                ul::query_univmon_lib_cardinality::<$k>,
                Some((
                    ul::merge_univmon_lib::<$k>,
                    ul::merge_step_univmon_lib::<$k>,
                    ul::merge_query_univmon_lib_cardinality::<$k>,
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

pub(crate) fn row_univmon_l1_norm_lib(
    req: &Requirement,
    value_column: &ValueColumn,
    table: GeneratedTable,
    want: &[(Operation, Metric)],
) -> Result<Measurements, RunError> {
    macro_rules! at {
        ($k:ty) => {
            keyed_row::<$k, _>(
                req,
                value_column,
                table,
                want,
                KeyedL1NormGT::<$k>::over_columns(KEYED_KEY_COLUMN, value_column.index),
                ul::insert_univmon_lib::<$k>,
                ul::insert_step_univmon_lib::<$k>,
                ul::query_univmon_lib_l1_norm::<$k>,
                Some((
                    ul::merge_univmon_lib::<$k>,
                    ul::merge_step_univmon_lib::<$k>,
                    ul::merge_query_univmon_lib_l1_norm::<$k>,
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

pub(crate) fn row_univmon_l1_norm_oxide(
    req: &Requirement,
    value_column: &ValueColumn,
    table: GeneratedTable,
    want: &[(Operation, Metric)],
) -> Result<Measurements, RunError> {
    macro_rules! at {
        ($k:ty) => {
            keyed_row::<$k, _>(
                req,
                value_column,
                table,
                want,
                KeyedL1NormGT::<$k>::over_columns(KEYED_KEY_COLUMN, value_column.index),
                uo::insert_univmon_oxide::<$k>,
                uo::insert_step_univmon_oxide::<$k>,
                uo::query_univmon_oxide_l1_norm::<$k>,
                Some((
                    uo::merge_univmon_oxide::<$k>,
                    uo::merge_step_univmon_oxide::<$k>,
                    uo::merge_query_univmon_oxide_l1_norm::<$k>,
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

pub(crate) fn row_univmon_l2_norm_lib(
    req: &Requirement,
    value_column: &ValueColumn,
    table: GeneratedTable,
    want: &[(Operation, Metric)],
) -> Result<Measurements, RunError> {
    macro_rules! at {
        ($k:ty) => {
            keyed_row::<$k, _>(
                req,
                value_column,
                table,
                want,
                KeyedL2NormGT::<$k>::over_columns(KEYED_KEY_COLUMN, value_column.index),
                ul::insert_univmon_lib::<$k>,
                ul::insert_step_univmon_lib::<$k>,
                ul::query_univmon_lib_l2_norm::<$k>,
                Some((
                    ul::merge_univmon_lib::<$k>,
                    ul::merge_step_univmon_lib::<$k>,
                    ul::merge_query_univmon_lib_l2_norm::<$k>,
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

pub(crate) fn row_univmon_l2_norm_oxide(
    req: &Requirement,
    value_column: &ValueColumn,
    table: GeneratedTable,
    want: &[(Operation, Metric)],
) -> Result<Measurements, RunError> {
    macro_rules! at {
        ($k:ty) => {
            keyed_row::<$k, _>(
                req,
                value_column,
                table,
                want,
                KeyedL2NormGT::<$k>::over_columns(KEYED_KEY_COLUMN, value_column.index),
                uo::insert_univmon_oxide::<$k>,
                uo::insert_step_univmon_oxide::<$k>,
                uo::query_univmon_oxide_l2_norm::<$k>,
                Some((
                    uo::merge_univmon_oxide::<$k>,
                    uo::merge_step_univmon_oxide::<$k>,
                    uo::merge_query_univmon_oxide_l2_norm::<$k>,
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

pub(crate) fn row_univmon_entropy_lib(
    req: &Requirement,
    value_column: &ValueColumn,
    table: GeneratedTable,
    want: &[(Operation, Metric)],
) -> Result<Measurements, RunError> {
    macro_rules! at {
        ($k:ty) => {
            keyed_row::<$k, _>(
                req,
                value_column,
                table,
                want,
                KeyedEntropyGT::<$k>::over_columns(KEYED_KEY_COLUMN, value_column.index),
                ul::insert_univmon_lib::<$k>,
                ul::insert_step_univmon_lib::<$k>,
                ul::query_univmon_lib_entropy::<$k>,
                Some((
                    ul::merge_univmon_lib::<$k>,
                    ul::merge_step_univmon_lib::<$k>,
                    ul::merge_query_univmon_lib_entropy::<$k>,
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

pub(crate) fn row_univmon_entropy_oxide(
    req: &Requirement,
    value_column: &ValueColumn,
    table: GeneratedTable,
    want: &[(Operation, Metric)],
) -> Result<Measurements, RunError> {
    macro_rules! at {
        ($k:ty) => {
            keyed_row::<$k, _>(
                req,
                value_column,
                table,
                want,
                KeyedEntropyGT::<$k>::over_columns(KEYED_KEY_COLUMN, value_column.index),
                uo::insert_univmon_oxide::<$k>,
                uo::insert_step_univmon_oxide::<$k>,
                uo::query_univmon_oxide_entropy::<$k>,
                Some((
                    uo::merge_univmon_oxide::<$k>,
                    uo::merge_step_univmon_oxide::<$k>,
                    uo::merge_query_univmon_oxide_entropy::<$k>,
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

fn f64_only(req: &Requirement, got: Dtype) -> RunError {
    RunError::Sketch(format!(
        "{}/{} keeps every value as f64, so a key past 2^53 would come back as another key; it builds at --dtype f64 only; --dtype {} is not it",
        req.variant,
        req.library,
        got.name(),
    ))
}

pub(crate) fn row_univmon_two_tier_cardinality_lib(
    req: &Requirement,
    value_column: &ValueColumn,
    table: GeneratedTable,
    want: &[(Operation, Metric)],
) -> Result<Measurements, RunError> {
    macro_rules! at {
        ($t:ty) => {
            keyed_row::<$t, _>(
                req,
                value_column,
                table,
                want,
                KeyedCardinalityGT::<$t>::over_columns(KEYED_KEY_COLUMN, value_column.index),
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
    value_column: &ValueColumn,
    table: GeneratedTable,
    want: &[(Operation, Metric)],
) -> Result<Measurements, RunError> {
    macro_rules! at {
        ($t:ty) => {
            keyed_row::<$t, _>(
                req,
                value_column,
                table,
                want,
                KeyedL1NormGT::<$t>::over_columns(KEYED_KEY_COLUMN, value_column.index),
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
    value_column: &ValueColumn,
    table: GeneratedTable,
    want: &[(Operation, Metric)],
) -> Result<Measurements, RunError> {
    macro_rules! at {
        ($t:ty) => {
            keyed_row::<$t, _>(
                req,
                value_column,
                table,
                want,
                KeyedL2NormGT::<$t>::over_columns(KEYED_KEY_COLUMN, value_column.index),
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
    value_column: &ValueColumn,
    table: GeneratedTable,
    want: &[(Operation, Metric)],
) -> Result<Measurements, RunError> {
    macro_rules! at {
        ($t:ty) => {
            keyed_row::<$t, _>(
                req,
                value_column,
                table,
                want,
                KeyedEntropyGT::<$t>::over_columns(KEYED_KEY_COLUMN, value_column.index),
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
    value_column: &ValueColumn,
    table: GeneratedTable,
    want: &[(Operation, Metric)],
) -> Result<Measurements, RunError> {
    macro_rules! at {
        ($t:ty) => {
            quantile_row::<$t>(
                req,
                value_column,
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
    value_column: &ValueColumn,
    table: GeneratedTable,
    want: &[(Operation, Metric)],
) -> Result<Measurements, RunError> {
    macro_rules! at {
        ($t:ty) => {
            cardinality_row::<$t>(
                req,
                value_column,
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
        Dtype::F64 => at!(f64),
        other => Err(f64_only(req, other)),
    }
}

pub(crate) fn row_univmon_q_frequency_lib(
    req: &Requirement,
    value_column: &ValueColumn,
    table: GeneratedTable,
    want: &[(Operation, Metric)],
) -> Result<Measurements, RunError> {
    macro_rules! at {
        ($t:ty) => {
            frequency_row::<$t>(
                req,
                value_column,
                table,
                want,
                uq::insert_univmon_q_lib::<$t>,
                uq::insert_step_univmon_q_lib::<$t>,
                uq::query_univmon_q_lib_frequency,
                Some((
                    uq::merge_univmon_q_lib::<$t>,
                    uq::merge_step_univmon_q_lib::<$t>,
                    uq::merge_query_univmon_q_lib_frequency,
                )),
                None,
            )
        };
    }
    match req.width {
        Dtype::F64 => at!(f64),
        other => Err(f64_only(req, other)),
    }
}

pub(crate) fn row_univmon_q_topk_lib(
    req: &Requirement,
    value_column: &ValueColumn,
    table: GeneratedTable,
    want: &[(Operation, Metric)],
) -> Result<Measurements, RunError> {
    macro_rules! at {
        ($t:ty) => {
            topk_row::<$t>(
                req,
                value_column,
                table,
                want,
                uq::answered_k_univmon_q_lib(&req.params).map_err(cannot_build)?,
                uq::insert_univmon_q_lib::<$t>,
                uq::insert_step_univmon_q_lib::<$t>,
                uq::query_univmon_q_lib_topk,
                Some((
                    uq::merge_univmon_q_lib::<$t>,
                    uq::merge_step_univmon_q_lib::<$t>,
                    uq::merge_query_univmon_q_lib_topk,
                )),
            )
        };
    }
    match req.width {
        Dtype::F64 => at!(f64),
        other => Err(f64_only(req, other)),
    }
}
