//! `approxbench`, the approximate query processing benchmark suite.
//!
//! `sketchbench` measures one row of the sketch bundle, and `sketchbench
//! --list-impls` enumerates that bundle's `(variant, library)` pairs.

mod atomic_costs_cmd;
mod cli;
mod erp_cmd;
mod external;
mod flatten_cmd;
mod flatten_record;
mod repeat;
mod report_sink;
mod trials;

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

use anyhow::{bail, Result};
use aqpbm_core::benchmark_result::bench_report::BenchReport;
use aqpbm_core::measure::MeasureConfig;
use aqpbm_core::measure::MIN_MERGE_SHARDS;
use aqpbm_core::metrics::{Metric, MetricsMask, Operation, OperationMask};
use aqpbm_core::{Record, TrialMode, WorkloadDescription};
use aqpbm_datagen::{
    BurstSpec, ColumnSpec, DataDistribution, GeneratedTable, ParetoParameter, StringOpts,
    TableDescription, UniformParameter, ZipfParameter, RULE_NONE,
};
use clap::Parser;
use sketch_bench::params::ParamSet;
use std::path::Path;

use cli::{Cli, Cmd, SketchbenchArgs};
use report_sink::ReportSink;
// The registry — which sketches exist, how to build them, which ground-truth calculator scores
// them — is sketch-domain knowledge and lives in `sketch-bench`. The CLI does
// not know the set; it asks.
use sketch_bench::registry;
use sketch_bench::request::Requirement;

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

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Cmd::Sketchbench(args) => run_sketchbench(args),
        Cmd::AtomicCosts(args) => atomic_costs_cmd::run(args),
        Cmd::Erp(args) => erp_cmd::run(args),
        Cmd::Flatten(args) => flatten_cmd::run(args),
    }
}

/// `--list-impls` prints and exits
fn list_impls() -> Result<()> {
    for line in registry::list() {
        println!("{line}");
    }
    Ok(())
}

#[derive(Debug, Clone)]
enum InputDataSetSpec {
    Generated(TableDescription),
    Inline(TableDescription),
}

impl InputDataSetSpec {
    /// The description to generate from at item type `item_type`. See the
    /// variants above for why the two cases answer differently.
    fn describe(&self, item_type: &str) -> TableDescription {
        match self {
            InputDataSetSpec::Generated(d) => d.clone(),
            InputDataSetSpec::Inline(d) => {
                let mut d = d.clone();
                for column in &mut d.column_spec {
                    column.data_type = item_type.to_string();
                }
                d
            }
        }
    }
}

/// Load a `--spec` file as one [`TableDescription`]. One reader, because one
/// description covers both cases: a plain row takes a one-column table, and a
/// record-ingesting row takes label columns before the value column.
fn load_spec(path: &str) -> Result<InputDataSetSpec> {
    TableDescription::from_path(std::path::Path::new(path))
        .map(InputDataSetSpec::Generated)
        .map_err(|e| anyhow::anyhow!("loading spec from {path}: {e}"))
}

/// Resolve where this run's items come from, in precedence order: `--spec` >
/// the `--dataset` flags. The flag path builds the same
/// `TableDescription` the spec path would, so it is sugar for a one-column
/// description — one generator.
fn dataset_spec(args: &SketchbenchArgs) -> Result<InputDataSetSpec> {
    if let Some(path) = args.spec.as_deref() {
        // A spec carries its own `string:` block, so `--alphabet`/`--key-len`
        // would be editing the user's file from the command line.
        return load_spec(path);
    }
    // Clap's `required_unless_present_any = ["list_impls", "spec"]` (and, for
    // `zipf_s`, `required_if_eq`) guarantees these are `Some` by the time
    // we're here: `--spec` already returned above, and `--list-impls` is
    // handled before `dataset_spec` is ever called.
    let dataset = args.dataset.as_deref().expect("clap requires --dataset");
    let cardinality = args.cardinality.expect("clap requires --cardinality");
    // `--cardinality` names the key space either way: for uniform it is the
    // exclusive upper bound of `[0, n)`, and for zipf the population its ranks
    // `1..=n` are drawn over.
    let distribution = match dataset {
        "uniform" => DataDistribution::Uniform(UniformParameter {
            lower_bound: 0.0,
            upper_bound: cardinality as f64,
            seed: args.seed,
        }),
        "zipf" => DataDistribution::Zipf(ZipfParameter {
            skewness: args
                .zipf_s
                .expect("clap requires --zipf-s when --dataset zipf"),
            population_size: cardinality,
            seed: args.seed,
        }),
        // Unbounded and continuous, so `--cardinality` has nothing to name. An
        // f64 column keeps the draws as drawn; an i64 column floors them (every
        // draw is >= scale > 0, so the renderer's truncation is a floor), which
        // is what the i64-only exact quantile baseline needs.
        "pareto" => {
            if !matches!(args.dtype.as_deref(), Some("f64" | "i64")) {
                bail!("--dataset pareto needs --dtype f64 or i64 (i64 floors each draw)");
            }
            DataDistribution::Pareto(ParetoParameter {
                alpha: args
                    .pareto_alpha
                    .expect("clap requires --pareto-alpha when --dataset pareto"),
                scale: args.pareto_scale,
                seed: args.seed,
            })
        }
        other => {
            bail!("unknown dataset shape: {other} (expected uniform|zipf|pareto, or use --spec)")
        }
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
            child_of: None,
            fan_out: None,
            scale_by: None,
            scale_range: None,
            key_offset: None,
        },
        args.size.expect("clap requires --size") as u64,
    )))
}

