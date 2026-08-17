//! Which sketch a pair of strings names: the only place in the program where a
//! name is bound to code. In the frontend, not `sketch-bench`, because that crate
//! need not know a command line exists — it hands back one erased [`Target`].

use aqpbm_core::dataset::DatasetSpec;
use aqpbm_core::ops::Target;
use aqpbm_core::request::{Numeric, Requirement};
use aqpbm_core::run_error::RunError;

/// The targets this binary can run, opened at the target's own item type — the
/// one point where that type is known, which is why the dataset is materialised
/// here rather than by the caller. Each arm names only the statistic and the
/// target's `build` / `insert` / `ask`. `None` means the pair has no arm yet.
///
/// The returned [`Target`] is then told what to measure, one instruction at a
/// time, over the data it already holds.
pub fn open_target(
    algorithm: &str,
    impl_name: &str,
    req: &Requirement,
    spec: &DatasetSpec,
) -> Option<Result<Box<dyn Target>, RunError>> {
    use aqpbm_core::ops::{
        open_target_cardinality, open_target_frequency, open_target_quantile,
        open_target_subpop_cardinality, open_target_subpop_frequency, open_target_subpop_quantile,
        open_target_timed_only,
    };
    use sketch_bench::wrappers::cms::{
        datasketches as cd, oxide as co, polars as cp, sketchlib as cl,
    };
    use sketch_bench::wrappers::cs::{oxide as so, polars as sp, sketchlib as sl};
    use sketch_bench::wrappers::hll::{
        datasketches as hd, oxide as ho, polars as hpo, sketchlib as hl,
    };
    use sketch_bench::wrappers::hydra::{polars as hp, sketchlib as hs};
    use sketch_bench::wrappers::kll::{oxide as ko, polars as kp, sketchlib as kl};

    Some(match (algorithm, impl_name) {
        // -------- CMS (frequency) --------
        ("cms", "oxide") => {
            if let Err(e) = i64_only(req) {
                return Some(Err(e));
            }
            open_target_frequency(
                req,
                spec,
                co::build_cms_oxide,
                co::memory_cms_oxide,
                co::insert_cms_oxide,
                co::ask_cms_oxide,
                Some(co::merge_cms_oxide),
                None,
            )
        }
        ("cms", "datasketches") => {
            if let Err(e) = i64_only(req) {
                return Some(Err(e));
            }
            open_target_frequency(
                req,
                spec,
                cd::build_cms_datasketches,
                cd::memory_cms_datasketches,
                cd::insert_cms_datasketches,
                cd::ask_cms_datasketches,
                Some(cd::merge_cms_datasketches),
                None,
            )
        }
        ("cms", "polars") => {
            if let Err(e) = i64_only(req) {
                return Some(Err(e));
            }
            open_target_frequency(
                req,
                spec,
                cp::build_polars_frequency_cms,
                cp::memory_polars_frequency_cms,
                cp::insert_polars_frequency_cms,
                |s, k| s.estimate_frequency(k),
                None,
                Some(cp::prepare_polars_frequency_cms),
            )
        }
        ("cms-fastpath-fixedmatrix", "lib") => return Some(fixed_matrix_cms(req, spec)),
        ("cms-fastpath-vector2d", "lib") => {
            if let Err(e) = i64_only(req) {
                return Some(Err(e));
            }
            open_target_frequency(
                req,
                spec,
                cl::build_cms_lib_vector2d_fast,
                cl::memory_cms_lib_vector2d_fast,
                cl::insert_cms_lib_vector2d_fast,
                cl::ask_cms_lib_vector2d_fast,
                Some(cl::merge_cms_lib_vector2d_fast),
                None,
            )
        }
        ("cms-regularpath-vector2d", "lib") => {
            if let Err(e) = i64_only(req) {
                return Some(Err(e));
            }
            open_target_frequency(
                req,
                spec,
                cl::build_cms_lib_vector2d_regular,
                cl::memory_cms_lib_vector2d_regular,
                cl::insert_cms_lib_vector2d_regular,
                cl::ask_cms_lib_vector2d_regular,
                Some(cl::merge_cms_lib_vector2d_regular),
                None,
            )
        }
        ("cms-fastpath-fixedmatrix-32k-parallel", "lib") => {
            open_target_timed_only::<_, i64, _, _, _>(
                req,
                spec,
                cl::build_parallel_cms_fast_path,
                cl::memory_parallel_cms_fast_path,
                cl::insert_parallel_cms_fast_path,
                None,
                Some(cl::prepare_parallel_cms_fast_path),
            )
        }

        // -------- CountSketch (frequency) --------
        ("countsketch", "oxide") => {
            if let Err(e) = i64_only(req) {
                return Some(Err(e));
            }
            open_target_frequency(
                req,
                spec,
                so::build_cs_oxide,
                so::memory_cs_oxide,
                so::insert_cs_oxide,
                so::ask_cs_oxide,
                Some(so::merge_cs_oxide),
                None,
            )
        }
        ("countsketch", "polars") => {
            if let Err(e) = i64_only(req) {
                return Some(Err(e));
            }
            open_target_frequency(
                req,
                spec,
                sp::build_polars_frequency_cs,
                sp::memory_polars_frequency_cs,
                sp::insert_polars_frequency_cs,
                |s, k| s.estimate_frequency(k),
                None,
                Some(sp::prepare_polars_frequency_cs),
            )
        }
        ("countsketch-fastpath-fixedmatrix", "lib") => return Some(fixed_matrix_cs(req, spec)),
        ("countsketch-fastpath-vector2d", "lib") => {
            if let Err(e) = i64_only(req) {
                return Some(Err(e));
            }
            open_target_frequency(
                req,
                spec,
                sl::build_cs_lib_vector2d_fast,
                sl::memory_cs_lib_vector2d_fast,
                sl::insert_cs_lib_vector2d_fast,
                sl::ask_cs_lib_vector2d_fast,
                Some(sl::merge_cs_lib_vector2d_fast),
                None,
            )
        }
        ("countsketch-regularpath-vector2d", "lib") => {
            if let Err(e) = i64_only(req) {
                return Some(Err(e));
            }
            open_target_frequency(
                req,
                spec,
                sl::build_cs_lib_vector2d_regular,
                sl::memory_cs_lib_vector2d_regular,
                sl::insert_cs_lib_vector2d_regular,
                sl::ask_cs_lib_vector2d_regular,
                Some(sl::merge_cs_lib_vector2d_regular),
                None,
            )
        }
        ("countsketch-fastpath-fixedmatrix-32k-parallel", "lib") => {
            open_target_timed_only::<_, i64, _, _, _>(
                req,
                spec,
                sl::build_parallel_cs_fast_path,
                sl::memory_parallel_cs_fast_path,
                sl::insert_parallel_cs_fast_path,
                None,
                Some(sl::prepare_parallel_cs_fast_path),
            )
        }

        // -------- HLL (cardinality) --------
        ("hll", "oxide") => {
            if let Err(e) = i64_only(req) {
                return Some(Err(e));
            }
            open_target_cardinality(
                req,
                spec,
                ho::build_hll_oxide,
                ho::memory_hll_oxide,
                ho::insert_hll_oxide,
                ho::ask_hll_oxide,
                Some(ho::merge_hll_oxide),
                None,
            )
        }
        ("hll", "datasketches") => {
            if let Err(e) = i64_only(req) {
                return Some(Err(e));
            }
            open_target_cardinality(
                req,
                spec,
                hd::build_hll_datasketches,
                hd::memory_hll_datasketches,
                hd::insert_hll_datasketches,
                hd::ask_hll_datasketches,
                Some(hd::merge_hll_datasketches),
                None,
            )
        }
        ("hll", "lib") => return Some(hll_lib(req, spec)),
        ("hll", "polars") => {
            if let Err(e) = i64_only(req) {
                return Some(Err(e));
            }
            open_target_cardinality(
                req,
                spec,
                hpo::build_polars_cardinality,
                hpo::memory_polars_cardinality,
                hpo::insert_polars_cardinality,
                |s, _: &()| s.estimate_distinct(),
                None,
                Some(hpo::prepare_polars_cardinality),
            )
        }
        ("hll-hip", "lib") => return Some(hll_lib_hip(req, spec)),
        ("hll-fastpath-parallel", "lib") => open_target_timed_only::<_, i64, _, _, _>(
            req,
            spec,
            hl::build_parallel_hll_fast_path,
            hl::memory_parallel_hll_fast_path,
            hl::insert_parallel_hll_fast_path,
            None,
            Some(hl::prepare_parallel_hll_fast_path),
        ),

        // -------- KLL (quantile) --------
        // The only targets `--dtype` selects anything for: their library is generic
        // over the value type, so a width picks a monomorphisation rather than
        // being refused. Each arm is its own instantiation.
        ("kll-percall", "oxide") => match req.width {
            Numeric::I64 => open_target_quantile(
                req,
                spec,
                ko::build_kll_oxide_per_call::<i64>,
                ko::memory_kll_oxide_per_call::<i64>,
                ko::insert_kll_oxide_per_call::<i64>,
                ko::ask_kll_oxide_per_call::<i64>,
                Some(ko::merge_kll_oxide_per_call::<i64>),
                None,
            ),
            Numeric::F64 => open_target_quantile(
                req,
                spec,
                ko::build_kll_oxide_per_call::<f64>,
                ko::memory_kll_oxide_per_call::<f64>,
                ko::insert_kll_oxide_per_call::<f64>,
                ko::ask_kll_oxide_per_call::<f64>,
                Some(ko::merge_kll_oxide_per_call::<f64>),
                None,
            ),
        },
        ("kll-percall", "lib") => match req.width {
            Numeric::I64 => open_target_quantile(
                req,
                spec,
                kl::build_kll_lib_per_call::<i64>,
                kl::memory_kll_lib_per_call::<i64>,
                kl::insert_kll_lib_per_call::<i64>,
                kl::ask_kll_lib_per_call::<i64>,
                Some(kl::merge_kll_lib_per_call::<i64>),
                None,
            ),
            Numeric::F64 => open_target_quantile(
                req,
                spec,
                kl::build_kll_lib_per_call::<f64>,
                kl::memory_kll_lib_per_call::<f64>,
                kl::insert_kll_lib_per_call::<f64>,
                kl::ask_kll_lib_per_call::<f64>,
                Some(kl::merge_kll_lib_per_call::<f64>),
                None,
            ),
        },
        ("kll-cdf", "oxide") => match req.width {
            Numeric::I64 => open_target_quantile(
                req,
                spec,
                ko::build_kll_oxide_cdf::<i64>,
                ko::memory_kll_oxide_cdf::<i64>,
                ko::insert_kll_oxide_cdf::<i64>,
                ko::ask_kll_oxide_cdf::<i64>,
                Some(ko::merge_kll_oxide_cdf::<i64>),
                Some(ko::prepare_kll_oxide_cdf::<i64>),
            ),
            Numeric::F64 => open_target_quantile(
                req,
                spec,
                ko::build_kll_oxide_cdf::<f64>,
                ko::memory_kll_oxide_cdf::<f64>,
                ko::insert_kll_oxide_cdf::<f64>,
                ko::ask_kll_oxide_cdf::<f64>,
                Some(ko::merge_kll_oxide_cdf::<f64>),
                Some(ko::prepare_kll_oxide_cdf::<f64>),
            ),
        },
        ("kll-cdf", "lib") => match req.width {
            Numeric::I64 => open_target_quantile(
                req,
                spec,
                kl::build_kll_lib_cdf::<i64>,
                kl::memory_kll_lib_cdf::<i64>,
                kl::insert_kll_lib_cdf::<i64>,
                kl::ask_kll_lib_cdf::<i64>,
                Some(kl::merge_kll_lib_cdf::<i64>),
                Some(kl::prepare_kll_lib_cdf::<i64>),
            ),
            Numeric::F64 => open_target_quantile(
                req,
                spec,
                kl::build_kll_lib_cdf::<f64>,
                kl::memory_kll_lib_cdf::<f64>,
                kl::insert_kll_lib_cdf::<f64>,
                kl::ask_kll_lib_cdf::<f64>,
                Some(kl::merge_kll_lib_cdf::<f64>),
                Some(kl::prepare_kll_lib_cdf::<f64>),
            ),
        },
        // The exact baseline is i64 only, unlike the four sketch targets above it:
        // its grid is built from a sorted i64 column.
        ("kll-cdf", "polars") => {
            if let Err(e) = i64_only(req) {
                return Some(Err(e));
            }
            open_target_quantile(
                req,
                spec,
                kp::build_polars_quantile_kll,
                kp::memory_polars_quantile_kll,
                kp::insert_polars_quantile_kll,
                |s, phi: &f64| s.estimate_quantile(*phi),
                None,
                Some(kp::prepare_polars_quantile_kll),
            )
        }

        // -------- Hydra (per-subpopulation statistics over labelled records) --------
        ("hydra-cms", "lib") => open_target_subpop_frequency(
            req,
            spec,
            hs::build_hydra_cms,
            hs::memory_hydra_cms,
            hs::insert_hydra_cms,
            hs::ask_hydra_cms,
            Some(hs::merge_hydra_cms),
            None,
        ),
        ("hydra-cms", "polars") => open_target_subpop_frequency(
            req,
            spec,
            hp::build_polars_subpop_frequency,
            hp::memory_polars_subpop_frequency,
            hp::insert_polars_subpop_frequency,
            |s, p| s.estimate_subpop_frequency(&[p.0.as_str()], &p.1),
            None,
            Some(hp::prepare_polars_subpop_frequency),
        ),
        ("hydra-hll", "lib") => open_target_subpop_cardinality(
            req,
            spec,
            hs::build_hydra_hll,
            hs::memory_hydra_hll,
            hs::insert_hydra_hll,
            hs::ask_hydra_hll,
            Some(hs::merge_hydra_hll),
            None,
        ),
        ("hydra-hll", "polars") => open_target_subpop_cardinality(
            req,
            spec,
            hp::build_polars_subpop_cardinality,
            hp::memory_polars_subpop_cardinality,
            hp::insert_polars_subpop_cardinality,
            |s, p: &String| s.estimate_subpop_cardinality(&[p.as_str()]),
            None,
            Some(hp::prepare_polars_subpop_cardinality),
        ),
        ("hydra-kll", "lib") => open_target_subpop_quantile(
            req,
            spec,
            hs::build_hydra_kll,
            hs::memory_hydra_kll,
            hs::insert_hydra_kll,
            hs::ask_hydra_kll,
            Some(hs::merge_hydra_kll),
            None,
        ),
        ("hydra-kll", "polars") => open_target_subpop_quantile(
            req,
            spec,
            hp::build_polars_subpop_quantile,
            hp::memory_polars_subpop_quantile,
            hp::insert_polars_subpop_quantile,
            |s, p: &(String, f64)| s.estimate_subpop_quantile(&[p.0.as_str()], p.1),
            None,
            Some(hp::prepare_polars_subpop_quantile),
        ),

        _ => return None,
    })
}

