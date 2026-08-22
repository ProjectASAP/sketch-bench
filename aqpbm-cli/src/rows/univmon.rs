use super::*;
use sketch_bench::wrappers::univmon::{oxide as uo, sketchlib as ul};

pub(crate) fn row_univmon_cardinality_lib(
    req: &Requirement,
    description: &TableDescription,
    table: GeneratedTable,
    want: &[(Operation, Metric)],
) -> Result<Measurements, RunError> {
    macro_rules! at {
        ($k:ty) => {
            keyed_row::<$k, _>(
                req,
                description,
                table,
                want,
                KeyedCardinalityGT::<$k>::over_columns(KEYED_KEY_COLUMN, value_column(description)),
                ul::insert_univmon_lib::<$k>,
                ul::insert_step_univmon_lib::<$k>,
                ul::query_univmon_lib_cardinality::<$k>,
                Some((
                    ul::merge_univmon_lib::<$k>,
                    ul::merge_step_univmon_lib::<$k>,
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
    description: &TableDescription,
    table: GeneratedTable,
    want: &[(Operation, Metric)],
) -> Result<Measurements, RunError> {
    macro_rules! at {
        ($k:ty) => {
            keyed_row::<$k, _>(
                req,
                description,
                table,
                want,
                KeyedL1NormGT::<$k>::over_columns(KEYED_KEY_COLUMN, value_column(description)),
                ul::insert_univmon_lib::<$k>,
                ul::insert_step_univmon_lib::<$k>,
                ul::query_univmon_lib_l1_norm::<$k>,
                Some((
                    ul::merge_univmon_lib::<$k>,
                    ul::merge_step_univmon_lib::<$k>,
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
    description: &TableDescription,
    table: GeneratedTable,
    want: &[(Operation, Metric)],
) -> Result<Measurements, RunError> {
    macro_rules! at {
        ($k:ty) => {
            keyed_row::<$k, _>(
                req,
                description,
                table,
                want,
                KeyedL1NormGT::<$k>::over_columns(KEYED_KEY_COLUMN, value_column(description)),
                uo::insert_univmon_oxide::<$k>,
                uo::insert_step_univmon_oxide::<$k>,
                uo::query_univmon_oxide_l1_norm::<$k>,
                Some((
                    uo::merge_univmon_oxide::<$k>,
                    uo::merge_step_univmon_oxide::<$k>,
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
    description: &TableDescription,
    table: GeneratedTable,
    want: &[(Operation, Metric)],
) -> Result<Measurements, RunError> {
    macro_rules! at {
        ($k:ty) => {
            keyed_row::<$k, _>(
                req,
                description,
                table,
                want,
                KeyedL2NormGT::<$k>::over_columns(KEYED_KEY_COLUMN, value_column(description)),
                ul::insert_univmon_lib::<$k>,
                ul::insert_step_univmon_lib::<$k>,
                ul::query_univmon_lib_l2_norm::<$k>,
                Some((
                    ul::merge_univmon_lib::<$k>,
                    ul::merge_step_univmon_lib::<$k>,
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
    description: &TableDescription,
    table: GeneratedTable,
    want: &[(Operation, Metric)],
) -> Result<Measurements, RunError> {
    macro_rules! at {
        ($k:ty) => {
            keyed_row::<$k, _>(
                req,
                description,
                table,
                want,
                KeyedL2NormGT::<$k>::over_columns(KEYED_KEY_COLUMN, value_column(description)),
                uo::insert_univmon_oxide::<$k>,
                uo::insert_step_univmon_oxide::<$k>,
                uo::query_univmon_oxide_l2_norm::<$k>,
                Some((
                    uo::merge_univmon_oxide::<$k>,
                    uo::merge_step_univmon_oxide::<$k>,
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
    description: &TableDescription,
    table: GeneratedTable,
    want: &[(Operation, Metric)],
) -> Result<Measurements, RunError> {
    macro_rules! at {
        ($k:ty) => {
            keyed_row::<$k, _>(
                req,
                description,
                table,
                want,
                KeyedEntropyGT::<$k>::over_columns(KEYED_KEY_COLUMN, value_column(description)),
                ul::insert_univmon_lib::<$k>,
                ul::insert_step_univmon_lib::<$k>,
                ul::query_univmon_lib_entropy::<$k>,
                Some((
                    ul::merge_univmon_lib::<$k>,
                    ul::merge_step_univmon_lib::<$k>,
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
    description: &TableDescription,
    table: GeneratedTable,
    want: &[(Operation, Metric)],
) -> Result<Measurements, RunError> {
    macro_rules! at {
        ($k:ty) => {
            keyed_row::<$k, _>(
                req,
                description,
                table,
                want,
                KeyedEntropyGT::<$k>::over_columns(KEYED_KEY_COLUMN, value_column(description)),
                uo::insert_univmon_oxide::<$k>,
                uo::insert_step_univmon_oxide::<$k>,
                uo::query_univmon_oxide_entropy::<$k>,
                Some((
                    uo::merge_univmon_oxide::<$k>,
                    uo::merge_step_univmon_oxide::<$k>,
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
