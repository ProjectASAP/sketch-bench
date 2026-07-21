//! `sketchlib` — unified CLI for sketchlib-tool.
//!
//! `bench` runs a family of sketches across a config grid —
//! see `docs/BENCH_SWEEP.md`. `list-impls` enumerates registered
//! `(family, impl)` pairs.

mod dispatch;
mod params;
mod raw_csv;
mod sweep;
mod workload_cmd;
mod wrappers;

// Global allocator selection. Four combinations of two feature
// flags (`heap-jemalloc`, `heap-track`):
//
// - heap-jemalloc, no heap-track: bare jemalloc (legacy default)
// - heap-jemalloc + heap-track:   TrackingAllocator wrapping
//   jemalloc — counters advance, stats.allocated still readable
// - heap-track, no heap-jemalloc: TrackingAllocator(System)
// - neither:                      implicit System allocator
//
// The static is required for `#[global_allocator]` to take effect.
#[cfg(all(feature = "heap-jemalloc", not(feature = "heap-track")))]
#[global_allocator]
static GLOBAL: tikv_jemallocator::Jemalloc = tikv_jemallocator::Jemalloc;

#[cfg(all(feature = "heap-jemalloc", feature = "heap-track"))]
#[global_allocator]
static GLOBAL: sketch_bench::metrics::heap_track::TrackingAllocator<
    tikv_jemallocator::Jemalloc,
> = sketch_bench::metrics::heap_track::TrackingAllocator(tikv_jemallocator::Jemalloc);

#[cfg(all(feature = "heap-track", not(feature = "heap-jemalloc")))]
#[global_allocator]
static GLOBAL: sketch_bench::metrics::heap_track::TrackingAllocator<std::alloc::System> =
    sketch_bench::metrics::heap_track::TrackingAllocator(std::alloc::System);

use std::fs::OpenOptions;
use std::io::Write;

use anyhow::{bail, Result};
use clap::{Parser, Subcommand};
use sketch_bench::{BenchConfig, MetricsMask};
use sketch_core::datagen::{DType, Distribution, GenSpec, Shape};

use dispatch::{AccuracyCfg, AccuracyKind, ImplEntry, WorkloadSpec};

#[derive(Parser, Debug)]
#[command(name = "sketchlib", version, about = "Unified sketchlib-tool CLI")]
struct Cli {
    #[command(subcommand)]
    command: Cmd,
}

#[derive(Subcommand, Debug)]
enum Cmd {
    /// Run a benchmark across a sketch family's config grid.
    Bench(BenchArgs),
    /// List every `(family, impl)` pair the CLI can drive.
    ListImpls,
    /// Generate or inspect synthetic `.bin` workloads.
    Workload(workload_cmd::WorkloadArgs),
}

