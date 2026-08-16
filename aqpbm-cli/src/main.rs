//! `approxbench`, the approximate query processing benchmark suite.
//!
//! `sketchbench` measures one cell of the sketch bundle, `sketchbench
//! --list-impls` enumerates that bundle's `(algorithm, impl)` pairs, and
//! `workload` generates or inspects synthetic `.bin` workloads.

mod cli;
mod flatten_record;
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
use aqpbm_core::measure::MeasureConfig;
use aqpbm_core::metrics::{MetricsMask, OperationMask};
use aqpbm_datagen::{
    ColumnSpec, DataDistribution, StringOpts, TableDescription, UniformParameter, ZipfParameter,
    RULE_NONE,
};
use clap::Parser;
use sketch_bench::params::ParamSet;

use cli::{Cli, Cmd, SketchbenchArgs};
// The registry — which sketches exist, how to build them, which ground-truth calculator scores
// them — is sketch-domain knowledge and lives in `sketch-bench`. The CLI does
// not know the set; it asks.
use aqpbm_core::cell::WorkloadSpec;
use aqpbm_core::request::Requirement;
use sketch_bench::registry;

/// What is measured. No default and no `all`: a request says which squares of
/// the grid it wants, and a shorthand that sweeps the grid would sweep squares
/// nothing measures.
fn parse_mask(s: &str) -> Result<MetricsMask> {
    let mut m = MetricsMask::empty();
    for token in s.split(',').map(|t| t.trim().to_ascii_lowercase()) {
        m |= match token.as_str() {
            "throughput" => MetricsMask::THROUGHPUT,
            "latency" => MetricsMask::LATENCY,
            "cpu" => MetricsMask::CPU,
            "memory" => MetricsMask::MEMORY,
            "accuracy" => MetricsMask::ACCURACY,
            "" => MetricsMask::empty(),
            // Refused, not warned past: a misspelling that measured nothing
            // and exited zero looks to a driver script like a run that
            // produced no data.
            other => bail!(
                "unknown metric '{other}'; --metrics takes throughput, latency, accuracy, cpu, memory"
            ),
        };
    }
    Ok(m)
}

