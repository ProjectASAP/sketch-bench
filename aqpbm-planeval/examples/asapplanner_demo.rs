//! Demo prototype: ASAPPlanner as planeval's runtime.
//!
//! Runs one PromQL query through ASAPPlanner alone, at the rev
//! ASAPQuery-backend pins (05fb36e4, see `aqpbm-planeval/pins.env`):
//! `plan_installable` plans it, and
//! ASAPPlanner's executor runs the plan on samples made up here. No backend,
//! no Prometheus, and the plan stays in memory (no JSON hand-off).
//!
//! The query is `quantile_over_time(0.99, latency_ms[60m])`, every 10 min,
//! over 2 series sampled every second: series `a` holds 1..=3600 over the
//! hour and `b` twice that, so the exact p99 is 3564.01 and 7128.02. It is
//! planned three times:
//!
//! - A: no accuracy requirement, so the planner picks the exact plan.
//! - B: 1% rank error allowed, so it picks a KLL built at query time.
//! - C: 1% rank error and raw data not kept, so it picks six 10-min KLL
//!   panes built at ingestion and merged at query time.
//!
//! For each it prints the selected plan's DAG, what the runtime must store
//! and read, and each answer against the exact one.
//!
//! ```sh
//! cargo run --release -p aqpbm-planeval --example asapplanner_demo
//! ```
//!
//! This is a prototype only. ASAPPlanner is a dev-dependency of planeval,
//! so only examples and tests build it; the `planeval` binary runs the
//! planner and executor through ASAPQuery-backend instead.
use std::collections::BTreeMap;

use asap_executor::operators::Operator;
use asap_executor::physical_planner::{compile, InputContract, Source};
use asap_executor::runtime::{Limits, RunContext, Scope};
use asap_executor::values::{Batch, Value};
use asap_executor::{series_labels_of_output_row, RawSampleSelector, Sample};
use asap_planner::{
    plan_installable, DeploymentCapabilities, FrontendInput, PhysicalASAPDAG, PlanningModels,
    PlanningWorkload, StoredOutputBuild, UserInput,
};
use futures::{executor::block_on, StreamExt};

const QUERY: &str = "quantile_over_time(0.99, latency_ms[60m])";
/// The evaluation time: 2 hours after the epoch, on the 10-minute grid.
const T: i64 = 7_200_000;

/// What we tell the planner besides the query text.
fn workload(accuracy: &str) -> PlanningWorkload {
    serde_json::from_str(&format!(
        r#"{{"query_workload": {{"language": "promql", "repeating_queries": [{{
              "query": "{QUERY}", "demand": {{"fixed_interval": 600000}},
              "requirements": {accuracy},
              "predictability": {{"predictable": {{"known_at": null}}}}}}]}},
            "data_workload": {{"arrival": {{"ingesting": {{"rows_per_second": 2.0}}}},
              "sources": {{"latency_ms": {{"series_count": 2, "sample_interval": 1000}}}}}}}}"#
    ))
    .unwrap()
}

/// Series a holds 1, 2, ..., 3600 over the hour ending at T; series b holds twice that.
fn samples() -> Vec<Sample> {
    let mut out = Vec::new();
    for (instance, scale) in [("a", 1.0), ("b", 2.0)] {
        for k in 1..=3600i64 {
            out.push(Sample {
                labels: BTreeMap::from([
                    ("__name__".into(), "latency_ms".into()),
                    ("instance".into(), instance.into()),
                ]),
                timestamp_ms: T - 3_600_000 + k * 1000,
                value: scale * k as f64,
            });
        }
    }
    out
}

/// Prometheus's quantile_over_time: linear interpolation at rank q * (n - 1).
fn exact(scale: f64) -> f64 {
    let rank: f64 = 0.99 * 3599.0;
    let (lo, frac) = (rank.floor(), rank - rank.floor());
    scale * ((lo + 1.0) + frac)
}

fn run(
    dag: &PhysicalASAPDAG,
    inputs: BTreeMap<u64, Batch>,
    roots: &[u64],
    scope: Scope,
) -> Vec<Batch> {
    let contracts = inputs
        .iter()
        .map(|(&id, b)| (id, InputContract::bounded(b.schema().clone())))
        .collect();
    let compiled = compile(dag, contracts, roots).expect("the executor compiles the plan");
    let sources = inputs
        .into_iter()
        .map(|(id, b)| {
            (
                id,
                Box::new(Operator::source(b.schema().clone(), vec![b]).unwrap()) as Source<'_>,
            )
        })
        .collect();
    let instance = compiled.instantiate(sources).unwrap();
    let context = RunContext::new(scope, Limits::default()).unwrap();
    let mut out = instance
        .execute(compiled.roots(), context)
        .unwrap()
        .remove(0);
    block_on(async {
        let mut batches = Vec::new();
        while let Some(b) = out.next().await {
            batches.push((*b.unwrap()).clone());
        }
        batches
    })
}

/// A node's operator, e.g. `NonASAP/Scan`, plus the sketch for a summary build.
fn describe(dag: &PhysicalASAPDAG, id: u64) -> String {
    let node = dag.nodes.iter().find(|n| n.id as u64 == id).unwrap();
    let v = serde_json::to_value(&node.payload).unwrap();
    let (layer, inner) = v.as_object().unwrap().iter().next().unwrap();
    let op = inner
        .as_object()
        .map(|m| m.keys().next().unwrap().clone())
        .unwrap_or_else(|| inner.to_string());
    let mut s = format!("{layer}/{op}");
    if let Some(family) = inner.pointer(&format!("/{op}/family/Sketch/0")) {
        s += &format!("  {} {}", family["algorithm"], family["params"]);
    }
    s
}

