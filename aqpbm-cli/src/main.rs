//! `sketchlib` — unified CLI for sketchlib-tool.
//!
//! `bench` measures one `(impl, config)` cell of a sketch family.
//! `list-impls` enumerates registered `(family, impl)` pairs.

mod dispatch;
mod raw_csv;
mod repeat;
mod workload_cmd;

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
static GLOBAL: sketch_bench::metrics::heap_track::TrackingAllocator<tikv_jemallocator::Jemalloc> =
    sketch_bench::metrics::heap_track::TrackingAllocator(tikv_jemallocator::Jemalloc);

#[cfg(all(feature = "heap-track", not(feature = "heap-jemalloc")))]
#[global_allocator]
static GLOBAL: sketch_bench::metrics::heap_track::TrackingAllocator<std::alloc::System> =
    sketch_bench::metrics::heap_track::TrackingAllocator(std::alloc::System);

use std::fs::OpenOptions;
use std::io::Write;

use anyhow::{bail, Result};
use aqpbm_datagen::{DType, Distribution, GenSpec, Shape};
use clap::{Parser, Subcommand};
use sketch_bench::params::ParamSet;
use sketch_bench::{BenchConfig, MetricsMask};

use dispatch::{AccuracyCfg, AccuracyKind, ImplEntry, WorkloadSpec};

#[derive(Parser, Debug)]
#[command(name = "sketchlib", version, about = "Unified sketchlib-tool CLI")]
struct Cli {
    #[command(subcommand)]
    command: Cmd,
}

#[derive(Subcommand, Debug)]
enum Cmd {
    /// Measure one `(impl, config)` cell of a sketch family.
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
    /// Implementation within the family — exactly one (`oxide`).
    /// `list-impls` shows the choices. One invocation measures one
    /// (impl, config) cell; to race several, invoke once per impl.
    #[arg(long = "impl")]
    impl_name: String,
    /// Number of measured iterations per `(impl, config)` pair, inside one
    /// process. Summarised as mean / stddev / `throughput_samples`. These
    /// iterations share a process, so they do **not** support a confidence
    /// interval — see `--repeats`.
    #[arg(long, default_value_t = 10)]
    runs: usize,
    /// Re-execute the whole benchmark in this many **separate processes** and
    /// report the 95% confidence interval over their per-process means.
    ///
    /// This is the only setting that makes `ci95` appear in the output: a
    /// fresh process is what varies the allocator arena, address-space layout,
    /// governor ramp and page-cache state that `--runs` holds constant. Left
    /// at 1 (the default) no interval is claimed, because none can be computed
    /// honestly. Costs R times the wall clock.
    #[arg(long, default_value_t = 1)]
    repeats: usize,
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
    /// Item type the sketches ingest: `i64` (default), `f64`, or `string`.
    ///
    /// `f64` runs the ordered families (`kll`, `dd`); every hash-based row is
    /// skipped with a reason, because `f64` is not `Hash` in Rust and hashing
    /// its bits would repeat the `i64` curve.
    ///
    /// `string` runs the rows whose wrappers take text (`elastic`, `nitro`,
    /// `univmon`) over **generated** strings — configurable alphabet, varying
    /// length. Those same rows run under `i64` too, but there they consume
    /// decimal-formatted integers, which is a different and much narrower
    /// workload. Comparing the two is the point.
    ///
    /// For `i64` and `f64` the values themselves do not change, only their
    /// encoding. `string` is the exception: the rank is rendered rather than
    /// cast, so the byte content is genuinely new.
    #[arg(long, default_value = "i64")]
    dtype: String,
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
    /// Split the stream into this many shards, build one sketch per shard,
    /// and time folding them into one — then compare the merged result
    /// against the whole stream. `1` (default) skips the merge pass.
    ///
    /// Mergeability is what lets a sketch be computed per shard, per node or
    /// per time window and combined later, and it is close to unmeasured in
    /// the literature: papers prove it and then evaluate insert and query.
    /// For linear sketches (Count-Min, Count Sketch, HLL at equal lg_k) the
    /// merge is exact, so accuracy here must match the single-pass figure and
    /// a gap is a defect. For KLL it is lossy, and the gap is the result.
    #[arg(long, default_value_t = 1)]
    merge_shards: usize,
    /// Comma-separated metric flags: throughput,latency,cpu,memory,accuracy,merge.
    /// Default: all except merge (merge needs `--merge-shards`).
    #[arg(long)]
    metrics: Option<String>,
    /// Construction config for this cell: `'k1=v1 k2=v2'`, one value
    /// per key (e.g. `'rows=5 cols=2048'` for cms). Omitted → a
    /// parameterless point, which impls with tunable knobs reject at
    /// construction (naming the missing field). A comma list of values
    /// is an error: one invocation is one cell, not a grid.
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
            "merge" => MetricsMask::MERGE,
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

/// Resolve `--sketch`/`--impl` to the one dispatch row they name.
fn select_impl(family: &str, impl_name: &str) -> Result<&'static ImplEntry> {
    if dispatch::impls_for_family(family).is_empty() {
        bail!("unknown sketch family: {family}");
    }
    dispatch::find(family, impl_name)
        .ok_or_else(|| anyhow::anyhow!("no impl '{impl_name}' for family '{family}'"))
}

