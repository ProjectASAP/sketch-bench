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
use sketch_core::datagen::{DType, GenSpec};
use sketch_core::workload::{BytesWorkload, F64Workload, I64Workload, StringWorkload, Workload};

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
    /// Materialise the items at the requested `dtype`.
    ///
    /// The dtype is not inferred from the spec, it is *checked* against it:
    /// `NumericWorkload::generate` refuses a column of the wrong type rather
    /// than widening it, so `--dtype f64` over an `i64` spec is an error and
    /// never a silent `as f64` on the insert path.
    pub fn build(self, dtype: DType) -> Result<Items> {
        match (self, dtype) {
            (WorkloadSpec::Generated(spec), DType::F64) => F64Workload::generate(&spec)
                .map(Items::F64)
                .map_err(|e| anyhow::anyhow!("{}", e)),
            (WorkloadSpec::Generated(spec), _) => I64Workload::generate(&spec)
                .map(Items::I64)
                .map_err(|e| anyhow::anyhow!("{}", e)),
            // The file loaders read `.bin`/`.csv`/`.pcap` as `i64` streams and
            // the `.bin` sidecar check already rejects a non-`i64` file, so
            // there is nothing to widen here either.
            (WorkloadSpec::File { path }, DType::I64) => {
                I64Workload::load(std::path::Path::new(&path))
                    .map(Items::I64)
                    .map_err(|e| anyhow::anyhow!("{}", e))
            }
            (WorkloadSpec::File { path }, other) => Err(anyhow::anyhow!(
                "--input {path} is read as an i64 stream, but --dtype {} was requested; \
                 generate the workload instead (--spec / --workload) to benchmark {}",
                other.as_str(),
                other.as_str(),
            )),
        }
    }
}

/// The materialised workload, at whichever dtype was asked for.
///
/// A benchmark row consumes exactly one item type — `type Item` is fixed per
/// wrapper — so this is the point where the run-time dtype meets the
/// compile-time one, and the only place allowed to decide they disagree.
pub enum Items {
    I64(I64Workload),
    F64(F64Workload),
}

impl Items {
    pub fn dtype(&self) -> DType {
        match self {
            Items::I64(_) => DType::I64,
            Items::F64(_) => DType::F64,
        }
    }
}

/// A row was handed a workload whose item type it cannot ingest.
///
/// Returned rather than reported as an empty result set, because "produced no
/// reports" and "cannot run at all" are different outcomes that look identical
/// in the output. A row that quietly returned nothing here would drop out of
/// the matrix without ever saying so — the same failure as a dispatch table
/// that silently loses rows.
#[derive(Debug, Clone)]
pub struct DtypeMismatch {
    pub wanted: &'static [DType],
    pub got: DType,
}

impl std::fmt::Display for DtypeMismatch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let wanted: Vec<&str> = self.wanted.iter().map(DType::as_str).collect();
        write!(
            f,
            "consumes {} items, workload is {}",
            wanted.join(" or "),
            self.got.as_str()
        )
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

/// The operations a row needs from its params type, resolved from a single
/// mention of that type.
///
/// These were two hand-written closures per row: 41 rows x 2 near-identical
/// closures, and — worse — two independent places to get the type wrong.
/// A countsketch row that still said `CmsParams` in its grid closure would
/// have made that row's grid the whole family's default sweep, since
/// `sweep::default_grid` reads a family's first row; it would have surfaced
/// only at runtime, and only if the *other* closure had been pasted
/// correctly. One mention makes the two structurally incapable of disagreeing.
#[derive(Debug, Clone, Copy)]
pub struct ParamsVTable {
    /// The family's sweep grid when `--config` is omitted.
    pub default_grid: fn() -> Vec<ParamSet>,
    /// Type-check a grid point without building anything. `--config` is user
    /// input, so a misspelled key must produce an error naming it — not the
    /// panic `params_of!` raises for a wiring bug. The CLI validates the whole
    /// grid up front.
    pub validate: fn(&ParamSet) -> Result<(), String>,
}

