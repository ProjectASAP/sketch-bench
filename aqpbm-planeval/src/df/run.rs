use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;

use aqpbm_core::measure::{measure, Measurement, Pass, Report, RunOutcome as MeasuredRun};
use aqpbm_core::metrics::{Metric, MetricsMask, RunMetrics};
use aqpbm_datagen::table::{GeneratedTable, TableDescription};
use asap_types::post_asap::{ExecutableOperatorPayload, PostAsapNodeId, SketchQuery};
use datafusion::arrow::array::Array;
use datafusion::arrow::record_batch::RecordBatch;
use datafusion::physical_plan::{collect, ExecutionPlan};

use crate::df::memtable::register_generated_table;
use crate::df::metrics::ElapsedCompute;
use crate::df::post_asap_arm::{answer, answer_without_split, PostAsapAnswer};
use crate::df::pre_asap::TableSources;
use crate::df::pre_asap_arm::answer_with_tables;
use crate::df::schema::declared_columns;
use crate::df::session::{MemoryPoolSettings, SeedSession, SessionError};
use crate::df::sketch_udaf::SummaryFunctions;
use crate::df::RefusalCounts;
use crate::plan::Plan;
use crate::run::{phase_config, ArmTiming, NodeTiming, Readout, RunConfig, RunOutcome, Runtime};
use crate::score::ObservedError;
use crate::types::{Answer, EvalError, GroupKey};

pub struct DataFusionRunConfig {
    pub table: String,
    pub description: TableDescription,
    pub rows: Rc<GeneratedTable>,
    pub split: bool,
}

pub const ANSWER_ERROR_METRIC: &str = "relative_to_pre_asap";

struct Prepared {
    session: SeedSession,
    summary: PostAsapAnswer,
    exact: Option<Vec<RecordBatch>>,
    exact_physical: Option<Arc<dyn ExecutionPlan>>,
    exact_peak_bytes: Option<usize>,
    source_bytes: usize,
    rows_scanned: u64,
}

pub fn run(
    plan: &Plan,
    cfg: &RunConfig,
    engine: &DataFusionRunConfig,
) -> Result<RunOutcome, EvalError> {
    let tokio = crate::sql::runtime();
    let prepared = tokio.block_on(prepare(plan, cfg, engine))?;

    let maintenance_plans = maintenance_plans(&prepared.summary);
    let read_plans = vec![Arc::clone(&prepared.summary.physical)];
    let answer_rows = prepared.summary.rows() as u64;

    let mut summary_plans = maintenance_plans.clone();
    summary_plans.extend(read_plans.iter().map(Arc::clone));

    let mut approximate = ArmTiming {
        maintenance: time_one_collect_per_pass(
            &maintenance_plans,
            &prepared.session,
            cfg,
            prepared.rows_scanned,
        ),
        read: time_one_collect_per_pass(&read_plans, &prepared.session, cfg, answer_rows),
        ..ArmTiming::default()
    };
    let charged = charge_one_execution(&summary_plans, &prepared.session);
    approximate.engine_overhead_ns = Some(charged.engine_overhead_ns);

    let mut pre_asap = ArmTiming::default();
    if let Some(physical) = prepared.exact_physical.as_ref() {
        pre_asap.evaluate = time_one_collect_per_pass(
            std::slice::from_ref(physical),
            &prepared.session,
            cfg,
            prepared.rows_scanned,
        );
        pre_asap.engine_overhead_ns = Some(
            charge_one_execution(std::slice::from_ref(physical), &prepared.session)
                .engine_overhead_ns,
        );
    }

    let readouts = compare_answers(plan, &prepared.summary, prepared.exact.as_deref());

    Ok(RunOutcome {
        runtime: Runtime::DataFusion,
        refusals: RefusalCounts::default(),
        rows_scanned: prepared.rows_scanned,
        rows_emitted: prepared.rows_scanned,
        root_rows: Some(prepared.summary.rows()),
        verified: prepared.exact.is_some(),
        retained_values: 0,
        retained_bytes: 0,
        readouts,
        node_footprints: state_bytes_per_node(&prepared.summary),
        node_times: node_times(plan, &charged),
        approximate,
        pre_asap,
        pre_asap_bytes: prepared.source_bytes,
        pre_asap_node_times: Vec::new(),
        pre_asap_answer: None,
        approximate_peak_bytes: Some(prepared.summary.maintenance_peak_reserved_bytes),
        pre_asap_peak_bytes: prepared.exact_peak_bytes,
    })
}

