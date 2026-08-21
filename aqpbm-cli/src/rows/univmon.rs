use super::*;
use sketch_bench::wrappers::univmon::{oxide as uo, sketchlib as ul};

pub(crate) fn row_univmon_cardinality_lib(
    req: &Requirement,
    description: &TableDescription,
    table: GeneratedTable,
    want: &[(Operation, Metric)],
) -> Result<Measurements, RunError> {
    keyed_row(
        req,
        description,
        table,
        want,
        KeyedCardinalityGT {
            key_column: KEYED_KEY_COLUMN,
            value_column: value_column(description),
        },
        ul::insert_univmon_lib,
        ul::insert_step_univmon_lib,
        ul::query_univmon_lib_cardinality,
        Some((ul::merge_univmon_lib, ul::merge_step_univmon_lib)),
        None,
    )
}

pub(crate) fn row_univmon_l1_norm_lib(
    req: &Requirement,
    description: &TableDescription,
    table: GeneratedTable,
    want: &[(Operation, Metric)],
) -> Result<Measurements, RunError> {
    keyed_row(
        req,
        description,
        table,
        want,
        KeyedL1NormGT {
            key_column: KEYED_KEY_COLUMN,
            value_column: value_column(description),
        },
        ul::insert_univmon_lib,
        ul::insert_step_univmon_lib,
        ul::query_univmon_lib_l1_norm,
        Some((ul::merge_univmon_lib, ul::merge_step_univmon_lib)),
        None,
    )
}

pub(crate) fn row_univmon_l1_norm_oxide(
    req: &Requirement,
    description: &TableDescription,
    table: GeneratedTable,
    want: &[(Operation, Metric)],
) -> Result<Measurements, RunError> {
    keyed_row(
        req,
        description,
        table,
        want,
        KeyedL1NormGT {
            key_column: KEYED_KEY_COLUMN,
            value_column: value_column(description),
        },
        uo::insert_univmon_oxide,
        uo::insert_step_univmon_oxide,
        uo::query_univmon_oxide_l1_norm,
        Some((uo::merge_univmon_oxide, uo::merge_step_univmon_oxide)),
        None,
    )
}

pub(crate) fn row_univmon_l2_norm_lib(
    req: &Requirement,
    description: &TableDescription,
    table: GeneratedTable,
    want: &[(Operation, Metric)],
) -> Result<Measurements, RunError> {
    keyed_row(
        req,
        description,
        table,
        want,
        KeyedL2NormGT {
            key_column: KEYED_KEY_COLUMN,
            value_column: value_column(description),
        },
        ul::insert_univmon_lib,
        ul::insert_step_univmon_lib,
        ul::query_univmon_lib_l2_norm,
        Some((ul::merge_univmon_lib, ul::merge_step_univmon_lib)),
        None,
    )
}

pub(crate) fn row_univmon_l2_norm_oxide(
    req: &Requirement,
    description: &TableDescription,
    table: GeneratedTable,
    want: &[(Operation, Metric)],
) -> Result<Measurements, RunError> {
    keyed_row(
        req,
        description,
        table,
        want,
        KeyedL2NormGT {
            key_column: KEYED_KEY_COLUMN,
            value_column: value_column(description),
        },
        uo::insert_univmon_oxide,
        uo::insert_step_univmon_oxide,
        uo::query_univmon_oxide_l2_norm,
        Some((uo::merge_univmon_oxide, uo::merge_step_univmon_oxide)),
        None,
    )
}

pub(crate) fn row_univmon_entropy_lib(
    req: &Requirement,
    description: &TableDescription,
    table: GeneratedTable,
    want: &[(Operation, Metric)],
) -> Result<Measurements, RunError> {
    keyed_row(
        req,
        description,
        table,
        want,
        KeyedEntropyGT {
            key_column: KEYED_KEY_COLUMN,
            value_column: value_column(description),
        },
        ul::insert_univmon_lib,
        ul::insert_step_univmon_lib,
        ul::query_univmon_lib_entropy,
        Some((ul::merge_univmon_lib, ul::merge_step_univmon_lib)),
        None,
    )
}

pub(crate) fn row_univmon_entropy_oxide(
    req: &Requirement,
    description: &TableDescription,
    table: GeneratedTable,
    want: &[(Operation, Metric)],
) -> Result<Measurements, RunError> {
    keyed_row(
        req,
        description,
        table,
        want,
        KeyedEntropyGT {
            key_column: KEYED_KEY_COLUMN,
            value_column: value_column(description),
        },
        uo::insert_univmon_oxide,
        uo::insert_step_univmon_oxide,
        uo::query_univmon_oxide_entropy,
        Some((uo::merge_univmon_oxide, uo::merge_step_univmon_oxide)),
        None,
    )
}