impl ParamsVTable {
    pub const fn of<P: SketchParams>() -> Self {
        Self {
            default_grid: grid_of::<P>,
            validate: validate_of::<P>,
        }
    }
}

fn grid_of<P: SketchParams>() -> Vec<ParamSet> {
    P::default_grid().iter().map(ParamSet::of).collect()
}

fn validate_of<P: SketchParams>(p: &ParamSet) -> Result<(), String> {
    p.parse::<P>().map(|_| ()).map_err(|e| e.to_string())
}

pub struct ImplEntry {
    pub family: &'static str,
    pub impl_name: &'static str,
    pub description: &'static str,
    pub constraint: Constraint,
    pub accuracy_kind: AccuracyKind,
    /// Everything derived from this row's params type, named once.
    pub params: ParamsVTable,
    run: fn(
        cfg: &BenchConfig,
        items: &Items,
        params: &ParamSet,
        accuracy: &AccuracyCfg,
    ) -> Result<Vec<BenchReport>, DtypeMismatch>,
}

impl ImplEntry {
    /// Run the bench. Returns one `BenchReport` per metric pass —
    /// see [`sketch_bench::MetricsMask::passes`]. Callers iterate
    /// the Vec and emit each report on its own JSONL line / CSV
    /// row group.
    pub fn run(
        &self,
        cfg: &BenchConfig,
        items: &Items,
        params: &ParamSet,
        accuracy: &AccuracyCfg,
    ) -> Result<Vec<BenchReport>, DtypeMismatch> {
        (self.run)(cfg, items, params, accuracy)
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
        params: ParamsVTable::of::<HllParams>(),
        run: run_hll_oxide,
    },
    ImplEntry {
        family: "hll",
        impl_name: "datasketches",
        description: "datasketches::hll::HllSketch (Hll8)",
        constraint: Constraint::Tunable,
        accuracy_kind: AccuracyKind::Cardinality,
        params: ParamsVTable::of::<HllParams>(),
        run: run_hll_datasketches,
    },
    ImplEntry {
        family: "hll",
        impl_name: "lib",
        description: "asap_sketchlib::HyperLogLog<Classic> (P14): O(m) estimate",
        constraint: Constraint::Tunable,
        accuracy_kind: AccuracyKind::Cardinality,
        params: ParamsVTable::of::<HllParams>(),
        run: run_hll_lib,
    },
    ImplEntry {
        family: "hll",
        impl_name: "lib-hip",
        description: "asap_sketchlib::HyperLogLogHIP (P14): O(1) estimate, slightly slower insert",
        constraint: Constraint::Tunable,
        accuracy_kind: AccuracyKind::Cardinality,
        params: ParamsVTable::of::<HllParams>(),
        run: run_hll_lib_hip,
    },
    ImplEntry {
        family: "hll",
        impl_name: "exact",
        description: "exact baseline: HashSet<i64>, cardinality = set.len()",
        constraint: Constraint::Unparameterized,
        accuracy_kind: AccuracyKind::Cardinality,
        params: ParamsVTable::of::<HllParams>(),
        run: run_hll_exact,
    },
    ImplEntry {
        family: "hll",
        impl_name: "null",
        description: "null baseline: cardinality estimate is 0 — pins relative error = 1.0",
        constraint: Constraint::Unparameterized,
        accuracy_kind: AccuracyKind::Cardinality,
        params: ParamsVTable::of::<HllParams>(),
        run: run_hll_null,
    },
    ImplEntry {
        family: "hll",
        impl_name: "polars",
        description: "polars exact: DataFrame.n_unique() (DataFrame baseline)",
        constraint: Constraint::Unparameterized,
        accuracy_kind: AccuracyKind::Cardinality,
        params: ParamsVTable::of::<HllParams>(),
        run: run_hll_polars,
    },
    ImplEntry {
        family: "hll",
        impl_name: "lib-fastpath-parallel",
        description:
            "asap_sketchlib HLL ErtlMLE, FastPath, parallel insert (workers from --workers)",
        constraint: Constraint::Tunable,
        accuracy_kind: AccuracyKind::None,
        params: ParamsVTable::of::<HllParams>(),
        run: run_hll_lib_fastpath_parallel,
    },
    // -------- KLL --------
    ImplEntry {
        family: "kll",
        impl_name: "oxide",
        description: "sketch_oxide::quantiles::KllSketch",
        constraint: Constraint::Tunable,
        accuracy_kind: AccuracyKind::Quantile,
        params: ParamsVTable::of::<KllParams>(),
        run: run_kll_oxide,
    },
    ImplEntry {
        family: "kll",
        impl_name: "lib",
        description: "asap_sketchlib::KLL<i64>",
        constraint: Constraint::Tunable,
        accuracy_kind: AccuracyKind::Quantile,
        params: ParamsVTable::of::<KllParams>(),
        run: run_kll_lib,
    },
    ImplEntry {
        family: "kll",
        impl_name: "exact",
        description: "exact baseline: Vec<i64> sorted, quantile = index lookup",
        constraint: Constraint::Unparameterized,
        accuracy_kind: AccuracyKind::Quantile,
        params: ParamsVTable::of::<KllParams>(),
        run: run_kll_exact,
    },
    ImplEntry {
        family: "kll",
        impl_name: "polars",
        description: "polars exact: 101-point quantile grid via DataFrame (DataFrame baseline)",
        constraint: Constraint::Unparameterized,
        accuracy_kind: AccuracyKind::Quantile,
        params: ParamsVTable::of::<KllParams>(),
        run: run_kll_polars,
    },
    // -------- CMS --------
    ImplEntry {
        family: "cms",
        impl_name: "oxide",
        description: "sketch_oxide::frequency::CountMinSketch",
        constraint: Constraint::Tunable,
        accuracy_kind: AccuracyKind::Frequency,
        params: ParamsVTable::of::<CmsParams>(),
        run: run_cms_oxide,
    },
    ImplEntry {
        family: "cms",
        impl_name: "datasketches",
        description: "datasketches::countmin::CountMinSketch",
        constraint: Constraint::Tunable,
        accuracy_kind: AccuracyKind::Frequency,
        params: ParamsVTable::of::<CmsParams>(),
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
        params: ParamsVTable::of::<CmsParams>(),
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
        params: ParamsVTable::of::<CmsParams>(),
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
        params: ParamsVTable::of::<CmsParams>(),
        run: run_cms_lib_fixedmatrix_fast_32k,
    },
    ImplEntry {
        family: "cms",
        impl_name: "lib-vector2d-fast",
        description: "asap_sketchlib CMS, Vector2D, FastPath",
        constraint: Constraint::Tunable,
        accuracy_kind: AccuracyKind::Frequency,
        params: ParamsVTable::of::<CmsParams>(),
        run: run_cms_lib_vector2d_fast,
    },
    ImplEntry {
        family: "cms",
        impl_name: "lib-vector2d-regular",
        description: "asap_sketchlib CMS, Vector2D, RegularPath",
        constraint: Constraint::Tunable,
        accuracy_kind: AccuracyKind::Frequency,
        params: ParamsVTable::of::<CmsParams>(),
        run: run_cms_lib_vector2d_regular,
    },
    ImplEntry {
        family: "cms",
        impl_name: "exact",
        description: "exact baseline: HashMap<i64,u64>, freq = map.get(k)",
        constraint: Constraint::Unparameterized,
        accuracy_kind: AccuracyKind::Frequency,
        params: ParamsVTable::of::<CmsParams>(),
        run: run_cms_exact,
    },
    ImplEntry {
        family: "cms",
        impl_name: "null",
        description:
            "null baseline: every count is 0 — pins ARE = 1.0, the disqualifying threshold",
        constraint: Constraint::Unparameterized,
        accuracy_kind: AccuracyKind::Frequency,
        params: ParamsVTable::of::<CmsParams>(),
        run: run_cms_null,
    },
    ImplEntry {
        family: "cms",
        impl_name: "polars",
        description: "polars exact: group_by(v).agg(len) → HashMap (DataFrame baseline)",
        constraint: Constraint::Unparameterized,
        accuracy_kind: AccuracyKind::Frequency,
        params: ParamsVTable::of::<CmsParams>(),
        run: run_cms_polars,
    },
    ImplEntry {
        family: "cms",
        impl_name: "lib-fastpath-parallel",
        description:
            "asap_sketchlib CMS, FastPath, parallel insert on M5x32K (workers from --workers)",
        constraint: Constraint::Tunable,
        accuracy_kind: AccuracyKind::None,
        params: ParamsVTable::of::<CmsParams>(),
        run: run_cms_lib_fastpath_parallel,
    },
    // -------- CountSketch --------
    ImplEntry {
        family: "countsketch",
        impl_name: "oxide",
        description: "sketch_oxide::frequency::CountSketch",
        constraint: Constraint::Tunable,
        accuracy_kind: AccuracyKind::Frequency,
        params: ParamsVTable::of::<CountSketchParams>(),
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
        params: ParamsVTable::of::<CountSketchParams>(),
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
        params: ParamsVTable::of::<CountSketchParams>(),
        run: run_cs_lib_fixedmatrix_fast_32k,
    },
    ImplEntry {
        family: "countsketch",
        impl_name: "lib-vector2d-fast",
        description: "asap_sketchlib Count, Vector2D, FastPath",
        constraint: Constraint::Tunable,
        accuracy_kind: AccuracyKind::Frequency,
        params: ParamsVTable::of::<CountSketchParams>(),
        run: run_cs_lib_vector2d_fast,
    },
    ImplEntry {
        family: "countsketch",
        impl_name: "lib-vector2d-regular",
        description: "asap_sketchlib Count, Vector2D, RegularPath",
        constraint: Constraint::Tunable,
        accuracy_kind: AccuracyKind::Frequency,
        params: ParamsVTable::of::<CountSketchParams>(),
        run: run_cs_lib_vector2d_regular,
    },
    ImplEntry {
        family: "countsketch",
        impl_name: "exact",
        description: "exact baseline: HashMap<i64,u64>, freq = map.get(k)",
        constraint: Constraint::Unparameterized,
        accuracy_kind: AccuracyKind::Frequency,
        params: ParamsVTable::of::<CountSketchParams>(),
        run: run_cs_exact,
    },
    ImplEntry {
        family: "countsketch",
        impl_name: "null",
        description:
            "null baseline: every count is 0 — pins ARE = 1.0, the disqualifying threshold",
        constraint: Constraint::Unparameterized,
        accuracy_kind: AccuracyKind::Frequency,
        params: ParamsVTable::of::<CountSketchParams>(),
        run: run_cs_null,
    },
    ImplEntry {
        family: "countsketch",
        impl_name: "polars",
        description: "polars exact: group_by(v).agg(len) → HashMap (DataFrame baseline)",
        constraint: Constraint::Unparameterized,
        accuracy_kind: AccuracyKind::Frequency,
        params: ParamsVTable::of::<CountSketchParams>(),
        run: run_cs_polars,
    },
    ImplEntry {
        family: "countsketch",
        impl_name: "lib-fastpath-parallel",
        description:
            "asap_sketchlib Count, FastPath, parallel insert on M5x32K (workers from --workers)",
        constraint: Constraint::Tunable,
        accuracy_kind: AccuracyKind::None,
        params: ParamsVTable::of::<CountSketchParams>(),
        run: run_cs_lib_fastpath_parallel,
    },
    // -------- DDSketch --------
    ImplEntry {
        family: "dd",
        impl_name: "lib",
        description: "asap_sketchlib::DDSketch (relative-error quantile)",
        constraint: Constraint::Tunable,
        accuracy_kind: AccuracyKind::Quantile,
        params: ParamsVTable::of::<DdParams>(),
        run: run_dd_lib,
    },
    ImplEntry {
        family: "dd",
        impl_name: "exact",
        description: "exact baseline: Vec<i64> sorted, quantile = Type-7 lookup",
        constraint: Constraint::Unparameterized,
        accuracy_kind: AccuracyKind::Quantile,
        params: ParamsVTable::of::<DdParams>(),
        run: run_dd_exact,
    },
    ImplEntry {
        family: "dd",
        impl_name: "polars",
        description: "polars exact: 101-point quantile grid via DataFrame (DataFrame baseline)",
        constraint: Constraint::Unparameterized,
        accuracy_kind: AccuracyKind::Quantile,
        params: ParamsVTable::of::<DdParams>(),
        run: run_dd_polars,
    },
    // -------- Elastic --------
    ImplEntry {
        family: "elastic",
        impl_name: "lib",
        description: "asap_sketchlib::Elastic<DefaultXxHasher>",
        constraint: Constraint::Tunable,
        accuracy_kind: AccuracyKind::Frequency,
        params: ParamsVTable::of::<ElasticParams>(),
        run: run_elastic_lib,
    },
    ImplEntry {
        family: "elastic",
        impl_name: "oxide",
        description: "sketch_oxide::frequency::ElasticSketch",
        constraint: Constraint::Tunable,
        accuracy_kind: AccuracyKind::Frequency,
        params: ParamsVTable::of::<ElasticParams>(),
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
        params: ParamsVTable::of::<NitroParams>(),
        run: run_nitro_lib,
    },
    ImplEntry {
        family: "nitro",
        impl_name: "oxide",
        description: "sketch_oxide::frequency::NitroSketch<CountMinSketch>",
        constraint: Constraint::Tunable,
        accuracy_kind: AccuracyKind::None,
        params: ParamsVTable::of::<NitroParams>(),
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
        params: ParamsVTable::of::<UnivMonParams>(),
        run: run_univmon_lib,
    },
    ImplEntry {
        family: "univmon",
        impl_name: "oxide",
        description: "sketch_oxide::universal::UnivMon",
        constraint: Constraint::Tunable,
        accuracy_kind: AccuracyKind::None,
        params: ParamsVTable::of::<UnivMonParams>(),
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

/// Narrow [`Items`] to the `i64` workload a key-shaped row needs, or report
/// the mismatch.
///
/// Every hash-based family lands here: their key is hashed, `f64` is not
/// `Hash` in Rust, and hashing its bit pattern would be the same operation on
/// the same bits as hashing an `i64` — a second curve that could only ever
/// retrace the first. The `string` / `bytes` views derive from this workload
/// too, so they inherit the same constraint.
macro_rules! keys_only {
    ($items:expr) => {
        match $items {
            Items::I64(wk) => wk,
            other => {
                return Err(DtypeMismatch {
                    wanted: &[DType::I64],
                    got: other.dtype(),
                })
            }
        }
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
            items: &Items,
            params: &ParamSet,
            _accuracy: &AccuracyCfg,
        ) -> Result<Vec<BenchReport>, DtypeMismatch> {
            let p = params_of!($params_ty, params, $impl);
            let wk = keys_only!(items);
            let w = wk_view!($view, wk);
            Ok(bench_no_gt::<$wrapper, _>(cfg, w, $family, $impl, || {
                <$wrapper>::new(&p)
            }))
        }
    };
    ($fn_name:ident, $wrapper:ty, $family:expr, $impl:expr, $params_ty:ty, $view:ident, $gt:ident) => {
        fn $fn_name(
            cfg: &BenchConfig,
            items: &Items,
            params: &ParamSet,
            accuracy: &AccuracyCfg,
        ) -> Result<Vec<BenchReport>, DtypeMismatch> {
            let p = params_of!($params_ty, params, $impl);
            let wk = keys_only!(items);
            let w = wk_view!($view, wk);
            Ok(if accuracy.enabled {
                gt_bench!($gt, $wrapper, cfg, w, $family, $impl, p, accuracy)
            } else {
                bench_no_gt::<$wrapper, _>(cfg, w, $family, $impl, || <$wrapper>::new(&p))
            })
        }
    };
}

/// Define a row for an **ordered** family (kll, dd), which runs on either
/// side of the dtype axis.
///
/// Takes the wrapper twice, once per instantiation, instead of one generic
/// path the macro appends `<i64>` / `<f64>` to: `macro_rules!` cannot build a
/// type by pasting arguments onto a `:ty` fragment. Spelling both out is more
/// characters but the compiler checks each of them, which a token-pasting
/// trick would not have made any safer.
macro_rules! run_ordered_impl {
    ($fn_name:ident, $w_i64:ty, $w_f64:ty, $family:expr, $impl:expr, $params_ty:ty, $gt:ident) => {
        fn $fn_name(
            cfg: &BenchConfig,
            items: &Items,
            params: &ParamSet,
            accuracy: &AccuracyCfg,
        ) -> Result<Vec<BenchReport>, DtypeMismatch> {
            let p = params_of!($params_ty, params, $impl);
            Ok(match items {
                Items::I64(wk) => {
                    if accuracy.enabled {
                        gt_bench!($gt, $w_i64, cfg, wk, $family, $impl, p, accuracy)
                    } else {
                        bench_no_gt::<$w_i64, _>(cfg, wk, $family, $impl, || <$w_i64>::new(&p))
                    }
                }
                Items::F64(wk) => {
                    if accuracy.enabled {
                        gt_bench!($gt, $w_f64, cfg, wk, $family, $impl, p, accuracy)
                    } else {
                        bench_no_gt::<$w_f64, _>(cfg, wk, $family, $impl, || <$w_f64>::new(&p))
                    }
                }
            })
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
            items: &Items,
            params: &ParamSet,
            _accuracy: &AccuracyCfg,
        ) -> Result<Vec<BenchReport>, DtypeMismatch> {
            let p = params_of!($params_ty, params, $impl);
            let wk = keys_only!(items);
            let workers = cfg.threads;
            Ok(bench_no_gt::<$wrapper, _>(
                cfg,
                wk,
                $family,
                $impl,
                move || <$wrapper>::new(&p, workers),
            ))
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
run_ordered_impl!(
    run_kll_oxide,
    kll::KllOxide<i64>,
    kll::KllOxide<f64>,
    "kll",
    "oxide",
    KllParams,
    quant
);
run_ordered_impl!(
    run_kll_lib,
    kll::KllLib<i64>,
    kll::KllLib<f64>,
    "kll",
    "lib",
    KllParams,
    quant
);
run_ordered_impl!(
    run_kll_exact,
    exact::ExactQuantile<i64>,
    exact::ExactQuantile<f64>,
    "kll",
    "exact",
    KllParams,
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
run_ordered_impl!(
    run_dd_lib,
    dd::DdLib<i64>,
    dd::DdLib<f64>,
    "dd",
    "lib",
    DdParams,
    quant_rel
);
run_ordered_impl!(
    run_dd_exact,
    exact::ExactQuantileDd<i64>,
    exact::ExactQuantileDd<f64>,
    "dd",
    "exact",
    DdParams,
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

#[cfg(test)]
mod registry_tests {
    use super::*;
    use std::collections::BTreeSet;

    /// The registry's size, pinned.
    ///
    /// A refactor of this file once deleted 32 of its 41 rows and still
    /// compiled, still passed every test, and still ran: nothing structurally
    /// requires the array to have any particular length, so 32 implementations
    /// silently left the benchmark. Bump this deliberately when adding or
    /// removing a row.
    const EXPECTED_ROWS: usize = 41;

    #[test]
    fn every_registered_implementation_is_still_registered() {
        assert_eq!(
            IMPLS.len(),
            EXPECTED_ROWS,
            "dispatch table changed size; update EXPECTED_ROWS if deliberate"
        );
    }

    /// Exactly these rows consume `f64`. Pinned as a set rather than a count,
    /// so both directions are caught: a row that quietly stops accepting
    /// `f64`, and a hash-based row that starts accepting it (which would be
    /// wrong — `f64` is not `Hash`, and its bit pattern is the same input the
    /// `i64` run already hashed).
    const ORDERED_ROWS: &[(&str, &str)] = &[
        ("kll", "oxide"),
        ("kll", "lib"),
        ("kll", "exact"),
        ("dd", "lib"),
        ("dd", "exact"),
    ];

    fn tiny(dtype: DType) -> Items {
        let spec = sketch_core::datagen::GenSpec {
            shape: sketch_core::datagen::Shape::Keys {
                cardinality: 64,
                dist: sketch_core::datagen::Distribution::Uniform,
                dtype,
            },
            size: 256,
            seed: 1,
        };
        WorkloadSpec::Generated(spec).build(dtype).unwrap()
    }

    /// The guard that matters most here. `keys_only!` could be "simplified"
    /// into returning an empty report vec instead of an error, and nothing
    /// else in the build would notice: the sweep would still exit 0, still
    /// write a file, and simply contain fewer rows than it claimed to run.
    /// That is the same failure as the refactor that deleted 32 rows.
    #[test]
    fn only_the_ordered_rows_accept_f64_and_they_really_run() {
        let f64_items = tiny(DType::F64);
        let cfg = BenchConfig {
            runs: 1,
            warmup_runs: 0,
            ..Default::default()
        };
        let acc = AccuracyCfg {
            enabled: false,
            max_probes: 0,
            record_query_calls: false,
        };
        for e in IMPLS {
            let params = (e.params.default_grid)().remove(0);
            let expected = ORDERED_ROWS.contains(&(e.family, e.impl_name));
            match e.run(&cfg, &f64_items, &params, &acc) {
                Ok(reports) => {
                    assert!(
                        expected,
                        "{}/{} accepted an f64 workload but is not an ordered row",
                        e.family, e.impl_name
                    );
                    assert!(
                        !reports.is_empty(),
                        "{}/{} accepted f64 but produced no report — a skip in disguise",
                        e.family,
                        e.impl_name
                    );
                }
                Err(m) => {
                    assert!(
                        !expected,
                        "{}/{} must accept f64, got: {m}",
                        e.family, e.impl_name
                    );
                    assert_eq!(m.got, DType::F64);
                }
            }
        }
    }

    /// Every row still runs on `i64`, so adding the axis did not quietly
    /// narrow the existing matrix.
    #[test]
    fn every_row_still_accepts_i64() {
        let i64_items = tiny(DType::I64);
        let cfg = BenchConfig {
            runs: 1,
            warmup_runs: 0,
            ..Default::default()
        };
        let acc = AccuracyCfg {
            enabled: false,
            max_probes: 0,
            record_query_calls: false,
        };
        for e in IMPLS {
            let params = (e.params.default_grid)().remove(0);
            assert!(
                e.run(&cfg, &i64_items, &params, &acc).is_ok(),
                "{}/{} stopped accepting i64",
                e.family,
                e.impl_name
            );
        }
    }

    #[test]
    fn family_impl_pairs_are_unique() {
        let mut seen = BTreeSet::new();
        for e in IMPLS {
            assert!(
                seen.insert((e.family, e.impl_name)),
                "duplicate row {}/{}",
                e.family,
                e.impl_name
            );
        }
    }

    /// Each row's params vtable must belong to its own family. Getting this
    /// wrong is invisible at compile time and would hand one family's default
    /// grid to another.
    #[test]
    fn every_rows_params_match_its_family() {
        for e in IMPLS {
            let grid = (e.params.default_grid)();
            assert!(!grid.is_empty(), "{}/{}: empty grid", e.family, e.impl_name);
            for p in &grid {
                assert_eq!(
                    p.family(),
                    e.family,
                    "{}/{}: grid is for family '{}'",
                    e.family,
                    e.impl_name,
                    p.family()
                );
                assert!(
                    (e.params.validate)(p).is_ok(),
                    "{}/{}: own default grid fails its own validate",
                    e.family,
                    e.impl_name
                );
            }
        }
    }
}