async fn prepare(
    plan: &Plan,
    cfg: &RunConfig,
    engine: &DataFusionRunConfig,
) -> Result<Prepared, EvalError> {
    let session = SeedSession::new(cfg.seed, MemoryPoolSettings::default(), &SummaryFunctions)
        .map_err(eval_error)?;
    let declared = declared_columns(&engine.description)
        .map_err(|refusal| EvalError::Untranslated(vec![refusal]))?;
    let source =
        register_generated_table(session.context(), &engine.table, &engine.rows, &declared)
            .map_err(|error| EvalError::RowSource(error.to_string()))?;
    let source_bytes = source.get_array_memory_size();
    let rows_scanned = source.num_rows() as u64;
    let tables = TableSources::of_context(session.context())
        .await
        .map_err(|error| EvalError::RowSource(error.to_string()))?;

    let summary = if engine.split {
        answer(plan, &session, &tables).await
    } else {
        answer_without_split(plan, &session, &tables).await
    }
    .map_err(eval_error)?;

    let mut exact = None;
    let mut exact_physical = None;
    let mut exact_peak_bytes = None;
    if let (true, Some(root)) = (cfg.pre_asap, plan.pre_asap.as_ref()) {
        let arm = answer_with_tables(root, &session, &tables)
            .await
            .map_err(eval_error)?;
        exact_peak_bytes = Some(arm.peak_reserved_bytes);
        exact_physical = Some(Arc::clone(&arm.physical));
        exact = Some(arm.batches);
    }

    Ok(Prepared {
        session,
        summary,
        exact,
        exact_physical,
        exact_peak_bytes,
        source_bytes,
        rows_scanned,
    })
}

fn eval_error(error: SessionError) -> EvalError {
    match error {
        SessionError::Refused(refusal) => EvalError::Untranslated(vec![refusal]),
        SessionError::DataFusion(error) => EvalError::RowSource(error.to_string()),
    }
}

fn maintenance_plans(summary: &PostAsapAnswer) -> Vec<Arc<dyn ExecutionPlan>> {
    summary
        .state_tables
        .iter()
        .map(|table| Arc::clone(&table.physical))
        .collect()
}

fn time_one_collect_per_pass(
    plans: &[Arc<dyn ExecutionPlan>],
    session: &SeedSession,
    cfg: &RunConfig,
    work: u64,
) -> Vec<RunMetrics> {
    if plans.is_empty() {
        return Vec::new();
    }
    let (total, config) = phase_config(Metric::Throughput, MetricsMask::empty(), cfg);
    let mut passes: Measurement = Vec::with_capacity(total);
    for _ in 0..total {
        let plans: Vec<Arc<dyn ExecutionPlan>> = plans.to_vec();
        let task = session.context().task_ctx();
        let pass: Pass = Box::new(move || {
            let tokio = crate::sql::runtime();
            let mut collected = 0usize;
            for plan in plans {
                if let Ok(batches) = tokio.block_on(collect(plan, Arc::clone(&task))) {
                    collected += batches.len();
                }
            }
            let report: Report = Box::new(move || {
                std::hint::black_box(collected);
                MeasuredRun {
                    work,
                    ..MeasuredRun::default()
                }
            });
            report
        });
        passes.push(pass);
    }
    measure(&config, passes)
}

fn charge_one_execution(plans: &[Arc<dyn ExecutionPlan>], session: &SeedSession) -> ElapsedCompute {
    let read = |plans: &[Arc<dyn ExecutionPlan>]| {
        let mut held = ElapsedCompute::default();
        for plan in plans {
            held.add_plan(plan.as_ref());
        }
        held
    };
    let before = read(plans);
    let tokio = crate::sql::runtime();
    for plan in plans {
        let _ = tokio.block_on(collect(Arc::clone(plan), session.context().task_ctx()));
    }
    read(plans).since(&before)
}

fn state_bytes_per_node(summary: &PostAsapAnswer) -> Vec<(PostAsapNodeId, usize)> {
    let mut held: Vec<(PostAsapNodeId, usize)> = summary
        .state_tables
        .iter()
        .map(|table| (table.node, table.bytes()))
        .collect();
    held.sort_by_key(|(id, _)| id.0);
    held
}