/// Seconds of CPU burn before the first measured loop, so the cpufreq governor
/// is at max turbo when timing starts
const DEFAULT_WARMUP_SECS: &str = "10";

fn run_sketchbench(args: SketchbenchArgs) -> Result<()> {
    if args.list_impls {
        return list_impls();
    }
    let (variant, library) = match (args.variant.as_deref(), args.library.as_deref()) {
        (Some(a), Some(i)) => (a.to_string(), i.to_string()),
        _ => bail!("--variant and --library are both required unless --list-impls is given"),
    };
    if args.repeat_experiment == 0 {
        bail!("--repeat-experiment must be >= 1");
    }
    if args.repeat_experiment > 1 && trials::trial_options_given(&args) {
        bail!(
            "--trials is the accuracy axis and --repeat-experiment the throughput one; \
             ask for one of them"
        );
    }
    let pretty = args.pretty_print && !repeat::is_child();
    if args.repeat_experiment > 1 && !repeat::is_child() {
        if args.flat {
            bail!(
                "--flat cannot be combined with --repeat-experiment: a flattened row holds one \
                 per measurement, and folding the repeats into it would have to decide which \
                 repeat that value came from"
            );
        }
        let records = repeat::run_repeats(args.repeat_experiment)?;
        let mut sink = ReportSink::open(args.report.as_deref())?;
        for r in &records {
            if pretty {
                sink.write_pretty(r)?;
            } else {
                sink.write_line(&r.to_jsonl())?;
            }
        }
        eprintln!(
            "approxbench: merged {} repeats into {} record(s)",
            args.repeat_experiment,
            records.len()
        );
        return Ok(());
    }
    let dtype = args.dtype.as_deref().unwrap_or("f64");
    let width = registry::Dtype::parse(dtype)
        .ok_or_else(|| anyhow::anyhow!("unknown --dtype: {dtype} (expected i64|u64|f64|string)"))?;
    let external_workload = match args.workload_spec.as_deref() {
        Some(path) => {
            let mut spec = external::load_spec(Path::new(path))?;
            if let Some(start) = args.window_start.clone() {
                spec.window.start = start;
                spec.window.end = args.window_end.clone().expect("clap requires --window-end");
            }
            let loaded = external::load(&spec, Path::new(&args.data_root))?;
            if loaded.is_none() {
                eprintln!("approxbench: external workload window is empty; skipping");
                return Ok(());
            }
            Some(loaded.expect("checked that the external window is non-empty"))
        }
        None => None,
    };
    if external_workload.is_some() && width != registry::Dtype::F64 {
        bail!("external workloads currently expose numeric values as f64; use --dtype f64");
    }
    let spec = if external_workload.is_none() {
        Some(dataset_spec(&args)?)
    } else {
        None
    };
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
    if metrics_mask == MetricsMask::ACCURACY {
        std::env::set_var("BENCH_WARMUP_SECS", "0");
    } else if std::env::var_os("BENCH_WARMUP_SECS").is_none() {
        std::env::set_var("BENCH_WARMUP_SECS", DEFAULT_WARMUP_SECS);
    }
    let want = selected(operations_mask, metrics_mask);
    let secondary = metrics_mask & MetricsMask::SECONDARY;
    let params = match args.config.as_deref() {
        Some(s) => ParamSet::single(&variant, s)?,
        None => ParamSet::empty(&variant),
    };

    let mut group_columns = args.group_columns.clone();
    group_columns.sort_unstable();
    group_columns.dedup();
    let row = registry::find(&variant, &library);
    if args.per_group_out.is_some() {
        if !row.is_some_and(|row| row.capability.name().starts_with("subpop-")) {
            bail!("--per-group-out: only the subpopulation rows score groups, and {variant}/{library} is not one");
        }
        match want.iter().filter(|(_, m)| *m == Metric::Accuracy).count() {
            0 => bail!("--per-group-out writes an accuracy measurement, and none was asked for"),
            1 => {}
            _ => bail!(
                "--per-group-out writes one accuracy measurement; ask for query or merge, not both"
            ),
        }
    }
    if !args.quantiles.is_empty() {
        let comparator = args
            .comparator
            .as_deref()
            .or(row.and_then(|row| row.comparator));
        if comparator != Some("rank-error") {
            bail!("--quantiles: only a row scored by rank error is asked at fractions, and {variant}/{library} is not one");
        }
        if let Some(q) = args.quantiles.iter().find(|q| !(0.0..=1.0).contains(*q)) {
            bail!("--quantiles: {q} is outside [0, 1]");
        }
    }
    let req = Requirement {
        variant: variant.clone(),
        library: library.clone(),
        params: params.clone(),
        width,
        workers: args.workers.max(1),
        merge_shards: args.merge_shards,
        group_columns,
        per_group_out: args.per_group_out.as_ref().map(std::path::PathBuf::from),
        merge_split: args.merge_split,
        comparator: args.comparator.clone(),
        quantiles: args.quantiles.clone(),
        runs: args.runs,
        warmup_runs: args.warmup_runs,
        trial: None,
    };

    // requirement quick check
    registry::check(&req, &want).map_err(|e| anyhow::anyhow!("{e}"))?;

    let row = row.expect("check proved the row is registered");
    let trial_mode = args.trial_mode.unwrap_or_default();
    let trial_record = trials::trial_options_given(&args).then_some(trial_mode);
    let base_description = spec.map(|spec| spec.describe(width.name()));
    trials::check(&args, metrics_mask, row, base_description.as_ref())?;

    // An empty mask on either axis names no measurements.
    if want.is_empty() {
        eprintln!("approxbench: {variant}/{library} selected no measurements; nothing to measure");
        return Ok(());
    }

    eprintln!(
        "approxbench: {}/{} config={} runs={} warmup={}",
        variant,
        library,
        params_pretty(&params),
        args.runs,
        args.warmup_runs,
    );

    if let Some(path) = args.per_group_out.as_deref() {
        std::fs::File::create(path).map_err(|e| anyhow::anyhow!("--per-group-out {path}: {e}"))?;
    }
    let algorithm =
        registry::algorithm_of(&variant).expect("check proved the variant is registered");
    let mut sink = ReportSink::open(args.report.as_deref())?;
    let mut run_trial = |dataset: TableDescription,
                         table: GeneratedTable,
                         workload: WorkloadDescription,
                         trial_params: &ParamSet,
                         trial: u64|
     -> Result<()> {
        let req = Requirement {
            params: trial_params.clone(),
            trial: trial_record.map(|_| trial),
            ..req.clone()
        };

        // The closures, built at the row's own item type over the data just made.
        // Construction failures land here, before anything is timed.
        let prepared = sketch_bench::rows::measurements(&req, &dataset, table, &want)
            .map_err(|e| anyhow::anyhow!("{variant}/{library} cannot run: {e}"))?;

        // One instruction at a time. Core is handed a closure and a run count and
        // told nothing else; which operation and which metric this was travels with
        // the closure that answers it.
        let mut reports = Vec::with_capacity(prepared.len());
        for ((operation, metric), body) in prepared {
            let cfg = MeasureConfig {
                runs: aqpbm_core::runs_for(metric, args.runs),
                warmup_runs: args.warmup_runs,
                // Exactly this measurement's recorders, plus whatever rides along.
                // Arming the rest would build a histogram nothing writes to.
                metrics: metric.bit() | secondary,
            };
            let runs = aqpbm_core::measure(&cfg, body);
            let mut report = BenchReport::from_runs(
                variant.as_str(),
                library.as_str(),
                workload.clone(),
                operation,
                metric,
                runs,
            );
            // The count that actually folded: a request below the floor is raised,
            // and a record states what ran rather than what was asked for.
            if operation == Operation::Merge {
                report.bench.merge_shards = Some(args.merge_shards.max(MIN_MERGE_SHARDS));
                report.bench.merge_split = Some(args.merge_split);
            }
            if trial_record.is_some() {
                report.bench.trial = Some(trial);
                report.bench.trial_mode = trial_record;
            }
            reports.push(report);
        }

        // One record per measurement, each on its own JSONL line; a downstream group-by on
        // (sketch, impl, sketch_config, dataset) merges them back. `sketch` names the
        // variant, `algorithm` groups the variants a cross-library comparison spans.
        let records: Vec<Record> = reports
            .iter()
            .map(|report| {
                let mut record = report.to_record();
                record.sketch_config = Some(trial_params.to_json_value());
                record.algorithm = Some(algorithm.to_string());
                record
            })
            .collect();

        // One trial is one row at one point, so every record it wrote shares an identity
        // and the whole vector is exactly what `flatten_record` expects.
        if args.flat {
            let merged =
                flatten_record::flatten_record(&records).map_err(|e| anyhow::anyhow!("{e}"))?;
            if pretty {
                sink.write_pretty(&merged)?;
            } else {
                sink.write_line(&serde_json::to_string(&merged)?)?;
            }
        } else {
            for record in &records {
                if pretty {
                    sink.write_pretty(record)?;
                } else {
                    sink.write_line(&record.to_jsonl())?;
                }
            }
        }
        Ok(())
    };

    if let Some(loaded) = external_workload {
        return run_trial(
            loaded.table_shape,
            loaded.table,
            loaded.workload,
            &params,
            0,
        );
    }
    let base_description = base_description.expect("synthetic workload spec");
    let key_columns = trials::key_columns(row.comparator, &base_description);
    let drawn_once = match trial_mode {
        TrialMode::Relabel | TrialMode::Reseed => Some(generate_with_bursts(
            base_description.clone(),
            &args,
            args.seed,
        )?),
        TrialMode::Fresh => None,
    };
    let first_trial = args.trial_start.unwrap_or(0);
    for trial in first_trial..first_trial + args.trials {
        let trial_params = match trial_mode {
            TrialMode::Reseed => trials::reseeded_params(&params, trial),
            TrialMode::Fresh | TrialMode::Relabel => params.clone(),
        };
        let trial_data = || -> Result<(TableDescription, GeneratedTable, WorkloadDescription)> {
            let (mut dataset, mut table, burst) = match &drawn_once {
                Some(drawn) => drawn.clone(),
                None => generate_with_bursts(
                    trials::reseeded(&base_description, trial),
                    &args,
                    trials::derived_seed(args.seed, trial),
                )?,
            };
            if trial_mode != TrialMode::Reseed {
                trials::offset_key_columns(&mut dataset, &mut table, &key_columns, trial)?;
            }
            let workload = WorkloadDescription::Synthetic {
                description: dataset.clone(),
                burst,
            };
            Ok((dataset, table, workload))
        };
        trial_data()
            .and_then(|(dataset, table, workload)| {
                run_trial(dataset, table, workload, &trial_params, trial)
            })
            .map_err(|e| match trial_record {
                Some(mode) => e.context(format!(
                    "trial {trial} ({})",
                    trials::seeds_message(&base_description, mode, trial, args.seed, &trial_params)
                )),
                None => e,
            })?;
    }
    Ok(())
}

fn generate_with_bursts(
    mut description: TableDescription,
    args: &SketchbenchArgs,
    burst_seed: u64,
) -> Result<(TableDescription, GeneratedTable, Option<BurstSpec>)> {
    let mut table = description.generate()?;
    let burst = if let (Some(interval_rows), Some(burst_intervals)) =
        (args.burst_interval_rows, args.burst_intervals)
    {
        let burst = BurstSpec {
            interval_rows,
            burst_intervals,
            extra_fraction: args.burst_extra_fraction,
            seed: burst_seed,
        };
        table.inject_bursts(burst)?;
        description.row_num = table.row_num;
        Some(burst)
    } else {
        None
    };
    Ok((description, table, burst))
}

fn params_pretty(p: &sketch_bench::params::ParamSet) -> String {
    // Strip quotes around the params sub-object for tighter logs.
    let v = p.to_json_value();
    v.get("params")
        .map(|x| x.to_string())
        .unwrap_or_else(|| v.to_string())
}
