//! `planeval` — plan a query, run it, and print what the plan bought.
//!
//! A thin shell over the library: everything here is argument parsing and
//! printing. `aqpbm-cli` is bin-only, which is why `rows::binding()` and
//! `rows::measurements()` are unreachable in-process; this crate keeps its
//! logic in the lib so the same does not happen again.

use std::path::PathBuf;
use std::process::ExitCode;
use std::rc::Rc;

use anyhow::{Context, Result};
use clap::Parser;

use aqpbm_datagen::table::TableDescription;
use aqpbm_planeval::plan::{plan_promql, to_json};
use aqpbm_planeval::record::{Phase, PlanEvalRecord};
use aqpbm_planeval::run::{run, RowsFrom, RunConfig};
use aqpbm_planeval::score::{GuaranteeObservations, ObservedError, ReadoutGuarantee};
use aqpbm_planeval::EvalError;
use asap_types::types::AccuracyTarget;

#[derive(Parser, Debug)]
#[command(
    name = "planeval",
    about = "Run a post-ASAP plan over rows and score it against the exact answer"
)]
struct Args {
    /// PromQL to plan, e.g. `quantile(0.5, cpu_cores)`.
    #[arg(long)]
    query: String,

    /// Generate the rows in process from a `datagen` spec file (examples in
    /// `configs/datagen/`). Wins over `--csv`.
    ///
    /// A path, not an inline description: the spec states one `data_type` per
    /// column and the plan's schema has to agree with it, so a flag that
    /// rewrote a field of the file would make the file a suggestion. This is
    /// the same rule `aqpbm-cli` follows.
    #[arg(long, conflicts_with = "csv")]
    spec: Option<PathBuf>,

    /// CSV to read. Its header names must match the leaf's schema; a PromQL
    /// leaf carries the usage-derived `ts,value`.
    #[arg(long, required_unless_present = "spec")]
    csv: Option<PathBuf>,

    /// End-to-end accuracy target the plan is sized against.
    #[arg(long, default_value_t = 0.01)]
    epsilon: f64,

    /// Run once per seed. One seed cannot check a bound that carries
    /// delta ~ 0.01: a single draw from an event designed to be true 99% of
    /// the time neither confirms nor refutes it.
    #[arg(long, default_value_t = 1)]
    seeds: u64,

    /// Skip the exact arm. Leaves the record with no ground truth in it, and
    /// says so rather than reporting zero error.
    #[arg(long)]
    no_verify: bool,

    /// Print the compiled document instead of running it.
    #[arg(long)]
    emit_json: bool,

    /// One JSONL record per seed on stdout, instead of the human summary.
    #[arg(long)]
    jsonl: bool,

    /// Timed passes per phase, per seed. One pass is one draw, not a
    /// distribution; the spread across them is still within one process, so no
    /// confidence interval is derived from it.
    #[arg(long, default_value_t = aqpbm_planeval::run::DEFAULT_TIMED_RUNS)]
    runs: usize,

    /// Passes run and discarded before the timed ones.
    #[arg(long, default_value_t = aqpbm_planeval::run::DEFAULT_WARMUP_RUNS)]
    warmup_runs: usize,
}

fn main() -> ExitCode {
    match real_main() {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("planeval: {err:#}");
            ExitCode::FAILURE
        }
    }
}

