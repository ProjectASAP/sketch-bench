use std::time::Instant;

use anyhow::{Context, Result};
use asap_planner::{
    plan_installable, CostInputs, DeploymentCapabilities, FrontendInput, InstallablePlan,
    PlanningModels, PlanningWorkload, SelectedPlanReport, UserInput,
};
use serde_json::{json, Value};

use crate::sketch_analytics::{analytics, statistics};
use crate::spec::{QuerySpec, Spec};

#[derive(Debug, Clone, Copy)]
pub struct Arm {
    pub max_error: Option<f64>,
    pub failure_probability: f64,
    pub raw_data_retained: bool,
    pub ignore_guarantees: bool,
}

impl Arm {
    pub fn describe(&self) -> Value {
        json!({
            "max_error": self.max_error,
            "failure_probability": self.max_error.map(|_| self.failure_probability),
            "raw_data_retained": self.raw_data_retained,
            "ignore_guarantees": self.ignore_guarantees,
        })
    }
}

pub fn workload(spec: &Spec, query: &QuerySpec, arm: &Arm) -> Result<PlanningWorkload> {
    let requirements = match arm.max_error {
        None => json!({}),
        Some(max_error) => {
            let mut accuracy = json!({
                "max_error": max_error,
                "failure_probability": arm.failure_probability,
            });
            if let Some(metric) = &query.accuracy_metric {
                accuracy["accuracy_metric"] = json!(metric);
            }
            json!({"accuracy": accuracy})
        }
    };
    let rows_per_second: f64 = spec.metrics.iter().map(|m| m.rows_per_second()).sum();
    let mut sources = serde_json::Map::new();
    for metric in &spec.metrics {
        let mut source = json!({
            "series_count": metric.series_count(),
            "sample_interval": metric.planned_sample_interval_ms(),
        });
        if let Some(distinct) = metric.max_distinct_per_evaluation {
            source["max_distinct_per_evaluation"] = json!(distinct);
        }
        if metric.non_negative {
            source["values_non_negative"] = json!(true);
        }
        sources.insert(metric.name.clone(), source);
    }
    let document = json!({
        "query_workload": {
            "language": "promql",
            "repeating_queries": [{
                "query": query.promql,
                "demand": {"fixed_interval": query.interval_ms},
                "requirements": requirements,
                "predictability": {"predictable": {"known_at": null}},
            }],
        },
        "data_workload": {
            "arrival": {"ingesting": {"rows_per_second": rows_per_second}},
            "sources": sources,
        },
    });
    serde_json::from_value(document).context("building the planning workload")
}

pub struct Planned {
    pub plan: InstallablePlan,
    pub report: Option<SelectedPlanReport>,
    pub planning_ms: f64,
}

pub async fn plan(spec: &Spec, query: &QuerySpec, arm: &Arm) -> Result<Planned, String> {
    let workload = workload(spec, query, arm).map_err(|e| format!("{e:#}"))?;
    let capabilities = DeploymentCapabilities {
        raw_data_retained: arm.raw_data_retained,
        ..Default::default()
    };
    let plan_selection = CostInputs {
        ignore_accuracy_guarantees: arm.ignore_guarantees,
        ..Default::default()
    };
    let started = Instant::now();
    let (plan, report) = plan_installable(UserInput::new(
        &workload,
        FrontendInput::Promql { histograms: None },
        PlanningModels::builtin()
            .with_deployment_capabilities(&capabilities)
            .with_plan_selection(&plan_selection),
    ))
    .await
    .map_err(|e| e.to_string())?;
    Ok(Planned {
        plan,
        report,
        planning_ms: started.elapsed().as_secs_f64() * 1e3,
    })
}

fn sketches(node: &Value, out: &mut Vec<Value>) {
    match node {
        Value::Object(map) => {
            match map.get("Sketch") {
                Some(Value::Array(parts)) if !parts.is_empty() => out.push(parts[0].clone()),
                Some(sketch) => out.push(sketch.clone()),
                None => {}
            }
            for value in map.values() {
                sketches(value, out);
            }
        }
        Value::Array(items) => items.iter().for_each(|item| sketches(item, out)),
        _ => {}
    }
}

pub fn summary(planned: &Planned, failure_probability: Option<f64>) -> Result<Value> {
    let dag = &planned.plan.dag;
    let mut all = Vec::new();
    let mut planned_statistics = Vec::new();
    for node in &dag.nodes {
        let payload = serde_json::to_value(&node.payload)?;
        sketches(&payload, &mut all);
        statistics(&payload, &mut planned_statistics);
    }
    let mut found: Vec<Value> = Vec::new();
    for sketch in all {
        if !found.contains(&sketch) {
            found.push(sketch);
        }
    }
    let sketch_analytics = found
        .iter()
        .map(|sketch| analytics(sketch, &planned_statistics, failure_probability))
        .collect::<Result<Vec<_>>>()?;
    let views = planned.plan.materialized_views()?;
    let report = planned.report.as_ref();
    Ok(json!({
        "selected_candidate_id": report.map(|r| r.selected_candidate_id.clone()),
        "cost_per_second": report.and_then(|r| r.cost_per_second),
        "predicted_peak_memory_bytes": report.and_then(|r| r.predicted_usage.as_ref().map(|u| u.peak_memory_bytes)),
        "sketches": found,
        "sketch_analytics": sketch_analytics,
        "materialized_views": views.iter().map(|v| json!({
            "node_id": v.node_id,
            "refresh": v.refresh,
            "retention_ms": v.retention_ms,
        })).collect::<Vec<_>>(),
        "nodes": dag.nodes.len(),
        "planning_ms": planned.planning_ms,
    }))
}