/// Which operations the metrics are taken over. No default, for the same
/// reason as the metrics: nothing is measured that was not asked for.
fn parse_operations(s: &str) -> Result<OperationMask> {
    let mut m = OperationMask::empty();
    for token in s.split(',').map(|t| t.trim().to_ascii_lowercase()) {
        m |= match token.as_str() {
            "insert" => OperationMask::INSERT,
            "query" => OperationMask::QUERY,
            "merge" => OperationMask::MERGE,
            "prepare" => OperationMask::PREPARE,
            "" => OperationMask::empty(),
            other => bail!(
                "unknown operation '{other}'; --operations takes insert, query, merge, prepare"
            ),
        };
    }
    Ok(m)
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

/// `--list-impls` prints and exits. Enumerating a bundle's registry hangs off
/// that bundle's subcommand, since a second bundle would make a free-standing
/// `list-impls` ambiguous about whose registry it means.
fn list_impls() -> Result<()> {
    // The registry owns the header too: it is the one place that knows how wide
    // the algorithm column has to be for the rows underneath it.
    for line in registry::list() {
        println!("{line}");
    }
    Ok(())
}

/// Load a `--spec` file as one [`TableDescription`].
///
/// One reader, because one description covers both cases: a one-column table is
/// what a plain row ingests, and a table with a label column before its value
/// column is what the record-ingesting rows read. Which a row wants is the row's
/// question, asked at materialisation.
fn load_spec(path: &str) -> Result<WorkloadSpec> {
    TableDescription::from_path(std::path::Path::new(path))
        .map(WorkloadSpec::Generated)
        .map_err(|e| anyhow::anyhow!("loading spec from {path}: {e}"))
}

/// Resolve where this run's items come from, in precedence order: `--input` >
/// `--spec` > the `--workload` flags. The flag path builds the same
/// `TableDescription` the spec path would, so it is sugar for a one-column
/// description — one generator.
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
    // `--cardinality` names the key space either way: for uniform it is the
    // exclusive upper bound of `[0, n)`, and for zipf the population its ranks
    // `1..=n` are drawn over.
    let distribution = match args.workload.as_str() {
        "uniform" => DataDistribution::Uniform(UniformParameter {
            lower_bound: 0.0,
            upper_bound: args.cardinality as f64,
            seed: args.seed,
        }),
        "zipf" => DataDistribution::Zipf(ZipfParameter {
            skewness: args.zipf_s,
            population_size: args.cardinality,
            seed: args.seed,
        }),
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
    // `Inline`, not `Generated`: these flags name a distribution and a size but
    // no type, so the row's item type is what fills `data_type` in. The
    // placeholder below is never the one that generates.
    Ok(WorkloadSpec::Inline(TableDescription::single(
        "key",
        ColumnSpec {
            distribution,
            shift: None,
            cardinality: None,
            special_rule: RULE_NONE,
            data_type: "i64".into(),
            string: {
                let opts = StringOpts {
                    alphabet: args.alphabet.clone(),
                    min_len,
                    max_len,
                };
                (opts != StringOpts::default()).then_some(opts)
            },
        },
        args.size as u64,
    )))
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
        if args.flat {
            bail!(
                "--flat cannot be combined with --repeats: a flattened row holds one value \
                 per square, and folding the repeats into it would have to decide which \
                 repeat that value came from"
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
    // Every other row's type is fixed by its Rust type, and `registry::resolve`
    // refuses a width it cannot honour before anything is generated.
    let width = match args.dtype.as_str() {
        "i64" => registry::Numeric::I64,
        "f64" => registry::Numeric::F64,
        other => bail!("unknown --dtype: {other} (expected i64|f64)"),
    };
    let spec = workload_spec(&args)?;
    // clap makes both required whenever a cell is selected, so the `bail`s
    // are unreachable from the command line and exist for the type.
    let metrics_mask = parse_mask(
        args.metrics
            .as_deref()
            .ok_or_else(|| anyhow::anyhow!("--metrics is required"))?,
    )?;
    let operations_mask = parse_operations(
        args.operations
            .as_deref()
            .ok_or_else(|| anyhow::anyhow!("--operations is required"))?,
    )?;
    // `--merge-shards` no longer selects anything: it says how many shards the
    // merge operation folds, and `--operations merge` is what asks for it.
    // The loop knobs, and only those. Which squares to run, at which config,
    // with how many workers, is the *request* — it goes in `Requirement`, not
    // in a whole-run config a single measurement would have to read past.
    let cfg = MeasureConfig {
        runs: args.runs,
        warmup_runs: args.warmup_runs,
        metrics: metrics_mask,
    };
    // One cell = one (impl, config). `--config` is one point, or a
    // parameterless point when omitted; keys are type-checked at
    // construction, where the impl reads them.
    let params = match args.config.as_deref() {
        Some(s) => registry::config_point(&algorithm, s)?,
        None => ParamSet::empty(&algorithm),
    };

    // Everything the registry needs, as one value. Assembled here and nowhere
    // else, so there is a single place that says what a request is.
    let req = Requirement {
        algorithm: algorithm.clone(),
        impl_name: impl_name.clone(),
        params: params.clone(),
        operations: operations_mask,
        metrics: metrics_mask,
        width,
        workers: args.workers.max(1),
        merge_shards: args.merge_shards,
        comparator: args.comparator.clone(),
    };

    // check against sketch-bench or other location
    // to see if this is runnable

    eprintln!(
        "approxbench: {}/{} config={} runs={} warmup={}",
        algorithm,
        impl_name,
        params_pretty(&params),
        cfg.runs,
        cfg.warmup_runs,
    );

    Ok(())
}

fn params_pretty(p: &sketch_bench::params::ParamSet) -> String {
    // Strip quotes around the params sub-object for tighter logs.
    let v = p.to_json_value();
    v.get("params")
        .map(|x| x.to_string())
        .unwrap_or_else(|| v.to_string())
}
