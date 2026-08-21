use super::*;
use sketch_bench::wrappers::hll::{
    datasketches as hd, oxide as ho, polars as hpo, sketchlib as hl,
};

// -------- HLL (cardinality) --------

pub(crate) fn row_hll_oxide(
    req: &Requirement,
    description: &TableDescription,
    table: GeneratedTable,
    want: &[(Operation, Metric)],
) -> Result<Measurements, RunError> {
    macro_rules! at {
        ($t:ty) => {
            cardinality_row::<$t>(
                req,
                description,
                table,
                want,
                ho::insert_hll_oxide::<$t>,
                ho::insert_step_hll_oxide::<$t>,
                ho::query_hll_oxide::<$t>,
                Some((ho::merge_hll_oxide::<$t>, ho::merge_step_hll_oxide::<$t>)),
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

pub(crate) fn row_hll_datasketches(
    req: &Requirement,
    description: &TableDescription,
    table: GeneratedTable,
    want: &[(Operation, Metric)],
) -> Result<Measurements, RunError> {
    macro_rules! at {
        ($t:ty) => {
            cardinality_row::<$t>(
                req,
                description,
                table,
                want,
                hd::insert_hll_datasketches::<$t>,
                hd::insert_step_hll_datasketches::<$t>,
                hd::query_hll_datasketches::<$t>,
                Some((
                    hd::merge_hll_datasketches::<$t>,
                    hd::merge_step_hll_datasketches::<$t>,
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

pub(crate) fn row_hll_polars(
    req: &Requirement,
    description: &TableDescription,
    table: GeneratedTable,
    want: &[(Operation, Metric)],
) -> Result<Measurements, RunError> {
    macro_rules! at {
        ($t:ty) => {
            cardinality_row::<$t>(
                req,
                description,
                table,
                want,
                hpo::insert_polars_cardinality::<$t>,
                hpo::insert_step_polars_cardinality::<$t>,
                hpo::query_polars_cardinality::<$t>,
                None,
                Some(hpo::prepare_polars_cardinality::<$t>),
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

pub(crate) fn row_hll_fastpath_parallel_lib(
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
                hl::insert_parallel_hll_fast_path::<$t>,
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

// ---------- the rows whose shape or precision selects a type ----------
//
// Three runtime values that are really type choices. Each match below turns one
// back into a monomorphisation; past it, nothing sees a runtime value again.

/// `lg_k` selects a register-storage type, because `asap_sketchlib` puts the
/// register count in the type rather than in a field. A value outside the three
/// is refused by name rather than run at 14 under its own label.
pub(crate) fn row_hll_lib(
    req: &Requirement,
    description: &TableDescription,
    table: GeneratedTable,
    want: &[(Operation, Metric)],
) -> Result<Measurements, RunError> {
    use sketch_bench::wrappers::hll::sketchlib as hl;
    use sketch_bench::wrappers::{HllBucketListP12, HllBucketListP14, HllBucketListP16};
    macro_rules! at {
        ($r:ty, $t:ty) => {
            cardinality_row::<$t>(
                req,
                description,
                table,
                want,
                hl::insert_hll_lib::<$r, $t>,
                hl::insert_step_hll_lib::<$r, $t>,
                hl::query_hll_lib::<$r, $t>,
                Some((
                    hl::merge_hll_lib::<$r, $t>,
                    hl::merge_step_hll_lib::<$r, $t>,
                )),
                None,
            )
        };
    }
    macro_rules! at_precision {
        ($t:ty) => {
            match lg_k(req)? {
                12 => at!(HllBucketListP12, $t),
                14 => at!(HllBucketListP14, $t),
                16 => at!(HllBucketListP16, $t),
                other => Err(unsupported_precision(other)),
            }
        };
    }
    match req.width {
        Dtype::I64 => at_precision!(i64),
        Dtype::U64 => at_precision!(u64),
        Dtype::F64 => at_precision!(f64),
        Dtype::Str => at_precision!(String),
    }
}

/// The HIP variant: same precision dispatch, but no merge — it maintains its
/// estimate on the insert path and the library supplies no fold.
pub(crate) fn row_hll_hip_lib(
    req: &Requirement,
    description: &TableDescription,
    table: GeneratedTable,
    want: &[(Operation, Metric)],
) -> Result<Measurements, RunError> {
    use sketch_bench::wrappers::hll::sketchlib as hl;
    use sketch_bench::wrappers::{HllBucketListP12, HllBucketListP14, HllBucketListP16};
    macro_rules! at {
        ($r:ty, $t:ty) => {
            cardinality_row::<$t>(
                req,
                description,
                table,
                want,
                hl::insert_hll_lib_hip::<$r, $t>,
                hl::insert_step_hll_lib_hip::<$r, $t>,
                hl::query_hll_lib_hip::<$r, $t>,
                None,
                None,
            )
        };
    }
    macro_rules! at_precision {
        ($t:ty) => {
            match lg_k(req)? {
                12 => at!(HllBucketListP12, $t),
                14 => at!(HllBucketListP14, $t),
                16 => at!(HllBucketListP16, $t),
                other => Err(unsupported_precision(other)),
            }
        };
    }
    match req.width {
        Dtype::I64 => at_precision!(i64),
        Dtype::U64 => at_precision!(u64),
        Dtype::F64 => at_precision!(f64),
        Dtype::Str => at_precision!(String),
    }
}

fn lg_k(req: &Requirement) -> Result<u8, RunError> {
    let p: sketch_bench::params::HllParams = req
        .params
        .parse()
        .map_err(|e: aqpbm_core::DataGenError| RunError::Sketch(e.to_string()))?;
    Ok(p.lg_k)
}

fn unsupported_precision(lg_k: u8) -> RunError {
    RunError::Sketch(format!(
        "asap_sketchlib HLL is compiled at lg_k 12, 14 and 16; requested {lg_k}"
    ))
}