/// Refuse `--dtype f64` on a target whose item type is `i64`. The registry says
/// nothing about width, so refusing it needs to know which target was asked for —
/// known here. The `kll-*` targets skip it: they genuinely build at both widths.
fn i64_only(req: &Requirement) -> Result<(), RunError> {
    if req.width == Numeric::F64 {
        return Err(RunError::Body(format!(
            "{}/{} ingests i64 only; --dtype f64 is honoured by the kll rows, \
             which build at either width",
            req.algorithm, req.impl_name
        )));
    }
    Ok(())
}

// ---------- the targets whose shape or precision selects a type ----------
//
// Three runtime values that are really type choices. Each match below turns one
// back into a monomorphisation; past it, nothing sees a runtime value again.

/// `lg_k` selects a register-storage type, because `asap_sketchlib` puts the
/// register count in the type rather than in a field. A value outside the three
/// is refused by name rather than run at 14 under its own label.
fn hll_lib(req: &Requirement, spec: &DatasetSpec) -> Result<Box<dyn Target>, RunError> {
    use aqpbm_core::ops::open_target_cardinality;
    use sketch_bench::wrappers::hll::sketchlib as hl;
    use sketch_bench::wrappers::{HllBucketListP12, HllBucketListP14, HllBucketListP16};
    i64_only(req)?;
    macro_rules! at {
        ($r:ty) => {
            open_target_cardinality(
                req,
                spec,
                hl::build_hll_lib::<$r>,
                hl::memory_hll_lib::<$r>,
                hl::insert_hll_lib::<$r>,
                hl::ask_hll_lib::<$r>,
                Some(hl::merge_hll_lib::<$r>),
                None,
            )
        };
    }
    match lg_k(req)? {
        12 => at!(HllBucketListP12),
        14 => at!(HllBucketListP14),
        16 => at!(HllBucketListP16),
        other => Err(unsupported_precision(other)),
    }
}

