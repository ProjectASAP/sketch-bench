use super::*;
use aqpbm_core::accuracy::aggregate::{Aggregate, AggregateGT};
use sketch_bench::wrappers::cs_heap as csh;
use sketch_bench::wrappers::exact::{self as ex, Cell, Increase, Max, Sum};
use sketch_bench::wrappers::quantile_value::ToF64;
use sketch_bench::wrappers::univmon::sketchlib as ul;

// -------- exact multi-subpopulation accumulators --------
//
// Every column before the value is a label; a group is the whole combination.
// Numeric value widths only: `string` has no sum, order, or difference.

fn aggregate_row<C: Cell, V: ColumnItem + ToF64 + Copy + 'static>(
    req: &Requirement,
    description: &TableDescription,
    table: GeneratedTable,
    want: &[(Operation, Metric)],
    aggregate: Aggregate,
) -> Result<Measurements, RunError> {
    let value_column = value_column(description);
    scored_row(
        req,
        description,
        table,
        want,
        AggregateGT {
            group_columns: (0..value_column).collect(),
            value_column,
            aggregate,
        },
        peel_grouped::<V>,
        ex::insert_exact::<C, V>,
        ex::insert_step_exact::<C, V>,
        ex::query_exact::<C, V>,
        Some((
            ex::merge_exact::<C, V>,
            ex::merge_step_exact::<C, V>,
            ex::merge_query_exact::<C, V>,
        )),
        None,
    )
}

fn exact_row<C: Cell>(
    req: &Requirement,
    description: &TableDescription,
    table: GeneratedTable,
    want: &[(Operation, Metric)],
    aggregate: Aggregate,
) -> Result<Measurements, RunError> {
    match req.width {
        Dtype::I64 => aggregate_row::<C, i64>(req, description, table, want, aggregate),
        Dtype::U64 => aggregate_row::<C, u64>(req, description, table, want, aggregate),
        Dtype::F64 => aggregate_row::<C, f64>(req, description, table, want, aggregate),
        other => Err(no_build_at(req, other)),
    }
}

pub(crate) fn row_exact_sum(
    req: &Requirement,
    description: &TableDescription,
    table: GeneratedTable,
    want: &[(Operation, Metric)],
) -> Result<Measurements, RunError> {
    exact_row::<Sum>(req, description, table, want, Aggregate::Sum)
}

pub(crate) fn row_exact_max(
    req: &Requirement,
    description: &TableDescription,
    table: GeneratedTable,
    want: &[(Operation, Metric)],
) -> Result<Measurements, RunError> {
    exact_row::<Max>(req, description, table, want, Aggregate::Max)
}

pub(crate) fn row_exact_increase(
    req: &Requirement,
    description: &TableDescription,
    table: GeneratedTable,
    want: &[(Operation, Metric)],
) -> Result<Measurements, RunError> {
    exact_row::<Increase>(req, description, table, want, Aggregate::Increase)
}

// -------- more top-k: CountSketch + heap, UnivMon --------

pub(crate) fn row_countsketch_heap_topk_fastpath_vector2d_lib(
    req: &Requirement,
    description: &TableDescription,
    table: GeneratedTable,
    want: &[(Operation, Metric)],
) -> Result<Measurements, RunError> {
    macro_rules! at {
        ($t:ty) => {
            topk_row::<$t>(
                req,
                description,
                table,
                want,
                csh::insert_cs_heap_lib_vector2d_fast::<$t>,
                csh::insert_step_cs_heap_lib_vector2d_fast::<$t>,
                csh::query_cs_heap_lib_vector2d_fast_topk::<$t>,
                Some((
                    csh::merge_cs_heap_lib_vector2d_fast::<$t>,
                    csh::merge_step_cs_heap_lib_vector2d_fast::<$t>,
                    csh::merge_query_cs_heap_lib_vector2d_fast_topk::<$t>,
                )),
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

pub(crate) fn row_univmon_topk_lib(
    req: &Requirement,
    description: &TableDescription,
    table: GeneratedTable,
    want: &[(Operation, Metric)],
) -> Result<Measurements, RunError> {
    macro_rules! at {
        ($t:ty) => {
            topk_row::<$t>(
                req,
                description,
                table,
                want,
                ul::insert_univmon_lib_items::<$t>,
                ul::insert_step_univmon_lib_items::<$t>,
                ul::query_univmon_lib_topk::<$t>,
                Some((
                    ul::merge_univmon_lib_items::<$t>,
                    ul::merge_step_univmon_lib_items::<$t>,
                    ul::merge_query_univmon_lib_topk::<$t>,
                )),
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
