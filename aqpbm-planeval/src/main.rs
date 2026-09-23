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
use clap::{Parser, ValueEnum};

use aqpbm_datagen::table::{GeneratedTable, TableDescription};
use aqpbm_planeval::df::run::{refusal_counts, run as run_datafusion, DataFusionRunConfig};
use aqpbm_planeval::exact::{run_tree, Data, ExactRun};
use aqpbm_planeval::plan::{plan_sql, to_json, Plan};
use aqpbm_planeval::record::{
    AnswerCheck, AnswerRecord, Arm, NodeCost, Phase, PlanEvalRecord, ReadoutRecord,
    RefusedPlanRecord, UnplannedQueryRecord,
};
use aqpbm_planeval::run::{run, RowsFrom, RunConfig, Runtime};
use aqpbm_planeval::runtimes::{
    quantities, readout_differences, readout_pairs, records_per_runtime, ReadoutDifference,
    RuntimeRecords,
};
use aqpbm_planeval::score::{GuaranteeObservations, ObservedError, ReadoutGuarantee};
use aqpbm_planeval::{sql, EvalError, Value};
use asap_types::types::AccuracyTarget;

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
enum RuntimeArg {
    Interp,
    Datafusion,
}

#[derive(Parser, Debug)]
#[command(
    name = "planeval",
    about = "Run a post-ASAP plan over rows and score it against the exact answer"
)]
struct Args {
    #[arg(
        long,
        help = "SQL to plan, e.g. `SELECT approx_percentile_cont(latency, 0.99) FROM t`. \
                A SQL query names tables, and the only catalog this binary has is the one \
                --spec describes"
    )]
    sql: String,

    #[arg(
        long,
        help = "The name --sql may refer to the --spec table by. Defaults to the spec file's \
                stem"
    )]
    table: Option<String>,

    /// Generate the rows in process from a `datagen` spec file (examples in
    /// `configs/datagen/`).
    ///
    /// A path, not an inline description: the spec states one `data_type` per
    /// column and the plan's schema has to agree with it, so a flag that
    /// rewrote a field of the file would make the file a suggestion. This is
    /// the same rule `aqpbm-cli` follows.
    #[arg(long)]
    spec: PathBuf,

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

    #[arg(
        long,
        help = "Also print the answer the pre-ASAP tree computed, before the plan runs. The \
                timed pre-ASAP arm runs either way unless --no-pre-asap turns it off"
    )]
    evaluate_exactly: bool,

    #[arg(
        long,
        help = "Skip the pre-ASAP arm. Leaves the record with no baseline in it, so the time \
                and memory ratios report nothing rather than dividing by the retained column, \
                which is not the query anyone would have run"
    )]
    no_pre_asap: bool,

    #[arg(
        long,
        help = "Time each call inside a phase and attribute it to the node it ran for. The \
                insert phase then reads a clock per update and its total pays for them, so the \
                per-node times and the phase total will not agree exactly"
    )]
    per_node_time: bool,

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

    #[arg(
        long,
        value_enum,
        default_value_t = RuntimeArg::Interp,
        help = "Which engine runs the two arms. `datafusion` plans both arms against the \
                catalog the spec describes and executes them over one in-memory table"
    )]
    runtime: RuntimeArg,

    #[arg(
        long,
        help = "Run the whole post-ASAP graph as one query instead of cutting it into a \
                maintenance query and a read query. The answer is the same bits either way; the \
                aggregate and query times are then one number and reported under the read phase. \
                DataFusion only"
    )]
    no_split: bool,

    #[arg(
        long,
        help = "Run the same plan over the same rows with the same seed on both runtimes and \
                print them side by side. Supersedes --runtime. \
                Every readout has to carry the same approximate and the same exact answer bit \
                for bit; a run where one does not fails. The time and memory rows say which \
                pairs are the same measurement and which only look like one"
    )]
    compare_runtimes: bool,
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

fn spec_table(args: &Args) -> Result<TableDescription> {
    let path = &args.spec;
    let description =
        TableDescription::from_path(path).with_context(|| format!("loading {}", path.display()))?;
    description
        .validate()
        .with_context(|| format!("validating {}", path.display()))?;
    Ok(description)
}

fn table_name(args: &Args) -> Result<String> {
    Ok(match &args.table {
        Some(named) => named.clone(),
        None => sql::table_name_from_path(&args.spec)?,
    })
}

fn generated_rows(args: &Args, description: &TableDescription) -> Result<Rc<GeneratedTable>> {
    let table = description
        .generate()
        .with_context(|| format!("generating from {}", args.spec.display()))?;
    let line = format!(
        "rows     generated from {} — {} columns x {} rows",
        args.spec.display(),
        table.column_num,
        table.row_num
    );
    if args.jsonl {
        eprintln!("{line}");
    } else {
        println!("{line}");
    }
    Ok(Rc::new(table))
}