/// The HIP variant: same precision dispatch, but no merge — it maintains its
/// estimate on the insert path and the library supplies no fold.
fn hll_lib_hip(req: &Requirement, spec: &DatasetSpec) -> Result<Box<dyn Target>, RunError> {
    use aqpbm_core::ops::open_target_cardinality;
    use sketch_bench::wrappers::hll::sketchlib as hl;
    use sketch_bench::wrappers::{HllBucketListP12, HllBucketListP14, HllBucketListP16};
    i64_only(req)?;
    macro_rules! at {
        ($r:ty) => {
            open_target_cardinality(
                req,
                spec,
                hl::build_hll_lib_hip::<$r>,
                hl::memory_hll_lib_hip::<$r>,
                hl::insert_hll_lib_hip::<$r>,
                hl::ask_hll_lib_hip::<$r>,
                None,
                None,
            )
        };
    }
    match lg_k(req)? {
        12 => at!(HllBucketListP12),
        14 => at!(HllBucketListP14),
        16 => at!(HllBucketListP16),
        other => Err(unsupported_precision(other)),
    }
}

fn lg_k(req: &Requirement) -> Result<u8, RunError> {
    let p: sketch_bench::params::HllParams = req
        .params
        .parse()
        .map_err(|e: aqpbm_core::DataGenError| RunError::Body(e.to_string()))?;
    Ok(p.lg_k)
}

