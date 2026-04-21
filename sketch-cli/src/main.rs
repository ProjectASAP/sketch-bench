//! `sketchlib` — unified CLI for sketchlib-tool (`bench` /
//! `profile` / `workload` subcommands, per docs/DESIGN.md §6).
//!
//! v1 covers `bench` + `list-impls`. `profile` and `workload
//! generate/describe` are tracked as TODOs (sketch-profile /
//! MERGE_PLAN Phase 5).

mod dispatch;
mod params;
mod wrappers;

use anyhow::Result;
use clap::{Parser, Subcommand};
use sketch_bench::{BenchConfig, MetricsMask};

use dispatch::WorkloadSpec;

#[derive(Parser, Debug)]
#[command(name = "sketchlib", version, about = "Unified sketchlib-tool CLI")]
struct Cli {
    #[command(subcommand)]
    command: Cmd,
}

#[derive(Subcommand, Debug)]
enum Cmd {
    /// Run a benchmark against a registered sketch impl.
    Bench(BenchArgs),
    /// List every `(family, impl)` pair the CLI can drive.
    ListImpls,
}

#[derive(Parser, Debug)]
struct BenchArgs {
    /// Sketch family (hll, kll, cms, countsketch, elastic, nitro, univmon).
    #[arg(long)]
    sketch: String,
    /// Implementation within that family. Use `list-impls` to see choices.
    #[arg(long = "impl")]
    impl_name: String,
    /// Number of measured runs (excludes warmup).
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
    /// Path to write the v1 JSONL record. `-` or omitted → stdout.
    #[arg(long)]
    report: Option<String>,
    /// Comma-separated metric flags: throughput,latency,cpu,memory,accuracy.
    /// Default: all.
    #[arg(long)]
    metrics: Option<String>,
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
        Cmd::Bench(args) => {
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
                other => anyhow::bail!("unknown workload shape: {other} (expected uniform|zipf)"),
            };
            let cfg = BenchConfig {
                runs: args.runs,
                warmup_runs: args.warmup_runs,
                metrics: parse_mask(args.metrics.as_deref()),
                query_count: None,
                threads: 1,
                seed: args.seed,
            };

            let report = dispatch::run(&args.sketch, &args.impl_name, &cfg, spec)?;
            let jsonl = report.to_jsonl();
            match args.report.as_deref() {
                None | Some("-") => {
                    println!("{jsonl}");
                }
                Some(path) => {
                    std::fs::write(path, format!("{jsonl}\n"))?;
                }
            }
            Ok(())
        }
    }
}