fn datafusion_config(
    args: &Args,
    description: &TableDescription,
    table: &Rc<GeneratedTable>,
) -> Result<Option<DataFusionRunConfig>> {
    if args.runtime != RuntimeArg::Datafusion && !args.compare_runtimes {
        return Ok(None);
    }
    Ok(Some(DataFusionRunConfig {
        table: table_name(args)?,
        description: description.clone(),
        rows: Rc::clone(table),
        split: !args.no_split,
    }))
}

fn refused_lines(err: &EvalError) -> Vec<String> {
    match err {
        EvalError::Refused(refusals) => refusals.iter().map(|r| r.to_string()).collect(),
        EvalError::Untranslated(refusals) => refusals.iter().map(|r| r.to_string()).collect(),
        _ => Vec::new(),
    }
}

fn print_refusals(
    err: &EvalError,
    jsonl: bool,
    runtime: &str,
    plan: &aqpbm_planeval::plan::Plan,
    query: &str,
    seed: u64,
) -> Result<()> {
    let refused = refused_lines(err);
    for line in &refused {
        eprintln!("REFUSED {line}");
    }
    let counts = refusal_counts(err);
    let record = RefusedPlanRecord::new(runtime, plan, query, seed, counts.clone(), refused);
    if jsonl {
        println!("{}", record.to_jsonl());
    } else {
        println!(
            "refusals  promql_only {}  time_axis {}  no_constructor {}  deferred {}  \
             unclassified {}",
            counts.promql_only,
            counts.time_axis,
            counts.no_constructor,
            counts.deferred,
            counts.unclassified
        );
    }
    Ok(())
}

fn runtime_tag(args: &Args) -> &'static str {
    match args.runtime {
        RuntimeArg::Interp => Runtime::Interpreter.tag(),
        RuntimeArg::Datafusion => Runtime::DataFusion.tag(),
    }
}

fn report_unplanned(args: &Args, query: &str, err: EvalError) -> anyhow::Error {
    if let Some(record) = UnplannedQueryRecord::of_error(runtime_tag(args), query, &err) {
        if args.jsonl || args.emit_json {
            println!("{}", record.to_jsonl());
        } else {
            println!(
                "unplanned  `{query}` stopped at the {} stage: {}",
                record.stage, record.detail
            );
        }
    }
    anyhow::Error::new(err).context(format!("planning `{query}`"))
}

fn real_main() -> Result<()> {
    let args = Args::parse();
    if args.no_split && args.runtime != RuntimeArg::Datafusion {
        anyhow::bail!(
            "--no-split describes a post-ASAP graph cut in two, which only the datafusion \
             runtime does; pass --runtime datafusion"
        );
    }
    let accuracy = AccuracyTarget::Epsilon(args.epsilon);
    let description = spec_table(&args)?;

    let query_text = args.sql.clone();
    let planned = sql::catalog_from_spec(&table_name(&args)?, &description)
        .and_then(|catalog| plan_sql(&query_text, &catalog, accuracy));
    let plan = match planned {
        Ok(plan) => plan,
        Err(err) => return Err(report_unplanned(&args, &query_text, err)),
    };

    if args.emit_json {
        println!("{}", to_json(&plan)?);
        return Ok(());
    }

    let mut last: Option<PlanEvalRecord> = None;
    let mut worst_accuracy: Option<f64> = None;
    let mut observations = GuaranteeObservations::default();
    // Built once, outside the seed loop: the rows are the same every seed, so
    // a difference between seeds is the sketch's and never the data's.
    let table = generated_rows(&args, &description)?;
    let rows = RowsFrom::Generated(Rc::clone(&table));
    let engine = datafusion_config(&args, &description, &table)?;

    if args.evaluate_exactly {
        let root = plan
            .pre_asap
            .as_ref()
            .context("a plan lowered from SQL carries its pre-ASAP tree")?;
        let evaluated = match run_tree(Rc::clone(root), &rows) {
            Ok(evaluated) => evaluated,
            Err(EvalError::Refused(refusals)) => {
                for refusal in &refusals {
                    eprintln!("REFUSED {refusal}");
                }
                anyhow::bail!("the pre-ASAP tree of `{query_text}` was refused");
            }
            Err(err) => {
                return Err(err).with_context(|| format!("evaluating `{query_text}` exactly"))
            }
        };
        let block = exact_block(&query_text, &evaluated);
        if args.jsonl {
            eprint!("{block}");
        } else {
            println!("{block}");
        }
    }

    if args.compare_runtimes {
        let engine = engine.context("--compare-runtimes builds a DataFusion configuration")?;
        return compare_runtimes(&args, &query_text, &plan, &rows, &engine);
    }

    for seed in 0..args.seeds {
        let config = run_config_of(&args, &rows, seed);
        let attempted = match engine.as_ref() {
            Some(engine) => run_datafusion(&plan, &config, engine),
            None => run(&plan, &config),
        };
        let outcome = match attempted {
            Ok(outcome) => outcome,
            // The refusal record is a deliverable in its own right, so it goes
            // to stdout as data while the prose behind it goes to stderr, and
            // a sweep runs on to the next seed rather than dying here.
            Err(err @ (EvalError::Refused(_) | EvalError::Untranslated(_))) => {
                print_refusals(
                    &err,
                    args.jsonl,
                    runtime_tag(&args),
                    &plan,
                    &query_text,
                    seed,
                )?;
                continue;
            }
            Err(err) => return Err(err).with_context(|| format!("running seed {seed}")),
        };
        for readout in &outcome.readouts {
            observations.observe(readout);
        }
        let record = PlanEvalRecord::from_run(&query_text, &plan, &outcome);
        if let Some(error) = record.advantage().accuracy {
            worst_accuracy = match worst_accuracy {
                Some(held) if held.total_cmp(&error).is_ge() => Some(held),
                _ => Some(error),
            };
        }

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
            print_advantage(&record, worst_accuracy, args.seeds);
        }
        print_guarantees(&guarantees, args.seeds);
    }
    Ok(())
}

