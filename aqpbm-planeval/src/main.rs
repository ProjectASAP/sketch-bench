use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::time::Instant;

use anyhow::{Context, Result};
use aqpbm_core::accuracy::scalar_error;
use clap::Parser;
use serde_json::{json, Value};

use aqpbm_planeval::accuracy::{keyed, without_name, AnswerAt, Labels, QueryTruth, Scores};
use aqpbm_planeval::data::generate;
use aqpbm_planeval::plan::{plan, summary, workload, Arm};
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

    #[arg(
        long,
        help = "Output of the exact run of the same spec; each answer is then also scored against the exact plan's answer"
    )]
    exact: Option<PathBuf>,

    #[arg(long)]
    out: PathBuf,
}

fn exact_answers(path: &Path) -> Result<BTreeMap<String, Vec<AnswerAt>>> {
    let text =
        std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    let document: Value =
        serde_json::from_str(&text).with_context(|| format!("parsing {}", path.display()))?;
    let mut out = BTreeMap::new();
    for query in document["queries"]
        .as_array()
        .with_context(|| format!("{} has no queries", path.display()))?
    {
        let id = query["id"]
            .as_str()
            .with_context(|| format!("a query in {} has no id", path.display()))?;
        let mut answers = Vec::new();
        for answer in query["answers"].as_array().into_iter().flatten() {
            let (Some(time), Some(value)) = (answer["time"].as_i64(), answer["value"].as_f64())
            else {
                continue;
            };
            let labels: Labels = serde_json::from_value(answer["series"].clone())?;
            answers.push((time, labels, value));
        }
        out.insert(id.to_string(), answers);
    }
    Ok(out)
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
    let exact = args.exact.as_deref().map(exact_answers).transpose()?;
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
        let mut answered: Vec<AnswerAt> = Vec::new();
        let mut costs: Vec<Value> = Vec::new();
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
            answered.extend(
                evaluation
                    .answers
                    .into_iter()
                    .map(|(labels, value)| (t, labels, value)),
            );
        }
        let elapsed = started.elapsed().as_secs_f64();
        let truth = QueryTruth::of(&workload(&spec, query, &arm)?)?;
        let computed_truth = truth
            .as_ref()
            .map(|truth| truth.score(&series, &times, &answered))
            .transpose()
            .with_context(|| format!("scoring {} against its computed truth", query.id))?;
        let reference = exact
            .as_ref()
            .map(|exact| keyed(exact.get(&query.id).map_or(&[][..], |a| a.as_slice())));
        let rank_errors: HashMap<(i64, Labels), Scores> = match (&truth, &reference) {
            (Some(truth), Some(reference)) => truth
                .reference_rank_errors(&series, &times, &answered, reference)
                .with_context(|| format!("scoring {} against the exact plan", query.id))?
                .into_iter()
                .filter_map(|(t, labels, scores)| Some(((t, labels), scores?)))
                .collect(),
            _ => HashMap::new(),
        };
        let answers: Vec<Value> = answered
            .iter()
            .map(|(t, labels, value)| {
                let mut answer = json!({"time": t, "series": labels, "value": value});
                let key = (*t, without_name(labels));
                if let Some(&exact_value) = reference.as_ref().and_then(|r| r.get(&key)) {
                    let mut errors = scalar_error(exact_value, &[*value]);
                    if let Some(rank_error) =
                        rank_errors.get(&key).and_then(|s| s.get("rank_error"))
                    {
                        errors.insert("rank_error".into(), *rank_error);
                    }
                    answer["exact_plan"] = json!(errors);
                }
                answer
            })
            .collect();
        let computed_truth: Option<Vec<Value>> = computed_truth.map(|cells| {
            cells
                .into_iter()
                .map(|(t, labels, scores)| json!({"time": t, "series": labels, "scores": scores}))
                .collect()
        });
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
            "computed_truth": computed_truth,
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
