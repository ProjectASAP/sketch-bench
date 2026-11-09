use super::*;
use crate::wrappers::{
    count_min_space_saving as cmss, count_sketch_space_saving as csss, space_saving as ss,
};

pub(crate) fn row_space_saving_lib(
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
                ss::insert_space_saving_lib::<$t>,
                ss::insert_step_space_saving_lib::<$t>,
                ss::query_space_saving_lib_estimate::<$t>,
                Some((
                    ss::merge_space_saving_lib::<$t>,
                    ss::merge_step_space_saving_lib::<$t>,
                    ss::merge_query_space_saving_lib_estimate::<$t>,
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

pub(crate) fn row_space_saving_topk_lib(
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
                ss::answered_k_space_saving_lib(&req.params).map_err(cannot_build)?,
                ss::insert_space_saving_lib::<$t>,
                ss::insert_step_space_saving_lib::<$t>,
                ss::query_space_saving_lib_topk::<$t>,
                Some((
                    ss::merge_space_saving_lib::<$t>,
                    ss::merge_step_space_saving_lib::<$t>,
                    ss::merge_query_space_saving_lib_topk::<$t>,
                )),
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

pub(crate) fn row_count_min_space_saving_lib(
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
                cmss::insert_count_min_space_saving_lib::<$t>,
                cmss::insert_step_count_min_space_saving_lib::<$t>,
                cmss::query_count_min_space_saving_lib_estimate::<$t>,
                Some((
                    cmss::merge_count_min_space_saving_lib::<$t>,
                    cmss::merge_step_count_min_space_saving_lib::<$t>,
                    cmss::merge_query_count_min_space_saving_lib_estimate::<$t>,
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

pub(crate) fn row_count_min_space_saving_topk_lib(
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
                cmss::answered_k_count_min_space_saving_lib(&req.params).map_err(cannot_build)?,
                cmss::insert_count_min_space_saving_lib::<$t>,
                cmss::insert_step_count_min_space_saving_lib::<$t>,
                cmss::query_count_min_space_saving_lib_topk::<$t>,
                Some((
                    cmss::merge_count_min_space_saving_lib::<$t>,
                    cmss::merge_step_count_min_space_saving_lib::<$t>,
                    cmss::merge_query_count_min_space_saving_lib_topk::<$t>,
                )),
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

pub(crate) fn row_count_sketch_space_saving_lib(
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
                csss::insert_count_sketch_space_saving_lib::<$t>,
                csss::insert_step_count_sketch_space_saving_lib::<$t>,
                csss::query_count_sketch_space_saving_lib_estimate::<$t>,
                Some((
                    csss::merge_count_sketch_space_saving_lib::<$t>,
                    csss::merge_step_count_sketch_space_saving_lib::<$t>,
                    csss::merge_query_count_sketch_space_saving_lib_estimate::<$t>,
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

pub(crate) fn row_count_sketch_space_saving_topk_lib(
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
                csss::answered_k_count_sketch_space_saving_lib(&req.params)
                    .map_err(cannot_build)?,
                csss::insert_count_sketch_space_saving_lib::<$t>,
                csss::insert_step_count_sketch_space_saving_lib::<$t>,
                csss::query_count_sketch_space_saving_lib_topk::<$t>,
                Some((
                    csss::merge_count_sketch_space_saving_lib::<$t>,
                    csss::merge_step_count_sketch_space_saving_lib::<$t>,
                    csss::merge_query_count_sketch_space_saving_lib_topk::<$t>,
                )),
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