fn run_config_of(args: &Args, rows: &RowsFrom, seed: u64) -> RunConfig {
    let mut config = RunConfig::new(rows.clone(), seed, !args.no_verify);
    config.timed_runs = args.runs;
    config.warmup_runs = args.warmup_runs;
    config.pre_asap = !args.no_pre_asap;
    config.per_node_time = args.per_node_time;
    config
}

fn compare_runtimes(
    args: &Args,
    query_text: &str,
    plan: &Plan,
    rows: &RowsFrom,
    engine: &DataFusionRunConfig,
) -> Result<()> {
    let mut disagreeing_seeds: Vec<u64> = Vec::new();
    for seed in 0..args.seeds {
        let config = run_config_of(args, rows, seed);
        let records = match records_per_runtime(query_text, plan, &config, engine) {
            Ok(records) => records,
            Err(err @ (EvalError::Refused(_) | EvalError::Untranslated(_))) => {
                print_refusals(&err, args.jsonl, "compare-runtimes", plan, query_text, seed)?;
                continue;
            }
            Err(err) => return Err(err).with_context(|| format!("running seed {seed}")),
        };
        let differences = readout_differences(&records);
        if !differences.is_empty() {
            disagreeing_seeds.push(seed);
        }
        if args.jsonl {
            println!("{}", records.interp.to_jsonl());
            println!("{}", records.datafusion.to_jsonl());
            for quantity in quantities(&records) {
                println!("{}", serde_json::to_string(&quantity)?);
            }
            for difference in &differences {
                println!("{}", serde_json::to_string(difference)?);
            }
        } else {
            print_runtime_comparison(seed, &records, &differences);
        }
    }
    if disagreeing_seeds.is_empty() {
        Ok(())
    } else {
        anyhow::bail!(
            "the two runtimes read different answers out of the same plan on seed(s) {}",
            disagreeing_seeds
                .iter()
                .map(u64::to_string)
                .collect::<Vec<_>>()
                .join(", ")
        )
    }
}

