//! `approxbench`, the approximate query processing benchmark suite.
//!
//! `sketchbench` measures one cell of the sketch bundle, `sketchbench
//! --list-impls` enumerates that bundle's `(algorithm, impl)` pairs, and
//! `workload` generates or inspects synthetic `.bin` workloads.

mod cli;
mod flatten_record;
mod raw_csv;
mod repeat;
mod workload_cmd;

// Global allocator selection across the `heap-jemalloc` / `heap-track` feature
// pair: bare jemalloc, `TrackingAllocator` wrapping jemalloc or System, or the
// implicit System allocator. The static is what `#[global_allocator]` needs.
#[cfg(all(feature = "heap-jemalloc", not(feature = "heap-track")))]
#[global_allocator]
static GLOBAL: tikv_jemallocator::Jemalloc = tikv_jemallocator::Jemalloc;

#[cfg(all(feature = "heap-jemalloc", feature = "heap-track"))]
#[global_allocator]
static GLOBAL: aqpbm_core::metrics::heap_track::TrackingAllocator<tikv_jemallocator::Jemalloc> =
    aqpbm_core::metrics::heap_track::TrackingAllocator(tikv_jemallocator::Jemalloc);

#[cfg(all(feature = "heap-track", not(feature = "heap-jemalloc")))]
#[global_allocator]
static GLOBAL: aqpbm_core::metrics::heap_track::TrackingAllocator<std::alloc::System> =
    aqpbm_core::metrics::heap_track::TrackingAllocator(std::alloc::System);

use std::fs::OpenOptions;
use std::io::Write;

use anyhow::{bail, Result};
use aqpbm_core::metrics::MetricsMask;
use aqpbm_core::runner::BenchConfig;
use aqpbm_datagen::{Distribution, GenSpec, Shape, StringOpts};
use clap::Parser;
use sketch_bench::params::ParamSet;

use cli::{Cli, Cmd, SketchbenchArgs};
// The catalog — which sketches exist, how to build them, which ground-truth calculator scores
// them — is sketch-domain knowledge and lives in `sketch-bench`. The CLI does
// not know the set; it asks.
use aqpbm_core::cell::{AccuracyCfg, WorkloadSpec};
use sketch_bench::catalog;

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
                eprintln!("approxbench: unknown metric flag '{other}', ignoring");
                MetricsMask::empty()
            }
        };
    }
    m
}

/// Validate `--algorithm`/`--impl` and report whether `--accuracy` can score it.
fn select_impl(algorithm: &str, impl_name: &str) -> Result<bool> {
    if !catalog::algorithm_exists(algorithm) {
        bail!("unknown algorithm: {algorithm}");
    }
    catalog::scores_accuracy(algorithm, impl_name)
        .ok_or_else(|| anyhow::anyhow!("no impl '{impl_name}' for algorithm '{algorithm}'"))
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
        Cmd::Sketchbench(args) => run_sketchbench(args),
        Cmd::Workload(args) => workload_cmd::run(args),
    }
}

/// `--list-impls` prints and exits. Enumerating a bundle's catalog hangs off
/// that bundle's subcommand, since a second bundle would make a free-standing
/// `list-impls` ambiguous about whose catalog it means.
fn list_impls() -> Result<()> {
    println!("# algorithm       impl                         description");
    for line in catalog::list() {
        println!("{line}");
    }
    Ok(())
}

/// Load a `--spec` file as either one column spec or a list of them.
///
/// The two shapes are unambiguous, since a mapping is never a sequence, so the
/// file itself picks and no second flag has to. A list is `n - 1` label columns
/// then one value column, which is what the record-ingesting rows read; every
/// other row refuses it by name.
fn load_spec(path: &str) -> Result<WorkloadSpec> {
    let p = std::path::Path::new(path);
    let text =
        std::fs::read_to_string(p).map_err(|e| anyhow::anyhow!("loading spec from {path}: {e}"))?;
    let yaml = matches!(
        p.extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_ascii_lowercase())
            .as_deref(),
        Some("yaml") | Some("yml")
    );
    if yaml {
        if let Ok(columns) = serde_yaml::from_str::<Vec<GenSpec>>(&text) {
            return Ok(WorkloadSpec::Columns(columns));
        }
        return serde_yaml::from_str::<GenSpec>(&text)
            .map(WorkloadSpec::Generated)
            .map_err(|e| anyhow::anyhow!("spec yaml {path}: {e}"));
    }
    if let Ok(columns) = serde_json::from_str::<Vec<GenSpec>>(&text) {
        return Ok(WorkloadSpec::Columns(columns));
    }
    serde_json::from_str::<GenSpec>(&text)
        .map(WorkloadSpec::Generated)
        .map_err(|e| anyhow::anyhow!("spec json {path}: {e}"))
}

