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
use sketch_bench::accuracy::quantile::{RankErrorGT, RelativeErrorGT, ToF64};
use sketch_bench::{BenchConfig, BenchReport, BenchRunner};
use sketch_core::config::{CmsParams, CountSketchParams, ParamSet};
use sketch_core::workload::{BytesFromI64, FileI64, StringFromI64, UniformI64, Workload, ZipfI64};

use crate::wrappers::{
    cms, countsketch, dd, elastic, exact, hll, kll, nitro, parallel, polars, univmon,
};

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
    /// Minimum true count for a key to be included in the
    /// frequency comparator's mean / p99 rel-err. `0` = probe
    /// every distinct key (legacy behaviour). Setting this >0
    /// restricts the metric to heavy hitters, which is the
    /// regime CMS / CountSketch are designed for. Ignored by
    /// cardinality / quantile comparators.
    pub min_true_count: u64,
    /// Ask the comparator to record per-call `(call_index, ns,
    /// estimate, percentile, repeat)` samples — the data the
    /// legacy `throughput/{hll,kll,dd}/rust/src/bin/query.rs`
    /// dump. Only cardinality + quantile comparators honour
    /// it; frequency / top-k ignore. Off by default; the CLI
    /// turns it on when `--raw-csv` is set together with
    /// `--accuracy`.
    pub record_query_calls: bool,
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

#[derive(Debug, Clone)]
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
    File {
        path: String,
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
            WorkloadSpec::File { path } => WorkloadAny::File(
                FileI64::load(std::path::Path::new(&path))
                    .map_err(|e| anyhow::anyhow!("{}", e))?,
            ),
        })
    }
}

pub enum WorkloadAny {
    I64(UniformI64),
    Zipf(ZipfI64),
    File(FileI64),
}

impl WorkloadAny {
    fn to_string_wk(&self) -> StringWk {
        match self {
            WorkloadAny::I64(w) => StringWk::FromUniform(StringFromI64::new(w)),
            WorkloadAny::Zipf(w) => StringWk::FromZipf(StringFromI64::new(w)),
            WorkloadAny::File(w) => StringWk::FromFile(StringFromI64::new(w)),
        }
    }
    fn to_bytes_wk(&self) -> BytesWk {
        match self {
            WorkloadAny::I64(w) => BytesWk::FromUniform(BytesFromI64::new(w)),
            WorkloadAny::Zipf(w) => BytesWk::FromZipf(BytesFromI64::new(w)),
            WorkloadAny::File(w) => BytesWk::FromFile(BytesFromI64::new(w)),
        }
    }
}

enum StringWk {
    FromUniform(StringFromI64<UniformI64>),
    FromZipf(StringFromI64<ZipfI64>),
    FromFile(StringFromI64<FileI64>),
}
enum BytesWk {
    FromUniform(BytesFromI64<UniformI64>),
    FromZipf(BytesFromI64<ZipfI64>),
    FromFile(BytesFromI64<FileI64>),
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
    ) -> Vec<BenchReport>,
}

