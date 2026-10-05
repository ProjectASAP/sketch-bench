use super::*;
use sketch_bench::wrappers::cms::sketchlib as cl;
use sketch_bench::wrappers::cms_heap::sketchlib as chl;

// -------- CMS + heap --------

pub(crate) fn row_cms_heap_fastpath_vector2d_lib(
    req: &Requirement,
    description: &TableDescription,
    table: GeneratedTable,
    want: &[(Operation, Metric)],
) -> Result<Measurements, RunError> {
    macro_rules! at {
        ($t:ty) => {
            frequency_row::<$t>(
                req,
                description,
                table,
                want,
                chl::insert_cms_heap_lib_vector2d_fast::<$t>,
                chl::insert_step_cms_heap_lib_vector2d_fast::<$t>,
                chl::query_cms_heap_lib_vector2d_fast_estimate::<$t>,
                Some((
                    chl::merge_cms_heap_lib_vector2d_fast::<$t>,
                    chl::merge_step_cms_heap_lib_vector2d_fast::<$t>,
                    chl::merge_query_cms_heap_lib_vector2d_fast_estimate::<$t>,
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

pub(crate) fn row_cms_heap_regularpath_vector2d_lib(
    req: &Requirement,
    description: &TableDescription,
    table: GeneratedTable,
    want: &[(Operation, Metric)],
) -> Result<Measurements, RunError> {
    macro_rules! at {
        ($t:ty) => {
            frequency_row::<$t>(
                req,
                description,
                table,
                want,
                chl::insert_cms_heap_lib_vector2d_regular::<$t>,
                chl::insert_step_cms_heap_lib_vector2d_regular::<$t>,
                chl::query_cms_heap_lib_vector2d_regular_estimate::<$t>,
                Some((
                    chl::merge_cms_heap_lib_vector2d_regular::<$t>,
                    chl::merge_step_cms_heap_lib_vector2d_regular::<$t>,
                    chl::merge_query_cms_heap_lib_vector2d_regular_estimate::<$t>,
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

pub(crate) fn row_cms_heap_topk_fastpath_vector2d_lib(
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
                chl::insert_cms_heap_lib_vector2d_fast::<$t>,
                chl::insert_step_cms_heap_lib_vector2d_fast::<$t>,
                chl::query_cms_heap_lib_vector2d_fast_topk::<$t>,
                Some((
                    chl::merge_cms_heap_lib_vector2d_fast::<$t>,
                    chl::merge_step_cms_heap_lib_vector2d_fast::<$t>,
                    chl::merge_query_cms_heap_lib_vector2d_fast_topk::<$t>,
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

pub(crate) fn row_cms_heap_topk_regularpath_vector2d_lib(
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
                chl::insert_cms_heap_lib_vector2d_regular::<$t>,
                chl::insert_step_cms_heap_lib_vector2d_regular::<$t>,
                chl::query_cms_heap_lib_vector2d_regular_topk::<$t>,
                Some((
                    chl::merge_cms_heap_lib_vector2d_regular::<$t>,
                    chl::merge_step_cms_heap_lib_vector2d_regular::<$t>,
                    chl::merge_query_cms_heap_lib_vector2d_regular_topk::<$t>,
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

pub(crate) fn row_cms_fastpath_fixedmatrix_32k_parallel_lib(
    req: &Requirement,
    description: &TableDescription,
    table: GeneratedTable,
    want: &[(Operation, Metric)],
) -> Result<Measurements, RunError> {
    macro_rules! at {
        ($t:ty) => {
            timed_row::<$t>(
                req,
                description,
                table,
                want,
                cl::insert_parallel_cms_fast_path::<$t>,
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
