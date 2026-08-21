use super::*;

/// `(rows, cols)` selects a matrix storage type, because `impl_fixed_matrix!`
/// bakes the shape in — which is the thing these rows exist to price. The visitor
/// is the only way to hand a monomorphisation back to a caller that picked it
/// with two runtime integers, and the measurements are what it hands back: an
/// erased list, so nothing in the return type mentions the shape it ran at.
macro_rules! fixed_matrix_row {
    ($fname:ident, $params:ty, $algo:literal, $module:ident, $insert:ident, $insert_step:ident,
     $query:ident, $merge:ident, $merge_step:ident) => {
        pub(crate) fn $fname(
            req: &Requirement,
            description: &TableDescription,
            table: GeneratedTable,
            want: &[(Operation, Metric)],
        ) -> Result<Measurements, RunError> {
            use sketch_bench::wrappers::fixed_matrix::{
                unsupported_shape, with_fixed_matrix, FixedMatrixVisitor,
            };
            use sketch_bench::wrappers::$module::sketchlib as w;
            use sketch_bench::wrappers::{DefaultXxHasher, FastPathHasher, MatrixStorage};

            let p: $params = req
                .params
                .parse()
                .map_err(|e: aqpbm_core::DataGenError| RunError::Sketch(e.to_string()))?;

            // Two runtime values, two type choices: the shape selects `M`
            // through the visitor below, and `--dtype` selects `T` in the match
            // that builds it. Ordered shape-inside-width because only the shape
            // can fail to resolve, and its refusal names the shape.
            struct V<'a, T> {
                req: &'a Requirement,
                description: &'a TableDescription,
                table: GeneratedTable,
                want: &'a [(Operation, Metric)],
                item: PhantomData<T>,
            }
            impl<T: CountedValue + FrequencyValue> FixedMatrixVisitor for V<'_, T> {
                type Out = Result<Measurements, RunError>;
                fn visit<M>(self) -> Self::Out
                where
                    M: MatrixStorage<Counter = i32>
                        + FastPathHasher<DefaultXxHasher>
                        + Default
                        + Clone
                        + 'static,
                {
                    frequency_row::<T>(
                        self.req,
                        self.description,
                        self.table,
                        self.want,
                        w::$insert::<M, T>,
                        w::$insert_step::<M, T>,
                        w::$query::<M, T>,
                        Some((w::$merge::<M, T>, w::$merge_step::<M, T>)),
                        None,
                    )
                }
            }

            macro_rules! at {
                ($t:ty) => {
                    with_fixed_matrix(
                        p.rows,
                        p.cols,
                        V::<$t> {
                            req,
                            description,
                            table,
                            want,
                            item: PhantomData,
                        },
                    )
                };
            }
            match req.width {
                Dtype::I64 => at!(i64),
                Dtype::U64 => at!(u64),
                Dtype::F64 => at!(f64),
                Dtype::Str => at!(String),
            }
            .unwrap_or_else(|| Err(RunError::Sketch(unsupported_shape($algo, p.rows, p.cols))))
        }
    };
}

fixed_matrix_row!(
    row_cms_fastpath_fixedmatrix_lib,
    sketch_bench::params::CmsParams,
    "cms-fastpath-fixedmatrix",
    cms,
    insert_cms_lib_fixedmatrix,
    insert_step_cms_lib_fixedmatrix,
    query_cms_lib_fixedmatrix,
    merge_cms_lib_fixedmatrix,
    merge_step_cms_lib_fixedmatrix
);

fixed_matrix_row!(
    row_countsketch_fastpath_fixedmatrix_lib,
    sketch_bench::params::CountSketchParams,
    "countsketch-fastpath-fixedmatrix",
    cs,
    insert_cs_lib_fixedmatrix,
    insert_step_cs_lib_fixedmatrix,
    query_cs_lib_fixedmatrix,
    merge_cs_lib_fixedmatrix,
    merge_step_cs_lib_fixedmatrix
);
