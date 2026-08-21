use super::*;
use sketch_bench::wrappers::cms::{datasketches as cd, oxide as co, polars as cp, sketchlib as cl};

// -------- CMS (frequency) --------

pub(crate) fn row_cms_oxide(
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
                co::insert_cms_oxide::<$t>,
                co::insert_step_cms_oxide::<$t>,
                co::query_cms_oxide::<$t>,
                Some((co::merge_cms_oxide::<$t>, co::merge_step_cms_oxide)),
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

pub(crate) fn row_cms_datasketches(
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
                cd::insert_cms_datasketches::<$t>,
                cd::insert_step_cms_datasketches::<$t>,
                cd::query_cms_datasketches::<$t>,
                Some((
                    cd::merge_cms_datasketches::<$t>,
                    cd::merge_step_cms_datasketches,
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

pub(crate) fn row_cms_polars(
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
                cp::insert_polars_frequency_cms::<$t>,
                cp::insert_step_polars_frequency_cms::<$t>,
                cp::query_polars_frequency_cms::<$t>,
                None,
                Some(cp::prepare_polars_frequency_cms),
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

pub(crate) fn row_cms_fastpath_vector2d_lib(
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
                cl::insert_cms_lib_vector2d_fast::<$t>,
                cl::insert_step_cms_lib_vector2d_fast::<$t>,
                cl::query_cms_lib_vector2d_fast::<$t>,
                Some((
                    cl::merge_cms_lib_vector2d_fast::<$t>,
                    cl::merge_step_cms_lib_vector2d_fast::<$t>,
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

pub(crate) fn row_cms_regularpath_vector2d_lib(
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
                cl::insert_cms_lib_vector2d_regular::<$t>,
                cl::insert_step_cms_lib_vector2d_regular::<$t>,
                cl::query_cms_lib_vector2d_regular::<$t>,
                Some((
                    cl::merge_cms_lib_vector2d_regular::<$t>,
                    cl::merge_step_cms_lib_vector2d_regular::<$t>,
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
