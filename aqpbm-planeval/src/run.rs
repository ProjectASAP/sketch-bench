use std::collections::{BTreeMap, BTreeSet};
use std::ops::Bound;
use std::sync::Arc;

use anyhow::{anyhow, bail, Result};
use asap_executor::physical_planner::compile;
use asap_executor::runtime::{Limits, RunContext, Scope};
use asap_executor::sources::InMemoryDataStore;
use asap_executor::values::{Batch, Value};
use asap_planner::{InstallablePlan, MaterializedView, MaterializedViewRefresh, PhysicalASAPDAG};
use asap_types::ir::operator::Source;
use asap_types::ir::physical_export::PhysicalASAPOperatorPayload;
use asap_types::ir::schema::{DataType, FieldDataType, Schema, PROMQL_SERIES_IDENTITY};
use asap_types::ir::NonASAPOp;
use futures::{executor::block_on, StreamExt};

use crate::data::{between, up_to, Series, Slice};

pub type Answer = (BTreeMap<String, String>, f64);

struct StoredState {
    state: Batch,
    start_ms: i64,
    retention_ms: u64,
}

pub struct Driver {
    plan: InstallablePlan,
    views: Vec<MaterializedView>,
    raw_lookback_ms: Option<u64>,
    store: BTreeMap<String, StoredState>,
    reference: Option<InstallablePlan>,
}

pub struct Evaluation {
    pub answers: Vec<Answer>,
    pub reference_answers: Option<Vec<Answer>>,
    pub views_refreshed: usize,
}

fn state_key(view: &MaterializedView, evaluation_ms: i64) -> String {
    let absolute = |bound: &Bound<i64>| match bound {
        Bound::Included(at) => Bound::Included(evaluation_ms + at),
        Bound::Excluded(at) => Bound::Excluded(evaluation_ms + at),
        Bound::Unbounded => Bound::Unbounded,
    };
    let mut selection = view.coverage.selection.clone();
    let timed = selection.iter().any(|b| b.relative_time.is_some());
    for selection_box in &mut selection {
        selection_box.relative_time = selection_box
            .relative_time
            .as_ref()
            .map(|(lower, upper)| (absolute(lower), absolute(upper)));
    }
    serde_json::json!({
        "definition_id": view.coverage.definition_id,
        "selection": selection,
        "evaluation_ms": (!timed).then_some(evaluation_ms),
    })
    .to_string()
}

fn row(
    schema: &Schema,
    labels: &BTreeMap<String, String>,
    identity: &str,
    point: (i64, f64),
) -> Vec<Value> {
    schema
        .fields
        .iter()
        .enumerate()
        .map(|(index, field)| match field.name.as_str() {
            PROMQL_SERIES_IDENTITY => Value::Utf8(identity.into()),
            _ if Some(index) == schema.time_index => Value::Timestamp(point.0),
            "value" if field.dtype == FieldDataType::Plain(DataType::Float64) => {
                Value::Float64(point.1)
            }
            label => Value::Utf8(labels.get(label).cloned().unwrap_or_default().into()),
        })
        .collect()
}

fn time_series_store(dag: &PhysicalASAPDAG, slices: &[Slice]) -> Result<InMemoryDataStore> {
    let mut schemas: BTreeMap<&str, Schema> = BTreeMap::new();
    for node in &dag.nodes {
        let PhysicalASAPOperatorPayload::NonASAP(NonASAPOp::Scan {
            source: Source::TimeSeries { metric },
            schema,
            ..
        }) = &node.payload
        else {
            continue;
        };
        let known = schemas.entry(metric).or_insert_with(|| schema.clone());
        for field in &schema.fields {
            if !known.fields.iter().any(|f| f.name == field.name) {
                known.fields.push(field.clone());
            }
        }
    }
    let mut store = InMemoryDataStore::default();
    for (metric, schema) in schemas {
        let schema = Arc::new(schema);
        let mut rows = Vec::new();
        for (series, points) in slices {
            if series.labels.get("__name__").map(String::as_str) != Some(metric) {
                continue;
            }
            let identity = serde_json::to_string(&series.labels)?;
            rows.extend(
                points
                    .iter()
                    .map(|&p| row(&schema, &series.labels, &identity, p)),
            );
        }
        let batch = Batch::try_new(schema.clone(), rows).map_err(|e| anyhow!("{e}"))?;
        store = store
            .with_raw_data(
                Source::TimeSeries {
                    metric: metric.to_string(),
                },
                schema,
                vec![batch],
            )
            .map_err(|e| anyhow!("{e}"))?;
    }
    Ok(store)
}

