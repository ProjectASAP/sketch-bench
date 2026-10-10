use super::*;
use sketch_bench::wrappers::hydra_univmon::{polars as hup, sketchlib as hus};

pub(crate) fn row_hydra_univmon_cardinality_lib(
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
                hus::insert_hydra_univmon::<$t>,
                hus::insert_step_hydra_univmon::<$t>,
                hus::query_hydra_univmon_cardinality::<$t>,
                Some((
                    hus::merge_hydra_univmon::<$t>,
                    hus::merge_step_hydra_univmon::<$t>,
                    hus::merge_query_hydra_univmon_cardinality::<$t>,
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

pub(crate) fn row_hydra_univmon_cardinality_polars(
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
                hup::insert_polars_subpop_cardinality_univmon::<$t>,
                hup::insert_step_polars_subpop_cardinality_univmon::<$t>,
                hup::query_polars_subpop_cardinality_univmon::<$t>,
                None,
                Some(hup::prepare_polars_subpop_cardinality_univmon::<$t>),
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

pub(crate) fn row_hydra_univmon_l1_norm_lib(
    req: &Requirement,
    description: &TableDescription,
    table: GeneratedTable,
    want: &[(Operation, Metric)],
) -> Result<Measurements, RunError> {
    macro_rules! at {
        ($t:ty) => {
            subpop_vector_row::<$t, _>(
                req,
                description,
                table,
                want,
                SubpopL1NormGT {
                    group_columns: group_columns(req, description)?,
                    value_column: value_column(description),
                },
                hus::insert_hydra_univmon::<$t>,
                hus::insert_step_hydra_univmon::<$t>,
                hus::query_hydra_univmon_l1_norm::<$t>,
                Some((
                    hus::merge_hydra_univmon::<$t>,
                    hus::merge_step_hydra_univmon::<$t>,
                    hus::merge_query_hydra_univmon_l1_norm::<$t>,
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

pub(crate) fn row_hydra_univmon_l1_norm_polars(
    req: &Requirement,
    description: &TableDescription,
    table: GeneratedTable,
    want: &[(Operation, Metric)],
) -> Result<Measurements, RunError> {
    macro_rules! at {
        ($t:ty) => {
            subpop_vector_row::<$t, _>(
                req,
                description,
                table,
                want,
                SubpopL1NormGT {
                    group_columns: group_columns(req, description)?,
                    value_column: value_column(description),
                },
                hup::insert_polars_subpop_l1_norm::<$t>,
                hup::insert_step_polars_subpop_l1_norm::<$t>,
                hup::query_polars_subpop_l1_norm::<$t>,
                None,
                Some(hup::prepare_polars_subpop_l1_norm::<$t>),
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

pub(crate) fn row_hydra_univmon_l2_norm_lib(
    req: &Requirement,
    description: &TableDescription,
    table: GeneratedTable,
    want: &[(Operation, Metric)],
) -> Result<Measurements, RunError> {
    macro_rules! at {
        ($t:ty) => {
            subpop_vector_row::<$t, _>(
                req,
                description,
                table,
                want,
                SubpopL2NormGT {
                    group_columns: group_columns(req, description)?,
                    value_column: value_column(description),
                },
                hus::insert_hydra_univmon::<$t>,
                hus::insert_step_hydra_univmon::<$t>,
                hus::query_hydra_univmon_l2_norm::<$t>,
                Some((
                    hus::merge_hydra_univmon::<$t>,
                    hus::merge_step_hydra_univmon::<$t>,
                    hus::merge_query_hydra_univmon_l2_norm::<$t>,
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

pub(crate) fn row_hydra_univmon_l2_norm_polars(
    req: &Requirement,
    description: &TableDescription,
    table: GeneratedTable,
    want: &[(Operation, Metric)],
) -> Result<Measurements, RunError> {
    macro_rules! at {
        ($t:ty) => {
            subpop_vector_row::<$t, _>(
                req,
                description,
                table,
                want,
                SubpopL2NormGT {
                    group_columns: group_columns(req, description)?,
                    value_column: value_column(description),
                },
                hup::insert_polars_subpop_l2_norm::<$t>,
                hup::insert_step_polars_subpop_l2_norm::<$t>,
                hup::query_polars_subpop_l2_norm::<$t>,
                None,
                Some(hup::prepare_polars_subpop_l2_norm::<$t>),
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

pub(crate) fn row_hydra_univmon_entropy_lib(
    req: &Requirement,
    description: &TableDescription,
    table: GeneratedTable,
    want: &[(Operation, Metric)],
) -> Result<Measurements, RunError> {
    macro_rules! at {
        ($t:ty) => {
            subpop_vector_row::<$t, _>(
                req,
                description,
                table,
                want,
                SubpopEntropyGT {
                    group_columns: group_columns(req, description)?,
                    value_column: value_column(description),
                },
                hus::insert_hydra_univmon::<$t>,
                hus::insert_step_hydra_univmon::<$t>,
                hus::query_hydra_univmon_entropy::<$t>,
                Some((
                    hus::merge_hydra_univmon::<$t>,
                    hus::merge_step_hydra_univmon::<$t>,
                    hus::merge_query_hydra_univmon_entropy::<$t>,
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

pub(crate) fn row_hydra_univmon_entropy_polars(
    req: &Requirement,
    description: &TableDescription,
    table: GeneratedTable,
    want: &[(Operation, Metric)],
) -> Result<Measurements, RunError> {
    macro_rules! at {
        ($t:ty) => {
            subpop_vector_row::<$t, _>(
                req,
                description,
                table,
                want,
                SubpopEntropyGT {
                    group_columns: group_columns(req, description)?,
                    value_column: value_column(description),
                },
                hup::insert_polars_subpop_entropy::<$t>,
                hup::insert_step_polars_subpop_entropy::<$t>,
                hup::query_polars_subpop_entropy::<$t>,
                None,
                Some(hup::prepare_polars_subpop_entropy::<$t>),
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

/// The sum rows insert each value as its own integer count, so they build at
/// the integer widths only.
fn no_sum_at(req: &Requirement, got: Dtype) -> RunError {
    RunError::Sketch(format!(
        "{}/{} counts each value by itself, an integer weight, so builds at i64 or u64; \
         --dtype {} is not an integer",
        req.variant,
        req.library,
        got.name(),
    ))
}

pub(crate) fn row_hydra_univmon_sum_lib(
    req: &Requirement,
    description: &TableDescription,
    table: GeneratedTable,
    want: &[(Operation, Metric)],
) -> Result<Measurements, RunError> {
    macro_rules! at {
        ($t:ty) => {
            subpop_vector_row::<$t, _>(
                req,
                description,
                table,
                want,
                SubpopSumGT {
                    group_columns: group_columns(req, description)?,
                    value_column: value_column(description),
                },
                hus::insert_hydra_univmon_sum::<$t>,
                hus::insert_step_hydra_univmon_sum::<$t>,
                hus::query_hydra_univmon_sum::<$t>,
                Some((
                    hus::merge_hydra_univmon_sum::<$t>,
                    hus::merge_step_hydra_univmon_sum::<$t>,
                    hus::merge_query_hydra_univmon_sum::<$t>,
                )),
                None,
            )
        };
    }
    match req.width {
        Dtype::I64 => at!(i64),
        Dtype::U64 => at!(u64),
        other => Err(no_sum_at(req, other)),
    }
}

pub(crate) fn row_hydra_univmon_sum_polars(
    req: &Requirement,
    description: &TableDescription,
    table: GeneratedTable,
    want: &[(Operation, Metric)],
) -> Result<Measurements, RunError> {
    macro_rules! at {
        ($t:ty) => {
            subpop_vector_row::<$t, _>(
                req,
                description,
                table,
                want,
                SubpopSumGT {
                    group_columns: group_columns(req, description)?,
                    value_column: value_column(description),
                },
                hup::insert_polars_subpop_sum::<$t>,
                hup::insert_step_polars_subpop_sum::<$t>,
                hup::query_polars_subpop_sum::<$t>,
                None,
                Some(hup::prepare_polars_subpop_sum::<$t>),
            )
        };
    }
    match req.width {
        Dtype::I64 => at!(i64),
        Dtype::U64 => at!(u64),
        other => Err(no_sum_at(req, other)),
    }
}
