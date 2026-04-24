//! Dispatch table: a `(family, impl)` pair picks one of the
//! 21 wrapped sketches and monomorphises a `BenchRunner` call
//! over the matching `Sketch` trait.
//!
//! Each row's `run` fn accepts a typed `&ParamSet` + an optional
//! accuracy config so the CLI can sweep a grid of configs and
//! optionally compute per-family ground-truth accuracy through
//! the same table.

use std::collections::HashSet;
use std::hash::Hash;

use anyhow::Result;
use sketch_bench::accuracy::cardinality::CardinalityGT;
use sketch_bench::accuracy::frequency::FrequencyGT;
use sketch_bench::accuracy::quantile::{QuantileGT, ToF64};
use sketch_bench::{BenchConfig, BenchReport, BenchRunner};
use sketch_core::config::{CmsParams, CountSketchParams, ParamSet};
use sketch_core::workload::{BytesFromI64, StringFromI64, UniformI64, Workload, ZipfI64};

use crate::wrappers::{cms, countsketch, elastic, exact, hll, kll, nitro, univmon};

/// CLI-side accuracy settings. `enabled = false` → dispatch
/// runs `NoGT` (no ground truth). `enabled = true` → each
/// family picks its own comparator (see `AccuracyKind`).
#[derive(Debug, Clone, Copy)]
pub struct AccuracyCfg {
    pub enabled: bool,
    /// Cap on the number of distinct keys probed by frequency
    /// comparators. `0` → no cap (probe every distinct key).
    /// Ignored by cardinality / quantile comparators (they're
    /// single-shot).
    pub max_probes: usize,
}

impl AccuracyCfg {
    pub fn disabled() -> Self {
        Self {
            enabled: false,
            max_probes: 0,
        }
    }
}

/// Which comparator a family supports. Used for stderr
/// diagnostics when `--accuracy` is on but the family has no
/// viable GT.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccuracyKind {
    Frequency,
    Cardinality,
    Quantile,
    /// Family has no viable comparator (e.g. UnivMon's
    /// `Query=()` + stub CLI query). `--accuracy` will be
    /// ignored with a stderr note.
    None,
}

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
    /// Exact baselines — they ignore the family's `ParamSet`. The
    /// sweep driver runs them at most once per invocation instead
    /// of once per config.
    Unparameterized,
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
            (Constraint::Unparameterized, _) => true,
            _ => false,
        }
    }

    pub fn is_unparameterized(&self) -> bool {
        matches!(self, Constraint::Unparameterized)
    }

    pub fn describe(&self) -> String {
        match self {
            Constraint::Tunable => "tunable".into(),
            Constraint::FixedCms { rows, cols } => format!("fixed cms ({rows}x{cols})"),
            Constraint::FixedCountSketch { rows, cols } => {
                format!("fixed countsketch ({rows}x{cols})")
            }
            Constraint::Unparameterized => "unparameterized".into(),
        }
    }
}

pub struct ImplEntry {
    pub family: &'static str,
    pub impl_name: &'static str,
    pub description: &'static str,
    pub constraint: Constraint,
    pub accuracy_kind: AccuracyKind,
    run: fn(
        cfg: &BenchConfig,
        wk: &WorkloadAny,
        params: &ParamSet,
        accuracy: &AccuracyCfg,
    ) -> BenchReport,
}

