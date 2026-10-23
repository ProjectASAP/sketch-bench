use super::*;
use sketch_bench::wrappers::cs::{oxide as so, polars as sp, sketchlib as sl};

// -------- CountSketch (frequency) --------

pub(crate) fn row_countsketch_oxide(
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
                so::insert_cs_oxide,
                so::insert_step_cs_oxide,
                so::query_cs_oxide,
                Some((
                    so::merge_cs_oxide,
                    so::merge_step_cs_oxide,
                    so::merge_query_cs_oxide,
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

pub(crate) fn row_countsketch_polars(
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
                sp::insert_polars_frequency_cs,
                sp::insert_step_polars_frequency_cs,
                sp::query_polars_frequency_cs,
                None,
                Some(sp::prepare_polars_frequency_cs),
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

pub(crate) fn row_countsketch_fastpath_vector2d_lib(
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
                sl::insert_cs_lib_vector2d_fast,
                sl::insert_step_cs_lib_vector2d_fast,
                sl::query_cs_lib_vector2d_fast,
                Some((
                    sl::merge_cs_lib_vector2d_fast,
                    sl::merge_step_cs_lib_vector2d_fast,
                    sl::merge_query_cs_lib_vector2d_fast,
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

pub(crate) fn row_countsketch_regularpath_vector2d_lib(
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
                sl::insert_cs_lib_vector2d_regular,
                sl::insert_step_cs_lib_vector2d_regular,
                sl::query_cs_lib_vector2d_regular,
                Some((
                    sl::merge_cs_lib_vector2d_regular,
                    sl::merge_step_cs_lib_vector2d_regular,
                    sl::merge_query_cs_lib_vector2d_regular,
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

pub(crate) fn row_countsketch_fastpath_fixedmatrix_32k_parallel_lib(
    req: &Requirement,
    description: &TableDescription,
    table: GeneratedTable,
    want: &[(Operation, Metric)],
) -> Result<Measurements, RunError> {
    macro_rules! at {
        ($t:ty) => {
            timed_row::<$t>(
                req,
                description,
                table,
                want,
                sl::insert_parallel_cs_fast_path::<$t>,
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