fn execute(
    dag: &PhysicalASAPDAG,
    views: BTreeMap<u64, Batch>,
    slices: &[Slice],
    ids: &[u64],
    scope: Scope,
) -> Result<Vec<Vec<Batch>>> {
    let read: Vec<MaterializedView> = if views.is_empty() {
        Vec::new()
    } else {
        dag.materialized_views()?
            .into_iter()
            .filter(|view| views.contains_key(&(view.node_id as u64)))
            .collect()
    };
    if read.len() != views.len() {
        bail!("a bound state is not one of the plan's materialized views");
    }
    let compiled = compile(dag, &read, ids).map_err(|e| anyhow!("compiling: {e}"))?;
    let mut store = time_series_store(dag, slices)?;
    for (id, batch) in views {
        store = store
            .with_materialized_view(id as _, vec![batch])
            .map_err(|e| anyhow!("{e}"))?;
    }
    let instance = compiled
        .instantiate(Arc::new(store))
        .map_err(|e| anyhow!("instantiating: {e}"))?;
    block_on(async {
        let context = RunContext::new(scope, Limits::default()).map_err(|e| anyhow!("{e}"))?;
        let outputs = instance
            .execute(compiled.roots(), context)
            .map_err(|e| anyhow!("executing: {e}"))?;
        let mut all = Vec::new();
        for mut output in outputs {
            let mut batches = Vec::new();
            while let Some(batch) = output.next().await {
                batches.push((*batch.map_err(|e| anyhow!("executing: {e}"))?).clone());
            }
            all.push(batches);
        }
        Ok(all)
    })
}

fn series_labels(schema: &Schema, row: &[Value]) -> Result<BTreeMap<String, String>> {
    let mut labels = BTreeMap::new();
    for (field, cell) in schema.fields.iter().zip(row) {
        let Value::Utf8(text) = cell else { continue };
        if field.name == PROMQL_SERIES_IDENTITY {
            let identity: BTreeMap<String, String> = serde_json::from_str(text)?;
            labels.extend(identity);
        } else if !text.is_empty() {
            labels.insert(field.name.clone(), text.to_string());
        }
    }
    Ok(labels)
}