impl ImplEntry {
    pub fn run(
        &self,
        cfg: &BenchConfig,
        wk: &WorkloadAny,
        params: &ParamSet,
        accuracy: &AccuracyCfg,
    ) -> BenchReport {
        (self.run)(cfg, wk, params, accuracy)
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
        accuracy_kind: AccuracyKind::Cardinality,
        run: run_hll_oxide,
    },
    ImplEntry {
        family: "hll",
        impl_name: "datasketches",
        description: "datasketches::hll::HllSketch (Hll8)",
        constraint: Constraint::Tunable,
        accuracy_kind: AccuracyKind::Cardinality,
        run: run_hll_datasketches,
    },
    ImplEntry {
        family: "hll",
        impl_name: "lib",
        description: "asap_sketchlib::HyperLogLog<ErtlMLE>",
        constraint: Constraint::Tunable,
        accuracy_kind: AccuracyKind::Cardinality,
        run: run_hll_lib,
    },
    ImplEntry {
        family: "hll",
        impl_name: "exact",
        description: "exact baseline: HashSet<i64>, cardinality = set.len()",
        constraint: Constraint::Unparameterized,
        accuracy_kind: AccuracyKind::Cardinality,
        run: run_hll_exact,
    },
    // -------- KLL --------
    ImplEntry {
        family: "kll",
        impl_name: "oxide",
        description: "sketch_oxide::quantiles::KllSketch",
        constraint: Constraint::Tunable,
        accuracy_kind: AccuracyKind::Quantile,
        run: run_kll_oxide,
    },
    ImplEntry {
        family: "kll",
        impl_name: "lib",
        description: "asap_sketchlib::KLL<i64>",
        constraint: Constraint::Tunable,
        accuracy_kind: AccuracyKind::Quantile,
        run: run_kll_lib,
    },
    ImplEntry {
        family: "kll",
        impl_name: "exact",
        description: "exact baseline: Vec<i64> sorted, quantile = index lookup",
        constraint: Constraint::Unparameterized,
        accuracy_kind: AccuracyKind::Quantile,
        run: run_kll_exact,
    },
    // -------- CMS --------
    ImplEntry {
        family: "cms",
        impl_name: "oxide",
        description: "sketch_oxide::frequency::CountMinSketch",
        constraint: Constraint::Tunable,
        accuracy_kind: AccuracyKind::Frequency,
        run: run_cms_oxide,
    },
    ImplEntry {
        family: "cms",
        impl_name: "datasketches",
        description: "datasketches::countmin::CountMinSketch",
        constraint: Constraint::Tunable,
        accuracy_kind: AccuracyKind::Frequency,
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
        accuracy_kind: AccuracyKind::Frequency,
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
        accuracy_kind: AccuracyKind::Frequency,
        run: run_cms_lib_fixedmatrix_fast,
    },
    ImplEntry {
        family: "cms",
        impl_name: "lib-vector2d-fast",
        description: "asap_sketchlib CMS, Vector2D, FastPath",
        constraint: Constraint::Tunable,
        accuracy_kind: AccuracyKind::Frequency,
        run: run_cms_lib_vector2d_fast,
    },
    ImplEntry {
        family: "cms",
        impl_name: "lib-vector2d-regular",
        description: "asap_sketchlib CMS, Vector2D, RegularPath",
        constraint: Constraint::Tunable,
        accuracy_kind: AccuracyKind::Frequency,
        run: run_cms_lib_vector2d_regular,
    },
    ImplEntry {
        family: "cms",
        impl_name: "exact",
        description: "exact baseline: HashMap<i64,u64>, freq = map.get(k)",
        constraint: Constraint::Unparameterized,
        accuracy_kind: AccuracyKind::Frequency,
        run: run_cms_exact,
    },
    // -------- CountSketch --------
    ImplEntry {
        family: "countsketch",
        impl_name: "oxide",
        description: "sketch_oxide::frequency::CountSketch",
        constraint: Constraint::Tunable,
        accuracy_kind: AccuracyKind::Frequency,
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
        accuracy_kind: AccuracyKind::Frequency,
        run: run_cs_lib_fixedmatrix_fast,
    },
    ImplEntry {
        family: "countsketch",
        impl_name: "lib-vector2d-fast",
        description: "asap_sketchlib Count, Vector2D, FastPath",
        constraint: Constraint::Tunable,
        accuracy_kind: AccuracyKind::Frequency,
        run: run_cs_lib_vector2d_fast,
    },
    ImplEntry {
        family: "countsketch",
        impl_name: "lib-vector2d-regular",
        description: "asap_sketchlib Count, Vector2D, RegularPath",
        constraint: Constraint::Tunable,
        accuracy_kind: AccuracyKind::Frequency,
        run: run_cs_lib_vector2d_regular,
    },
    // -------- Elastic --------
    ImplEntry {
        family: "elastic",
        impl_name: "lib",
        description: "asap_sketchlib::Elastic<DefaultXxHasher>",
        constraint: Constraint::Tunable,
        accuracy_kind: AccuracyKind::Frequency,
        run: run_elastic_lib,
    },
    ImplEntry {
        family: "elastic",
        impl_name: "oxide",
        description: "sketch_oxide::frequency::ElasticSketch",
        constraint: Constraint::Tunable,
        accuracy_kind: AccuracyKind::Frequency,
        run: run_elastic_oxide,
    },
    // -------- Nitro --------
    // `query()` returns 0 for both wrappers (stub); no viable
    // accuracy comparator — CLI ignores --accuracy and logs.
    ImplEntry {
        family: "nitro",
        impl_name: "lib",
        description: "asap_sketchlib::NitroBatch<Vector2D<u32>>",
        constraint: Constraint::Tunable,
        accuracy_kind: AccuracyKind::None,
        run: run_nitro_lib,
    },
    ImplEntry {
        family: "nitro",
        impl_name: "oxide",
        description: "sketch_oxide::frequency::NitroSketch<CountMinSketch>",
        constraint: Constraint::Tunable,
        accuracy_kind: AccuracyKind::None,
        run: run_nitro_oxide,
    },
    // -------- UnivMon --------
    // Multi-query moment sketch; `query()` returns 0 (stub).
    // No viable comparator here either.
    ImplEntry {
        family: "univmon",
        impl_name: "lib",
        description: "asap_sketchlib::UnivMon",
        constraint: Constraint::Tunable,
        accuracy_kind: AccuracyKind::None,
        run: run_univmon_lib,
    },
    ImplEntry {
        family: "univmon",
        impl_name: "oxide",
        description: "sketch_oxide::universal::UnivMon",
        constraint: Constraint::Tunable,
        accuracy_kind: AccuracyKind::None,
        run: run_univmon_oxide,
    },
];

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