#[derive(Parser, Debug)]
struct BenchArgs {
    /// Sketch family (hll, kll, cms, countsketch, elastic, nitro, univmon).
    #[arg(long)]
    sketch: String,
    /// Implementation filter within the family. Accepts a single
    /// name (`oxide`), a comma list (`oxide,datasketches`), or
    /// `all` (the default). `list-impls` shows choices.
    #[arg(long = "impl", default_value = "all")]
    impl_name: String,
    /// Number of measured runs per `(impl, config)` pair.
    #[arg(long, default_value_t = 10)]
    runs: usize,
    /// Warm-up runs before measurement.
    #[arg(long, default_value_t = 3)]
    warmup_runs: usize,
    /// Workload shape: "uniform" or "zipf". Ignored when
    /// `--input` is set (the file replaces the generator).
    #[arg(long, default_value = "uniform")]
    workload: String,
    /// Number of items in the workload.
    #[arg(long, default_value_t = 1_000_000)]
    size: usize,
    /// Cardinality (uniform: max key; zipf: key-space size).
    #[arg(long, default_value_t = 100_000)]
    cardinality: u64,
    /// Zipf `s` exponent (only used when `--workload zipf`).
    #[arg(long, default_value_t = 1.1)]
    zipf_s: f64,
    /// Seed for reproducibility.
    #[arg(long, default_value_t = 42)]
    seed: u64,
    /// Load the workload from a file instead of generating it.
    /// Format is auto-detected from the extension:
    /// `.bin` (little-endian i64 stream), `.pcap` (IPv4 src
    /// addr per packet), `.csv` (first column parsed as i64
    /// after a header row). Overrides `--workload/--size/
    /// --cardinality/--zipf-s/--seed`.
    #[arg(long)]
    input: Option<String>,
    /// Generate the workload in-process from a `datagen` spec file
    /// (`.yaml`/`.yml`/`.json`, same format `workload generate --spec`
    /// takes; examples in `configs/datagen/`). Unlocks every generator
    /// shape — categorical id domains, monotonic timestamp series —
    /// without a round-trip through disk. Overrides
    /// `--workload/--size/--cardinality/--zipf-s/--seed`; `--input`
    /// wins over it.
    #[arg(long)]
    spec: Option<String>,
    /// Path to append JSONL records to. `-` or omitted → stdout.
    #[arg(long)]
    report: Option<String>,
    /// Optional output directory for legacy long-format CSVs (one
    /// row per measured run). Files are named
    /// `<family>_throughput_results_rust.csv` and, when a query
    /// phase runs, `<family>_throughput_query_results_rust.csv`.
    /// Coexists with `--report`; intended for plot scripts that
    /// still consume the historical CSV shape.
    #[arg(long)]
    raw_csv: Option<String>,
    /// Worker threads for parallel-insert impls
    /// (`lib-fastpath-parallel` under cms / countsketch / hll).
    /// Other impls ignore it. Default `1` reproduces the
    /// single-threaded behaviour.
    #[arg(long, default_value_t = 1)]
    workers: usize,
    /// Comma-separated metric flags: throughput,latency,cpu,memory,accuracy.
    /// Default: all.
    #[arg(long)]
    metrics: Option<String>,
    /// Sweep grid override. Format: `'k1=v1,v2 k2=v3,v4'`
    /// (whitespace separates keys; commas separate values).
    /// Example: `'rows=3,5 cols=1024,2048'` for cms. When
    /// omitted, the family's default grid is used.
    #[arg(long)]
    config: Option<String>,
    /// Compute ground-truth accuracy per run. Implies
    /// `MetricsMask::ACCURACY`. Per-family comparator: CMS /
    /// CountSketch / Elastic → frequency (L1/L2/rel-err p99);
    /// HLL → cardinality (rel-err); KLL → quantile rank-err.
    /// Nitro / UnivMon are ignored with a stderr note (their
    /// CLI-wrapper `query` is a stub).
    #[arg(long, default_value_t = false)]
    accuracy: bool,
    /// Cap on the number of distinct keys probed by the
    /// frequency comparator (CMS / CountSketch / Elastic). `0`
    /// = probe every distinct key. Ignored by cardinality /
    /// quantile comparators.
    #[arg(long, default_value_t = 100_000)]
    accuracy_probes: usize,
    /// Heavy-hitter threshold for the frequency comparator.
    /// Only keys whose true count is at least this value are
    /// included in the mean / p99 relative-error metric. `0` =
    /// no filter (probe every distinct key — the legacy
    /// behaviour). Setting this >0 reports the metric on the
    /// regime CMS / CountSketch are designed for; under heavy
    /// Zipf with many count-1 rare keys, the unfiltered mean is
    /// dominated by collision noise on those rare keys and is
    /// not what the sketch was meant to bound.
    #[arg(long, default_value_t = 0)]
    accuracy_min_count: u64,
}

fn parse_mask(s: Option<&str>) -> MetricsMask {
    let s = match s {
        Some(v) => v,
        None => return MetricsMask::all(),
    };
    let mut m = MetricsMask::empty();
    for token in s.split(',').map(|t| t.trim().to_ascii_lowercase()) {
        m |= match token.as_str() {
            "throughput" => MetricsMask::THROUGHPUT,
            "latency" => MetricsMask::LATENCY,
            "cpu" => MetricsMask::CPU,
            "memory" => MetricsMask::MEMORY,
            "accuracy" => MetricsMask::ACCURACY,
            "all" => MetricsMask::all(),
            "" => MetricsMask::empty(),
            other => {
                eprintln!("sketchlib: unknown metric flag '{other}', ignoring");
                MetricsMask::empty()
            }
        };
    }
    m
}