async fn show(title: &str, accuracy: &str, caps: DeploymentCapabilities) {
    println!("\n=============== {title}");
    // 1. Plan: one call into the planner, the result stays in memory.
    let wl = workload(accuracy);
    let plan = plan_installable(UserInput::new(
        &wl,
        FrontendInput::Promql { histograms: None },
        PlanningModels::builtin().with_deployment_capabilities(&caps),
    ))
    .await
    .expect("the planner plans the query");
    let dag = &plan.dag.dag;
    let sel = plan.selection.as_ref().unwrap();
    println!(
        "selected plan {}  (estimated cost {:?} per second)",
        sel.selected_candidate_id, sel.cost_per_second
    );
    println!("DAG, {} nodes:", dag.nodes.len());
    for n in &dag.nodes {
        let inputs: Vec<_> = dag
            .edges
            .iter()
            .filter(|e| e.consumer == n.id)
            .map(|e| e.producer)
            .collect();
        println!(
            "  node {:>2} <- {:<8} {}",
            n.id,
            format!("{inputs:?}"),
            describe(dag, n.id as u64)
        );
    }
    println!("answers query at node {}", plan.queries[0].root_node_id);
    println!(
        "stored outputs (built before the query, kept): {}",
        plan.stored_outputs.len()
    );
    for o in &plan.stored_outputs {
        let windows: Vec<String> = o
            .inputs
            .raw_sample_windows
            .iter()
            .map(|w| {
                format!(
                    "{} min ending {} min before T",
                    w.width_ms / 60_000,
                    w.end_offset_ms / 60_000
                )
            })
            .collect();
        println!(
            "  node {:>2}: built in {:?} from raw windows {:?} and stored outputs {:?}, kept {} min",
            o.node_id,
            o.built_at,
            windows,
            o.inputs.stored_outputs,
            o.retention_ms / 60_000
        );
    }
    println!(
        "raw windows read at query time: {}",
        plan.query_time_raw_windows.len()
    );
    for w in &plan.query_time_raw_windows {
        println!(
            "  {} min of raw samples ending at T, into node {}",
            w.width_ms / 60_000,
            w.time_range_node_id
        );
    }

    // 2. Execute with the planner's own executor.
    let all = samples();
    let mut inputs: BTreeMap<u64, Batch> = BTreeMap::new();
    // Stored outputs come in dependency order: each after those it reads.
    for o in &plan.stored_outputs {
        let mut build_inputs = BTreeMap::new();
        let (mut start, mut end) = (T, T);
        for w in &o.inputs.raw_sample_windows {
            let (s, e) = w.bounds_ms(T);
            (start, end) = (start.min(s), end.max(e));
            let rows = RawSampleSelector::new(dag, w)
                .unwrap()
                .window_batch(e, &all)
                .unwrap();
            build_inputs.insert(w.time_range_node_id as u64, rows);
        }
        for &id in &o.inputs.stored_outputs {
            build_inputs.insert(id as u64, inputs[&(id as u64)].clone());
        }
        let scope = match o.built_at {
            StoredOutputBuild::IngestionRun => Scope::Ingestion {
                window_start_ms: start,
                window_end_ms: end,
                revision: 1,
            },
            StoredOutputBuild::QueryRun => Scope::Query {
                evaluation_time_ms: T,
                revision: 1,
            },
        };
        let built = run(dag, build_inputs, &[o.node_id as u64], scope);
        let rows = built.iter().flat_map(|b| b.to_rows().to_vec()).collect();
        inputs.insert(
            o.node_id as u64,
            Batch::try_new(built[0].schema().clone(), rows).unwrap(),
        );
    }
    for w in &plan.query_time_raw_windows {
        let (_, end) = w.bounds_ms(T);
        inputs.insert(
            w.time_range_node_id as u64,
            RawSampleSelector::new(dag, w)
                .unwrap()
                .window_batch(end, &all)
                .unwrap(),
        );
    }
    let root = plan.queries[0].root_node_id as u64;
    let out = run(
        dag,
        inputs,
        &[root],
        Scope::Query {
            evaluation_time_ms: T,
            revision: 1,
        },
    );

    // 3. Compare with the exact answer.
    println!("answers at T:");
    for b in &out {
        for row in b.to_rows() {
            let labels = series_labels_of_output_row(b.schema(), &row).unwrap();
            let v = row
                .iter()
                .find_map(|v| {
                    if let Value::Float64(x) = v {
                        Some(*x)
                    } else {
                        None
                    }
                })
                .unwrap();
            let truth = exact(if labels["instance"] == "a" { 1.0 } else { 2.0 });
            println!(
                "  instance={}  answer {:>9.2}   exact {:>9.2}   relative error {:.5}",
                labels["instance"],
                v,
                truth,
                (v - truth).abs() / truth
            );
        }
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let approx = r#"{"accuracy": {"accuracy_metric": "rank", "max_error": 0.01,
                    "failure_probability": 0.01, "accuracy_scope": "whole_result"}}"#;
    show(
        "A. query only, exact (the default)",
        "{}",
        DeploymentCapabilities::default(),
    )
    .await;
    show(
        "B. allow 1% rank error",
        approx,
        DeploymentCapabilities::default(),
    )
    .await;
    show(
        "C. allow 1% rank error, raw data not kept",
        approx,
        DeploymentCapabilities {
            raw_data_retained: false,
            ..Default::default()
        },
    )
    .await;
}