fn node_times(plan: &Plan, charged: &ElapsedCompute) -> Vec<(PostAsapNodeId, NodeTiming)> {
    plan.dag
        .nodes
        .iter()
        .filter_map(|node| {
            charged.per_node.get(&node.id).map(|elapsed_ns| {
                (
                    node.id,
                    NodeTiming {
                        elapsed_compute_ns: Some(*elapsed_ns),
                        ..NodeTiming::default()
                    },
                )
            })
        })
        .collect()
}

fn compare_answers(
    plan: &Plan,
    summary: &PostAsapAnswer,
    exact: Option<&[RecordBatch]>,
) -> Vec<Readout> {
    let estimates = estimate_nodes(plan);
    let [(node, query)] = estimates.as_slice() else {
        return Vec::new();
    };
    let approximate = keyed_values(&summary.batches);
    let truth: HashMap<GroupKey, Vec<f64>> = exact
        .map(|batches| keyed_values(batches).into_iter().collect())
        .unwrap_or_default();

    let mut readouts = Vec::new();
    for (group, values) in approximate {
        let against = truth.get(&group);
        for (position, estimate) in values.into_iter().enumerate() {
            let truth = against.and_then(|held| held.get(position)).copied();
            readouts.push(Readout {
                node: *node,
                producer: *node,
                group: group.clone(),
                query: query.clone(),
                approximate: Answer::Scalar(estimate),
                exact: truth.map(Answer::Scalar),
                observed_error: match truth {
                    Some(truth) => ObservedError::Measured {
                        metric: ANSWER_ERROR_METRIC.to_string(),
                        error: relative_error(estimate, truth),
                    },
                    None => ObservedError::NotVerified,
                },
                guarantee: None,
                observations: 0,
            });
        }
    }
    readouts
}

fn relative_error(estimate: f64, truth: f64) -> f64 {
    let difference = (estimate - truth).abs();
    if truth == 0.0 {
        difference
    } else {
        difference / truth.abs()
    }
}

fn estimate_nodes(plan: &Plan) -> Vec<(PostAsapNodeId, SketchQuery)> {
    plan.dag
        .nodes
        .iter()
        .filter_map(|node| match &node.payload {
            ExecutableOperatorPayload::SummaryEstimate { query } => Some((node.id, query.clone())),
            _ => None,
        })
        .collect()
}

fn keyed_values(batches: &[RecordBatch]) -> Vec<(GroupKey, Vec<f64>)> {
    let mut rows: Vec<(GroupKey, Vec<f64>)> = Vec::new();
    for batch in batches {
        for row in 0..batch.num_rows() {
            let mut key = String::new();
            let mut values = Vec::new();
            for column in batch.columns() {
                match numeric(column.as_ref(), row) {
                    Some(value) => values.push(value),
                    None => {
                        if !key.is_empty() {
                            key.push('|');
                        }
                        key.push_str(&rendered(column.as_ref(), row));
                    }
                }
            }
            rows.push((key, values));
        }
    }
    rows.sort_by(|left, right| left.0.cmp(&right.0));
    rows
}

fn numeric(column: &dyn Array, row: usize) -> Option<f64> {
    use datafusion::arrow::array::{Float64Array, Int64Array, UInt64Array};
    if column.is_null(row) {
        return None;
    }
    if let Some(held) = column.as_any().downcast_ref::<Float64Array>() {
        return Some(held.value(row));
    }
    if let Some(held) = column.as_any().downcast_ref::<Int64Array>() {
        return Some(held.value(row) as f64);
    }
    if let Some(held) = column.as_any().downcast_ref::<UInt64Array>() {
        return Some(held.value(row) as f64);
    }
    None
}

fn rendered(column: &dyn Array, row: usize) -> String {
    use datafusion::arrow::array::StringArray;
    if column.is_null(row) {
        return "null".to_string();
    }
    match column.as_any().downcast_ref::<StringArray>() {
        Some(held) => held.value(row).to_string(),
        None => format!("{:?}", column.data_type()),
    }
}

pub fn refusal_counts(error: &EvalError) -> RefusalCounts {
    let mut counts = RefusalCounts::default();
    match error {
        EvalError::Refused(refusals) => counts.add_unclassified(refusals.len()),
        EvalError::Untranslated(refusals) => {
            for refusal in refusals {
                counts.add(refusal);
            }
        }
        _ => {}
    }
    counts
}