/// Resolve `--impl` into a concrete list of dispatch rows.
fn select_impls(family: &str, filter: &str) -> Result<Vec<&'static ImplEntry>> {
    let available = dispatch::impls_for_family(family);
    if available.is_empty() {
        bail!("unknown sketch family: {family}");
    }
    if filter == "all" {
        return Ok(available);
    }
    let wanted: Vec<&str> = filter.split(',').map(|s| s.trim()).collect();
    let mut out = Vec::new();
    for name in &wanted {
        match available.iter().find(|e| e.impl_name == *name) {
            Some(e) => out.push(*e),
            None => bail!("no impl '{name}' for family '{family}'"),
        }
    }
    Ok(out)
}

/// Open the `--report` destination. `None` or `"-"` → stdout.
enum ReportSink {
    Stdout,
    File(std::fs::File),
}

impl ReportSink {
    fn open(spec: Option<&str>) -> Result<Self> {
        match spec {
            None | Some("-") => Ok(ReportSink::Stdout),
            Some(path) => Ok(ReportSink::File(
                OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(path)?,
            )),
        }
    }
    fn write_line(&mut self, line: &str) -> Result<()> {
        match self {
            ReportSink::Stdout => {
                println!("{line}");
                Ok(())
            }
            ReportSink::File(f) => {
                f.write_all(line.as_bytes())?;
                f.write_all(b"\n")?;
                Ok(())
            }
        }
    }
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Cmd::ListImpls => {
            println!("# family       impl                         description");
            for line in dispatch::list() {
                println!("{line}");
            }
            Ok(())
        }
        Cmd::Bench(args) => run_bench(args),
        Cmd::Workload(args) => workload_cmd::run(args),
    }
}

/// Resolve where this run's items come from, in precedence order:
/// `--input` (a file on disk) > `--spec` (a full generator spec) >
/// the `--workload` flags.
///
/// The flag path builds the same `GenSpec` the spec path would, so
/// `--workload zipf --cardinality N --zipf-s S` is exactly sugar for a
/// `keys`/`zipf` spec — one generator, not two.
fn workload_spec(args: &BenchArgs) -> Result<WorkloadSpec> {
    if let Some(path) = args.input.as_deref() {
        return Ok(WorkloadSpec::File {
            path: path.to_string(),
        });
    }
    if let Some(path) = args.spec.as_deref() {
        let spec = GenSpec::from_path(std::path::Path::new(path))
            .map_err(|e| anyhow::anyhow!("loading spec from {path}: {e}"))?;
        return Ok(WorkloadSpec::Generated(spec));
    }
    let dist = match args.workload.as_str() {
        "uniform" => Distribution::Uniform,
        "zipf" => Distribution::Zipf { s: args.zipf_s },
        other => bail!("unknown workload shape: {other} (expected uniform|zipf, or use --spec)"),
    };
    Ok(WorkloadSpec::Generated(GenSpec {
        shape: Shape::Keys {
            cardinality: args.cardinality,
            dist,
            dtype: DType::I64,
        },
        size: args.size,
        seed: args.seed,
    }))
}

