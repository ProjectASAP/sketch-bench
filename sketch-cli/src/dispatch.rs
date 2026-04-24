//! Dispatch table: a `(family, impl)` pair picks one of the
//! 21 wrapped sketches and monomorphises a `BenchRunner` call
//! over the matching `Sketch` trait.
//!
//! Each row's `run` fn accepts a typed `&ParamSet` so the CLI
//! can sweep a grid of configs through the same table without
//! rebuilding.

use anyhow::Result;
use sketch_bench::{BenchConfig, BenchReport, BenchRunner};
use sketch_core::config::{CmsParams, CountSketchParams, ParamSet};
use sketch_core::workload::{BytesFromI64, StringFromI64, UniformI64, ZipfI64};

use crate::wrappers::{cms, countsketch, elastic, hll, kll, nitro, univmon};

#[derive(Debug, Clone, Copy)]
pub enum WorkloadSpec {
    Uniform {
        size: usize,
        cardinality: u64,
        seed: u64,
    },
    Zipf {
        size: usize,
        cardinality: u64,
        s: f64,
        seed: u64,
    },
}

impl WorkloadSpec {
    pub fn build_i64(self) -> Result<WorkloadAny> {
        Ok(match self {
            WorkloadSpec::Uniform {
                size,
                cardinality,
                seed,
            } => WorkloadAny::I64(UniformI64::new(size, cardinality, seed)),
            WorkloadSpec::Zipf {
                size,
                cardinality,
                s,
                seed,
            } => WorkloadAny::Zipf(
                ZipfI64::new(size, cardinality, s, seed).map_err(|e| anyhow::anyhow!("{}", e))?,
            ),
        })
    }
}

pub enum WorkloadAny {
    I64(UniformI64),
    Zipf(ZipfI64),
}

impl WorkloadAny {
    fn to_string_wk(&self) -> StringWk {
        match self {
            WorkloadAny::I64(w) => StringWk::FromUniform(StringFromI64::new(w)),
            WorkloadAny::Zipf(w) => StringWk::FromZipf(StringFromI64::new(w)),
        }
    }
    fn to_bytes_wk(&self) -> BytesWk {
        match self {
            WorkloadAny::I64(w) => BytesWk::FromUniform(BytesFromI64::new(w)),
            WorkloadAny::Zipf(w) => BytesWk::FromZipf(BytesFromI64::new(w)),
        }
    }
}

enum StringWk {
    FromUniform(StringFromI64<UniformI64>),
    FromZipf(StringFromI64<ZipfI64>),
}
enum BytesWk {
    FromUniform(BytesFromI64<UniformI64>),
    FromZipf(BytesFromI64<ZipfI64>),
}

/// Constraint that a given impl places on the `ParamSet` it
/// will accept. Most impls honour whatever the caller passes
/// (`Tunable`); a handful have compile-time-fixed internal
/// shapes (`Fixed`) and can only run when the requested params
/// exactly match.
#[derive(Debug, Clone, Copy)]
pub enum Constraint {
    Tunable,
    FixedCms { rows: usize, cols: usize },
    FixedCountSketch { rows: usize, cols: usize },
}

impl Constraint {
    pub fn accepts(&self, params: &ParamSet) -> bool {
        match (self, params) {
            (Constraint::Tunable, _) => true,
            (Constraint::FixedCms { rows, cols }, ParamSet::Cms(p)) => {
                p.rows == *rows && p.cols == *cols
            }
            (Constraint::FixedCountSketch { rows, cols }, ParamSet::Countsketch(p)) => {
                p.rows == *rows && p.cols == *cols
            }
            _ => false,
        }
    }

    /// Human-readable description for --fail-fast / skip logs.
    pub fn describe(&self) -> String {
        match self {
            Constraint::Tunable => "tunable".into(),
            Constraint::FixedCms { rows, cols } => format!("fixed cms ({rows}x{cols})"),
            Constraint::FixedCountSketch { rows, cols } => {
                format!("fixed countsketch ({rows}x{cols})")
            }
        }
    }
}

/// One row of the dispatch table: one sketch family × impl.
pub struct ImplEntry {
    pub family: &'static str,
    pub impl_name: &'static str,
    pub description: &'static str,
    pub constraint: Constraint,
    run: fn(cfg: &BenchConfig, wk: &WorkloadAny, params: &ParamSet) -> BenchReport,
}

