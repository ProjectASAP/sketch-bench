//! Dispatch table: a `(family, impl)` pair picks one of the
//! 21 wrapped sketches and monomorphises a `BenchRunner` call
//! over the matching `Sketch` trait.
//!
//! The runtime is type-polymorphic — every wrapper has its own
//! `Item` type (i64 / String / Vec<u8>) — so the dispatch goes
//! via one `fn`-pointer per entry; each fn eats the parsed
//! workload + config and emits a `BenchReport`.

use anyhow::{bail, Result};
use sketch_bench::{BenchConfig, BenchReport, BenchRunner};
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
    fn build_i64(self) -> Result<WorkloadAny> {
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

/// One row of the dispatch table: one sketch family × impl.
pub struct ImplEntry {
    pub family: &'static str,
    pub impl_name: &'static str,
    pub description: &'static str,
    run: fn(cfg: &BenchConfig, wk: &WorkloadAny) -> BenchReport,
}

/// Every registered implementation. Extending the matrix = add
/// a row here. The CLI lists these by `--list-impls`.
pub const IMPLS: &[ImplEntry] = &[
    // -------- HLL --------
    ImplEntry {
        family: "hll",
        impl_name: "oxide",
        description: "sketch_oxide::cardinality::HyperLogLog (P14)",
        run: run_hll_oxide,
    },
    ImplEntry {
        family: "hll",
        impl_name: "datasketches",
        description: "datasketches::hll::HllSketch (Hll8, P14)",
        run: run_hll_datasketches,
    },
    ImplEntry {
        family: "hll",
        impl_name: "lib",
        description: "asap_sketchlib::HyperLogLog<ErtlMLE> (P14)",
        run: run_hll_lib,
    },
    // -------- KLL --------
    ImplEntry {
        family: "kll",
        impl_name: "oxide",
        description: "sketch_oxide::quantiles::KllSketch",
        run: run_kll_oxide,
    },
    ImplEntry {
        family: "kll",
        impl_name: "lib",
        description: "asap_sketchlib::KLL<i64>",
        run: run_kll_lib,
    },
    // -------- CMS --------
    ImplEntry {
        family: "cms",
        impl_name: "oxide",
        description: "sketch_oxide::frequency::CountMinSketch",
        run: run_cms_oxide,
    },
    ImplEntry {
        family: "cms",
        impl_name: "datasketches",
        description: "datasketches::countmin::CountMinSketch",
        run: run_cms_datasketches,
    },
    ImplEntry {
        family: "cms",
        impl_name: "lib-fixedmatrix-custom-fast",
        description: "asap_sketchlib CMS, custom FixedMatrix (5x65538), FastPath",
        run: run_cms_lib_fixedmatrix_custom_fast,
    },
    ImplEntry {
        family: "cms",
        impl_name: "lib-fixedmatrix-fast",
        description: "asap_sketchlib CMS, FixedMatrix, FastPath",
        run: run_cms_lib_fixedmatrix_fast,
    },
    ImplEntry {
        family: "cms",
        impl_name: "lib-vector2d-fast",
        description: "asap_sketchlib CMS, Vector2D (5x2048), FastPath",
        run: run_cms_lib_vector2d_fast,
    },
    ImplEntry {
        family: "cms",
        impl_name: "lib-vector2d-regular",
        description: "asap_sketchlib CMS, Vector2D (5x2048), RegularPath",
        run: run_cms_lib_vector2d_regular,
    },
    // -------- CountSketch --------
    ImplEntry {
        family: "countsketch",
        impl_name: "oxide",
        description: "sketch_oxide::frequency::CountSketch",
        run: run_cs_oxide,
    },
    ImplEntry {
        family: "countsketch",
        impl_name: "lib-fixedmatrix-fast",
        description: "asap_sketchlib Count, FixedMatrix, FastPath",
        run: run_cs_lib_fixedmatrix_fast,
    },
    ImplEntry {
        family: "countsketch",
        impl_name: "lib-vector2d-fast",
        description: "asap_sketchlib Count, Vector2D (5x2048), FastPath",
        run: run_cs_lib_vector2d_fast,
    },
    ImplEntry {
        family: "countsketch",
        impl_name: "lib-vector2d-regular",
        description: "asap_sketchlib Count, Vector2D (5x2048), RegularPath",
        run: run_cs_lib_vector2d_regular,
    },
    // -------- Elastic --------
    ImplEntry {
        family: "elastic",
        impl_name: "lib",
        description: "asap_sketchlib::Elastic<DefaultXxHasher>",
        run: run_elastic_lib,
    },
    ImplEntry {
        family: "elastic",
        impl_name: "oxide",
        description: "sketch_oxide::frequency::ElasticSketch",
        run: run_elastic_oxide,
    },
    // -------- Nitro --------
    ImplEntry {
        family: "nitro",
        impl_name: "lib",
        description: "asap_sketchlib::NitroBatch<Vector2D<u32>>",
        run: run_nitro_lib,
    },
    ImplEntry {
        family: "nitro",
        impl_name: "oxide",
        description: "sketch_oxide::frequency::NitroSketch<CountMinSketch>",
        run: run_nitro_oxide,
    },
    // -------- UnivMon --------
    ImplEntry {
        family: "univmon",
        impl_name: "lib",
        description: "asap_sketchlib::UnivMon",
        run: run_univmon_lib,
    },
    ImplEntry {
        family: "univmon",
        impl_name: "oxide",
        description: "sketch_oxide::universal::UnivMon",
        run: run_univmon_oxide,
    },
];

impl ImplEntry {
    pub fn run(&self, cfg: &BenchConfig, wk: &WorkloadAny) -> BenchReport {
        (self.run)(cfg, wk)
    }
}

pub fn find(family: &str, impl_name: &str) -> Option<&'static ImplEntry> {
    IMPLS
        .iter()
        .find(|e| e.family == family && e.impl_name == impl_name)
}