impl ImplEntry {
    /// Run the bench. Returns one `BenchReport` per metric pass —
    /// see [`sketch_bench::MetricsMask::passes`]. Callers iterate
    /// the Vec and emit each report on its own JSONL line / CSV
    /// row group.
    pub fn run(
        &self,
        cfg: &BenchConfig,
        wk: &WorkloadAny,
        params: &ParamSet,
        accuracy: &AccuracyCfg,
    ) -> Vec<BenchReport> {
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
        description: "asap_sketchlib::HyperLogLogHIP (P14)",
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
    ImplEntry {
        family: "hll",
        impl_name: "polars",
        description: "polars exact: DataFrame.n_unique() (DataFrame baseline)",
        constraint: Constraint::Unparameterized,
        accuracy_kind: AccuracyKind::Cardinality,
        run: run_hll_polars,
    },
    ImplEntry {
        family: "hll",
        impl_name: "lib-fastpath-parallel",
        description: "asap_sketchlib HLL ErtlMLE, FastPath, parallel insert (workers from --workers)",
        constraint: Constraint::Tunable,
        accuracy_kind: AccuracyKind::None,
        run: run_hll_lib_fastpath_parallel,
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
    ImplEntry {
        family: "kll",
        impl_name: "polars",
        description: "polars exact: 101-point quantile grid via DataFrame (DataFrame baseline)",
        constraint: Constraint::Unparameterized,
        accuracy_kind: AccuracyKind::Quantile,
        run: run_kll_polars,
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
        impl_name: "lib-fixedmatrix-fast-32k",
        description: "asap_sketchlib CMS, FixedMatrix (5x32768), FastPath",
        constraint: Constraint::FixedCms {
            rows: cms::CMS_FIXED_32K_ROWS,
            cols: cms::CMS_FIXED_32K_COLS,
        },
        accuracy_kind: AccuracyKind::Frequency,
        run: run_cms_lib_fixedmatrix_fast_32k,
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
    ImplEntry {
        family: "cms",
        impl_name: "polars",
        description: "polars exact: group_by(v).agg(len) → HashMap (DataFrame baseline)",
        constraint: Constraint::Unparameterized,
        accuracy_kind: AccuracyKind::Frequency,
        run: run_cms_polars,
    },
    ImplEntry {
        family: "cms",
        impl_name: "lib-fastpath-parallel",
        description: "asap_sketchlib CMS, FastPath, parallel insert on M5x32K (workers from --workers)",
        constraint: Constraint::Tunable,
        accuracy_kind: AccuracyKind::None,
        run: run_cms_lib_fastpath_parallel,
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
        impl_name: "lib-fixedmatrix-fast-32k",
        description: "asap_sketchlib Count, FixedMatrix (5x32768), FastPath",
        constraint: Constraint::FixedCountSketch {
            rows: cms::CMS_FIXED_32K_ROWS,
            cols: cms::CMS_FIXED_32K_COLS,
        },
        accuracy_kind: AccuracyKind::Frequency,
        run: run_cs_lib_fixedmatrix_fast_32k,
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
    ImplEntry {
        family: "countsketch",
        impl_name: "exact",
        description: "exact baseline: HashMap<i64,u64>, freq = map.get(k)",
        constraint: Constraint::Unparameterized,
        accuracy_kind: AccuracyKind::Frequency,
        run: run_cs_exact,
    },
    ImplEntry {
        family: "countsketch",
        impl_name: "polars",
        description: "polars exact: group_by(v).agg(len) → HashMap (DataFrame baseline)",
        constraint: Constraint::Unparameterized,
        accuracy_kind: AccuracyKind::Frequency,
        run: run_cs_polars,
    },
    ImplEntry {
        family: "countsketch",
        impl_name: "lib-fastpath-parallel",
        description: "asap_sketchlib Count, FastPath, parallel insert on M5x32K (workers from --workers)",
        constraint: Constraint::Tunable,
        accuracy_kind: AccuracyKind::None,
        run: run_cs_lib_fastpath_parallel,
    },
    // -------- DDSketch --------
    ImplEntry {
        family: "dd",
        impl_name: "lib",
        description: "asap_sketchlib::DDSketch (relative-error quantile)",
        constraint: Constraint::Tunable,
        accuracy_kind: AccuracyKind::Quantile,
        run: run_dd_lib,
    },
    ImplEntry {
        family: "dd",
        impl_name: "exact",
        description: "exact baseline: Vec<i64> sorted, quantile = Type-7 lookup",
        constraint: Constraint::Unparameterized,
        accuracy_kind: AccuracyKind::Quantile,
        run: run_dd_exact,
    },
    ImplEntry {
        family: "dd",
        impl_name: "polars",
        description: "polars exact: 101-point quantile grid via DataFrame (DataFrame baseline)",
        constraint: Constraint::Unparameterized,
        accuracy_kind: AccuracyKind::Quantile,
        run: run_dd_polars,
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
) -> Vec<BenchReport>
where
    W: Workload,
    W::Item: Clone,
    S: sketch_core::sketch::Sketch<Item = W::Item>,
{
    let runner = BenchRunner::new(cfg.clone(), wk, family, impl_name);
    // Throughput-only short-circuit: pass an `insert` closure
    // defined here in sketch-cli so the wrapper's `update` body
    // (also in sketch-cli) is in the same crate as the closure
    // body at codegen time.
    if cfg.metrics == sketch_bench::MetricsMask::THROUGHPUT {
        return vec![runner.run_throughput(factory, |s, it| s.update(it))];
    }
    runner.run::<S, _, sketch_bench::NoGT>(factory, None)
}

fn bench_freq_gt<S, W>(
    cfg: &BenchConfig,
    wk: &W,
    family: &str,
    impl_name: &str,
    factory: impl FnMut() -> S,
    max_probes: usize,
    min_true_count: u64,
) -> Vec<BenchReport>
where
    W: Workload,
    W::Item: Clone + Eq + Hash,
    S: sketch_core::sketch::Sketch<Item = W::Item, Query = W::Item, Answer = u64>,
{
    let keys_to_probe = sample_distinct(wk.items(), max_probes);
    let gt = FrequencyGT {
        keys_to_probe,
        min_true_count,
    };
    BenchRunner::new(cfg.clone(), wk, family, impl_name)
        .run::<S, _, FrequencyGT<W::Item>>(factory, Some(&gt))
}

fn bench_card_gt<S, W>(
    cfg: &BenchConfig,
    wk: &W,
    family: &str,
    impl_name: &str,
    factory: impl FnMut() -> S,
    record_calls: bool,
) -> Vec<BenchReport>
where
    W: Workload,
    W::Item: Clone + Eq + Hash,
    S: sketch_core::sketch::Sketch<Item = W::Item, Query = (), Answer = f64>,
{
    let gt = CardinalityGT { record_calls };
    BenchRunner::new(cfg.clone(), wk, family, impl_name)
        .run::<S, _, CardinalityGT>(factory, Some(&gt))
}

fn bench_quant_gt<S, W>(
    cfg: &BenchConfig,
    wk: &W,
    family: &str,
    impl_name: &str,
    factory: impl FnMut() -> S,
    record_calls: bool,
) -> Vec<BenchReport>
where
    W: Workload,
    W::Item: Clone + PartialOrd + ToF64,
    S: sketch_core::sketch::Sketch<Item = W::Item, Query = f64, Answer = f64>,
{
    let gt = RankErrorGT { record_calls };
    BenchRunner::new(cfg.clone(), wk, family, impl_name)
        .run::<S, _, RankErrorGT>(factory, Some(&gt))
}

fn bench_quant_rel_gt<S, W>(
    cfg: &BenchConfig,
    wk: &W,
    family: &str,
    impl_name: &str,
    factory: impl FnMut() -> S,
    record_calls: bool,
) -> Vec<BenchReport>
where
    W: Workload,
    W::Item: Clone + PartialOrd + ToF64,
    S: sketch_core::sketch::Sketch<Item = W::Item, Query = f64, Answer = f64>,
{
    let gt = RelativeErrorGT { record_calls };
    BenchRunner::new(cfg.clone(), wk, family, impl_name)
        .run::<S, _, RelativeErrorGT>(factory, Some(&gt))
}

/// Collect distinct keys from `items` and return them in a
/// **uniformly shuffled** order, capped at `max_probes` (`0` =
/// no cap). The shuffle defeats the cache locality the encounter
/// order would have given the exact-baseline HashMap probe (under
/// Zipf the heavy hitters appear first, so probing in encounter
/// order keeps them L1-resident); shuffling models a uniform
/// random query workload over the distinct-key set, which is
/// what an offline accuracy bench should be measuring.
///
/// Seeded with a fixed value so the probe order is reproducible
/// across runs of the same workload.
fn sample_distinct<K>(items: &[K], max_probes: usize) -> Vec<K>
where
    K: Clone + Eq + Hash,
{
    use rand::seq::SliceRandom;
    use rand::SeedableRng;

    let mut seen: HashSet<K> = HashSet::new();
    let mut out: Vec<K> = Vec::new();
    for it in items {
        if seen.insert(it.clone()) {
            out.push(it.clone());
        }
    }
    let mut rng = rand_xoshiro::Xoshiro256PlusPlus::seed_from_u64(0xA5AC_F00D_5EED_BEEF);
    out.shuffle(&mut rng);
    if max_probes != 0 && out.len() > max_probes {
        out.truncate(max_probes);
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
        ) -> Vec<BenchReport> {
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
                    accuracy.max_probes, accuracy.min_true_count,
                ),
                (WorkloadAny::Zipf(w), true) => bench_freq_gt::<$wrapper, _>(
                    cfg,
                    w,
                    $family,
                    $impl,
                    || <$wrapper>::new(&p),
                    accuracy.max_probes, accuracy.min_true_count,
                ),
                (WorkloadAny::File(w), true) => bench_freq_gt::<$wrapper, _>(
                    cfg,
                    w,
                    $family,
                    $impl,
                    || <$wrapper>::new(&p),
                    accuracy.max_probes, accuracy.min_true_count,
                ),
                (WorkloadAny::I64(w), false) => {
                    bench_no_gt::<$wrapper, _>(cfg, w, $family, $impl, || <$wrapper>::new(&p))
                }
                (WorkloadAny::Zipf(w), false) => {
                    bench_no_gt::<$wrapper, _>(cfg, w, $family, $impl, || <$wrapper>::new(&p))
                }
                (WorkloadAny::File(w), false) => {
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
        ) -> Vec<BenchReport> {
            let p = match params {
                ParamSet::$param_variant(p) => *p,
                _ => panic!("dispatch::{} wrong family: {:?}", $impl, params.family()),
            };
            match (wk, accuracy.enabled) {
                (WorkloadAny::I64(w), true) => bench_card_gt::<$wrapper, _>(
                    cfg, w, $family, $impl, || <$wrapper>::new(&p),
                    accuracy.record_query_calls,
                ),
                (WorkloadAny::Zipf(w), true) => bench_card_gt::<$wrapper, _>(
                    cfg, w, $family, $impl, || <$wrapper>::new(&p),
                    accuracy.record_query_calls,
                ),
                (WorkloadAny::File(w), true) => bench_card_gt::<$wrapper, _>(
                    cfg, w, $family, $impl, || <$wrapper>::new(&p),
                    accuracy.record_query_calls,
                ),
                (WorkloadAny::I64(w), false) => {
                    bench_no_gt::<$wrapper, _>(cfg, w, $family, $impl, || <$wrapper>::new(&p))
                }
                (WorkloadAny::Zipf(w), false) => {
                    bench_no_gt::<$wrapper, _>(cfg, w, $family, $impl, || <$wrapper>::new(&p))
                }
                (WorkloadAny::File(w), false) => {
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
        ) -> Vec<BenchReport> {
            let p = match params {
                ParamSet::$param_variant(p) => *p,
                _ => panic!("dispatch::{} wrong family: {:?}", $impl, params.family()),
            };
            match (wk, accuracy.enabled) {
                (WorkloadAny::I64(w), true) => bench_quant_gt::<$wrapper, _>(
                    cfg, w, $family, $impl, || <$wrapper>::new(&p),
                    accuracy.record_query_calls,
                ),
                (WorkloadAny::Zipf(w), true) => bench_quant_gt::<$wrapper, _>(
                    cfg, w, $family, $impl, || <$wrapper>::new(&p),
                    accuracy.record_query_calls,
                ),
                (WorkloadAny::File(w), true) => bench_quant_gt::<$wrapper, _>(
                    cfg, w, $family, $impl, || <$wrapper>::new(&p),
                    accuracy.record_query_calls,
                ),
                (WorkloadAny::I64(w), false) => {
                    bench_no_gt::<$wrapper, _>(cfg, w, $family, $impl, || <$wrapper>::new(&p))
                }
                (WorkloadAny::Zipf(w), false) => {
                    bench_no_gt::<$wrapper, _>(cfg, w, $family, $impl, || <$wrapper>::new(&p))
                }
                (WorkloadAny::File(w), false) => {
                    bench_no_gt::<$wrapper, _>(cfg, w, $family, $impl, || <$wrapper>::new(&p))
                }
            }
        }
    };
}

macro_rules! run_i64_quant_rel {
    ($fn_name:ident, $wrapper:ty, $family:expr, $impl:expr, $param_variant:ident) => {
        fn $fn_name(
            cfg: &BenchConfig,
            wk: &WorkloadAny,
            params: &ParamSet,
            accuracy: &AccuracyCfg,
        ) -> Vec<BenchReport> {
            let p = match params {
                ParamSet::$param_variant(p) => *p,
                _ => panic!("dispatch::{} wrong family: {:?}", $impl, params.family()),
            };
            match (wk, accuracy.enabled) {
                (WorkloadAny::I64(w), true) => bench_quant_rel_gt::<$wrapper, _>(
                    cfg, w, $family, $impl, || <$wrapper>::new(&p),
                    accuracy.record_query_calls,
                ),
                (WorkloadAny::Zipf(w), true) => bench_quant_rel_gt::<$wrapper, _>(
                    cfg, w, $family, $impl, || <$wrapper>::new(&p),
                    accuracy.record_query_calls,
                ),
                (WorkloadAny::File(w), true) => bench_quant_rel_gt::<$wrapper, _>(
                    cfg, w, $family, $impl, || <$wrapper>::new(&p),
                    accuracy.record_query_calls,
                ),
                (WorkloadAny::I64(w), false) => {
                    bench_no_gt::<$wrapper, _>(cfg, w, $family, $impl, || <$wrapper>::new(&p))
                }
                (WorkloadAny::Zipf(w), false) => {
                    bench_no_gt::<$wrapper, _>(cfg, w, $family, $impl, || <$wrapper>::new(&p))
                }
                (WorkloadAny::File(w), false) => {
                    bench_no_gt::<$wrapper, _>(cfg, w, $family, $impl, || <$wrapper>::new(&p))
                }
            }
        }
    };
}

/// Parallel-insert impls (`lib-fastpath-parallel` under cms /
/// countsketch / hll). Threads come from `cfg.threads` (driven
/// by `--workers N`); accuracy is None because the partitions
/// are intentionally not merged (matches legacy octo).
macro_rules! run_i64_parallel {
    ($fn_name:ident, $wrapper:ty, $family:expr, $impl:expr, $param_variant:ident) => {
        fn $fn_name(
            cfg: &BenchConfig,
            wk: &WorkloadAny,
            params: &ParamSet,
            _accuracy: &AccuracyCfg,
        ) -> Vec<BenchReport> {
            let p = match params {
                ParamSet::$param_variant(p) => *p,
                _ => panic!("dispatch::{} wrong family: {:?}", $impl, params.family()),
            };
            let workers = cfg.threads;
            match wk {
                WorkloadAny::I64(w) => bench_no_gt::<$wrapper, _>(
                    cfg, w, $family, $impl, move || <$wrapper>::new(&p, workers),
                ),
                WorkloadAny::Zipf(w) => bench_no_gt::<$wrapper, _>(
                    cfg, w, $family, $impl, move || <$wrapper>::new(&p, workers),
                ),
                WorkloadAny::File(w) => bench_no_gt::<$wrapper, _>(
                    cfg, w, $family, $impl, move || <$wrapper>::new(&p, workers),
                ),
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
        ) -> Vec<BenchReport> {
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
                WorkloadAny::File(w) => {
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
        ) -> Vec<BenchReport> {
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
                    accuracy.max_probes, accuracy.min_true_count,
                ),
                (StringWk::FromZipf(w), true) => bench_freq_gt::<$wrapper, _>(
                    cfg,
                    &w,
                    $family,
                    $impl,
                    || <$wrapper>::new(&p),
                    accuracy.max_probes, accuracy.min_true_count,
                ),
                (StringWk::FromFile(w), true) => bench_freq_gt::<$wrapper, _>(
                    cfg,
                    &w,
                    $family,
                    $impl,
                    || <$wrapper>::new(&p),
                    accuracy.max_probes, accuracy.min_true_count,
                ),
                (StringWk::FromUniform(w), false) => {
                    bench_no_gt::<$wrapper, _>(cfg, &w, $family, $impl, || <$wrapper>::new(&p))
                }
                (StringWk::FromZipf(w), false) => {
                    bench_no_gt::<$wrapper, _>(cfg, &w, $family, $impl, || <$wrapper>::new(&p))
                }
                (StringWk::FromFile(w), false) => {
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
        ) -> Vec<BenchReport> {
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
                    accuracy.max_probes, accuracy.min_true_count,
                ),
                (BytesWk::FromZipf(w), true) => bench_freq_gt::<$wrapper, _>(
                    cfg,
                    &w,
                    $family,
                    $impl,
                    || <$wrapper>::new(&p),
                    accuracy.max_probes, accuracy.min_true_count,
                ),
                (BytesWk::FromFile(w), true) => bench_freq_gt::<$wrapper, _>(
                    cfg,
                    &w,
                    $family,
                    $impl,
                    || <$wrapper>::new(&p),
                    accuracy.max_probes, accuracy.min_true_count,
                ),
                (BytesWk::FromUniform(w), false) => {
                    bench_no_gt::<$wrapper, _>(cfg, &w, $family, $impl, || <$wrapper>::new(&p))
                }
                (BytesWk::FromZipf(w), false) => {
                    bench_no_gt::<$wrapper, _>(cfg, &w, $family, $impl, || <$wrapper>::new(&p))
                }
                (BytesWk::FromFile(w), false) => {
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
        ) -> Vec<BenchReport> {
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
                StringWk::FromFile(w) => {
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
        ) -> Vec<BenchReport> {
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
                BytesWk::FromFile(w) => {
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
run_i64_card!(run_hll_exact, exact::ExactCardinality, "hll", "exact", Hll);

// -- KLL --
run_i64_quant!(run_kll_oxide, kll::KllOxide, "kll", "oxide", Kll);
run_i64_quant!(run_kll_lib, kll::KllLib, "kll", "lib", Kll);
run_i64_quant!(run_kll_exact, exact::ExactQuantile, "kll", "exact", Kll);

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
    run_cms_lib_fixedmatrix_fast_32k,
    cms::CmsLibFixedmatrixFast32k,
    "cms",
    "lib-fixedmatrix-fast-32k",
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
run_i64_freq!(run_cms_exact, exact::ExactFrequency, "cms", "exact", Cms);

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
    run_cs_lib_fixedmatrix_fast_32k,
    countsketch::CsLibFixedmatrixFast32k,
    "countsketch",
    "lib-fixedmatrix-fast-32k",
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

run_i64_freq!(
    run_cs_exact,
    exact::ExactFrequencyCs,
    "countsketch",
    "exact",
    Countsketch
);

// -- DDSketch --
run_i64_quant_rel!(run_dd_lib, dd::DdLib, "dd", "lib", Dd);
run_i64_quant_rel!(run_dd_exact, exact::ExactQuantileDd, "dd", "exact", Dd);

// -- Polars-backed baselines (one per family). Same `Sketch`
//    contract as the in-tree exact baselines; the legacy
//    `throughput/polars_*/` binaries are folded into these.
run_i64_card!(run_hll_polars, polars::PolarsCardinality, "hll", "polars", Hll);
run_i64_quant!(run_kll_polars, polars::PolarsQuantileKll, "kll", "polars", Kll);
run_i64_quant_rel!(run_dd_polars, polars::PolarsQuantileDd, "dd", "polars", Dd);
run_i64_freq!(run_cms_polars, polars::PolarsFrequencyCms, "cms", "polars", Cms);
run_i64_freq!(
    run_cs_polars,
    polars::PolarsFrequencyCs,
    "countsketch",
    "polars",
    Countsketch
);

// -- Parallel-insert FastPath baselines (one per family). Workers
//    are read from `cfg.threads` (= `--workers N`). Folds the
//    legacy `throughput/octo/` binary into sketch-cli.
run_i64_parallel!(
    run_cms_lib_fastpath_parallel,
    parallel::ParallelCmsFastPath,
    "cms",
    "lib-fastpath-parallel",
    Cms
);
run_i64_parallel!(
    run_cs_lib_fastpath_parallel,
    parallel::ParallelCsFastPath,
    "countsketch",
    "lib-fastpath-parallel",
    Countsketch
);
run_i64_parallel!(
    run_hll_lib_fastpath_parallel,
    parallel::ParallelHllFastPath,
    "hll",
    "lib-fastpath-parallel",
    Hll
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