impl ImplEntry {
    pub fn run(&self, cfg: &BenchConfig, wk: &WorkloadAny, params: &ParamSet) -> BenchReport {
        (self.run)(cfg, wk, params)
    }
    pub fn accepts(&self, params: &ParamSet) -> bool {
        self.constraint.accepts(params)
    }
}

/// Every registered implementation. Extending the matrix = add
/// a row here. The CLI lists these by `list-impls`.
pub const IMPLS: &[ImplEntry] = &[
    // -------- HLL --------
    ImplEntry {
        family: "hll",
        impl_name: "oxide",
        description: "sketch_oxide::cardinality::HyperLogLog",
        constraint: Constraint::Tunable,
        run: run_hll_oxide,
    },
    ImplEntry {
        family: "hll",
        impl_name: "datasketches",
        description: "datasketches::hll::HllSketch (Hll8)",
        constraint: Constraint::Tunable,
        run: run_hll_datasketches,
    },
    ImplEntry {
        family: "hll",
        impl_name: "lib",
        description: "asap_sketchlib::HyperLogLog<ErtlMLE>",
        constraint: Constraint::Tunable,
        run: run_hll_lib,
    },
    // -------- KLL --------
    ImplEntry {
        family: "kll",
        impl_name: "oxide",
        description: "sketch_oxide::quantiles::KllSketch",
        constraint: Constraint::Tunable,
        run: run_kll_oxide,
    },
    ImplEntry {
        family: "kll",
        impl_name: "lib",
        description: "asap_sketchlib::KLL<i64>",
        constraint: Constraint::Tunable,
        run: run_kll_lib,
    },
    // -------- CMS --------
    ImplEntry {
        family: "cms",
        impl_name: "oxide",
        description: "sketch_oxide::frequency::CountMinSketch",
        constraint: Constraint::Tunable,
        run: run_cms_oxide,
    },
    ImplEntry {
        family: "cms",
        impl_name: "datasketches",
        description: "datasketches::countmin::CountMinSketch",
        constraint: Constraint::Tunable,
        run: run_cms_datasketches,
    },
    ImplEntry {
        family: "cms",
        impl_name: "lib-fixedmatrix-custom-fast",
        description: "asap_sketchlib CMS, custom FixedMatrix (5x65538), FastPath",
        constraint: Constraint::FixedCms {
            rows: cms::CMS_CUSTOM_FIXED_ROWS,
            cols: cms::CMS_CUSTOM_FIXED_COLS,
        },
        run: run_cms_lib_fixedmatrix_custom_fast,
    },
    ImplEntry {
        family: "cms",
        impl_name: "lib-fixedmatrix-fast",
        description: "asap_sketchlib CMS, FixedMatrix (5x2048), FastPath",
        constraint: Constraint::FixedCms {
            rows: cms::CMS_FIXED_ROWS,
            cols: cms::CMS_FIXED_COLS,
        },
        run: run_cms_lib_fixedmatrix_fast,
    },
    ImplEntry {
        family: "cms",
        impl_name: "lib-vector2d-fast",
        description: "asap_sketchlib CMS, Vector2D, FastPath",
        constraint: Constraint::Tunable,
        run: run_cms_lib_vector2d_fast,
    },
    ImplEntry {
        family: "cms",
        impl_name: "lib-vector2d-regular",
        description: "asap_sketchlib CMS, Vector2D, RegularPath",
        constraint: Constraint::Tunable,
        run: run_cms_lib_vector2d_regular,
    },
    // -------- CountSketch --------
    ImplEntry {
        family: "countsketch",
        impl_name: "oxide",
        description: "sketch_oxide::frequency::CountSketch",
        constraint: Constraint::Tunable,
        run: run_cs_oxide,
    },
    ImplEntry {
        family: "countsketch",
        impl_name: "lib-fixedmatrix-fast",
        description: "asap_sketchlib Count, FixedMatrix (5x2048), FastPath",
        constraint: Constraint::FixedCountSketch {
            rows: cms::CMS_FIXED_ROWS,
            cols: cms::CMS_FIXED_COLS,
        },
        run: run_cs_lib_fixedmatrix_fast,
    },
    ImplEntry {
        family: "countsketch",
        impl_name: "lib-vector2d-fast",
        description: "asap_sketchlib Count, Vector2D, FastPath",
        constraint: Constraint::Tunable,
        run: run_cs_lib_vector2d_fast,
    },
    ImplEntry {
        family: "countsketch",
        impl_name: "lib-vector2d-regular",
        description: "asap_sketchlib Count, Vector2D, RegularPath",
        constraint: Constraint::Tunable,
        run: run_cs_lib_vector2d_regular,
    },
    // -------- Elastic --------
    ImplEntry {
        family: "elastic",
        impl_name: "lib",
        description: "asap_sketchlib::Elastic<DefaultXxHasher>",
        constraint: Constraint::Tunable,
        run: run_elastic_lib,
    },
    ImplEntry {
        family: "elastic",
        impl_name: "oxide",
        description: "sketch_oxide::frequency::ElasticSketch",
        constraint: Constraint::Tunable,
        run: run_elastic_oxide,
    },
    // -------- Nitro --------
    ImplEntry {
        family: "nitro",
        impl_name: "lib",
        description: "asap_sketchlib::NitroBatch<Vector2D<u32>>",
        constraint: Constraint::Tunable,
        run: run_nitro_lib,
    },
    ImplEntry {
        family: "nitro",
        impl_name: "oxide",
        description: "sketch_oxide::frequency::NitroSketch<CountMinSketch>",
        constraint: Constraint::Tunable,
        run: run_nitro_oxide,
    },
    // -------- UnivMon --------
    ImplEntry {
        family: "univmon",
        impl_name: "lib",
        description: "asap_sketchlib::UnivMon",
        constraint: Constraint::Tunable,
        run: run_univmon_lib,
    },
    ImplEntry {
        family: "univmon",
        impl_name: "oxide",
        description: "sketch_oxide::universal::UnivMon",
        constraint: Constraint::Tunable,
        run: run_univmon_oxide,
    },
];

