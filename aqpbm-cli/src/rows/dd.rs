use super::*;
use sketch_bench::wrappers::dd::{oxide as ddo, sketchlib as ddl};

// -------- DDSketch (quantile) --------

pub(crate) fn row_dd_lib(
    req: &Requirement,
    description: &TableDescription,
    table: GeneratedTable,
    want: &[(Operation, Metric)],
) -> Result<Measurements, RunError> {
    macro_rules! at {
        ($t:ty) => {
            relative_value_quantile_row::<$t>(
                req,
                description,
                table,
                want,
                ddl::insert_dd_lib::<$t>,
                ddl::insert_step_dd_lib::<$t>,
                ddl::query_dd_lib::<$t>,
                Some((
                    ddl::merge_dd_lib::<$t>,
                    ddl::merge_step_dd_lib::<$t>,
                    ddl::merge_query_dd_lib::<$t>,
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

pub(crate) fn row_dd_oxide(
    req: &Requirement,
    description: &TableDescription,
    table: GeneratedTable,
    want: &[(Operation, Metric)],
) -> Result<Measurements, RunError> {
    macro_rules! at {
        ($t:ty) => {
            relative_value_quantile_row::<$t>(
                req,
                description,
                table,
                want,
                ddo::insert_dd_oxide::<$t>,
                ddo::insert_step_dd_oxide::<$t>,
                ddo::query_dd_oxide::<$t>,
                Some((
                    ddo::merge_dd_oxide::<$t>,
                    ddo::merge_step_dd_oxide::<$t>,
                    ddo::merge_query_dd_oxide::<$t>,
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