fn run_bench(args: BenchArgs) -> Result<()> {
    let spec = workload_spec(&args)?;
    let mut metrics_mask = parse_mask(args.metrics.as_deref());
    if args.accuracy {
        // --accuracy implies the accuracy mask bit, regardless of
        // what --metrics said. Otherwise the runner would build the
        // GT but silently drop its output.
        metrics_mask |= MetricsMask::ACCURACY;
    }
    let cfg = BenchConfig {
        runs: args.runs,
        warmup_runs: args.warmup_runs,
        metrics: metrics_mask,
        query_count: None,
        threads: args.workers.max(1),
        seed: args.seed,
    };
    let accuracy_cfg = AccuracyCfg {
        enabled: args.accuracy,
        max_probes: args.accuracy_probes,
        min_true_count: args.accuracy_min_count,
        // Per-call CSV (hll/kll/dd) is only emittable when both
        // `--raw-csv` and `--accuracy` are on: the comparator is
        // what owns the query phase + per-call instrumentation.
        record_query_calls: args.accuracy && args.raw_csv.is_some(),
    };

    let impls = select_impls(&args.sketch, &args.impl_name)?;
    let grid = match args.config.as_deref() {
        Some(s) => sweep::parse_config(&args.sketch, s)?,
        None => sweep::default_grid(&args.sketch)?,
    };
    if grid.is_empty() {
        bail!("empty config grid for family '{}'", args.sketch);
    }

    // Build the workload once — it's shared across all (impl, config) pairs.
    let workload = spec.build_i64()?;

    // Unparameterized impls (e.g. exact baselines) run once per
    // sweep, not once per config.
    let total: usize = impls
        .iter()
        .map(|e| {
            if e.constraint.is_unparameterized() {
                1
            } else {
                grid.len()
            }
        })
        .sum();
    eprintln!(
        "sketchlib: {} family={} impls=[{}] configs={} total={}",
        "sweep",
        args.sketch,
        impls
            .iter()
            .map(|e| e.impl_name)
            .collect::<Vec<_>>()
            .join(","),
        grid.len(),
        total,
    );

    let mut sink = ReportSink::open(args.report.as_deref())?;
    let mut emitted = 0usize;
    let mut skipped = 0usize;

    // One-time warning per impl when --accuracy is on but the
    // family has no viable comparator — avoids a stderr line per
    // config in a big sweep.
    if accuracy_cfg.enabled {
        for entry in &impls {
            if entry.accuracy_kind == AccuracyKind::None {
                eprintln!(
                    "sketchlib: --accuracy has no comparator for {}/{} (wrapper query is a stub) — running without ground truth",
                    entry.family, entry.impl_name
                );
            }
        }
    }

    for (idx_cfg, params) in grid.iter().enumerate() {
        for entry in &impls {
            // Unparameterized impls run once per sweep — skip all
            // configs after the first.
            if entry.constraint.is_unparameterized() && idx_cfg > 0 {
                continue;
            }
            if !entry.accepts(params) {
                eprintln!(
                    "sketchlib: skip {}/{} — {} does not match {:?}",
                    entry.family,
                    entry.impl_name,
                    entry.constraint.describe(),
                    params,
                );
                skipped += 1;
                continue;
            }
            emitted += 1;
            let cfg_label = if entry.constraint.is_unparameterized() {
                "exact".to_string()
            } else {
                params_pretty(params)
            };
            eprintln!(
                "sketchlib: [{emitted}/{total}] {}/{} config={} runs={} warmup={}",
                entry.family, entry.impl_name, cfg_label, cfg.runs, cfg.warmup_runs,
            );
            // Each `entry.run` call returns one report per metric
            // pass (see `MetricsMask::passes()`); emit each on its
            // own JSONL line and its own CSV row group. Downstream
            // group-by on (sketch, impl, sketch_config, workload)
            // merges them back.
            let reports = entry.run(&cfg, &workload, params, &accuracy_cfg);
            for report in &reports {
                if let Some(dir) = args.raw_csv.as_deref() {
                    let params_opt = if entry.constraint.is_unparameterized() {
                        None
                    } else {
                        Some(params)
                    };
                    raw_csv::write_runs(
                        std::path::Path::new(dir),
                        entry,
                        params_opt,
                        cfg.seed,
                        cfg.threads,
                        report,
                    )?;
                }
                let mut record = report.to_record();
                record.sketch_config = if entry.constraint.is_unparameterized() {
                    None
                } else {
                    Some(params.to_json_value())
                };
                sink.write_line(&record.to_jsonl())?;
            }
        }
    }

    eprintln!(
        "sketchlib: done. emitted={emitted} skipped={skipped} total_planned={total}",
    );
    Ok(())
}

fn params_pretty(p: &sketch_core::config::ParamSet) -> String {
    // Strip quotes around the params sub-object for tighter logs.
    let v = p.to_json_value();
    v.get("params")
        .map(|x| x.to_string())
        .unwrap_or_else(|| v.to_string())
}