// ---------- shared runners ----------

fn bench_no_gt<S, W>(
    cfg: &BenchConfig,
    wk: &W,
    family: &str,
    impl_name: &str,
    factory: impl FnMut() -> S,
) -> BenchReport
where
    W: Workload,
    W::Item: Clone,
    S: sketch_core::sketch::Sketch<Item = W::Item>,
{
    BenchRunner::new(cfg.clone(), wk, family, impl_name)
        .run::<S, _, sketch_bench::NoGT>(factory, None)
}

fn bench_freq_gt<S, W>(
    cfg: &BenchConfig,
    wk: &W,
    family: &str,
    impl_name: &str,
    factory: impl FnMut() -> S,
    max_probes: usize,
) -> BenchReport
where
    W: Workload,
    W::Item: Clone + Eq + Hash,
    S: sketch_core::sketch::Sketch<Item = W::Item, Query = W::Item, Answer = u64>,
{
    let keys_to_probe = sample_distinct(wk.items(), max_probes);
    let gt = FrequencyGT { keys_to_probe };
    BenchRunner::new(cfg.clone(), wk, family, impl_name)
        .run::<S, _, FrequencyGT<W::Item>>(factory, Some(&gt))
}

fn bench_card_gt<S, W>(
    cfg: &BenchConfig,
    wk: &W,
    family: &str,
    impl_name: &str,
    factory: impl FnMut() -> S,
) -> BenchReport
where
    W: Workload,
    W::Item: Clone + Eq + Hash,
    S: sketch_core::sketch::Sketch<Item = W::Item, Query = (), Answer = f64>,
{
    let gt = CardinalityGT;
    BenchRunner::new(cfg.clone(), wk, family, impl_name)
        .run::<S, _, CardinalityGT>(factory, Some(&gt))
}

