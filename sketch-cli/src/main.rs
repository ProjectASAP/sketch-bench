//! `sketchlib` — unified CLI for sketchlib-tool.
//!
//! `bench` runs a family of sketches across a config grid —
//! see `docs/BENCH_SWEEP.md`. `list-impls` enumerates registered
//! `(family, impl)` pairs.

mod dispatch;
mod params;
mod sweep;
mod wrappers;

use std::fs::OpenOptions;
use std::io::Write;

use anyhow::{bail, Result};
use clap::{Parser, Subcommand};
use sketch_bench::{BenchConfig, MetricsMask};

use dispatch::{ImplEntry, WorkloadSpec};

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
    /// Workload shape: "uniform" or "zipf".
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
    /// Path to append JSONL records to. `-` or omitted → stdout.
    #[arg(long)]
    report: Option<String>,
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
    }
}

fn run_bench(args: BenchArgs) -> Result<()> {
    let spec = match args.workload.as_str() {
        "uniform" => WorkloadSpec::Uniform {
            size: args.size,
            cardinality: args.cardinality,
            seed: args.seed,
        },
        "zipf" => WorkloadSpec::Zipf {
            size: args.size,
            cardinality: args.cardinality,
            s: args.zipf_s,
            seed: args.seed,
        },
        other => bail!("unknown workload shape: {other} (expected uniform|zipf)"),
    };
    let cfg = BenchConfig {
        runs: args.runs,
        warmup_runs: args.warmup_runs,
        metrics: parse_mask(args.metrics.as_deref()),
        query_count: None,
        threads: 1,
        seed: args.seed,
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

    let total = impls.len() * grid.len();
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

    for (idx_cfg, params) in grid.iter().enumerate() {
        for entry in &impls {
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
            eprintln!(
                "sketchlib: [{emitted}/{total}] {}/{} config={} runs={} warmup={}",
                entry.family,
                entry.impl_name,
                params_pretty(params),
                cfg.runs,
                cfg.warmup_runs,
            );
            let report = entry.run(&cfg, &workload, params);
            let mut record = report.to_record();
            record.sketch_config = Some(params.to_json_value());
            sink.write_line(&record.to_jsonl())?;
        }
        // Unused but keeps clippy quiet about the index.
        let _ = idx_cfg;
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
