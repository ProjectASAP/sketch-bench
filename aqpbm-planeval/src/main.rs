//! `planeval` — plan a query, run it, and print what the plan bought.
//!
//! A thin shell over the library: everything here is argument parsing and
//! printing. `aqpbm-cli` is bin-only, which is why `rows::binding()` and
//! `rows::measurements()` are unreachable in-process; this crate keeps its
//! logic in the lib so the same does not happen again.

use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::{Context, Result};
use clap::Parser;

use aqpbm_planeval::admit::admit;
use aqpbm_planeval::plan::{plan_promql, to_json};
use aqpbm_planeval::record::PlanEvalRecord;
use aqpbm_planeval::run::{run, RowsFrom, RunConfig};
use aqpbm_datagen::table::TableDescription;
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

    // Admission runs once: it depends on the plan, not on the seed or the data.
    let admitted = match admit(&plan.dag) {
        Ok(admitted) => admitted,
        Err(refusals) => {
            // The refusal table is a deliverable in its own right, so it goes
            // to stdout as data rather than to stderr as a complaint.
            for refusal in &refusals {
                println!("REFUSED {refusal}");
            }
            anyhow::bail!("{} of the plan's nodes were refused", refusals.len());
        }
    };

    let mut last: Option<PlanEvalRecord> = None;
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
            println!(
                "rows     generated from {} — {} columns x {} rows",
                path.display(),
                table.column_num,
                table.row_num
            );
            RowsFrom::Generated(table)
        }
        (None, Some(path)) => RowsFrom::Csv(path.clone()),
        // clap's `required_unless_present` already rejects this.
        (None, None) => anyhow::bail!("one of --spec or --csv is required"),
    };

    for seed in 0..args.seeds {
        let config = RunConfig {
            rows: rows.clone(),
            seed,
            verify: !args.no_verify,
        };
        let outcome = run(&plan, &admitted, &config)
            .with_context(|| format!("running seed {seed}"))?;
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

    if let Some(record) = last {
        print_advantage(&record);
    }
    Ok(())
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
        println!("rows     {} scanned, {} emitted", record.rows_scanned, record.rows_emitted);
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
        print!("seed {seed:<3}{group} {} => {:.6}", readout.query, readout.approximate);
        match (readout.exact, readout.rank_error, readout.claimed_bound) {
            (Some(exact), Some(observed), Some(claimed)) => {
                let verdict = if observed <= claimed { "within" } else { "VIOLATED" };
                println!(
                    "   exact {exact:.6}   rank err {observed:.6} {verdict} {claimed:.6}"
                );
            }
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
}