fn bench_quant_gt<S, W>(
    cfg: &BenchConfig,
    wk: &W,
    family: &str,
    impl_name: &str,
    factory: impl FnMut() -> S,
) -> BenchReport
where
    W: Workload,
    W::Item: Clone + PartialOrd + ToF64,
    S: sketch_core::sketch::Sketch<Item = W::Item, Query = f64, Answer = f64>,
{
    let gt = QuantileGT;
    BenchRunner::new(cfg.clone(), wk, family, impl_name)
        .run::<S, _, QuantileGT>(factory, Some(&gt))
}

/// Pick up to `max_probes` distinct keys from `items`. `0` =
/// no cap. Iteration order of a `HashSet` is non-deterministic
/// across runs, but the GT comparator averages over the probe
/// set so that's fine for p99 rel-err estimation; tighten if a
/// reproducibility need arises.
fn sample_distinct<K>(items: &[K], max_probes: usize) -> Vec<K>
where
    K: Clone + Eq + Hash,
{
    let mut seen: HashSet<K> = HashSet::new();
    let mut out: Vec<K> = Vec::new();
    let cap = if max_probes == 0 {
        usize::MAX
    } else {
        max_probes
    };
    for it in items {
        if out.len() >= cap {
            break;
        }
        if seen.insert(it.clone()) {
            out.push(it.clone());
        }
    }
    out
}

// ---------- per-family dispatch macros ----------
//
// One macro per (key-type × GT-family) pair. The macro fans the
// WorkloadAny variants into typed `bench_*_gt` calls. When
// `accuracy.enabled` is false we fall through to `bench_no_gt`.

macro_rules! run_i64_freq {
    ($fn_name:ident, $wrapper:ty, $family:expr, $impl:expr, $param_variant:ident) => {
        fn $fn_name(
            cfg: &BenchConfig,
            wk: &WorkloadAny,
            params: &ParamSet,
            accuracy: &AccuracyCfg,
        ) -> BenchReport {
            let p = match params {
                ParamSet::$param_variant(p) => *p,
                _ => panic!("dispatch::{} wrong family: {:?}", $impl, params.family()),
            };
            match (wk, accuracy.enabled) {
                (WorkloadAny::I64(w), true) => bench_freq_gt::<$wrapper, _>(
                    cfg,
                    w,
                    $family,
                    $impl,
                    || <$wrapper>::new(&p),
                    accuracy.max_probes,
                ),
                (WorkloadAny::Zipf(w), true) => bench_freq_gt::<$wrapper, _>(
                    cfg,
                    w,
                    $family,
                    $impl,
                    || <$wrapper>::new(&p),
                    accuracy.max_probes,
                ),
                (WorkloadAny::I64(w), false) => {
                    bench_no_gt::<$wrapper, _>(cfg, w, $family, $impl, || <$wrapper>::new(&p))
                }
                (WorkloadAny::Zipf(w), false) => {
                    bench_no_gt::<$wrapper, _>(cfg, w, $family, $impl, || <$wrapper>::new(&p))
                }
            }
        }
    };
}

