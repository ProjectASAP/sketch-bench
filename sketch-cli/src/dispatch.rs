//! Dispatch table: a `(family, impl)` pair picks one of the
//! 21 wrapped sketches and monomorphises a `BenchRunner` call
//! over the matching `Sketch` trait.
//!
//! Each row's `run` fn accepts a typed `&ParamSet` + an optional
//! accuracy config so the CLI can sweep a grid of configs and
//! optionally compute per-family ground-truth accuracy through
//! the same table.

use std::hash::Hash;

use anyhow::Result;
use sketch_bench::accuracy::cardinality::CardinalityGT;
use sketch_bench::accuracy::frequency::FrequencyGT;
use sketch_bench::accuracy::quantile::{RankErrorGT, RelativeErrorGT, ToF64};
use sketch_bench::{BenchConfig, BenchReport, BenchRunner};
use sketch_core::config::{
    CmsParams, CountSketchParams, DdParams, ElasticParams, HllParams, KllParams, NitroParams,
    ParamSet, SketchParams, UnivMonParams,
};
use sketch_core::datagen::GenSpec;
use sketch_core::workload::{BytesWorkload, I64Workload, StringWorkload, Workload};

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

/// Where a benchmark's items come from: generated in-process by
/// `datagen`, or loaded from a file.
///
/// There is exactly one generator in the tool — `sketchlib workload
/// generate` and `sketchlib bench` drive the same `GenSpec` through the
/// same samplers, differing only in the [`Sink`](sketch_core::datagen::Sink)
/// they push into (file vs memory). A shape reachable from one is
/// reachable from the other by construction.
#[derive(Debug, Clone)]
pub enum WorkloadSpec {
    Generated(GenSpec),
    File { path: String },
}

impl WorkloadSpec {
    pub fn build_i64(self) -> Result<I64Workload> {
        match self {
            WorkloadSpec::Generated(spec) => {
                I64Workload::generate(&spec).map_err(|e| anyhow::anyhow!("{}", e))
            }
            WorkloadSpec::File { path } => {
                I64Workload::load(std::path::Path::new(&path)).map_err(|e| anyhow::anyhow!("{}", e))
            }
        }
    }
}

/// Constraint that a given impl places on the `ParamSet` it
/// will accept. Most impls honour whatever the caller passes
/// (`Tunable`); a handful have compile-time-fixed internal
/// shapes (`Fixed`) and can only run when the requested params
/// exactly match.
#[derive(Debug, Clone, Copy)]
pub enum Constraint {
    Tunable,
    FixedCms {
        rows: usize,
        cols: usize,
    },
    FixedCountSketch {
        rows: usize,
        cols: usize,
    },
    /// Exact baselines — they ignore the family's `ParamSet`. The
    /// sweep driver runs them at most once per invocation instead
    /// of once per config.
    Unparameterized,
}