/// Resolve where this run's items come from, in precedence order: `--input` >
/// `--spec` > the `--workload` flags. The flag path builds the same `GenSpec`
/// the spec path would, so it is sugar for a `keys`/`zipf` spec — one generator.
fn workload_spec(args: &SketchbenchArgs) -> Result<WorkloadSpec> {
    if let Some(path) = args.input.as_deref() {
        return Ok(WorkloadSpec::File {
            path: path.to_string(),
        });
    }
    if let Some(path) = args.spec.as_deref() {
        // A spec carries its own `string:` block, so `--alphabet`/`--key-len`
        // would be editing the user's file from the command line.
        return load_spec(path);
    }
    let dist = match args.workload.as_str() {
        "uniform" => Distribution::Uniform,
        "zipf" => Distribution::Zipf { s: args.zipf_s },
        other => bail!("unknown workload shape: {other} (expected uniform|zipf, or use --spec)"),
    };
    // Only the rows that ingest text read this. Left `None` at the defaults
    // so a run that did not ask for the axis keeps the descriptor — and so
    // the record — it had before the flags existed.
    let (min_len, max_len) = match args.key_len.as_slice() {
        [n] => (*n, *n),
        [lo, hi] => (*lo, *hi),
        _ => bail!("--key-len takes one value (fixed) or two (min max)"),
    };
    if min_len == 0 || min_len > max_len {
        bail!("--key-len must be non-zero and non-decreasing, got {min_len}..={max_len}");
    }
    if args.alphabet.is_empty() {
        bail!("--alphabet cannot be empty");
    }
    Ok(WorkloadSpec::Generated(GenSpec {
        shape: Shape::Keys {
            cardinality: args.cardinality,
            dist,
        },
        size: args.size,
        seed: args.seed,
        string: {
            let opts = StringOpts {
                alphabet: args.alphabet.clone(),
                min_len,
                max_len,
            };
            (opts != StringOpts::default()).then_some(opts)
        },
    }))
}

/// Seconds of CPU burn before the first measured loop, so the cpufreq governor
/// is at max turbo when timing starts. The default lives here because only a
/// measurement run wants it — linking the runner should not cost ten seconds.
const DEFAULT_WARMUP_SECS: &str = "10";

fn run_sketchbench(args: SketchbenchArgs) -> Result<()> {
    // Selects no cell and writes no record, so it runs before anything is
    // validated and ignores every other option.
    if args.list_impls {
        return list_impls();
    }
    // `required_unless_present = "list_impls"` on both, so clap has already
    // rejected the invocation that reaches here without them.
    let (algorithm, impl_name) = match (args.algorithm.as_deref(), args.impl_name.as_deref()) {
        (Some(a), Some(i)) => (a.to_string(), i.to_string()),
        _ => bail!("--algorithm and --impl are both required unless --list-impls is given"),
    };
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
            "approxbench: merged {} repeats into {} record(s)",
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
    // The only item-type choice left: an `ordered` row builds at either width.
    // Every other row's type is fixed by its Rust type, and `catalog::run`
    // refuses a width it cannot honour before anything is generated.
    let width = match args.dtype.as_str() {
        "i64" => catalog::Numeric::I64,
        "f64" => catalog::Numeric::F64,
        other => bail!("unknown --dtype: {other} (expected i64|f64)"),
    };
    let spec = workload_spec(&args)?;
    let mut metrics_mask = parse_mask(args.metrics.as_deref());
    if args.merge_shards > 1 {
        metrics_mask |= MetricsMask::MERGE;
    }
    // No `else` clearing the bit: the runner already skips a merge pass with
    // fewer than two shards, so one guard covers CLI and library callers
    // alike.
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

    let scores_accuracy = select_impl(&algorithm, &impl_name)?;
    // One cell = one (impl, config). `--config` is one point, or a
    // parameterless point when omitted; keys are type-checked at
    // construction, where the impl reads them.
    let params = match args.config.as_deref() {
        Some(s) => catalog::config_point(&algorithm, s)?,
        None => ParamSet::empty(&algorithm),
    };

    if accuracy_cfg.enabled && !scores_accuracy {
        eprintln!(
            "approxbench: --accuracy has no comparator for {algorithm}/{impl_name} (throughput-only row) — running without ground truth"
        );
    }

    eprintln!(
        "approxbench: {}/{} config={} runs={} warmup={}",
        algorithm,
        impl_name,
        params_pretty(&params),
        cfg.runs,
        cfg.warmup_runs,
    );

    // Whether this cell can run is decided at construction: a wrong dtype, a
    // missed fixed shape, or a missing param all surface here. The tool ran
    // exactly what it was asked, so it fails rather than skipping on.
    let reports = catalog::run(
        &algorithm,
        &impl_name,
        &cfg,
        &spec,
        &params,
        &accuracy_cfg,
        width,
    )
    .map_err(|e| anyhow::anyhow!("{algorithm}/{impl_name} cannot run: {e}"))?;

    // `catalog::run` returns one report per metric pass; emit each on its own
    // JSONL line and CSV row group. A downstream group-by on
    // (sketch, impl, sketch_config, workload) merges them back.
    let mut sink = ReportSink::open(args.report.as_deref())?;
    for report in &reports {
        if let Some(dir) = args.raw_csv.as_deref() {
            raw_csv::write_runs(
                std::path::Path::new(dir),
                &algorithm,
                &impl_name,
                Some(&params),
                cfg.seed,
                cfg.threads,
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