macro_rules! run_i64_card {
    ($fn_name:ident, $wrapper:ty, $family:expr, $impl:expr, $param_variant:ident) => {
        fn $fn_name(
            cfg: &BenchConfig,
            wk: &WorkloadAny,
            params: &ParamSet,
            accuracy: &AccuracyCfg,
        ) -> BenchReport {
            let p = match params {
                ParamSet::$param_variant(p) => *p,
                _ => panic!("dispatch::{} wrong family: {:?}", $impl, params.family()),
            };
            match (wk, accuracy.enabled) {
                (WorkloadAny::I64(w), true) => {
                    bench_card_gt::<$wrapper, _>(cfg, w, $family, $impl, || <$wrapper>::new(&p))
                }
                (WorkloadAny::Zipf(w), true) => {
                    bench_card_gt::<$wrapper, _>(cfg, w, $family, $impl, || <$wrapper>::new(&p))
                }
                (WorkloadAny::I64(w), false) => {
                    bench_no_gt::<$wrapper, _>(cfg, w, $family, $impl, || <$wrapper>::new(&p))
                }
                (WorkloadAny::Zipf(w), false) => {
                    bench_no_gt::<$wrapper, _>(cfg, w, $family, $impl, || <$wrapper>::new(&p))
                }
            }
        }
    };
}

macro_rules! run_i64_quant {
    ($fn_name:ident, $wrapper:ty, $family:expr, $impl:expr, $param_variant:ident) => {
        fn $fn_name(
            cfg: &BenchConfig,
            wk: &WorkloadAny,
            params: &ParamSet,
            accuracy: &AccuracyCfg,
        ) -> BenchReport {
            let p = match params {
                ParamSet::$param_variant(p) => *p,
                _ => panic!("dispatch::{} wrong family: {:?}", $impl, params.family()),
            };
            match (wk, accuracy.enabled) {
                (WorkloadAny::I64(w), true) => {
                    bench_quant_gt::<$wrapper, _>(cfg, w, $family, $impl, || <$wrapper>::new(&p))
                }
                (WorkloadAny::Zipf(w), true) => {
                    bench_quant_gt::<$wrapper, _>(cfg, w, $family, $impl, || <$wrapper>::new(&p))
                }
                (WorkloadAny::I64(w), false) => {
                    bench_no_gt::<$wrapper, _>(cfg, w, $family, $impl, || <$wrapper>::new(&p))
                }
                (WorkloadAny::Zipf(w), false) => {
                    bench_no_gt::<$wrapper, _>(cfg, w, $family, $impl, || <$wrapper>::new(&p))
                }
            }
        }
    };
}

/// For Nitro lib: `Query = i64` but `query()` returns a stub
/// 0 — a frequency GT would report 100% error, which is
/// misleading. Always run with NoGT.
macro_rules! run_i64_none {
    ($fn_name:ident, $wrapper:ty, $family:expr, $impl:expr, $param_variant:ident) => {
        fn $fn_name(
            cfg: &BenchConfig,
            wk: &WorkloadAny,
            params: &ParamSet,
            _accuracy: &AccuracyCfg,
        ) -> BenchReport {
            let p = match params {
                ParamSet::$param_variant(p) => *p,
                _ => panic!("dispatch::{} wrong family: {:?}", $impl, params.family()),
            };
            match wk {
                WorkloadAny::I64(w) => {
                    bench_no_gt::<$wrapper, _>(cfg, w, $family, $impl, || <$wrapper>::new(&p))
                }
                WorkloadAny::Zipf(w) => {
                    bench_no_gt::<$wrapper, _>(cfg, w, $family, $impl, || <$wrapper>::new(&p))
                }
            }
        }
    };
}

macro_rules! run_string_freq {
    ($fn_name:ident, $wrapper:ty, $family:expr, $impl:expr, $param_variant:ident) => {
        fn $fn_name(
            cfg: &BenchConfig,
            wk: &WorkloadAny,
            params: &ParamSet,
            accuracy: &AccuracyCfg,
        ) -> BenchReport {
            let p = match params {
                ParamSet::$param_variant(p) => *p,
                _ => panic!("dispatch::{} wrong family: {:?}", $impl, params.family()),
            };
            match (wk.to_string_wk(), accuracy.enabled) {
                (StringWk::FromUniform(w), true) => bench_freq_gt::<$wrapper, _>(
                    cfg,
                    &w,
                    $family,
                    $impl,
                    || <$wrapper>::new(&p),
                    accuracy.max_probes,
                ),
                (StringWk::FromZipf(w), true) => bench_freq_gt::<$wrapper, _>(
                    cfg,
                    &w,
                    $family,
                    $impl,
                    || <$wrapper>::new(&p),
                    accuracy.max_probes,
                ),
                (StringWk::FromUniform(w), false) => {
                    bench_no_gt::<$wrapper, _>(cfg, &w, $family, $impl, || <$wrapper>::new(&p))
                }
                (StringWk::FromZipf(w), false) => {
                    bench_no_gt::<$wrapper, _>(cfg, &w, $family, $impl, || <$wrapper>::new(&p))
                }
            }
        }
    };
}