fn unsupported_precision(lg_k: u8) -> RunError {
    RunError::Body(format!(
        "asap_sketchlib HLL is compiled at lg_k 12, 14 and 16; requested {lg_k}"
    ))
}

/// `(rows, cols)` selects a matrix storage type, because `impl_fixed_matrix!`
/// bakes the shape in — which is the thing these targets exist to price. The visitor
/// is the only way to hand a monomorphisation back to a caller that picked it
/// with two runtime integers, and it lives here because only here are the
/// closures and the comparator both in scope.
macro_rules! fixed_matrix_row {
    ($fname:ident, $params:ty, $algo:literal, $module:ident, $build:ident, $memory:ident,
     $insert:ident, $ask:ident, $merge:ident) => {
        fn $fname(req: &Requirement, spec: &DatasetSpec) -> Result<Box<dyn Target>, RunError> {
            use aqpbm_core::ops::open_target_frequency;
            use sketch_bench::wrappers::fixed_matrix::{
                unsupported_shape, with_fixed_matrix, FixedMatrixVisitor,
            };
            use sketch_bench::wrappers::$module::sketchlib as w;
            use sketch_bench::wrappers::{DefaultXxHasher, FastPathHasher, MatrixStorage};

            i64_only(req)?;
            let p: $params = req
                .params
                .parse()
                .map_err(|e: aqpbm_core::DataGenError| RunError::Body(e.to_string()))?;

            struct V<'a>(&'a Requirement, &'a DatasetSpec);
            impl FixedMatrixVisitor for V<'_> {
                type Out = Result<Box<dyn Target>, RunError>;
                fn visit<M>(self) -> Self::Out
                where
                    M: MatrixStorage<Counter = i32>
                        + FastPathHasher<DefaultXxHasher>
                        + Default
                        + Clone
                        + 'static,
                {
                    open_target_frequency(
                        self.0,
                        self.1,
                        w::$build::<M>,
                        w::$memory::<M>,
                        w::$insert::<M>,
                        w::$ask::<M>,
                        Some(w::$merge::<M>),
                        None,
                    )
                }
            }

            with_fixed_matrix(p.rows, p.cols, V(req, spec))
                .unwrap_or_else(|| Err(RunError::Body(unsupported_shape($algo, p.rows, p.cols))))
        }
    };
}