/// Every impl registered for the given family. Empty if the
/// family name is unknown.
pub fn impls_for_family(family: &str) -> Vec<&'static ImplEntry> {
    IMPLS.iter().filter(|e| e.family == family).collect()
}

#[allow(dead_code)]
pub fn find(family: &str, impl_name: &str) -> Option<&'static ImplEntry> {
    IMPLS
        .iter()
        .find(|e| e.family == family && e.impl_name == impl_name)
}

pub fn list() -> Vec<String> {
    IMPLS
        .iter()
        .map(|e| format!("{:12} {:28} {}", e.family, e.impl_name, e.description))
        .collect()
}

// ---------- concrete run fns ----------

fn bench_over<S, W>(
    cfg: &BenchConfig,
    wk: &W,
    family: &str,
    impl_name: &str,
    factory: impl FnMut() -> S,
) -> BenchReport
where
    W: sketch_core::workload::Workload,
    W::Item: Clone,
    S: sketch_core::sketch::Sketch<Item = W::Item>,
{
    BenchRunner::new(cfg.clone(), wk, family, impl_name)
        .run::<S, _, sketch_bench::NoGT>(factory, None)
}

macro_rules! run_i64 {
    ($fn_name:ident, $wrapper:ty, $family:expr, $impl:expr, $param_variant:ident) => {
        fn $fn_name(cfg: &BenchConfig, wk: &WorkloadAny, params: &ParamSet) -> BenchReport {
            let p = match params {
                ParamSet::$param_variant(p) => *p,
                _ => panic!(
                    "dispatch::{} called with wrong family: {:?}",
                    $impl,
                    params.family()
                ),
            };
            match wk {
                WorkloadAny::I64(w) => {
                    bench_over::<$wrapper, _>(cfg, w, $family, $impl, || <$wrapper>::new(&p))
                }
                WorkloadAny::Zipf(w) => {
                    bench_over::<$wrapper, _>(cfg, w, $family, $impl, || <$wrapper>::new(&p))
                }
            }
        }
    };
}

macro_rules! run_string {
    ($fn_name:ident, $wrapper:ty, $family:expr, $impl:expr, $param_variant:ident) => {
        fn $fn_name(cfg: &BenchConfig, wk: &WorkloadAny, params: &ParamSet) -> BenchReport {
            let p = match params {
                ParamSet::$param_variant(p) => *p,
                _ => panic!(
                    "dispatch::{} called with wrong family: {:?}",
                    $impl,
                    params.family()
                ),
            };
            match wk.to_string_wk() {
                StringWk::FromUniform(w) => {
                    bench_over::<$wrapper, _>(cfg, &w, $family, $impl, || <$wrapper>::new(&p))
                }
                StringWk::FromZipf(w) => {
                    bench_over::<$wrapper, _>(cfg, &w, $family, $impl, || <$wrapper>::new(&p))
                }
            }
        }
    };
}