macro_rules! run_bytes_freq {
    ($fn_name:ident, $wrapper:ty, $family:expr, $impl:expr, $param_variant:ident) => {
        fn $fn_name(
            cfg: &BenchConfig,
            wk: &WorkloadAny,
            params: &ParamSet,
            accuracy: &AccuracyCfg,
        ) -> BenchReport {
            let p = match params {
                ParamSet::$param_variant(p) => *p,
                _ => panic!("dispatch::{} wrong family: {:?}", $impl, params.family()),
            };
            match (wk.to_bytes_wk(), accuracy.enabled) {
                (BytesWk::FromUniform(w), true) => bench_freq_gt::<$wrapper, _>(
                    cfg,
                    &w,
                    $family,
                    $impl,
                    || <$wrapper>::new(&p),
                    accuracy.max_probes,
                ),
                (BytesWk::FromZipf(w), true) => bench_freq_gt::<$wrapper, _>(
                    cfg,
                    &w,
                    $family,
                    $impl,
                    || <$wrapper>::new(&p),
                    accuracy.max_probes,
                ),
                (BytesWk::FromUniform(w), false) => {
                    bench_no_gt::<$wrapper, _>(cfg, &w, $family, $impl, || <$wrapper>::new(&p))
                }
                (BytesWk::FromZipf(w), false) => {
                    bench_no_gt::<$wrapper, _>(cfg, &w, $family, $impl, || <$wrapper>::new(&p))
                }
            }
        }
    };
}

macro_rules! run_string_none {
    ($fn_name:ident, $wrapper:ty, $family:expr, $impl:expr, $param_variant:ident) => {
        fn $fn_name(
            cfg: &BenchConfig,
            wk: &WorkloadAny,
            params: &ParamSet,
            _accuracy: &AccuracyCfg,
        ) -> BenchReport {
            let p = match params {
                ParamSet::$param_variant(p) => *p,
                _ => panic!("dispatch::{} wrong family: {:?}", $impl, params.family()),
            };
            match wk.to_string_wk() {
                StringWk::FromUniform(w) => {
                    bench_no_gt::<$wrapper, _>(cfg, &w, $family, $impl, || <$wrapper>::new(&p))
                }
                StringWk::FromZipf(w) => {
                    bench_no_gt::<$wrapper, _>(cfg, &w, $family, $impl, || <$wrapper>::new(&p))
                }
            }
        }
    };
}

macro_rules! run_bytes_none {
    ($fn_name:ident, $wrapper:ty, $family:expr, $impl:expr, $param_variant:ident) => {
        fn $fn_name(
            cfg: &BenchConfig,
            wk: &WorkloadAny,
            params: &ParamSet,
            _accuracy: &AccuracyCfg,
        ) -> BenchReport {
            let p = match params {
                ParamSet::$param_variant(p) => *p,
                _ => panic!("dispatch::{} wrong family: {:?}", $impl, params.family()),
            };
            match wk.to_bytes_wk() {
                BytesWk::FromUniform(w) => {
                    bench_no_gt::<$wrapper, _>(cfg, &w, $family, $impl, || <$wrapper>::new(&p))
                }
                BytesWk::FromZipf(w) => {
                    bench_no_gt::<$wrapper, _>(cfg, &w, $family, $impl, || <$wrapper>::new(&p))
                }
            }
        }
    };
}