fn answers(batches: &[Batch]) -> Result<Vec<Answer>> {
    let mut out = Vec::new();
    for batch in batches {
        for row in batch.to_rows() {
            let value = row.iter().find_map(|v| match v {
                Value::Float64(x) => Some(*x),
                Value::Int64(x) => Some(*x as f64),
                _ => None,
            });
            let value =
                value.ok_or_else(|| anyhow!("an answer row has no numeric value: {row:?}"))?;
            out.push((series_labels(batch.schema(), &row)?, value));
        }
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(out)
}

impl Driver {
    pub fn new(plan: InstallablePlan, check_reference: bool) -> Result<Driver> {
        plan.validate()?;
        let views = plan.materialized_views()?;
        let mut raw_lookback_ms = Some(0);
        for retention in plan.raw_data_retention()? {
            raw_lookback_ms = match (raw_lookback_ms, retention.lookback_ms) {
                (Some(a), Some(b)) => Some(a.max(b)),
                _ => None,
            };
        }
        let reference = if check_reference {
            Some(asap_planner::test_support::query_time_reference(&plan)?)
        } else {
            None
        };
        Ok(Driver {
            plan,
            views,
            raw_lookback_ms,
            store: BTreeMap::new(),
            reference,
        })
    }

    fn roots(plan: &InstallablePlan) -> Vec<u64> {
        plan.dag.roots.iter().map(|&root| root as u64).collect()
    }

    pub fn evaluate(&mut self, series: &[Series], evaluation_ms: i64) -> Result<Evaluation> {
        let dag = &self.plan.dag;
        let mut views_refreshed = 0;
        let mut built: BTreeMap<u64, (Batch, (i64, i64))> = BTreeMap::new();
        for view in &self.views {
            let built_for_ms = match view.refresh {
                MaterializedViewRefresh::AtIngestion => view
                    .inputs
                    .raw_data_windows
                    .first()
                    .map_or(evaluation_ms, |w| {
                        evaluation_ms - evaluation_ms.rem_euclid(w.width_ms)
                    }),
                MaterializedViewRefresh::OnFirstRead => evaluation_ms,
            };
            let span = view
                .inputs
                .raw_data_windows
                .iter()
                .map(|window| window.bounds_ms(built_for_ms))
                .chain(
                    view.inputs
                        .materialized_views
                        .iter()
                        .map(|id| built[&(*id as u64)].1),
                )
                .reduce(|(a, b), (c, d)| (a.min(c), b.max(d)))
                .ok_or_else(|| anyhow!("materialized view {} reads nothing", view.node_id))?;
            let key = state_key(view, built_for_ms);
            if !self.store.contains_key(&key) {
                let mut sources = BTreeMap::new();
                for id in &view.inputs.materialized_views {
                    sources.insert(*id as u64, built[&(*id as u64)].0.clone());
                }
                let scope = match view.refresh {
                    MaterializedViewRefresh::AtIngestion => Scope::Ingestion {
                        window_start_ms: span.0,
                        window_end_ms: span.1,
                    },
                    MaterializedViewRefresh::OnFirstRead => Scope::Query {
                        evaluation_time_ms: evaluation_ms,
                    },
                };
                let slices = between(series, span.0, span.1);
                let batches =
                    execute(dag, sources, &slices, &[view.node_id as u64], scope)?.remove(0);
                let schema = batches
                    .first()
                    .map(|b| b.schema().clone())
                    .ok_or_else(|| anyhow!("refreshing view {} returned no batch", view.node_id))?;
                let rows = batches.iter().flat_map(|b| b.to_rows().to_vec()).collect();
                self.store.insert(
                    key.clone(),
                    StoredState {
                        state: Batch::try_new(schema, rows).map_err(|e| anyhow!("{e}"))?,
                        start_ms: span.0,
                        retention_ms: view.retention_ms,
                    },
                );
                views_refreshed += 1;
            }
            let stored = self.store.get_mut(&key).expect("just inserted");
            stored.retention_ms = stored.retention_ms.max(view.retention_ms);
            built.insert(view.node_id as u64, (stored.state.clone(), span));
        }
        let mut inputs = BTreeMap::new();
        let mut pending = Self::roots(&self.plan);
        let mut seen = BTreeSet::new();
        while let Some(id) = pending.pop() {
            if !seen.insert(id) {
                continue;
            }
            match built.get(&id) {
                Some((state, _)) => {
                    inputs.insert(id, state.clone());
                }
                None => pending.extend(
                    dag.edges
                        .iter()
                        .filter(|edge| edge.consumer as u64 == id)
                        .map(|edge| edge.producer as u64),
                ),
            }
        }
        let raw = match self.raw_lookback_ms {
            Some(lookback) => between(series, evaluation_ms - lookback as i64, evaluation_ms),
            None => up_to(series, evaluation_ms),
        };
        let query = Scope::Query {
            evaluation_time_ms: evaluation_ms,
        };
        let answers_now = answers(
            &execute(dag, inputs, &raw, &Self::roots(&self.plan), query.clone())?.remove(0),
        )?;
        self.store
            .retain(|_, state| state.start_ms >= evaluation_ms - state.retention_ms as i64);
        let reference_answers = match &self.reference {
            None => None,
            Some(reference) => {
                let lookback = reference
                    .raw_data_retention()?
                    .iter()
                    .map(|r| r.lookback_ms)
                    .try_fold(0u64, |a, b| b.map(|b| a.max(b)));
                let raw = match lookback {
                    Some(lookback) => {
                        between(series, evaluation_ms - lookback as i64, evaluation_ms)
                    }
                    None => up_to(series, evaluation_ms),
                };
                let out = execute(
                    &reference.dag,
                    BTreeMap::new(),
                    &raw,
                    &Self::roots(reference),
                    query,
                )?;
                Some(answers(&out[0])?)
            }
        };
        Ok(Evaluation {
            answers: answers_now,
            reference_answers,
            views_refreshed,
        })
    }
}