pub fn run(
    family: &str,
    impl_name: &str,
    cfg: &BenchConfig,
    spec: WorkloadSpec,
) -> Result<BenchReport> {
    let entry = find(family, impl_name).ok_or_else(|| {
        anyhow::anyhow!("no sketch impl registered: family={family}, impl={impl_name}")
    })?;
    let wk = spec.build_i64()?;
    Ok(entry.run(cfg, &wk))
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
    ($fn_name:ident, $wrapper:ty, $ctor:expr, $family:expr, $impl:expr) => {
        fn $fn_name(cfg: &BenchConfig, wk: &WorkloadAny) -> BenchReport {
            match wk {
                WorkloadAny::I64(w) => bench_over::<$wrapper, _>(cfg, w, $family, $impl, || $ctor),
                WorkloadAny::Zipf(w) => bench_over::<$wrapper, _>(cfg, w, $family, $impl, || $ctor),
            }
        }
    };
}

macro_rules! run_string {
    ($fn_name:ident, $wrapper:ty, $ctor:expr, $family:expr, $impl:expr) => {
        fn $fn_name(cfg: &BenchConfig, wk: &WorkloadAny) -> BenchReport {
            match wk.to_string_wk() {
                StringWk::FromUniform(w) => {
                    bench_over::<$wrapper, _>(cfg, &w, $family, $impl, || $ctor)
                }
                StringWk::FromZipf(w) => {
                    bench_over::<$wrapper, _>(cfg, &w, $family, $impl, || $ctor)
                }
            }
        }
    };
}

macro_rules! run_bytes {
    ($fn_name:ident, $wrapper:ty, $ctor:expr, $family:expr, $impl:expr) => {
        fn $fn_name(cfg: &BenchConfig, wk: &WorkloadAny) -> BenchReport {
            match wk.to_bytes_wk() {
                BytesWk::FromUniform(w) => {
                    bench_over::<$wrapper, _>(cfg, &w, $family, $impl, || $ctor)
                }
                BytesWk::FromZipf(w) => {
                    bench_over::<$wrapper, _>(cfg, &w, $family, $impl, || $ctor)
                }
            }
        }
    };
}

