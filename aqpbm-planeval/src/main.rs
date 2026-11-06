use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::Instant;

use anyhow::{bail, Context, Result};
use clap::Parser;
use serde_json::{json, Value};

use aqpbm_planeval::data::{generate, Series};
use aqpbm_planeval::plan::{plan, summary, Arm};
use aqpbm_planeval::run::Driver;
use aqpbm_planeval::spec::Spec;

#[derive(Parser, Debug)]
#[command(
    name = "planeval",
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

fn rank_in_window(
    series: &[Series],
    labels: &BTreeMap<String, String>,
    start_exclusive: i64,
    end_inclusive: i64,
    value: f64,
) -> Result<(f64, f64)> {
    let unnamed = |l: &BTreeMap<String, String>| {
        l.iter()
            .filter(|(k, _)| k.as_str() != "__name__")
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect::<BTreeMap<_, _>>()
    };
    let wanted = unnamed(labels);
    let mut matched = series.iter().filter(|s| unnamed(&s.labels) == wanted);
    let (Some(found), None) = (matched.next(), matched.next()) else {
        bail!("no single series has the answer's labels {labels:?}");
    };
    found
        .rank_of(start_exclusive, end_inclusive, value)
        .with_context(|| format!("series {labels:?} has no sample in the window"))
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
        let mut costs: Vec<Value> = Vec::new();
        let ranked = query.accuracy_metric.as_deref() == Some("rank");
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
            let cost = &evaluation.cost;
            costs.push(json!({
                "time": t,
                "refresh_seconds": cost.refresh_seconds,
                "samples_ingested": cost.samples_ingested,
                "query_seconds": cost.query_seconds,
                "compile_seconds": cost.compile_seconds,
                "samples_read_at_query": cost.samples_read_at_query,
                "state_bytes": cost.state_bytes,
            }));
            for (labels, value) in evaluation.answers {
                let mut answer = json!({"time": t, "series": labels, "value": value});
                if ranked {
                    let (below, at_most) =
                        rank_in_window(&series, &labels, t - query.window_ms, t, value)
                            .with_context(|| format!("{} at {t}", query.id))?;
                    answer["rank"] = json!([below, at_most]);
                }
                answers.push(answer);
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
            "costs": costs,
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