macro_rules! run_bytes {
    ($fn_name:ident, $wrapper:ty, $family:expr, $impl:expr, $param_variant:ident) => {
        fn $fn_name(cfg: &BenchConfig, wk: &WorkloadAny, params: &ParamSet) -> BenchReport {
            let p = match params {
                ParamSet::$param_variant(p) => *p,
                _ => panic!(
                    "dispatch::{} called with wrong family: {:?}",
                    $impl,
                    params.family()
                ),
            };
            match wk.to_bytes_wk() {
                BytesWk::FromUniform(w) => {
                    bench_over::<$wrapper, _>(cfg, &w, $family, $impl, || <$wrapper>::new(&p))
                }
                BytesWk::FromZipf(w) => {
                    bench_over::<$wrapper, _>(cfg, &w, $family, $impl, || <$wrapper>::new(&p))
                }
            }
        }
    };
}

// -- HLL --
run_i64!(run_hll_oxide, hll::HllOxide, "hll", "oxide", Hll);
run_i64!(
    run_hll_datasketches,
    hll::HllDatasketches,
    "hll",
    "datasketches",
    Hll
);
run_i64!(run_hll_lib, hll::HllLib, "hll", "lib", Hll);

// -- KLL --
run_i64!(run_kll_oxide, kll::KllOxide, "kll", "oxide", Kll);
run_i64!(run_kll_lib, kll::KllLib, "kll", "lib", Kll);

// -- CMS --
run_i64!(run_cms_oxide, cms::CmsOxide, "cms", "oxide", Cms);
run_i64!(
    run_cms_datasketches,
    cms::CmsDatasketches,
    "cms",
    "datasketches",
    Cms
);
run_i64!(
    run_cms_lib_fixedmatrix_custom_fast,
    cms::CmsLibFixedmatrixCustomFast,
    "cms",
    "lib-fixedmatrix-custom-fast",
    Cms
);
run_i64!(
    run_cms_lib_fixedmatrix_fast,
    cms::CmsLibFixedmatrixFast,
    "cms",
    "lib-fixedmatrix-fast",
    Cms
);
run_i64!(
    run_cms_lib_vector2d_fast,
    cms::CmsLibVector2dFast,
    "cms",
    "lib-vector2d-fast",
    Cms
);
run_i64!(
    run_cms_lib_vector2d_regular,
    cms::CmsLibVector2dRegular,
    "cms",
    "lib-vector2d-regular",
    Cms
);

// -- CountSketch --
run_i64!(
    run_cs_oxide,
    countsketch::CsOxide,
    "countsketch",
    "oxide",
    Countsketch
);
run_i64!(
    run_cs_lib_fixedmatrix_fast,
    countsketch::CsLibFixedmatrixFast,
    "countsketch",
    "lib-fixedmatrix-fast",
    Countsketch
);
run_i64!(
    run_cs_lib_vector2d_fast,
    countsketch::CsLibVector2dFast,
    "countsketch",
    "lib-vector2d-fast",
    Countsketch
);
run_i64!(
    run_cs_lib_vector2d_regular,
    countsketch::CsLibVector2dRegular,
    "countsketch",
    "lib-vector2d-regular",
    Countsketch
);

// -- Elastic --
run_string!(
    run_elastic_lib,
    elastic::ElasticLib,
    "elastic",
    "lib",
    Elastic
);
run_bytes!(
    run_elastic_oxide,
    elastic::ElasticOxide,
    "elastic",
    "oxide",
    Elastic
);

// -- Nitro --
run_i64!(run_nitro_lib, nitro::NitroLib, "nitro", "lib", Nitro);
run_bytes!(
    run_nitro_oxide,
    nitro::NitroOxide,
    "nitro",
    "oxide",
    Nitro
);

// -- UnivMon --
run_string!(
    run_univmon_lib,
    univmon::UnivMonLib,
    "univmon",
    "lib",
    Univmon
);
run_bytes!(
    run_univmon_oxide,
    univmon::UnivMonOxide,
    "univmon",
    "oxide",
    Univmon
);

// Suppress unused-import warning until the full suite compiles.
#[allow(dead_code)]
fn _silence_unused_warnings(_: CmsParams, _: CountSketchParams) {}
