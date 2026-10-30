use std::path::PathBuf;
use std::time::Instant;

use anyhow::{Context, Result};
use clap::Parser;
use serde_json::{json, Value};

use aqpbm_inprocess::data::generate;
use aqpbm_inprocess::plan::{plan, summary, Arm};
use aqpbm_inprocess::run::Driver;
use aqpbm_inprocess::spec::Spec;

#[derive(Parser, Debug)]
#[command(
    name = "aqpbm-inprocess",
    about = "Plan each query of a spec with ASAPPlanner and run the plan with its executor at every evaluation time"
)]
struct Args {
    #[arg(long)]
    spec: PathBuf,

    #[arg(
        long,
        help = "Accuracy target the plan is sized for; omitted plans the exact answer"
    )]
    max_error: Option<f64>,

    #[arg(long, default_value_t = 0.01)]
    failure_probability: f64,

    #[arg(long, help = "Plan for a deployment that keeps no raw samples")]
    no_raw: bool,

    #[arg(
        long,
        help = "Accept a plan whose output has no accuracy guarantee for the target"
    )]
    ignore_guarantees: bool,

    #[arg(long = "query", help = "Only these query ids; repeatable")]
    queries: Vec<String>,

    #[arg(
        long,
        help = "Also run the plan's query-time reference at every evaluation and count disagreements"
    )]
    check_reference: bool,

    #[arg(long)]
    out: PathBuf,
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<()> {
    let args = Args::parse();
    let spec = Spec::read(&args.spec)?;
    let arm = Arm {
        max_error: args.max_error,
        failure_probability: args.failure_probability,
        raw_data_retained: !args.no_raw,
        ignore_guarantees: args.ignore_guarantees,
    };
    let generated = Instant::now();
    let series = generate(&spec)?;
    let samples: usize = series.iter().map(|s| s.points.len()).sum();
    eprintln!(
        "generated {} samples in {} series in {:.1} s",
        samples,
        series.len(),
        generated.elapsed().as_secs_f64()
    );
    let mut queries = Vec::new();
    for query in &spec.queries {
        if !args.queries.is_empty() && !args.queries.contains(&query.id) {
            continue;
        }
        let planned = match plan(&spec, query, &arm).await {
            Ok(planned) => planned,
            Err(reason) => {
                eprintln!("{}: unsupported: {reason}", query.id);
                queries
                    .push(json!({"id": query.id, "promql": query.promql, "unsupported": reason}));
                continue;
            }
        };
        let plan_summary = summary(&planned, arm.max_error.map(|_| arm.failure_probability))?;
        eprintln!("{}: plan {}", query.id, plan_summary);
        let mut driver = Driver::new(planned.plan, args.check_reference)
            .with_context(|| format!("installing the plan of {}", query.id))?;
        let started = Instant::now();
        let mut answers: Vec<Value> = Vec::new();
        let mut disagreements = 0usize;
        let mut views_refreshed = 0usize;
        let times = spec.evaluation_times(query);
        for &t in &times {
            let evaluation = driver
                .evaluate(&series, t)
                .with_context(|| format!("{} at {t}", query.id))?;
            views_refreshed += evaluation.views_refreshed;
            if let Some(reference) = &evaluation.reference_answers {
                if reference != &evaluation.answers {
                    disagreements += 1;
                }
            }
            for (labels, value) in evaluation.answers {
                answers.push(json!({"time": t, "series": labels, "value": value}));
            }
        }
        let elapsed = started.elapsed().as_secs_f64();
        eprintln!(
            "{}: {} evaluations in {:.1} s, {} view refreshes{}",
            query.id,
            times.len(),
            elapsed,
            views_refreshed,
            if args.check_reference {
                format!(", {disagreements} disagreements with the query-time reference")
            } else {
                String::new()
            }
        );
        queries.push(json!({
            "id": query.id,
            "promql": query.promql,
            "unit": query.unit,
            "threshold": query.threshold,
            "plan": plan_summary,
            "evaluations": times.len(),
            "evaluation_seconds": elapsed,
            "views_refreshed": views_refreshed,
            "reference_disagreements": args.check_reference.then_some(disagreements),
            "answers": answers,
        }));
    }
    let document = json!({
        "use_case": spec.use_case,
        "arm": arm.describe(),
        "planner_rev": env!("ASAP_PLANNER_REV"),
        "samples": samples,
        "queries": queries,
    });
    std::fs::write(&args.out, serde_json::to_string(&document)?)
        .with_context(|| format!("writing {}", args.out.display()))?;
    Ok(())
}