// -- HLL --
run_i64!(
    run_hll_oxide,
    hll::HllOxide,
    hll::HllOxide::new(),
    "hll",
    "oxide"
);
run_i64!(
    run_hll_datasketches,
    hll::HllDatasketches,
    hll::HllDatasketches::new(),
    "hll",
    "datasketches"
);
run_i64!(run_hll_lib, hll::HllLib, hll::HllLib::new(), "hll", "lib");

// -- KLL --
run_i64!(
    run_kll_oxide,
    kll::KllOxide,
    kll::KllOxide::new(),
    "kll",
    "oxide"
);
run_i64!(run_kll_lib, kll::KllLib, kll::KllLib::new(), "kll", "lib");

// -- CMS --
run_i64!(
    run_cms_oxide,
    cms::CmsOxide,
    cms::CmsOxide::new(),
    "cms",
    "oxide"
);
run_i64!(
    run_cms_datasketches,
    cms::CmsDatasketches,
    cms::CmsDatasketches::new(),
    "cms",
    "datasketches"
);
run_i64!(
    run_cms_lib_fixedmatrix_custom_fast,
    cms::CmsLibFixedmatrixCustomFast,
    cms::CmsLibFixedmatrixCustomFast::new(),
    "cms",
    "lib-fixedmatrix-custom-fast"
);
run_i64!(
    run_cms_lib_fixedmatrix_fast,
    cms::CmsLibFixedmatrixFast,
    cms::CmsLibFixedmatrixFast::new(),
    "cms",
    "lib-fixedmatrix-fast"
);
run_i64!(
    run_cms_lib_vector2d_fast,
    cms::CmsLibVector2dFast,
    cms::CmsLibVector2dFast::new(),
    "cms",
    "lib-vector2d-fast"
);
run_i64!(
    run_cms_lib_vector2d_regular,
    cms::CmsLibVector2dRegular,
    cms::CmsLibVector2dRegular::new(),
    "cms",
    "lib-vector2d-regular"
);

// -- CountSketch --
run_i64!(
    run_cs_oxide,
    countsketch::CsOxide,
    countsketch::CsOxide::new(),
    "countsketch",
    "oxide"
);
run_i64!(
    run_cs_lib_fixedmatrix_fast,
    countsketch::CsLibFixedmatrixFast,
    countsketch::CsLibFixedmatrixFast::new(),
    "countsketch",
    "lib-fixedmatrix-fast"
);
run_i64!(
    run_cs_lib_vector2d_fast,
    countsketch::CsLibVector2dFast,
    countsketch::CsLibVector2dFast::new(),
    "countsketch",
    "lib-vector2d-fast"
);
run_i64!(
    run_cs_lib_vector2d_regular,
    countsketch::CsLibVector2dRegular,
    countsketch::CsLibVector2dRegular::new(),
    "countsketch",
    "lib-vector2d-regular"
);

// -- Elastic --
run_string!(
    run_elastic_lib,
    elastic::ElasticLib,
    elastic::ElasticLib::new(),
    "elastic",
    "lib"
);
run_bytes!(
    run_elastic_oxide,
    elastic::ElasticOxide,
    elastic::ElasticOxide::new(),
    "elastic",
    "oxide"
);

// -- Nitro --
run_i64!(
    run_nitro_lib,
    nitro::NitroLib,
    nitro::NitroLib::new(),
    "nitro",
    "lib"
);
run_bytes!(
    run_nitro_oxide,
    nitro::NitroOxide,
    nitro::NitroOxide::new(),
    "nitro",
    "oxide"
);

// -- UnivMon --
run_string!(
    run_univmon_lib,
    univmon::UnivMonLib,
    univmon::UnivMonLib::new(),
    "univmon",
    "lib"
);
run_bytes!(
    run_univmon_oxide,
    univmon::UnivMonOxide,
    univmon::UnivMonOxide::new(),
    "univmon",
    "oxide"
);

// -- dead_code silencer for unused helpers when the table gets
// trimmed in future refactors
#[allow(dead_code)]
fn _unused() {
    let _ = |e: &ImplEntry| -> Result<()> {
        bail!("{}/{}: {}", e.family, e.impl_name, e.description);
    };
}
