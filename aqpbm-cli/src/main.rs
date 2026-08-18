//! `approxbench`, the approximate query processing benchmark suite.
//!
//! `sketchbench` measures one row of the sketch bundle, and `sketchbench
//! --list-impls` enumerates that bundle's `(algorithm, impl)` pairs.

mod cli;
mod flatten_record;
mod repeat;

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
use aqpbm_core::benchmark_result::bench_report::BenchReport;
use aqpbm_core::measure::MeasureConfig;
use aqpbm_core::measurement::MIN_MERGE_SHARDS;
use aqpbm_core::metrics::{Metric, MetricsMask, Operation, OperationMask};
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
use aqpbm_core::input_dataset::InputDataSetSpec;
use sketch_bench::registry;
use sketch_bench::request::Requirement;
use sketch_bench::rows;

/// What is measured. No default and no `all`: a request names the measurements
/// it wants, and a shorthand that swept every operation against every metric
/// would sweep pairs nothing measures.
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

/// The measurements two masks name between them: every (operation, metric) pair
/// they cross to. This is the frontend's decision, so it is made here — core is
/// told one pair at a time and never sees a mask.
///
/// A pair nothing implements is still produced; `registry::check` is what
/// refuses it, by name, before any data is generated.
fn selected(operations: OperationMask, metrics: MetricsMask) -> Vec<(Operation, Metric)> {
    let mut out = Vec::new();
    for operation in Operation::ALL {
        if !operations.contains(operation.bit()) {
            continue;
        }
        for metric in Metric::ALL {
            if metrics.contains(metric.bit()) {
                out.push((operation, metric));
            }
        }
    }
    out
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

/// Load a `--spec` file as one [`TableDescription`]. One reader, because one
/// description covers both cases: a plain row takes a one-column table, and a
/// record-ingesting row takes label columns before the value column.
fn load_spec(path: &str) -> Result<InputDataSetSpec> {
    TableDescription::from_path(std::path::Path::new(path))
        .map(InputDataSetSpec::Generated)
        .map_err(|e| anyhow::anyhow!("loading spec from {path}: {e}"))
}

/// Resolve where this run's items come from, in precedence order: `--input` >
/// `--spec` > the `--dataset` flags. The flag path builds the same
/// `TableDescription` the spec path would, so it is sugar for a one-column
/// description — one generator.
fn dataset_spec(args: &SketchbenchArgs) -> Result<InputDataSetSpec> {
    if let Some(path) = args.input.as_deref() {
        return Ok(InputDataSetSpec::File {
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
    let distribution = match args.dataset.as_str() {
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
        other => bail!("unknown dataset shape: {other} (expected uniform|zipf, or use --spec)"),
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
    Ok(InputDataSetSpec::Inline(TableDescription::single(
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
    // Measures nothing and writes no record, so it runs before anything is
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
                 per measurement, and folding the repeats into it would have to decide which \
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
    // Every type `aqpbm-datagen` renders is spellable; which of them a row can
    // ingest is the row's own answer, and it refuses the rest — a construction
    // choice like any other, so the registry does not screen it.
    let width = registry::Dtype::parse(&args.dtype).ok_or_else(|| {
        anyhow::anyhow!(
            "unknown --dtype: {} (expected i64|u64|f64|string)",
            args.dtype
        )
    })?;
    let spec = dataset_spec(&args)?;
    // clap makes both required whenever a measurement is asked for, so the `bail`s
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
    // What this invocation will measure, decided here and passed down one pair
    // at a time. Cpu and memory are not measurements of their own; they ride
    // along with each one, so they are split off rather than crossed.
    let want = selected(operations_mask, metrics_mask);
    let secondary = metrics_mask & MetricsMask::SECONDARY;
    // One invocation is one row at one parameter point over one dataset.
    // `--config` is one point, or a parameterless point when omitted. Syntax
    // only here: whether the algorithm exists is `registry::check`'s answer
    // below, and the keys are type-checked at construction, where the impl
    // reads them.
    let params = match args.config.as_deref() {
        Some(s) => ParamSet::single(&algorithm, s)?,
        None => ParamSet::empty(&algorithm),
    };

    // Everything the registry needs, as one value. Assembled here and nowhere
    // else, so there is a single place that says what a request is.
    let req = Requirement {
        algorithm: algorithm.clone(),
        impl_name: impl_name.clone(),
        params: params.clone(),
        width,
        workers: args.workers.max(1),
        merge_shards: args.merge_shards,
        comparator: args.comparator.clone(),
    };

    // Can this run? The registry owns the answer, asked before the dataset is
    // generated, so an operation, metric, width or comparator this row cannot
    // honour costs nothing to refuse. The whole list is checked at once, so a
    // request is refused as a unit rather than part-way through measuring it.
    registry::check(&req, &want).map_err(|e| anyhow::anyhow!("{e}"))?;

    // An empty mask on either axis names no measurements. Legal: the caller gets
    // no records because it asked for none — and no dataset is generated for
    // measurements nobody asked for.
    if want.is_empty() {
        eprintln!(
            "approxbench: {algorithm}/{impl_name} selected no measurements; nothing to measure"
        );
        return Ok(());
    }

    eprintln!(
        "approxbench: {}/{} config={} runs={} warmup={}",
        algorithm,
        impl_name,
        params_pretty(&params),
        args.runs,
        args.warmup_runs,
    );

    // The hand-off `docs/component_walk_through.md` describes: the frontend asks
    // the bundle what the row ingests, produces exactly that, and hands the data
    // over. The item type is the row's answer, and generating needs it, so it is
    // asked for first.
    let wired = |e| {
        anyhow::anyhow!(
            "{algorithm}/{impl_name} is registered but not yet wired to its wrapper: {e}"
        )
    };
    let item_type = rows::item_type(&req)
        .ok_or_else(|| wired(""))?
        .map_err(|e| anyhow::anyhow!("{algorithm}/{impl_name} cannot run: {e}"))?;
    let data = spec.generate_at(item_type)?;

    // The closures, built at the row's own item type over the data just made.
    // Construction failures land here, before anything is timed.
    let prepared = rows::measurements(&req, data, &want)
        .ok_or_else(|| wired(""))?
        .map_err(|e| anyhow::anyhow!("{algorithm}/{impl_name} cannot run: {e}"))?;
    let dataset = prepared.dataset;

    // One instruction at a time. Core is handed a closure and a run count and
    // told nothing else; which operation and which metric this was travels with
    // the closure that answers it.
    let mut reports = Vec::with_capacity(prepared.bodies.len());
    for ((operation, metric), body) in prepared.bodies {
        // Error is deterministic given (data, parameters), and a dataset is
        // drawn once and not redrawn — so looping an accuracy measurement would
        // fabricate spread: ten identical answers averaged to `stddev: 0.0` over
        // `n: 10`. A timing measurement is where repeating one draw *is* a real
        // repeat, so only that one takes `--runs`.
        let cfg = MeasureConfig {
            runs: if metric == Metric::Accuracy {
                1
            } else {
                args.runs
            },
            warmup_runs: args.warmup_runs,
            // Exactly this measurement's recorders, plus whatever rides along.
            // Arming the rest would build a histogram nothing writes to.
            metrics: metric.bit() | secondary,
        };
        let runs = aqpbm_core::measure(&cfg, body);
        let mut report = BenchReport::from_runs(
            algorithm.as_str(),
            impl_name.as_str(),
            dataset.clone(),
            operation,
            metric,
            runs,
        );
        // The count that actually folded: a request below the floor is raised,
        // and a record states what ran rather than what was asked for.
        if operation == Operation::Merge {
            report.bench.merge_shards = Some(args.merge_shards.max(MIN_MERGE_SHARDS));
        }
        reports.push(report);
    }

    // One record per measurement, each on its own JSONL line; a downstream group-by on
    // (sketch, impl, sketch_config, dataset) merges them back. `sketch` names the
    // variant, `family` groups the variants a cross-library comparison spans.
    let family = registry::family_of(&algorithm).expect("check proved the algorithm is registered");
    let records: Vec<_> = reports
        .iter()
        .map(|report| {
            let mut record = report.to_record();
            record.sketch_config = Some(params.to_json_value());
            record.family = Some(family.to_string());
            record
        })
        .collect();

    let mut sink = ReportSink::open(args.report.as_deref())?;
    // One invocation is one row at one point, so every record here shares an identity and the
    // whole vector is exactly what `flatten_record` expects.
    if args.flat {
        let merged =
            flatten_record::flatten_record(&records).map_err(|e| anyhow::anyhow!("{e}"))?;
        sink.write_line(&serde_json::to_string(&merged)?)?;
    } else {
        for record in &records {
            sink.write_line(&record.to_jsonl())?;
        }
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
