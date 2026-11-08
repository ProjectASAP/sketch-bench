use super::*;
use crate::wrappers::uniform_sampling as us;

pub(crate) fn row_uniform_sampling_lib(
    req: &Requirement,
    columns: &ColumnRoles,
    table: GeneratedTable,
    want: &[(Operation, Metric)],
) -> Result<Measurements, RunError> {
    macro_rules! at {
        ($t:ty) => {
            quantile_row::<$t>(
                req,
                columns,
                table,
                want,
                us::insert_uniform_sampling_lib::<$t>,
                us::insert_step_uniform_sampling_lib::<$t>,
                us::query_uniform_sampling_lib::<$t>,
                Some((
                    us::merge_uniform_sampling_lib::<$t>,
                    us::merge_step_uniform_sampling_lib::<$t>,
                    us::merge_query_uniform_sampling_lib::<$t>,
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
