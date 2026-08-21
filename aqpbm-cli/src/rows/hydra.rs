use super::*;
use sketch_bench::wrappers::hydra_cms::{polars as hp, sketchlib as hs};
use sketch_bench::wrappers::hydra_cs::{polars as hcp, sketchlib as hcs};
use sketch_bench::wrappers::hydra_hll::{polars as hhp, sketchlib as hhs};
use sketch_bench::wrappers::hydra_kll::{polars as hkp, sketchlib as hks};

// -------- Hydra (per-subpopulation statistics over labelled records) --------

pub(crate) fn row_hydra_cms_lib(
    req: &Requirement,
    description: &TableDescription,
    table: GeneratedTable,
    want: &[(Operation, Metric)],
) -> Result<Measurements, RunError> {
    macro_rules! at {
        ($t:ty) => {
            subpop_frequency_row::<$t>(
                req,
                description,
                table,
                want,
                hs::insert_hydra_cms::<$t>,
                hs::insert_step_hydra_cms::<$t>,
                hs::query_hydra_cms::<$t>,
                Some((hs::merge_hydra_cms::<$t>, hs::merge_step_hydra_cms::<$t>)),
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

pub(crate) fn row_hydra_cms_polars(
    req: &Requirement,
    description: &TableDescription,
    table: GeneratedTable,
    want: &[(Operation, Metric)],
) -> Result<Measurements, RunError> {
    macro_rules! at {
        ($t:ty) => {
            subpop_frequency_row::<$t>(
                req,
                description,
                table,
                want,
                hp::insert_polars_subpop_frequency_cms::<$t>,
                hp::insert_step_polars_subpop_frequency_cms::<$t>,
                hp::query_polars_subpop_frequency_cms::<$t>,
                None,
                Some(hp::prepare_polars_subpop_frequency_cms::<$t>),
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

pub(crate) fn row_hydra_cs_lib(
    req: &Requirement,
    description: &TableDescription,
    table: GeneratedTable,
    want: &[(Operation, Metric)],
) -> Result<Measurements, RunError> {
    macro_rules! at {
        ($t:ty) => {
            subpop_frequency_row::<$t>(
                req,
                description,
                table,
                want,
                hcs::insert_hydra_cs::<$t>,
                hcs::insert_step_hydra_cs::<$t>,
                hcs::query_hydra_cs::<$t>,
                Some((hcs::merge_hydra_cs::<$t>, hcs::merge_step_hydra_cs::<$t>)),
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

pub(crate) fn row_hydra_cs_polars(
    req: &Requirement,
    description: &TableDescription,
    table: GeneratedTable,
    want: &[(Operation, Metric)],
) -> Result<Measurements, RunError> {
    macro_rules! at {
        ($t:ty) => {
            subpop_frequency_row::<$t>(
                req,
                description,
                table,
                want,
                hcp::insert_polars_subpop_frequency_cs::<$t>,
                hcp::insert_step_polars_subpop_frequency_cs::<$t>,
                hcp::query_polars_subpop_frequency_cs::<$t>,
                None,
                Some(hcp::prepare_polars_subpop_frequency_cs::<$t>),
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

pub(crate) fn row_hydra_hll_lib(
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
                hhs::insert_hydra_hll::<$t>,
                hhs::insert_step_hydra_hll::<$t>,
                hhs::query_hydra_hll::<$t>,
                Some((hhs::merge_hydra_hll::<$t>, hhs::merge_step_hydra_hll::<$t>)),
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

pub(crate) fn row_hydra_hll_polars(
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
                hhp::insert_polars_subpop_cardinality::<$t>,
                hhp::insert_step_polars_subpop_cardinality::<$t>,
                hhp::query_polars_subpop_cardinality::<$t>,
                None,
                Some(hhp::prepare_polars_subpop_cardinality::<$t>),
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

pub(crate) fn row_hydra_kll_lib(
    req: &Requirement,
    description: &TableDescription,
    table: GeneratedTable,
    want: &[(Operation, Metric)],
) -> Result<Measurements, RunError> {
    macro_rules! at {
        ($t:ty) => {
            subpop_quantile_row::<$t>(
                req,
                description,
                table,
                want,
                hks::insert_hydra_kll::<$t>,
                hks::insert_step_hydra_kll::<$t>,
                hks::query_hydra_kll::<$t>,
                Some((hks::merge_hydra_kll::<$t>, hks::merge_step_hydra_kll::<$t>)),
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

pub(crate) fn row_hydra_kll_polars(
    req: &Requirement,
    description: &TableDescription,
    table: GeneratedTable,
    want: &[(Operation, Metric)],
) -> Result<Measurements, RunError> {
    macro_rules! at {
        ($t:ty) => {
            subpop_quantile_row::<$t>(
                req,
                description,
                table,
                want,
                hkp::insert_polars_subpop_quantile::<$t>,
                hkp::insert_step_polars_subpop_quantile::<$t>,
                hkp::query_polars_subpop_quantile::<$t>,
                None,
                Some(hkp::prepare_polars_subpop_quantile::<$t>),
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
