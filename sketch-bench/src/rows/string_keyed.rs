use super::*;
use crate::wrappers::{coco as co, elastic as el};

fn string_only(req: &Requirement, got: Dtype) -> RunError {
    RunError::Sketch(format!(
        "{}/{} keys by String in the library's own API, so it builds at --dtype string only; --dtype {} is not it",
        req.variant,
        req.library,
        got.name(),
    ))
}

pub(crate) fn row_elastic_lib(
    req: &Requirement,
    columns: &ColumnRoles,
    table: GeneratedTable,
    want: &[(Operation, Metric)],
) -> Result<Measurements, RunError> {
    macro_rules! at {
        ($t:ty) => {
            frequency_row(
                req,
                columns,
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
    columns: &ColumnRoles,
    table: GeneratedTable,
    want: &[(Operation, Metric)],
) -> Result<Measurements, RunError> {
    macro_rules! at {
        ($t:ty) => {
            topk_row(
                req,
                columns,
                table,
                want,
                el::answered_k_elastic_lib(&req.params).map_err(cannot_build)?,
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
    columns: &ColumnRoles,
    table: GeneratedTable,
    want: &[(Operation, Metric)],
) -> Result<Measurements, RunError> {
    macro_rules! at {
        ($t:ty) => {
            frequency_row(
                req,
                columns,
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
