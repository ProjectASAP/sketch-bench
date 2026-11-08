use super::*;
use crate::wrappers::bloom as bl;

pub(crate) fn row_bloom_lib(
    req: &Requirement,
    columns: &ColumnRoles,
    table: GeneratedTable,
    want: &[(Operation, Metric)],
) -> Result<Measurements, RunError> {
    macro_rules! at {
        ($t:ty) => {
            membership_row::<$t>(
                req,
                columns,
                table,
                want,
                bl::insert_bloom_lib::<$t>,
                bl::insert_step_bloom_lib::<$t>,
                bl::query_bloom_lib::<$t>,
                Some((
                    bl::merge_bloom_lib::<$t>,
                    bl::merge_step_bloom_lib::<$t>,
                    bl::merge_query_bloom_lib::<$t>,
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