/// Open the `--report` destination. `None` or `"-"` → stdout.
enum ReportSink {
    Stdout,
    File(std::fs::File),
}

impl ReportSink {
    fn open(spec: Option<&str>) -> Result<Self> {
        // A repeat child always writes to stdout: the parent captures it and
        // owns the real `--report` destination. Otherwise each child would
        // also append its own unmerged records to that file.
        if repeat::is_child() {
            return Ok(ReportSink::Stdout);
        }
        match spec {
            None | Some("-") => Ok(ReportSink::Stdout),
            Some(path) => Ok(ReportSink::File(
                OpenOptions::new().create(true).append(true).open(path)?,
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
fn workload_spec(args: &BenchArgs, dtype: DType) -> Result<WorkloadSpec> {
    if let Some(path) = args.input.as_deref() {
        return Ok(WorkloadSpec::File {
            path: path.to_string(),
        });
    }
    if let Some(path) = args.spec.as_deref() {
        let spec = GenSpec::from_path(std::path::Path::new(path))
            .map_err(|e| anyhow::anyhow!("loading spec from {path}: {e}"))?;
        // A spec file names its own dtype. Silently overriding it would edit
        // the user's file from the command line; silently ignoring `--dtype`
        // would run a different measurement than the one asked for. Neither is
        // recoverable from the output, so disagreeing is an error.
        if spec.dtype != dtype {
            bail!(
                "--dtype {} but {path} generates {}; drop --dtype or edit the spec",
                dtype.as_str(),
                spec.dtype.as_str(),
            );
        }
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
        },
        size: args.size,
        seed: args.seed,
        dtype,
        string: None,
    }))
}

/// Seconds of CPU burn before the first measured loop, so the cpufreq
/// governor is at max turbo when timing starts. This default lives here
/// rather than in `sketch-bench` because only a measurement run wants it:
/// a test binary or an embedding application that links the runner should
/// not pay ten seconds of spin merely for linking it.
const DEFAULT_WARMUP_SECS: &str = "10";

fn run_bench(args: BenchArgs) -> Result<()> {
    if args.repeats == 0 {
        bail!("--repeats must be >= 1");
    }
    // Parent role: spawn the repeats, merge, emit. A child (marked by the
    // env var) falls through and runs the measurement itself.
    if args.repeats > 1 && !repeat::is_child() {
        if args.raw_csv.is_some() {
            bail!(
                "--raw-csv cannot be combined with --repeats: the legacy CSV shape has no \
                 repeat column, so every repeat would append indistinguishable rows"
            );
        }
        let records = repeat::run_repeats(args.repeats)?;
        let mut sink = ReportSink::open(args.report.as_deref())?;
        for r in &records {
            sink.write_line(&r.to_jsonl())?;
        }
        eprintln!(
            "sketchlib: merged {} repeats into {} record(s)",
            args.repeats,
            records.len()
        );
        return Ok(());
    }
    if std::env::var_os("BENCH_WARMUP_SECS").is_none() {
        // SAFETY-equivalent note: single-threaded, before any bench thread
        // is spawned, and only when the operator has not chosen a value.
        std::env::set_var("BENCH_WARMUP_SECS", DEFAULT_WARMUP_SECS);
    }
    let dtype = match args.dtype.as_str() {
        "i64" => DType::I64,
        "f64" => DType::F64,
        "string" => DType::Str,
        other => bail!("unknown --dtype: {other} (expected i64|f64|string)"),
    };
    let spec = workload_spec(&args, dtype)?;
    let mut metrics_mask = parse_mask(args.metrics.as_deref());
    if args.merge_shards > 1 {
        metrics_mask |= MetricsMask::MERGE;
    }
    // No `else` clearing the bit: `BenchRunner::run` already skips a merge
    // pass with fewer than two shards, so one guard covers CLI and library
    // callers alike.
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
        merge_shards: args.merge_shards,
        seed: args.seed,
    };
    let accuracy_cfg = AccuracyCfg {
        enabled: args.accuracy,
        max_probes: args.accuracy_probes,
        // Per-call CSV (hll/kll/dd) is only emittable when both
        // `--raw-csv` and `--accuracy` are on: the comparator is
        // what owns the query phase + per-call instrumentation.
        record_query_calls: args.accuracy && args.raw_csv.is_some(),
    };

    let entry = select_impl(&args.sketch, &args.impl_name)?;
    // One cell = one (impl, config). `--config` is one point, or a
    // parameterless point when omitted; keys are type-checked at
    // construction, where the impl reads them.
    let params = match args.config.as_deref() {
        Some(s) => dispatch::config_point(&args.sketch, s)?,
        None => ParamSet::empty(&args.sketch),
    };

    let workload = spec.build(dtype)?;

    if accuracy_cfg.enabled && entry.accuracy_kind == AccuracyKind::None {
        eprintln!(
            "sketchlib: --accuracy has no comparator for {}/{} (wrapper query is a stub) — running without ground truth",
            entry.family, entry.impl_name
        );
    }

    eprintln!(
        "sketchlib: {}/{} config={} runs={} warmup={}",
        entry.family,
        entry.impl_name,
        params_pretty(&params),
        cfg.runs,
        cfg.warmup_runs,
    );

    // Whether this cell can run is decided at construction: a wrong dtype, a
    // fixed shape the config misses, or a params struct missing a value all
    // surface here as an error — the tool ran exactly what it was asked and
    // that one thing could not run, so it fails rather than skipping on.
    let reports = entry
        .run(&cfg, &workload, &params, &accuracy_cfg)
        .map_err(|e| anyhow::anyhow!("{}/{} cannot run: {e}", entry.family, entry.impl_name))?;

    // Each `entry.run` call returns one report per metric pass (see
    // `MetricsMask::passes()`); emit each on its own JSONL line and its own
    // CSV row group. Downstream group-by on (sketch, impl, sketch_config,
    // workload) merges them back.
    let mut sink = ReportSink::open(args.report.as_deref())?;
    for report in &reports {
        if let Some(dir) = args.raw_csv.as_deref() {
            raw_csv::write_runs(
                std::path::Path::new(dir),
                entry,
                Some(&params),
                cfg.seed,
                cfg.threads,
                dtype,
                report,
            )?;
        }
        let mut record = report.to_record();
        record.sketch_config = Some(params.to_json_value());
        sink.write_line(&record.to_jsonl())?;
    }
    Ok(())
}

fn params_pretty(p: &sketch_bench::params::ParamSet) -> String {
    // Strip quotes around the params sub-object for tighter logs.
    let v = p.to_json_value();
    v.get("params")
        .map(|x| x.to_string())
        .unwrap_or_else(|| v.to_string())
}