fixed_matrix_row!(
    fixed_matrix_cms,
    sketch_bench::params::CmsParams,
    "cms-fastpath-fixedmatrix",
    cms,
    build_cms_lib_fixedmatrix,
    memory_cms_lib_fixedmatrix,
    insert_cms_lib_fixedmatrix,
    ask_cms_lib_fixedmatrix,
    merge_cms_lib_fixedmatrix
);

fixed_matrix_row!(
    fixed_matrix_cs,
    sketch_bench::params::CountSketchParams,
    "countsketch-fastpath-fixedmatrix",
    cs,
    build_cs_lib_fixedmatrix,
    memory_cs_lib_fixedmatrix,
    insert_cs_lib_fixedmatrix,
    ask_cs_lib_fixedmatrix,
    merge_cs_lib_fixedmatrix
);

#[cfg(test)]
mod tests {
    use super::*;

    /// Every registered pair can actually be run. `registry::check` and the match
    /// above live in different crates, so nothing but this makes them agree —
    /// a pair that passes the check and finds no code reads as a broken tool.
    #[test]
    fn every_registry_entry_has_an_arm() {
        // A parameterless request over an unreadable file reaches the arm and
        // stops at the first thing it needs, which is enough to prove an arm
        // exists: what is being checked is that the pair is not `None`, and
        // `Some(Err(_))` answers that as well as `Some(Ok(_))` does.
        let req = |e: &sketch_bench::registry::SketchId| Requirement {
            algorithm: e.algorithm.to_string(),
            impl_name: e.impl_name.to_string(),
            params: aqpbm_core::ParamSet::empty(e.algorithm),
            width: Numeric::I64,
            workers: 1,
            merge_shards: 2,
            comparator: None,
        };
        let spec = DatasetSpec::File {
            path: "unreadable on purpose: only the dispatch is under test".to_string(),
        };
        let missing: Vec<String> = sketch_bench::registry::REGISTRY
            .iter()
            .filter(|e| open_target(e.algorithm, e.impl_name, &req(e), &spec).is_none())
            .map(|e| format!("{}/{}", e.algorithm, e.impl_name))
            .collect();
        assert!(
            missing.is_empty(),
            "registered but unrunnable: {}",
            missing.join(", ")
        );
    }
}
