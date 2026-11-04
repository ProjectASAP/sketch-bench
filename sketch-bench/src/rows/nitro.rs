use super::*;
use crate::wrappers::{nitro_count_min as ncm, nitro_count_sketch as ncs};

fn i64_only(req: &Requirement, got: Dtype) -> RunError {
    RunError::Sketch(format!(
        "{}/{} ingests i64 batches in the library's own API, so it builds at --dtype i64 only; --dtype {} is not it",
        req.variant,
        req.library,
        got.name(),
    ))
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
