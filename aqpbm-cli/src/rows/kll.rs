use super::*;
use sketch_bench::wrappers::kll::{oxide as ko, polars as kp, sketchlib as kl};

// -------- KLL (quantile) --------
//
// Rows `--dtype` selects a type for: their library is generic over the value
// type, so a width picks a monomorphisation rather than being refused. Each
// arm is its own instantiation, and the one width these rows cannot order —
// `string` — is refused by name.

pub(crate) fn row_kll_percall_oxide(
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
                ko::insert_kll_oxide_per_call::<$t>,
                ko::insert_step_kll_oxide_per_call::<$t>,
                ko::query_kll_oxide_per_call::<$t>,
                Some((
                    ko::merge_kll_oxide_per_call::<$t>,
                    ko::merge_step_kll_oxide_per_call::<$t>,
                    ko::merge_query_kll_oxide_per_call::<$t>,
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

pub(crate) fn row_kll_percall_lib(
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
                kl::insert_kll_lib_per_call::<$t>,
                kl::insert_step_kll_lib_per_call::<$t>,
                kl::query_kll_lib_per_call::<$t>,
                Some((
                    kl::merge_kll_lib_per_call::<$t>,
                    kl::merge_step_kll_lib_per_call::<$t>,
                    kl::merge_query_kll_lib_per_call::<$t>,
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

pub(crate) fn row_kll_cdf_oxide(
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
                ko::insert_kll_oxide_cdf::<$t>,
                ko::insert_step_kll_oxide_cdf::<$t>,
                ko::query_kll_oxide_cdf::<$t>,
                Some((
                    ko::merge_kll_oxide_cdf::<$t>,
                    ko::merge_step_kll_oxide_cdf::<$t>,
                    ko::merge_query_kll_oxide_cdf::<$t>,
                )),
                Some(ko::prepare_kll_oxide_cdf::<$t>),
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

pub(crate) fn row_kll_cdf_lib(
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
                kl::insert_kll_lib_cdf::<$t>,
                kl::insert_step_kll_lib_cdf::<$t>,
                kl::query_kll_lib_cdf::<$t>,
                Some((
                    kl::merge_kll_lib_cdf::<$t>,
                    kl::merge_step_kll_lib_cdf::<$t>,
                    kl::merge_query_kll_lib_cdf::<$t>,
                )),
                Some(kl::prepare_kll_lib_cdf::<$t>),
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

// The exact baseline reads every width the sketch rows above it build at. It
// has to: it is what their reported error is measured against, so a width they
// run at and this row refused would leave that error with nothing to subtract.
pub(crate) fn row_kll_cdf_polars(
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
                kp::insert_polars_quantile_kll::<$t>,
                kp::insert_step_polars_quantile_kll::<$t>,
                kp::query_polars_quantile_kll::<$t>,
                None,
                Some(kp::prepare_polars_quantile_kll::<$t>),
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