impl Constraint {
    pub fn accepts(&self, params: &ParamSet) -> bool {
        match self {
            Constraint::Tunable | Constraint::Unparameterized => true,
            // Compile-time-fixed matrix shapes: the row can only run when the
            // requested grid point happens to be its shape. Both families
            // carry `rows`/`cols`, so one parse covers them — the enum-variant
            // match this replaced needed one arm per family and silently
            // returned `false` for any family it had not been taught about.
            Constraint::FixedCms { rows, cols } => params
                .parse::<CmsParams>()
                .is_ok_and(|p| p.rows == *rows && p.cols == *cols),
            Constraint::FixedCountSketch { rows, cols } => params
                .parse::<CountSketchParams>()
                .is_ok_and(|p| p.rows == *rows && p.cols == *cols),
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
    /// The family's sweep grid when `--config` is omitted, supplied by the
    /// row's params type. Previously a `match family` table in `sweep.rs`
    /// that nothing tied to the type it configured.
    pub default_grid: fn() -> Vec<ParamSet>,
    /// Type-check a grid point against this row's params type, without
    /// building anything. `--config` is user input, so a misspelled key must
    /// produce an error naming it — not the panic `params_of!` raises for a
    /// dispatch-table wiring bug. The CLI validates the whole grid up front.
    pub validate: fn(&ParamSet) -> Result<(), String>,
    run: fn(
        cfg: &BenchConfig,
        wk: &I64Workload,
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
        wk: &I64Workload,
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
        default_grid: || HllParams::default_grid().iter().map(ParamSet::of).collect(),
        validate: |p| {
            p.parse::<HllParams>()
                .map(|_| ())
                .map_err(|e| e.to_string())
        },
        run: run_hll_oxide,
    },
    ImplEntry {
        family: "hll",
        impl_name: "datasketches",
        description: "datasketches::hll::HllSketch (Hll8)",
        constraint: Constraint::Tunable,
        accuracy_kind: AccuracyKind::Cardinality,
        default_grid: || HllParams::default_grid().iter().map(ParamSet::of).collect(),
        validate: |p| {
            p.parse::<HllParams>()
                .map(|_| ())
                .map_err(|e| e.to_string())
        },
        run: run_hll_datasketches,
    },
    ImplEntry {
        family: "hll",
        impl_name: "lib",
        description: "asap_sketchlib::HyperLogLog<Classic> (P14): O(m) estimate",
        constraint: Constraint::Tunable,
        accuracy_kind: AccuracyKind::Cardinality,
        default_grid: || HllParams::default_grid().iter().map(ParamSet::of).collect(),
        validate: |p| {
            p.parse::<HllParams>()
                .map(|_| ())
                .map_err(|e| e.to_string())
        },
        run: run_hll_lib,
    },
    ImplEntry {
        family: "hll",
        impl_name: "lib-hip",
        description: "asap_sketchlib::HyperLogLogHIP (P14): O(1) estimate, slightly slower insert",
        constraint: Constraint::Tunable,
        accuracy_kind: AccuracyKind::Cardinality,
        default_grid: || HllParams::default_grid().iter().map(ParamSet::of).collect(),
        validate: |p| {
            p.parse::<HllParams>()
                .map(|_| ())
                .map_err(|e| e.to_string())
        },
        run: run_hll_lib_hip,
    },
    ImplEntry {
        family: "hll",
        impl_name: "exact",
        description: "exact baseline: HashSet<i64>, cardinality = set.len()",
        constraint: Constraint::Unparameterized,
        accuracy_kind: AccuracyKind::Cardinality,
        default_grid: || HllParams::default_grid().iter().map(ParamSet::of).collect(),
        validate: |p| {
            p.parse::<HllParams>()
                .map(|_| ())
                .map_err(|e| e.to_string())
        },
        run: run_hll_exact,
    },
    ImplEntry {
        family: "hll",
        impl_name: "null",
        description: "null baseline: cardinality estimate is 0 — pins relative error = 1.0",
        constraint: Constraint::Unparameterized,
        accuracy_kind: AccuracyKind::Cardinality,
        default_grid: || HllParams::default_grid().iter().map(ParamSet::of).collect(),
        validate: |p| {
            p.parse::<HllParams>()
                .map(|_| ())
                .map_err(|e| e.to_string())
        },
        run: run_hll_null,
    },
    ImplEntry {
        family: "hll",
        impl_name: "polars",
        description: "polars exact: DataFrame.n_unique() (DataFrame baseline)",
        constraint: Constraint::Unparameterized,
        accuracy_kind: AccuracyKind::Cardinality,
        default_grid: || HllParams::default_grid().iter().map(ParamSet::of).collect(),
        validate: |p| {
            p.parse::<HllParams>()
                .map(|_| ())
                .map_err(|e| e.to_string())
        },
        run: run_hll_polars,
    },
    ImplEntry {
        family: "hll",
        impl_name: "lib-fastpath-parallel",
        description:
            "asap_sketchlib HLL ErtlMLE, FastPath, parallel insert (workers from --workers)",
        constraint: Constraint::Tunable,
        accuracy_kind: AccuracyKind::None,
        default_grid: || HllParams::default_grid().iter().map(ParamSet::of).collect(),
        validate: |p| {
            p.parse::<HllParams>()
                .map(|_| ())
                .map_err(|e| e.to_string())
        },
        run: run_hll_lib_fastpath_parallel,
    },
    // -------- KLL --------
    ImplEntry {
        family: "kll",
        impl_name: "oxide",
        description: "sketch_oxide::quantiles::KllSketch",
        constraint: Constraint::Tunable,
        accuracy_kind: AccuracyKind::Quantile,
        default_grid: || KllParams::default_grid().iter().map(ParamSet::of).collect(),
        validate: |p| {
            p.parse::<KllParams>()
                .map(|_| ())
                .map_err(|e| e.to_string())
        },
        run: run_kll_oxide,
    },
    ImplEntry {
        family: "kll",
        impl_name: "lib",
        description: "asap_sketchlib::KLL<i64>",
        constraint: Constraint::Tunable,
        accuracy_kind: AccuracyKind::Quantile,
        default_grid: || KllParams::default_grid().iter().map(ParamSet::of).collect(),
        validate: |p| {
            p.parse::<KllParams>()
                .map(|_| ())
                .map_err(|e| e.to_string())
        },
        run: run_kll_lib,
    },
    ImplEntry {
        family: "kll",
        impl_name: "exact",
        description: "exact baseline: Vec<i64> sorted, quantile = index lookup",
        constraint: Constraint::Unparameterized,
        accuracy_kind: AccuracyKind::Quantile,
        default_grid: || KllParams::default_grid().iter().map(ParamSet::of).collect(),
        validate: |p| {
            p.parse::<KllParams>()
                .map(|_| ())
                .map_err(|e| e.to_string())
        },
        run: run_kll_exact,
    },
    ImplEntry {
        family: "kll",
        impl_name: "polars",
        description: "polars exact: 101-point quantile grid via DataFrame (DataFrame baseline)",
        constraint: Constraint::Unparameterized,
        accuracy_kind: AccuracyKind::Quantile,
        default_grid: || KllParams::default_grid().iter().map(ParamSet::of).collect(),
        validate: |p| {
            p.parse::<KllParams>()
                .map(|_| ())
                .map_err(|e| e.to_string())
        },
        run: run_kll_polars,
    },
    // -------- CMS --------
    ImplEntry {
        family: "cms",
        impl_name: "oxide",
        description: "sketch_oxide::frequency::CountMinSketch",
        constraint: Constraint::Tunable,
        accuracy_kind: AccuracyKind::Frequency,
        default_grid: || CmsParams::default_grid().iter().map(ParamSet::of).collect(),
        validate: |p| {
            p.parse::<CmsParams>()
                .map(|_| ())
                .map_err(|e| e.to_string())
        },
        run: run_cms_oxide,
    },
    ImplEntry {
        family: "cms",
        impl_name: "datasketches",
        description: "datasketches::countmin::CountMinSketch",
        constraint: Constraint::Tunable,
        accuracy_kind: AccuracyKind::Frequency,
        default_grid: || CmsParams::default_grid().iter().map(ParamSet::of).collect(),
        validate: |p| {
            p.parse::<CmsParams>()
                .map(|_| ())
                .map_err(|e| e.to_string())
        },
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
        default_grid: || CmsParams::default_grid().iter().map(ParamSet::of).collect(),
        validate: |p| {
            p.parse::<CmsParams>()
                .map(|_| ())
                .map_err(|e| e.to_string())
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
        accuracy_kind: AccuracyKind::Frequency,
        default_grid: || CmsParams::default_grid().iter().map(ParamSet::of).collect(),
        validate: |p| {
            p.parse::<CmsParams>()
                .map(|_| ())
                .map_err(|e| e.to_string())
        },
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
        default_grid: || CmsParams::default_grid().iter().map(ParamSet::of).collect(),
        validate: |p| {
            p.parse::<CmsParams>()
                .map(|_| ())
                .map_err(|e| e.to_string())
        },
        run: run_cms_lib_fixedmatrix_fast_32k,
    },
    ImplEntry {
        family: "cms",
        impl_name: "lib-vector2d-fast",
        description: "asap_sketchlib CMS, Vector2D, FastPath",
        constraint: Constraint::Tunable,
        accuracy_kind: AccuracyKind::Frequency,
        default_grid: || CmsParams::default_grid().iter().map(ParamSet::of).collect(),
        validate: |p| {
            p.parse::<CmsParams>()
                .map(|_| ())
                .map_err(|e| e.to_string())
        },
        run: run_cms_lib_vector2d_fast,
    },
    ImplEntry {
        family: "cms",
        impl_name: "lib-vector2d-regular",
        description: "asap_sketchlib CMS, Vector2D, RegularPath",
        constraint: Constraint::Tunable,
        accuracy_kind: AccuracyKind::Frequency,
        default_grid: || CmsParams::default_grid().iter().map(ParamSet::of).collect(),
        validate: |p| {
            p.parse::<CmsParams>()
                .map(|_| ())
                .map_err(|e| e.to_string())
        },
        run: run_cms_lib_vector2d_regular,
    },
    ImplEntry {
        family: "cms",
        impl_name: "exact",
        description: "exact baseline: HashMap<i64,u64>, freq = map.get(k)",
        constraint: Constraint::Unparameterized,
        accuracy_kind: AccuracyKind::Frequency,
        default_grid: || CmsParams::default_grid().iter().map(ParamSet::of).collect(),
        validate: |p| {
            p.parse::<CmsParams>()
                .map(|_| ())
                .map_err(|e| e.to_string())
        },
        run: run_cms_exact,
    },
    ImplEntry {
        family: "cms",
        impl_name: "null",
        description:
            "null baseline: every count is 0 — pins ARE = 1.0, the disqualifying threshold",
        constraint: Constraint::Unparameterized,
        accuracy_kind: AccuracyKind::Frequency,
        default_grid: || CmsParams::default_grid().iter().map(ParamSet::of).collect(),
        validate: |p| {
            p.parse::<CmsParams>()
                .map(|_| ())
                .map_err(|e| e.to_string())
        },
        run: run_cms_null,
    },
    ImplEntry {
        family: "cms",
        impl_name: "polars",
        description: "polars exact: group_by(v).agg(len) → HashMap (DataFrame baseline)",
        constraint: Constraint::Unparameterized,
        accuracy_kind: AccuracyKind::Frequency,
        default_grid: || CmsParams::default_grid().iter().map(ParamSet::of).collect(),
        validate: |p| {
            p.parse::<CmsParams>()
                .map(|_| ())
                .map_err(|e| e.to_string())
        },
        run: run_cms_polars,
    },
    ImplEntry {
        family: "cms",
        impl_name: "lib-fastpath-parallel",
        description:
            "asap_sketchlib CMS, FastPath, parallel insert on M5x32K (workers from --workers)",
        constraint: Constraint::Tunable,
        accuracy_kind: AccuracyKind::None,
        default_grid: || CmsParams::default_grid().iter().map(ParamSet::of).collect(),
        validate: |p| {
            p.parse::<CmsParams>()
                .map(|_| ())
                .map_err(|e| e.to_string())
        },
        run: run_cms_lib_fastpath_parallel,
    },
    // -------- CountSketch --------
    ImplEntry {
        family: "countsketch",
        impl_name: "oxide",
        description: "sketch_oxide::frequency::CountSketch",
        constraint: Constraint::Tunable,
        accuracy_kind: AccuracyKind::Frequency,
        default_grid: || {
            CountSketchParams::default_grid()
                .iter()
                .map(ParamSet::of)
                .collect()
        },
        validate: |p| {
            p.parse::<CountSketchParams>()
                .map(|_| ())
                .map_err(|e| e.to_string())
        },
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
        default_grid: || {
            CountSketchParams::default_grid()
                .iter()
                .map(ParamSet::of)
                .collect()
        },
        validate: |p| {
            p.parse::<CountSketchParams>()
                .map(|_| ())
                .map_err(|e| e.to_string())
        },
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
        default_grid: || {
            CountSketchParams::default_grid()
                .iter()
                .map(ParamSet::of)
                .collect()
        },
        validate: |p| {
            p.parse::<CountSketchParams>()
                .map(|_| ())
                .map_err(|e| e.to_string())
        },
        run: run_cs_lib_fixedmatrix_fast_32k,
    },
    ImplEntry {
        family: "countsketch",
        impl_name: "lib-vector2d-fast",
        description: "asap_sketchlib Count, Vector2D, FastPath",
        constraint: Constraint::Tunable,
        accuracy_kind: AccuracyKind::Frequency,
        default_grid: || {
            CountSketchParams::default_grid()
                .iter()
                .map(ParamSet::of)
                .collect()
        },
        validate: |p| {
            p.parse::<CountSketchParams>()
                .map(|_| ())
                .map_err(|e| e.to_string())
        },
        run: run_cs_lib_vector2d_fast,
    },
    ImplEntry {
        family: "countsketch",
        impl_name: "lib-vector2d-regular",
        description: "asap_sketchlib Count, Vector2D, RegularPath",
        constraint: Constraint::Tunable,
        accuracy_kind: AccuracyKind::Frequency,
        default_grid: || {
            CountSketchParams::default_grid()
                .iter()
                .map(ParamSet::of)
                .collect()
        },
        validate: |p| {
            p.parse::<CountSketchParams>()
                .map(|_| ())
                .map_err(|e| e.to_string())
        },
        run: run_cs_lib_vector2d_regular,
    },
    ImplEntry {
        family: "countsketch",
        impl_name: "exact",
        description: "exact baseline: HashMap<i64,u64>, freq = map.get(k)",
        constraint: Constraint::Unparameterized,
        accuracy_kind: AccuracyKind::Frequency,
        default_grid: || {
            CountSketchParams::default_grid()
                .iter()
                .map(ParamSet::of)
                .collect()
        },
        validate: |p| {
            p.parse::<CountSketchParams>()
                .map(|_| ())
                .map_err(|e| e.to_string())
        },
        run: run_cs_exact,
    },
    ImplEntry {
        family: "countsketch",
        impl_name: "null",
        description:
            "null baseline: every count is 0 — pins ARE = 1.0, the disqualifying threshold",
        constraint: Constraint::Unparameterized,
        accuracy_kind: AccuracyKind::Frequency,
        default_grid: || {
            CountSketchParams::default_grid()
                .iter()
                .map(ParamSet::of)
                .collect()
        },
        validate: |p| {
            p.parse::<CountSketchParams>()
                .map(|_| ())
                .map_err(|e| e.to_string())
        },
        run: run_cs_null,
    },
    ImplEntry {
        family: "countsketch",
        impl_name: "polars",
        description: "polars exact: group_by(v).agg(len) → HashMap (DataFrame baseline)",
        constraint: Constraint::Unparameterized,
        accuracy_kind: AccuracyKind::Frequency,
        default_grid: || {
            CountSketchParams::default_grid()
                .iter()
                .map(ParamSet::of)
                .collect()
        },
        validate: |p| {
            p.parse::<CountSketchParams>()
                .map(|_| ())
                .map_err(|e| e.to_string())
        },
        run: run_cs_polars,
    },
    ImplEntry {
        family: "countsketch",
        impl_name: "lib-fastpath-parallel",
        description:
            "asap_sketchlib Count, FastPath, parallel insert on M5x32K (workers from --workers)",
        constraint: Constraint::Tunable,
        accuracy_kind: AccuracyKind::None,
        default_grid: || {
            CountSketchParams::default_grid()
                .iter()
                .map(ParamSet::of)
                .collect()
        },
        validate: |p| {
            p.parse::<CountSketchParams>()
                .map(|_| ())
                .map_err(|e| e.to_string())
        },
        run: run_cs_lib_fastpath_parallel,
    },
    // -------- DDSketch --------
    ImplEntry {
        family: "dd",
        impl_name: "lib",
        description: "asap_sketchlib::DDSketch (relative-error quantile)",
        constraint: Constraint::Tunable,
        accuracy_kind: AccuracyKind::Quantile,
        default_grid: || DdParams::default_grid().iter().map(ParamSet::of).collect(),
        validate: |p| p.parse::<DdParams>().map(|_| ()).map_err(|e| e.to_string()),
        run: run_dd_lib,
    },
    ImplEntry {
        family: "dd",
        impl_name: "exact",
        description: "exact baseline: Vec<i64> sorted, quantile = Type-7 lookup",
        constraint: Constraint::Unparameterized,
        accuracy_kind: AccuracyKind::Quantile,
        default_grid: || DdParams::default_grid().iter().map(ParamSet::of).collect(),
        validate: |p| p.parse::<DdParams>().map(|_| ()).map_err(|e| e.to_string()),
        run: run_dd_exact,
    },
    ImplEntry {
        family: "dd",
        impl_name: "polars",
        description: "polars exact: 101-point quantile grid via DataFrame (DataFrame baseline)",
        constraint: Constraint::Unparameterized,
        accuracy_kind: AccuracyKind::Quantile,
        default_grid: || DdParams::default_grid().iter().map(ParamSet::of).collect(),
        validate: |p| p.parse::<DdParams>().map(|_| ()).map_err(|e| e.to_string()),
        run: run_dd_polars,
    },
    // -------- Elastic --------
    ImplEntry {
        family: "elastic",
        impl_name: "lib",
        description: "asap_sketchlib::Elastic<DefaultXxHasher>",
        constraint: Constraint::Tunable,
        accuracy_kind: AccuracyKind::Frequency,
        default_grid: || {
            ElasticParams::default_grid()
                .iter()
                .map(ParamSet::of)
                .collect()
        },
        validate: |p| {
            p.parse::<ElasticParams>()
                .map(|_| ())
                .map_err(|e| e.to_string())
        },
        run: run_elastic_lib,
    },
    ImplEntry {
        family: "elastic",
        impl_name: "oxide",
        description: "sketch_oxide::frequency::ElasticSketch",
        constraint: Constraint::Tunable,
        accuracy_kind: AccuracyKind::Frequency,
        default_grid: || {
            ElasticParams::default_grid()
                .iter()
                .map(ParamSet::of)
                .collect()
        },
        validate: |p| {
            p.parse::<ElasticParams>()
                .map(|_| ())
                .map_err(|e| e.to_string())
        },
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
        default_grid: || {
            NitroParams::default_grid()
                .iter()
                .map(ParamSet::of)
                .collect()
        },
        validate: |p| {
            p.parse::<NitroParams>()
                .map(|_| ())
                .map_err(|e| e.to_string())
        },
        run: run_nitro_lib,
    },
    ImplEntry {
        family: "nitro",
        impl_name: "oxide",
        description: "sketch_oxide::frequency::NitroSketch<CountMinSketch>",
        constraint: Constraint::Tunable,
        accuracy_kind: AccuracyKind::None,
        default_grid: || {
            NitroParams::default_grid()
                .iter()
                .map(ParamSet::of)
                .collect()
        },
        validate: |p| {
            p.parse::<NitroParams>()
                .map(|_| ())
                .map_err(|e| e.to_string())
        },
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
        default_grid: || {
            UnivMonParams::default_grid()
                .iter()
                .map(ParamSet::of)
                .collect()
        },
        validate: |p| {
            p.parse::<UnivMonParams>()
                .map(|_| ())
                .map_err(|e| e.to_string())
        },
        run: run_univmon_lib,
    },
    ImplEntry {
        family: "univmon",
        impl_name: "oxide",
        description: "sketch_oxide::universal::UnivMon",
        constraint: Constraint::Tunable,
        accuracy_kind: AccuracyKind::None,
        default_grid: || {
            UnivMonParams::default_grid()
                .iter()
                .map(ParamSet::of)
                .collect()
        },
        validate: |p| {
            p.parse::<UnivMonParams>()
                .map(|_| ())
                .map_err(|e| e.to_string())
        },
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
    BenchRunner::new(cfg.clone(), wk, family, impl_name).run::<S, _, sketch_bench::NoGT, _>(
        factory,
        insert_body,
        None,
    )
}

/// The hot-loop body for every dispatch row.
///
/// Defined here in `sketch-cli`, the crate that also defines the wrappers,
/// so the wrapper's `update` and this call site land in the same codegen
/// unit and LLVM can fold the update into the loop. Handing this to the
/// runner — rather than letting the runner call `sketch.update(it)` from
/// inside `sketch-bench` — is what keeps `lib-fixedmatrix-fast-*` unrolled.
#[inline(always)]
fn insert_body<S: sketch_core::sketch::Sketch>(s: &mut S, it: &S::Item) {
    s.update(it);
}

fn bench_freq_gt<S, W>(
    cfg: &BenchConfig,
    wk: &W,
    family: &str,
    impl_name: &str,
    factory: impl FnMut() -> S,
    max_probes: usize,
) -> Vec<BenchReport>
where
    W: Workload,
    W::Item: Clone + Eq + Hash + Ord,
    S: sketch_core::sketch::Sketch<Item = W::Item, Query = W::Item, Answer = u64>,
{
    let gt = FrequencyGT { max_probes };
    BenchRunner::new(cfg.clone(), wk, family, impl_name).run::<S, _, FrequencyGT, _>(
        factory,
        insert_body,
        Some(&gt),
    )
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
    BenchRunner::new(cfg.clone(), wk, family, impl_name).run::<S, _, CardinalityGT, _>(
        factory,
        insert_body,
        Some(&gt),
    )
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
    BenchRunner::new(cfg.clone(), wk, family, impl_name).run::<S, _, RankErrorGT, _>(
        factory,
        insert_body,
        Some(&gt),
    )
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
    BenchRunner::new(cfg.clone(), wk, family, impl_name).run::<S, _, RelativeErrorGT, _>(
        factory,
        insert_body,
        Some(&gt),
    )
}

// ---------- dispatch macros ----------
//
// A dispatch row differs from its neighbours along exactly three axes:
// the wrapper type, how the workload's `i64` items are viewed by that
// wrapper (raw / decimal string / decimal bytes), and which
// ground-truth comparator its family supports. `run_impl!` takes those
// as tokens and expands the single `fn` shape they all share, so
// adding an impl is one line and adding an axis value is one macro
// arm — rather than one macro per (view × GT) pair.

/// Unwrap the family-typed params. A mismatch means the dispatch table
/// wired a row to the wrong `ParamSet` variant — a build-time wiring
/// bug, not user input — so it panics rather than degrading.
/// Recover this row's typed params.
///
/// A failure is a wiring bug (the sweep handed a row another family's set) or
/// a user typo in `--config`; serde names the offending key either way, which
/// is what replaced the hand-written per-family allowed-key lists.
macro_rules! params_of {
    ($ty:ty, $params:expr, $impl:expr) => {
        match $params.parse::<$ty>() {
            Ok(p) => p,
            Err(e) => panic!("dispatch::{}: {}", $impl, e),
        }
    };
}

/// The item view a wrapper consumes. `string` / `bytes` materialise a
/// derived workload; binding the result with `let` extends the
/// temporary's lifetime over the benchmark call.
macro_rules! wk_view {
    (i64, $wk:expr) => {
        $wk
    };
    (string, $wk:expr) => {
        &StringWorkload::from_i64($wk)
    };
    (bytes, $wk:expr) => {
        &BytesWorkload::from_i64($wk)
    };
}

/// The ground-truth comparator a family supports, and the extra
/// `AccuracyCfg` knobs that comparator honours.
macro_rules! gt_bench {
    (freq, $wrapper:ty, $cfg:expr, $w:expr, $family:expr, $impl:expr, $p:expr, $acc:expr) => {
        bench_freq_gt::<$wrapper, _>(
            $cfg,
            $w,
            $family,
            $impl,
            || <$wrapper>::new(&$p),
            $acc.max_probes,
        )
    };
    (card, $wrapper:ty, $cfg:expr, $w:expr, $family:expr, $impl:expr, $p:expr, $acc:expr) => {
        bench_card_gt::<$wrapper, _>(
            $cfg,
            $w,
            $family,
            $impl,
            || <$wrapper>::new(&$p),
            $acc.record_query_calls,
        )
    };
    (quant, $wrapper:ty, $cfg:expr, $w:expr, $family:expr, $impl:expr, $p:expr, $acc:expr) => {
        bench_quant_gt::<$wrapper, _>(
            $cfg,
            $w,
            $family,
            $impl,
            || <$wrapper>::new(&$p),
            $acc.record_query_calls,
        )
    };
    (quant_rel, $wrapper:ty, $cfg:expr, $w:expr, $family:expr, $impl:expr, $p:expr, $acc:expr) => {
        bench_quant_rel_gt::<$wrapper, _>(
            $cfg,
            $w,
            $family,
            $impl,
            || <$wrapper>::new(&$p),
            $acc.record_query_calls,
        )
    };
}

/// Define one dispatch row's `run` fn.
///
/// `$view` ∈ `i64 | string | bytes`; `$gt` ∈ `freq | card | quant |
/// quant_rel | none`. `none` marks a family whose wrapper `query` is a
/// stub (Nitro / UnivMon): a comparator there would report ~100% error
/// against a hardcoded 0, which is worse than no number, so those rows
/// ignore `--accuracy` entirely.
macro_rules! run_impl {
    ($fn_name:ident, $wrapper:ty, $family:expr, $impl:expr, $params_ty:ty, $view:ident, none) => {
        fn $fn_name(
            cfg: &BenchConfig,
            wk: &I64Workload,
            params: &ParamSet,
            _accuracy: &AccuracyCfg,
        ) -> Vec<BenchReport> {
            let p = params_of!($params_ty, params, $impl);
            let w = wk_view!($view, wk);
            bench_no_gt::<$wrapper, _>(cfg, w, $family, $impl, || <$wrapper>::new(&p))
        }
    };
    ($fn_name:ident, $wrapper:ty, $family:expr, $impl:expr, $params_ty:ty, $view:ident, $gt:ident) => {
        fn $fn_name(
            cfg: &BenchConfig,
            wk: &I64Workload,
            params: &ParamSet,
            accuracy: &AccuracyCfg,
        ) -> Vec<BenchReport> {
            let p = params_of!($params_ty, params, $impl);
            let w = wk_view!($view, wk);
            if accuracy.enabled {
                gt_bench!($gt, $wrapper, cfg, w, $family, $impl, p, accuracy)
            } else {
                bench_no_gt::<$wrapper, _>(cfg, w, $family, $impl, || <$wrapper>::new(&p))
            }
        }
    };
}

/// Parallel-insert impls (`lib-fastpath-parallel` under cms /
/// countsketch / hll). Separate from `run_impl!` only because the
/// wrapper ctor takes a second `workers` argument (from `--workers N`).
/// Accuracy is `None` because the partitions are intentionally not
/// merged (matches legacy octo).
macro_rules! run_parallel_impl {
    ($fn_name:ident, $wrapper:ty, $family:expr, $impl:expr, $params_ty:ty) => {
        fn $fn_name(
            cfg: &BenchConfig,
            wk: &I64Workload,
            params: &ParamSet,
            _accuracy: &AccuracyCfg,
        ) -> Vec<BenchReport> {
            let p = params_of!($params_ty, params, $impl);
            let workers = cfg.threads;
            bench_no_gt::<$wrapper, _>(cfg, wk, $family, $impl, move || {
                <$wrapper>::new(&p, workers)
            })
        }
    };
}

// -- HLL --
run_impl!(
    run_hll_oxide,
    hll::HllOxide,
    "hll",
    "oxide",
    HllParams,
    i64,
    card
);
run_impl!(
    run_hll_datasketches,
    hll::HllDatasketches,
    "hll",
    "datasketches",
    HllParams,
    i64,
    card
);
run_impl!(run_hll_lib, hll::HllLib, "hll", "lib", HllParams, i64, card);
run_impl!(
    run_hll_lib_hip,
    hll::HllLibHip,
    "hll",
    "lib-hip",
    HllParams,
    i64,
    card
);
run_impl!(
    run_hll_exact,
    exact::ExactCardinality,
    "hll",
    "exact",
    HllParams,
    i64,
    card
);
run_impl!(
    run_hll_null,
    exact::NullCardinality,
    "hll",
    "null",
    HllParams,
    i64,
    card
);

// -- KLL --
run_impl!(
    run_kll_oxide,
    kll::KllOxide,
    "kll",
    "oxide",
    KllParams,
    i64,
    quant
);
run_impl!(
    run_kll_lib,
    kll::KllLib,
    "kll",
    "lib",
    KllParams,
    i64,
    quant
);
run_impl!(
    run_kll_exact,
    exact::ExactQuantile,
    "kll",
    "exact",
    KllParams,
    i64,
    quant
);

// -- CMS --
run_impl!(
    run_cms_oxide,
    cms::CmsOxide,
    "cms",
    "oxide",
    CmsParams,
    i64,
    freq
);
run_impl!(
    run_cms_datasketches,
    cms::CmsDatasketches,
    "cms",
    "datasketches",
    CmsParams,
    i64,
    freq
);
run_impl!(
    run_cms_lib_fixedmatrix_custom_fast,
    cms::CmsLibFixedmatrixCustomFast,
    "cms",
    "lib-fixedmatrix-custom-fast",
    CmsParams,
    i64,
    freq
);
run_impl!(
    run_cms_lib_fixedmatrix_fast,
    cms::CmsLibFixedmatrixFast,
    "cms",
    "lib-fixedmatrix-fast",
    CmsParams,
    i64,
    freq
);
run_impl!(
    run_cms_lib_fixedmatrix_fast_32k,
    cms::CmsLibFixedmatrixFast32k,
    "cms",
    "lib-fixedmatrix-fast-32k",
    CmsParams,
    i64,
    freq
);
run_impl!(
    run_cms_lib_vector2d_fast,
    cms::CmsLibVector2dFast,
    "cms",
    "lib-vector2d-fast",
    CmsParams,
    i64,
    freq
);
run_impl!(
    run_cms_lib_vector2d_regular,
    cms::CmsLibVector2dRegular,
    "cms",
    "lib-vector2d-regular",
    CmsParams,
    i64,
    freq
);
run_impl!(
    run_cms_exact,
    exact::ExactFrequency,
    "cms",
    "exact",
    CmsParams,
    i64,
    freq
);
run_impl!(
    run_cms_null,
    exact::NullFrequency,
    "cms",
    "null",
    CmsParams,
    i64,
    freq
);

// -- CountSketch --
run_impl!(
    run_cs_oxide,
    countsketch::CsOxide,
    "countsketch",
    "oxide",
    CountSketchParams,
    i64,
    freq
);
run_impl!(
    run_cs_lib_fixedmatrix_fast,
    countsketch::CsLibFixedmatrixFast,
    "countsketch",
    "lib-fixedmatrix-fast",
    CountSketchParams,
    i64,
    freq
);
run_impl!(
    run_cs_lib_fixedmatrix_fast_32k,
    countsketch::CsLibFixedmatrixFast32k,
    "countsketch",
    "lib-fixedmatrix-fast-32k",
    CountSketchParams,
    i64,
    freq
);
run_impl!(
    run_cs_lib_vector2d_fast,
    countsketch::CsLibVector2dFast,
    "countsketch",
    "lib-vector2d-fast",
    CountSketchParams,
    i64,
    freq
);
run_impl!(
    run_cs_lib_vector2d_regular,
    countsketch::CsLibVector2dRegular,
    "countsketch",
    "lib-vector2d-regular",
    CountSketchParams,
    i64,
    freq
);
run_impl!(
    run_cs_exact,
    exact::ExactFrequencyCs,
    "countsketch",
    "exact",
    CountSketchParams,
    i64,
    freq
);
run_impl!(
    run_cs_null,
    exact::NullFrequencyCs,
    "countsketch",
    "null",
    CountSketchParams,
    i64,
    freq
);

// -- DDSketch --
run_impl!(run_dd_lib, dd::DdLib, "dd", "lib", DdParams, i64, quant_rel);
run_impl!(
    run_dd_exact,
    exact::ExactQuantileDd,
    "dd",
    "exact",
    DdParams,
    i64,
    quant_rel
);

// -- Polars-backed baselines (one per family). Same `Sketch`
//    contract as the in-tree exact baselines; the legacy
//    `throughput/polars_*/` binaries are folded into these.
run_impl!(
    run_hll_polars,
    polars::PolarsCardinality,
    "hll",
    "polars",
    HllParams,
    i64,
    card
);
run_impl!(
    run_kll_polars,
    polars::PolarsQuantileKll,
    "kll",
    "polars",
    KllParams,
    i64,
    quant
);
run_impl!(
    run_dd_polars,
    polars::PolarsQuantileDd,
    "dd",
    "polars",
    DdParams,
    i64,
    quant_rel
);
run_impl!(
    run_cms_polars,
    polars::PolarsFrequencyCms,
    "cms",
    "polars",
    CmsParams,
    i64,
    freq
);
run_impl!(
    run_cs_polars,
    polars::PolarsFrequencyCs,
    "countsketch",
    "polars",
    CountSketchParams,
    i64,
    freq
);

// -- Parallel-insert FastPath baselines (one per family). Workers
//    are read from `cfg.threads` (= `--workers N`). Folds the
//    legacy `throughput/octo/` binary into sketch-cli.
run_parallel_impl!(
    run_cms_lib_fastpath_parallel,
    parallel::ParallelCmsFastPath,
    "cms",
    "lib-fastpath-parallel",
    CmsParams
);
run_parallel_impl!(
    run_cs_lib_fastpath_parallel,
    parallel::ParallelCsFastPath,
    "countsketch",
    "lib-fastpath-parallel",
    CountSketchParams
);
run_parallel_impl!(
    run_hll_lib_fastpath_parallel,
    parallel::ParallelHllFastPath,
    "hll",
    "lib-fastpath-parallel",
    HllParams
);

// -- Elastic --
run_impl!(
    run_elastic_lib,
    elastic::ElasticLib,
    "elastic",
    "lib",
    ElasticParams,
    string,
    freq
);
run_impl!(
    run_elastic_oxide,
    elastic::ElasticOxide,
    "elastic",
    "oxide",
    ElasticParams,
    bytes,
    freq
);

// -- Nitro / UnivMon: wrapper `query` is a stub, so no comparator.
run_impl!(
    run_nitro_lib,
    nitro::NitroLib,
    "nitro",
    "lib",
    NitroParams,
    i64,
    none
);
run_impl!(
    run_nitro_oxide,
    nitro::NitroOxide,
    "nitro",
    "oxide",
    NitroParams,
    bytes,
    none
);
run_impl!(
    run_univmon_lib,
    univmon::UnivMonLib,
    "univmon",
    "lib",
    UnivMonParams,
    string,
    none
);
run_impl!(
    run_univmon_oxide,
    univmon::UnivMonOxide,
    "univmon",
    "oxide",
    UnivMonParams,
    bytes,
    none
);

#[allow(dead_code)]
fn _silence_unused_warnings(_: CmsParams, _: CountSketchParams) {}