fn real_main() -> Result<()> {
    let args = Args::parse();

    let plan = plan_promql(&args.query, AccuracyTarget::Epsilon(args.epsilon))
        .with_context(|| format!("planning `{}`", args.query))?;

    if args.emit_json {
        println!("{}", to_json(&plan)?);
        return Ok(());
    }

    let mut last: Option<PlanEvalRecord> = None;
    let mut observations = GuaranteeObservations::default();
    // Built once, outside the seed loop: the rows are the same every seed, so
    // a difference between seeds is the sketch's and never the data's.
    let rows = match (&args.spec, &args.csv) {
        (Some(path), _) => {
            let description = TableDescription::from_path(path)
                .with_context(|| format!("loading {}", path.display()))?;
            description
                .validate()
                .with_context(|| format!("validating {}", path.display()))?;
            let table = description
                .generate()
                .with_context(|| format!("generating from {}", path.display()))?;
            let line = format!(
                "rows     generated from {} — {} columns x {} rows",
                path.display(),
                table.column_num,
                table.row_num
            );
            if args.jsonl {
                eprintln!("{line}");
            } else {
                println!("{line}");
            }
            RowsFrom::Generated(Rc::new(table))
        }
        (None, Some(path)) => RowsFrom::Csv(path.clone()),
        // clap's `required_unless_present` already rejects this.
        (None, None) => anyhow::bail!("one of --spec or --csv is required"),
    };

    for seed in 0..args.seeds {
        let mut config = RunConfig::new(rows.clone(), seed, !args.no_verify);
        config.timed_runs = args.runs;
        config.warmup_runs = args.warmup_runs;
        let outcome = match run(&plan, &config) {
            Ok(outcome) => outcome,
            // The refusal table is a deliverable in its own right, so it goes
            // to stdout as data rather than to stderr as a complaint.
            Err(EvalError::Refused(refusals)) => {
                for refusal in &refusals {
                    println!("REFUSED {refusal}");
                }
                anyhow::bail!("{} of the plan's nodes were refused", refusals.len());
            }
            Err(err) => return Err(err).with_context(|| format!("running seed {seed}")),
        };
        for readout in &outcome.readouts {
            observations.observe(readout);
        }
        let record = PlanEvalRecord::from_run(&args.query, &plan, &outcome);

        if args.jsonl {
            println!("{}", record.to_jsonl());
        } else {
            if seed == 0 {
                print_plan(&record);
            }
            print_readouts(seed, &record);
            last = Some(record);
        }
    }

    let guarantees = observations.checks();
    if args.jsonl {
        for guarantee in &guarantees {
            println!("{}", serde_json::to_string(guarantee)?);
        }
    } else {
        if let Some(record) = last {
            print_advantage(&record);
        }
        print_guarantees(&guarantees, args.seeds);
    }
    Ok(())
}

fn print_guarantees(guarantees: &[ReadoutGuarantee], seeds: u64) {
    if guarantees.is_empty() {
        println!("\nguarantee  none \u{2014} no readout carried one");
        return;
    }
    for entry in guarantees {
        let check = &entry.check;
        let group = if entry.group.is_empty() {
            String::new()
        } else {
            format!(" [{}]", entry.group)
        };
        println!("\nguarantee  node {}{} {}", entry.node, group, entry.query);
        println!(
            "  metric {}   claimed {}   delta {}",
            check.metric,
            opt(check.claimed_bound),
            opt(check.failure_probability)
        );
        if check.observed_violation_rate.is_nan() {
            let why = match (&check.unevaluatable, check.claimed_bound) {
                (Some(reason), _) => format!(
                    "the exact arm cannot measure a {} error: it would need {reason:?}",
                    check.metric
                ),
                _ if check.uncomputed_errors > 0 => format!(
                    "{} of {} observations produced no {} error at all",
                    check.uncomputed_errors, check.seeds, check.metric
                ),
                (None, Some(_)) => format!(
                    "{} observations over {seeds} seeds, bound present",
                    check.seeds
                ),
                (None, None) => format!(
                    "{} observations over {seeds} seeds, bound unevaluatable",
                    check.seeds
                ),
            };
            println!("  violation rate  not checked ({why})");
        } else {
            println!(
                "  violation rate  {:.6} over {} observations ({seeds} seeds)   mean err {:.6}   max {:.6}",
                check.observed_violation_rate, check.seeds, check.mean_error, check.max_error
            );
        }
        if let Some(contract) = &check.contract {
            println!("  contract   {contract}");
        }
        if let Some(fit) = &check.implied_fit {
            let verdict = if fit.agrees_with_reference {
                "agrees"
            } else {
                "DISAGREES \u{2014} the contract id is stale"
            };
            println!(
                "  implied fit  k={}  coefficient {:.6} (reference {:.6})  exponent {:.6} (reference {:.6})  {verdict}",
                fit.k,
                fit.coefficient_at_reference_exponent,
                fit.reference_coefficient,
                fit.exponent_at_reference_coefficient,
                fit.reference_exponent
            );
        }
    }
}