fn print_runtime_comparison(
    seed: u64,
    records: &RuntimeRecords,
    differences: &[ReadoutDifference],
) {
    println!("query    {}", records.interp.plan.query);
    println!(
        "plan     {}  ({} nodes, {} edges)",
        &records.interp.plan.plan_id[..16],
        records.interp.plan.nodes,
        records.interp.plan.edges
    );
    println!(
        "rows     {} scanned, seed {seed}",
        records.interp.rows_scanned
    );
    println!(
        "plan id  {}",
        if records.interp.plan.plan_id == records.datafusion.plan.plan_id {
            "the same document on both runtimes"
        } else {
            "DIFFERENT documents — the two runtimes did not run the same plan"
        }
    );

    println!("\naccuracy");
    let pairs = readout_pairs(records);
    if pairs.is_empty() {
        println!(
            "  no readout on either runtime: this plan carries no SummaryEstimate, so there is \
             no approximate answer to compare"
        );
    }
    for pair in &pairs {
        println!("  {}", pair.readout);
        print_readout_side("interp     ", &pair.interp);
        print_readout_side("datafusion ", &pair.datafusion);
        let verdict = if pair.differences().is_empty() {
            "identical, bit for bit"
        } else {
            "DIFFERENT"
        };
        println!("    {verdict}");
    }
    if differences.is_empty() {
        println!("  every readout agreed");
    } else {
        for difference in differences {
            println!("  DIFFERENT {difference}");
        }
    }

    println!("\ntime and memory");
    println!(
        "  the three memory rows are separate quantities and are not summed into one number; \
         a ratio between two rows marked NO has no meaning"
    );
    println!(
        "  {:<12} {:<18} {:>14} {:>14}  same measurement",
        "arm", "quantity", "interp", "datafusion"
    );
    for quantity in quantities(records) {
        println!(
            "  {:<12} {:<18} {:>14} {:>14}  {}",
            quantity.arm,
            quantity.name,
            quantity.interp.as_deref().unwrap_or("-"),
            quantity.datafusion.as_deref().unwrap_or("-"),
            if quantity.same_measurement {
                "yes"
            } else {
                "NO"
            }
        );
        println!("               {}", quantity.reason);
    }
}

fn print_readout_side(runtime: &str, held: &[ReadoutRecord]) {
    match held {
        [] => println!("    {runtime} produced no readout of this name"),
        [only] => println!(
            "    {runtime} approximate {}   exact {}",
            answer_column(Some(&only.approximate)),
            answer_column(only.exact.as_ref())
        ),
        many => {
            println!(
                "    {runtime} produced {} readouts of this name",
                many.len()
            );
            for one in many {
                println!(
                    "      approximate {}   exact {}",
                    answer_column(Some(&one.approximate)),
                    answer_column(one.exact.as_ref())
                );
            }
        }
    }
}

fn answer_column(answer: Option<&AnswerRecord>) -> String {
    match answer {
        Some(answer) => format!("{answer:.6}"),
        None => "not computed".to_string(),
    }
}

const PRINTED_ROWS: usize = 20;

fn exact_block(query: &str, evaluated: &ExactRun) -> String {
    let mut out = format!(
        "query    {query}\nrows     {} scanned, {} emitted\n",
        evaluated.rows_scanned, evaluated.rows_emitted
    );
    match &evaluated.answer {
        Data::Scalar(value) => out.push_str(&format!("exact    scalar {value}\n")),
        Data::Rows(rows) => {
            let names: Vec<String> = match evaluated.root.output_schema() {
                Ok(schema) => schema
                    .columns
                    .iter()
                    .map(|column| column.name.clone())
                    .collect(),
                Err(_) => Vec::new(),
            };
            out.push_str(&format!(
                "exact    {} rows [{}]\n",
                rows.len(),
                names.join(", ")
            ));
            for row in rows.iter().take(PRINTED_ROWS) {
                let rendered: Vec<String> = row.0.iter().map(render).collect();
                out.push_str(&format!("  {}\n", rendered.join("  ")));
            }
            if rows.len() > PRINTED_ROWS {
                out.push_str(&format!("  ... {} more\n", rows.len() - PRINTED_ROWS));
            }
        }
    }
    out
}

