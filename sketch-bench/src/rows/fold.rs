use super::*;
use crate::wrappers::{count_min_fold as cmf, count_sketch_fold as csf};

pub(crate) fn row_count_min_fold_lib(
    req: &Requirement,
    columns: &ColumnRoles,
    table: GeneratedTable,
    want: &[(Operation, Metric)],
) -> Result<Measurements, RunError> {
    macro_rules! at {
        ($t:ty) => {
            frequency_row::<$t>(
                req,
                columns,
                table,
                want,
                cmf::insert_count_min_fold_lib::<$t>,
                cmf::insert_step_count_min_fold_lib::<$t>,
                cmf::query_count_min_fold_lib_estimate::<$t>,
                Some((
                    cmf::merge_count_min_fold_lib::<$t>,
                    cmf::merge_step_count_min_fold_lib::<$t>,
                    cmf::merge_query_count_min_fold_lib_estimate::<$t>,
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

pub(crate) fn row_count_min_fold_topk_lib(
    req: &Requirement,
    columns: &ColumnRoles,
    table: GeneratedTable,
    want: &[(Operation, Metric)],
) -> Result<Measurements, RunError> {
    macro_rules! at {
        ($t:ty) => {
            topk_row::<$t>(
                req,
                columns,
                table,
                want,
                cmf::answered_k_count_min_fold_lib(&req.params).map_err(cannot_build)?,
                cmf::insert_count_min_fold_lib::<$t>,
                cmf::insert_step_count_min_fold_lib::<$t>,
                cmf::query_count_min_fold_lib_topk::<$t>,
                Some((
                    cmf::merge_count_min_fold_lib::<$t>,
                    cmf::merge_step_count_min_fold_lib::<$t>,
                    cmf::merge_query_count_min_fold_lib_topk::<$t>,
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

pub(crate) fn row_count_sketch_fold_lib(
    req: &Requirement,
    columns: &ColumnRoles,
    table: GeneratedTable,
    want: &[(Operation, Metric)],
) -> Result<Measurements, RunError> {
    macro_rules! at {
        ($t:ty) => {
            frequency_row::<$t>(
                req,
                columns,
                table,
                want,
                csf::insert_count_sketch_fold_lib::<$t>,
                csf::insert_step_count_sketch_fold_lib::<$t>,
                csf::query_count_sketch_fold_lib_estimate::<$t>,
                Some((
                    csf::merge_count_sketch_fold_lib::<$t>,
                    csf::merge_step_count_sketch_fold_lib::<$t>,
                    csf::merge_query_count_sketch_fold_lib_estimate::<$t>,
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

pub(crate) fn row_count_sketch_fold_topk_lib(
    req: &Requirement,
    columns: &ColumnRoles,
    table: GeneratedTable,
    want: &[(Operation, Metric)],
) -> Result<Measurements, RunError> {
    macro_rules! at {
        ($t:ty) => {
            topk_row::<$t>(
                req,
                columns,
                table,
                want,
                csf::answered_k_count_sketch_fold_lib(&req.params).map_err(cannot_build)?,
                csf::insert_count_sketch_fold_lib::<$t>,
                csf::insert_step_count_sketch_fold_lib::<$t>,
                csf::query_count_sketch_fold_lib_topk::<$t>,
                Some((
                    csf::merge_count_sketch_fold_lib::<$t>,
                    csf::merge_step_count_sketch_fold_lib::<$t>,
                    csf::merge_query_count_sketch_fold_lib_topk::<$t>,
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