fn opt(value: Option<f64>) -> String {
    match value {
        Some(v) => format!("{v:.6}"),
        None => "unevaluatable".to_string(),
    }
}

fn print_plan(record: &PlanEvalRecord) {
    {
        println!("query    {}", record.plan.query);
        println!(
            "plan     {}  ({} nodes, {} edges, wire v{})",
            &record.plan.plan_id[..16],
            record.plan.nodes,
            record.plan.edges,
            record.plan.document_schema_version
        );
        for node in &record.nodes {
            let state = match node.state_bytes {
                Some(bytes) => format!("{bytes} B"),
                None => "-".to_string(),
            };
            println!(
                "  node {}  {:<16} state {:>9}  {}",
                node.node,
                node.operator,
                state,
                node.family.as_deref().unwrap_or("")
            );
        }
        println!(
            "rows     {} scanned, {} emitted",
            record.rows_scanned, record.rows_emitted
        );
        if let Some(root_rows) = record.root_rows {
            println!("result   {root_rows} rows out of node {}", record.plan.root);
        }
        println!();
    }
}

fn print_readouts(seed: u64, record: &PlanEvalRecord) {
    for readout in &record.readouts {
        let group = if readout.group.is_empty() {
            String::new()
        } else {
            format!(" [{}]", readout.group)
        };
        print!(
            "seed {seed:<3}{group} {} => {:.6}",
            readout.query, readout.approximate
        );
        match (
            readout.exact.as_ref(),
            &readout.observed_error,
            readout.claimed_bound,
        ) {
            (Some(exact), ObservedError::Measured { metric, error }, Some(claimed)) => {
                let verdict = if *error <= claimed {
                    "within"
                } else {
                    "VIOLATED"
                };
                println!("   exact {exact:.6}   {metric} err {error:.6} {verdict} {claimed:.6}");
            }
            (Some(exact), ObservedError::Measured { metric, error }, None) => {
                println!(
                    "   exact {exact:.6}   {metric} err {error:.6}, no bound to check it against"
                )
            }
            (Some(exact), ObservedError::Unevaluatable { metric, reason }, _) => println!(
                "   exact {exact:.6}   {metric} err not evaluatable, it would need {reason:?}"
            ),
            (Some(exact), _, _) => println!("   exact {exact:.6}"),
            // Never 0.0 here: "not computed" must not read as "exact".
            _ => println!("   exact not computed"),
        }
    }
}

fn print_advantage(record: &PlanEvalRecord) {
    match record.advantage().memory {
        Some(ratio) => println!(
            "\nmemory   {} B summary vs {} B retained exactly  =>  {ratio:.1}x",
            record.approximate.state_bytes, record.exact.retained_bytes
        ),
        // Not 1.0: nothing was measured on the exact arm.
        None => println!("\nmemory   not comparable (the exact arm did not run)"),
    }
    print_phase(
        "insert ",
        record.approximate.update.as_ref(),
        record.exact.update.as_ref(),
    );
    print_phase(
        "readout",
        record.approximate.readout.as_ref(),
        record.exact.readout.as_ref(),
    );
    print_phase(
        "bind   ",
        record.approximate.build.as_ref(),
        record.exact.build.as_ref(),
    );
}

fn print_phase(name: &str, approximate: Option<&Phase>, exact: Option<&Phase>) {
    let Some(approximate) = approximate else {
        return;
    };
    print!(
        "{name}  {} units: summary {}",
        approximate.work,
        spread(approximate)
    );
    match exact {
        Some(exact) => {
            let ratio = exact.elapsed_ms.mean / approximate.elapsed_ms.mean;
            println!("   exact {}  =>  {ratio:.1}x", spread(exact));
        }
        None => println!("   exact not measured"),
    }
}

fn spread(phase: &Phase) -> String {
    format!(
        "{:.4} ms +/- {:.4} (n={})",
        phase.elapsed_ms.mean, phase.elapsed_ms.stddev, phase.elapsed_ms.n
    )
}