fn render(value: &Value) -> String {
    match value {
        Value::Null => "null".to_owned(),
        Value::Int(held) => held.to_string(),
        Value::Float(held) => format!("{held:?}"),
        Value::Str(held) => held.clone(),
        Value::Timestamp(held) => held.to_string(),
    }
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
                "  node {}  {:<16} state {:>9}  {}{}",
                node.node,
                node.operator,
                state,
                node.family.as_deref().unwrap_or(""),
                node_time(node)
            );
        }
        for node in &record.pre_asap_nodes {
            println!(
                "  tree {}  {:<16} {} ms",
                node.node,
                node.operator,
                ms(node.elapsed_ns)
            );
        }
        match record.rows_emitted {
            Some(emitted) => println!(
                "rows     {} scanned, {emitted} emitted",
                record.rows_scanned
            ),
            None => println!(
                "rows     {} scanned, emitted not counted on this runtime",
                record.rows_scanned
            ),
        }
        if let Some(root_rows) = record.root_rows {
            println!("result   {root_rows} rows out of node {}", record.plan.root);
        }
        if record.no_summary_in_plan {
            println!(
                "no summary in plan  the planner left the graph whole, so the approximate arm is \
                 the pre-ASAP arm and its ratios are not an advantage"
            );
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

fn node_time(node: &NodeCost) -> String {
    let mut out = String::new();
    for (name, elapsed_ns) in [
        ("bind", node.build_ns),
        ("insert", node.update_ns),
        ("readout", node.readout_ns),
        ("compute", node.elapsed_compute_ns),
    ] {
        if let Some(elapsed_ns) = elapsed_ns {
            out.push_str(&format!("  {name} {} ms", ms(elapsed_ns)));
        }
    }
    out
}

fn ms(elapsed_ns: u64) -> String {
    format!("{:.4}", elapsed_ns as f64 / 1e6)
}

fn print_advantage(record: &PlanEvalRecord, worst_accuracy: Option<f64>, seeds: u64) {
    println!("runtime  {}", record.runtime);
    print_phase("bind       ", record.approximate.build.as_ref());
    print_phase("insert     ", record.approximate.update.as_ref());
    print_phase("readout    ", record.approximate.readout.as_ref());
    print_phase("maintenance", record.approximate.maintenance.as_ref());
    print_phase("read       ", record.approximate.read.as_ref());
    match record.pre_asap.evaluate.as_ref() {
        Some(phase) => println!(
            "pre-ASAP  {} rows: query {}   {} B held",
            phase.work,
            spread(phase),
            record.pre_asap.retained_bytes
        ),
        None => println!("pre-ASAP  not measured"),
    }
    print_engine_cost("summary ", &record.approximate);
    print_engine_cost("pre-ASAP", &record.pre_asap);
    if record.answer_check != AnswerCheck::ExactArmDidNotRun {
        println!(
            "ground truth  {} B retained, untimed",
            record.exact.retained_bytes
        );
    }
    println!("answer check  {}", record.answer_check);

    let advantage = record.advantage();
    let tree = record.pre_asap.evaluate.as_ref().map(mean);
    println!();
    println!(
        "advantage over {} rows of input, {seeds} seed(s)",
        record.rows_scanned
    );
    print_ratio(
        "aggregate time",
        advantage.aggregate_time,
        tree.map(|ms| format!("{ms:.4} ms")),
        positive(record.approximate.aggregate_ms()).map(|ms| format!("{ms:.4} ms")),
    );
    print_ratio(
        "query time    ",
        advantage.query_time,
        tree.map(|ms| format!("{ms:.4} ms")),
        positive(record.approximate.query_ms()).map(|ms| format!("{ms:.4} ms")),
    );
    print_ratio(
        "memory        ",
        advantage.memory,
        Some(format!("{} B", record.pre_asap.memory_bytes())),
        Some(format!("{} B", record.approximate.memory_bytes())),
    );
    println!("  memory column   {}", record.memory_column);
    match worst_accuracy {
        Some(error) => println!(
            "  accuracy        worst readout error {error:.6}, in that readout's own metric"
        ),
        None => println!("  accuracy        not measured (nothing was verified)"),
    }
}

fn mean(phase: &Phase) -> f64 {
    phase.elapsed_ms.mean
}

fn positive(ms: f64) -> Option<f64> {
    (ms > 0.0).then_some(ms)
}

fn print_engine_cost(name: &str, arm: &Arm) {
    let mut line = String::new();
    for (phase, peak) in [
        ("maintenance", arm.maintenance_peak_reserved_bytes),
        ("read", arm.read_peak_reserved_bytes),
        ("evaluate", arm.evaluate_peak_reserved_bytes),
    ] {
        if let Some(peak) = peak {
            line.push_str(&format!("  {peak} B {phase} peak in the memory pool"));
        }
    }
    if let Some(overhead_ns) = arm.engine_overhead_ns {
        line.push_str(&format!(
            "  {} ms in operators carrying no node's alias",
            ms(overhead_ns)
        ));
    }
    if !line.is_empty() {
        println!("{name} {line}");
    }
}

fn print_ratio(
    name: &str,
    ratio: Option<f64>,
    without_approximation: Option<String>,
    with_approximation: Option<String>,
) {
    match (ratio, without_approximation, with_approximation) {
        (Some(ratio), Some(without), Some(with)) => println!(
            "  {name}  {without} without approximation vs {with} with approximation  =>  {ratio:.1}x"
        ),
        _ => println!("  {name}  not comparable (one of the two sides was not measured)"),
    }
}

fn print_phase(name: &str, approximate: Option<&Phase>) {
    let Some(approximate) = approximate else {
        return;
    };
    println!(
        "{name}  {} units: summary {}",
        approximate.work,
        spread(approximate)
    );
}

fn spread(phase: &Phase) -> String {
    format!(
        "{:.4} ms +/- {:.4} (n={})",
        phase.elapsed_ms.mean, phase.elapsed_ms.stddev, phase.elapsed_ms.n
    )
}
