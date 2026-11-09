use super::*;
use crate::wrappers::{count_min_hll as cmh, count_sketch_l2_norm as csl};

pub(crate) fn row_count_sketch_l2_norm_lib(
    req: &Requirement,
    value_column: &ValueColumn,
    table: GeneratedTable,
    want: &[(Operation, Metric)],
) -> Result<Measurements, RunError> {
    macro_rules! at {
        ($t:ty) => {
            keyed_row::<$t, _>(
                req,
                value_column,
                table,
                want,
                KeyedL2NormGT::<$t>::over_columns(KEYED_KEY_COLUMN, value_column.index),
                csl::insert_count_sketch_l2_norm_lib::<$t>,
                csl::insert_step_count_sketch_l2_norm_lib::<$t>,
                csl::query_count_sketch_l2_norm_lib::<$t>,
                Some((
                    csl::merge_count_sketch_l2_norm_lib::<$t>,
                    csl::merge_step_count_sketch_l2_norm_lib::<$t>,
                    csl::merge_query_count_sketch_l2_norm_lib::<$t>,
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

pub(crate) fn row_count_min_hll_lib(
    req: &Requirement,
    value_column: &ValueColumn,
    table: GeneratedTable,
    want: &[(Operation, Metric)],
) -> Result<Measurements, RunError> {
    macro_rules! at {
        ($t:ty) => {
            subpop_cardinality_row::<$t>(
                req,
                value_column,
                table,
                want,
                cmh::insert_count_min_hll_lib::<$t>,
                cmh::insert_step_count_min_hll_lib::<$t>,
                cmh::query_count_min_hll_lib::<$t>,
                Some((
                    cmh::merge_count_min_hll_lib::<$t>,
                    cmh::merge_step_count_min_hll_lib::<$t>,
                    cmh::merge_query_count_min_hll_lib::<$t>,
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
