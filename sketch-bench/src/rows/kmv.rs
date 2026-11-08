use super::*;
use crate::wrappers::kmv as km;

pub(crate) fn row_kmv_lib(
    req: &Requirement,
    columns: &ColumnRoles,
    table: GeneratedTable,
    want: &[(Operation, Metric)],
) -> Result<Measurements, RunError> {
    macro_rules! at {
        ($t:ty) => {
            cardinality_row::<$t>(
                req,
                columns,
                table,
                want,
                km::insert_kmv_lib::<$t>,
                km::insert_step_kmv_lib::<$t>,
                km::query_kmv_lib::<$t>,
                Some((
                    km::merge_kmv_lib::<$t>,
                    km::merge_step_kmv_lib::<$t>,
                    km::merge_query_kmv_lib::<$t>,
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