// -- HLL --
run_i64_card!(run_hll_oxide, hll::HllOxide, "hll", "oxide", Hll);
run_i64_card!(
    run_hll_datasketches,
    hll::HllDatasketches,
    "hll",
    "datasketches",
    Hll
);
run_i64_card!(run_hll_lib, hll::HllLib, "hll", "lib", Hll);
run_i64_card!(run_hll_exact, exact::ExactHll, "hll", "exact", Hll);

// -- KLL --
run_i64_quant!(run_kll_oxide, kll::KllOxide, "kll", "oxide", Kll);
run_i64_quant!(run_kll_lib, kll::KllLib, "kll", "lib", Kll);
run_i64_quant!(run_kll_exact, exact::ExactKll, "kll", "exact", Kll);

// -- CMS --
run_i64_freq!(run_cms_oxide, cms::CmsOxide, "cms", "oxide", Cms);
run_i64_freq!(
    run_cms_datasketches,
    cms::CmsDatasketches,
    "cms",
    "datasketches",
    Cms
);
run_i64_freq!(
    run_cms_lib_fixedmatrix_custom_fast,
    cms::CmsLibFixedmatrixCustomFast,
    "cms",
    "lib-fixedmatrix-custom-fast",
    Cms
);
run_i64_freq!(
    run_cms_lib_fixedmatrix_fast,
    cms::CmsLibFixedmatrixFast,
    "cms",
    "lib-fixedmatrix-fast",
    Cms
);
run_i64_freq!(
    run_cms_lib_vector2d_fast,
    cms::CmsLibVector2dFast,
    "cms",
    "lib-vector2d-fast",
    Cms
);
run_i64_freq!(
    run_cms_lib_vector2d_regular,
    cms::CmsLibVector2dRegular,
    "cms",
    "lib-vector2d-regular",
    Cms
);
run_i64_freq!(run_cms_exact, exact::ExactCms, "cms", "exact", Cms);

// -- CountSketch --
run_i64_freq!(
    run_cs_oxide,
    countsketch::CsOxide,
    "countsketch",
    "oxide",
    Countsketch
);
run_i64_freq!(
    run_cs_lib_fixedmatrix_fast,
    countsketch::CsLibFixedmatrixFast,
    "countsketch",
    "lib-fixedmatrix-fast",
    Countsketch
);
run_i64_freq!(
    run_cs_lib_vector2d_fast,
    countsketch::CsLibVector2dFast,
    "countsketch",
    "lib-vector2d-fast",
    Countsketch
);
run_i64_freq!(
    run_cs_lib_vector2d_regular,
    countsketch::CsLibVector2dRegular,
    "countsketch",
    "lib-vector2d-regular",
    Countsketch
);

// -- Elastic --
run_string_freq!(
    run_elastic_lib,
    elastic::ElasticLib,
    "elastic",
    "lib",
    Elastic
);
run_bytes_freq!(
    run_elastic_oxide,
    elastic::ElasticOxide,
    "elastic",
    "oxide",
    Elastic
);

// -- Nitro --
run_i64_none!(run_nitro_lib, nitro::NitroLib, "nitro", "lib", Nitro);
run_bytes_none!(
    run_nitro_oxide,
    nitro::NitroOxide,
    "nitro",
    "oxide",
    Nitro
);

// -- UnivMon --
run_string_none!(
    run_univmon_lib,
    univmon::UnivMonLib,
    "univmon",
    "lib",
    Univmon
);
run_bytes_none!(
    run_univmon_oxide,
    univmon::UnivMonOxide,
    "univmon",
    "oxide",
    Univmon
);

#[allow(dead_code)]
fn _silence_unused_warnings(_: CmsParams, _: CountSketchParams) {}
