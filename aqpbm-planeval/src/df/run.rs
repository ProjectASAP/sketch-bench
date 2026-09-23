use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::rc::Rc;
use std::sync::Arc;

use aqpbm_core::measure::{measure, Measurement, Pass, Report, RunOutcome as MeasuredRun};
use aqpbm_core::metrics::{Metric, MetricsMask, RunMetrics};
use aqpbm_datagen::table::{GeneratedTable, TableDescription};
use asap_types::post_asap::{
    EdgeRole, ExecutableDagNode, ExecutableOperatorPayload, PostAsapNodeId, SketchQuery,
    SummaryFamilyType, ValueOperation,
};
use asap_types::pre_asap::{QueryExpr, Reduction};
use datafusion::arrow::array::Array;
use datafusion::arrow::datatypes::Schema as ArrowSchema;
use datafusion::arrow::record_batch::RecordBatch;
use datafusion::arrow::util::display::{ArrayFormatter, FormatOptions};
use datafusion::physical_plan::{collect, ExecutionPlan};

use crate::df::memtable::register_generated_table;
use crate::df::metrics::{strip_node_alias, ElapsedCompute};
use crate::df::post_asap_arm::{answer, answer_without_split, PostAsapAnswer};
use crate::df::pre_asap::TableSources;
use crate::df::pre_asap_arm::answer_with_tables;
use crate::df::refusal::Refusal;
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
        )?,
        read: time_one_collect_per_pass(&read_plans, &prepared.session, cfg, answer_rows)?,
        ..ArmTiming::default()
    };
    let charged = charge_one_execution(&summary_plans, &prepared.session)?;
    approximate.engine_overhead_ns = Some(charged.engine_overhead_ns);

    let mut pre_asap = ArmTiming::default();
    if let Some(physical) = prepared.exact_physical.as_ref() {
        pre_asap.evaluate = time_one_collect_per_pass(
            std::slice::from_ref(physical),
            &prepared.session,
            cfg,
            prepared.rows_scanned,
        )?;
        pre_asap.engine_overhead_ns = Some(
            charge_one_execution(std::slice::from_ref(physical), &prepared.session)?
                .engine_overhead_ns,
        );
    }

    let readouts = compare_answers(plan, &prepared.summary, prepared.exact.as_deref())
        .map_err(|refusal| EvalError::Untranslated(vec![refusal]))?;

    Ok(RunOutcome {
        runtime: Runtime::DataFusion,
        refusals: RefusalCounts::default(),
        rows_scanned: prepared.rows_scanned,
        rows_emitted: None,
        root_rows: Some(prepared.summary.rows()),
        verified: prepared.exact.is_some(),
        no_summary_in_plan: prepared.summary.no_summary_in_plan,
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
        maintenance_peak_bytes: Some(prepared.summary.maintenance_peak_reserved_bytes),
        read_peak_bytes: Some(prepared.summary.read_peak_reserved_bytes),
        pre_asap_evaluate_peak_bytes: prepared.exact_peak_bytes,
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
        if cfg.verify {
            exact = Some(arm.batches);
        }
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
) -> Result<Vec<RunMetrics>, EvalError> {
    if plans.is_empty() {
        return Ok(Vec::new());
    }
    let (total, config) = phase_config(Metric::Throughput, MetricsMask::empty(), cfg);
    let failed: Rc<RefCell<Option<String>>> = Rc::new(RefCell::new(None));
    let mut passes: Measurement = Vec::with_capacity(total);
    for _ in 0..total {
        let plans: Vec<Arc<dyn ExecutionPlan>> = plans.to_vec();
        let task = session.context().task_ctx();
        let failed = Rc::clone(&failed);
        let pass: Pass = Box::new(move || {
            let tokio = crate::sql::runtime();
            let mut collected = 0usize;
            for plan in plans {
                match tokio.block_on(collect(plan, Arc::clone(&task))) {
                    Ok(batches) => collected += batches.len(),
                    Err(error) => {
                        let mut held = failed.borrow_mut();
                        if held.is_none() {
                            *held = Some(error.to_string());
                        }
                    }
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
    let measured = measure(&config, passes);
    let held = failed.borrow_mut().take();
    match held {
        Some(error) => Err(EvalError::RowSource(format!(
            "a timed execution did not finish, so its elapsed time measures nothing: {error}"
        ))),
        None => Ok(measured),
    }
}

fn charge_one_execution(
    plans: &[Arc<dyn ExecutionPlan>],
    session: &SeedSession,
) -> Result<ElapsedCompute, EvalError> {
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
        tokio
            .block_on(collect(Arc::clone(plan), session.context().task_ctx()))
            .map_err(|error| {
                EvalError::RowSource(format!(
                    "the execution the per-node attribution reads did not finish: {error}"
                ))
            })?;
    }
    Ok(read(plans).since(&before))
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

const REFUSED_ANSWER: &str = "PlanAnswer";

const NAMED_GROUPS_IN_A_REFUSAL: usize = 8;

#[derive(Debug, Clone, PartialEq)]
struct ValueColumn {
    position: usize,
    name: String,
    node: PostAsapNodeId,
    query: Option<SketchQuery>,
}

#[derive(Debug, Clone, PartialEq)]
struct AnswerLayout {
    keys: Vec<usize>,
    values: Vec<ValueColumn>,
}

#[derive(Debug, Default)]
struct PlanAnswerShape {
    key_names: BTreeSet<String>,
    value_owners: BTreeMap<String, (PostAsapNodeId, Option<SketchQuery>)>,
}

fn answer_shape(plan: &Plan) -> Result<PlanAnswerShape, Refusal> {
    let mut shape = PlanAnswerShape::default();
    for node in &plan.dag.nodes {
        let ExecutableOperatorPayload::SummaryAgg {
            reduction: Reduction::Reduce(by),
            ..
        } = &node.payload
        else {
            continue;
        };
        for field in node.output_schema.fields.iter().take(by.keys().len()) {
            shape.key_names.insert(field.name.clone());
        }
    }

    let nodes: BTreeMap<PostAsapNodeId, &ExecutableDagNode> =
        plan.dag.nodes.iter().map(|node| (node.id, node)).collect();
    let mut roles: BTreeMap<PostAsapNodeId, Vec<ColumnRole>> = BTreeMap::new();
    for id in &plan.order {
        let Some(node) = nodes.get(id) else {
            continue;
        };
        let produced = column_roles(plan, node, &shape.key_names, &roles);
        for (field, role) in node.output_schema.fields.iter().zip(&produced) {
            match role {
                ColumnRole::Key => {
                    shape.key_names.insert(field.name.clone());
                }
                ColumnRole::Owned(owner, query) => {
                    register_value(&mut shape, &field.name, *owner, query)?;
                }
                ColumnRole::Unattributed => {}
            }
        }
        roles.insert(node.id, produced);
    }
    Ok(shape)
}

#[derive(Debug, Clone, PartialEq)]
enum ColumnRole {
    Key,
    Owned(PostAsapNodeId, Option<SketchQuery>),
    Unattributed,
}

fn column_roles(
    plan: &Plan,
    node: &ExecutableDagNode,
    key_names: &BTreeSet<String>,
    roles: &BTreeMap<PostAsapNodeId, Vec<ColumnRole>>,
) -> Vec<ColumnRole> {
    let width = node.output_schema.fields.len();
    let produced = |query: Option<SketchQuery>| -> Vec<ColumnRole> {
        node.output_schema
            .fields
            .iter()
            .map(|field| match key_names.contains(&field.name) {
                true => ColumnRole::Key,
                false => ColumnRole::Owned(node.id, query.clone()),
            })
            .collect()
    };
    match &node.payload {
        ExecutableOperatorPayload::SummaryEstimate { query } => produced(Some(query.clone())),
        ExecutableOperatorPayload::SummaryAgg {
            family: SummaryFamilyType::ExactAggregate(..),
            ..
        } => produced(None),
        ExecutableOperatorPayload::Value { operation, .. } => {
            let read = sole_input(plan, node.id).and_then(|producer| roles.get(&producer));
            let Some(read) = read else {
                return vec![ColumnRole::Unattributed; width];
            };
            carried_through(operation, read, width)
        }
        _ => vec![ColumnRole::Unattributed; width],
    }
}

fn carried_through(
    operation: &ValueOperation,
    read: &[ColumnRole],
    width: usize,
) -> Vec<ColumnRole> {
    match operation {
        ValueOperation::Project { cols, .. } if cols.len() == width => cols
            .iter()
            .map(|item| match &item.expr {
                QueryExpr::Column(position) => read
                    .get(*position)
                    .cloned()
                    .unwrap_or(ColumnRole::Unattributed),
                _ => ColumnRole::Unattributed,
            })
            .collect(),
        ValueOperation::Filter { .. }
        | ValueOperation::Sort { .. }
        | ValueOperation::Limit { .. }
        | ValueOperation::FinalizeExactAccumulator
            if read.len() == width =>
        {
            read.to_vec()
        }
        _ => vec![ColumnRole::Unattributed; width],
    }
}

fn sole_input(plan: &Plan, consumer: PostAsapNodeId) -> Option<PostAsapNodeId> {
    let mut producers = plan
        .dag
        .edges
        .iter()
        .filter(|edge| edge.consumer == consumer && matches!(edge.role, EdgeRole::Input))
        .map(|edge| edge.producer);
    let first = producers.next()?;
    producers.next().is_none().then_some(first)
}

fn register_value(
    shape: &mut PlanAnswerShape,
    name: &str,
    node: PostAsapNodeId,
    query: &Option<SketchQuery>,
) -> Result<(), Refusal> {
    if shape.key_names.contains(name) {
        return Ok(());
    }
    match shape.value_owners.get(name) {
        Some(held) if held.0 == node && &held.1 == query => Ok(()),
        Some((held, _)) => Err(Refusal::no_constructor(
            REFUSED_ANSWER,
            format!(
                "column {name} is produced by both {held:?} and {node:?}, so a readout taken from \
                 it names no one node"
            ),
        )),
        None => {
            shape
                .value_owners
                .insert(name.to_string(), (node, query.clone()));
            Ok(())
        }
    }
}

fn answer_layout(schema: &ArrowSchema, shape: &PlanAnswerShape) -> Result<AnswerLayout, Refusal> {
    let mut layout = AnswerLayout {
        keys: Vec::new(),
        values: Vec::new(),
    };
    for (position, field) in schema.fields().iter().enumerate() {
        let name = strip_node_alias(field.name());
        if shape.key_names.contains(name) {
            layout.keys.push(position);
            continue;
        }
        match shape.value_owners.get(name) {
            Some((node, query)) => layout.values.push(ValueColumn {
                position,
                name: name.to_string(),
                node: *node,
                query: query.clone(),
            }),
            None => {
                return Err(Refusal::no_constructor(
                    REFUSED_ANSWER,
                    format!(
                        "column {name} of the answer is neither a group key the plan reduces by \
                         nor a value any summary node produces, so it cannot be scored"
                    ),
                ))
            }
        }
    }
    Ok(layout)
}

fn keyed_rows(
    batches: &[RecordBatch],
    layout: &AnswerLayout,
    arm: &str,
) -> Result<BTreeMap<GroupKey, Vec<Option<f64>>>, Refusal> {
    let options = FormatOptions::default().with_null("null");
    let mut rows: BTreeMap<GroupKey, Vec<Option<f64>>> = BTreeMap::new();
    for batch in batches {
        let mut keys = Vec::with_capacity(layout.keys.len());
        for position in &layout.keys {
            let column = batch.column(*position);
            keys.push(
                ArrayFormatter::try_new(column.as_ref(), &options).map_err(|error| {
                    Refusal::no_constructor(
                        REFUSED_ANSWER,
                        format!(
                            "group key {} is a {} and arrow will not spell it out: {error}",
                            batch.schema().field(*position).name(),
                            column.data_type()
                        ),
                    )
                })?,
            );
        }
        let mut values = Vec::with_capacity(layout.values.len());
        for value in &layout.values {
            let column = batch.column(value.position);
            values.push(
                datafusion::arrow::compute::cast(
                    column.as_ref(),
                    &datafusion::arrow::datatypes::DataType::Float64,
                )
                .map_err(|error| {
                    Refusal::no_constructor(
                        REFUSED_ANSWER,
                        format!(
                            "{} is a {} and this comparison scores numbers: {error}",
                            value.name,
                            column.data_type()
                        ),
                    )
                })?,
            );
        }
        let read: Vec<&datafusion::arrow::array::Float64Array> = values
            .iter()
            .map(|column| {
                column
                    .as_any()
                    .downcast_ref::<datafusion::arrow::array::Float64Array>()
                    .expect("a cast to Float64 produces a Float64Array")
            })
            .collect();

        for row in 0..batch.num_rows() {
            let key: GroupKey = keys
                .iter()
                .map(|formatter| formatter.value(row).to_string())
                .collect::<Vec<String>>()
                .join("|");
            let held: Vec<Option<f64>> = read
                .iter()
                .map(|column| (!column.is_null(row)).then(|| column.value(row)))
                .collect();
            if rows.insert(key.clone(), held).is_some() {
                return Err(Refusal::no_constructor(
                    REFUSED_ANSWER,
                    format!(
                        "the {arm} arm answers group [{key}] twice, so scoring it would pick one \
                         of the two rows and drop the other"
                    ),
                ));
            }
        }
    }
    Ok(rows)
}

fn refuse_unless_the_group_sets_match(
    approximate: &BTreeMap<GroupKey, Vec<Option<f64>>>,
    truth: &BTreeMap<GroupKey, Vec<Option<f64>>>,
) -> Result<(), Refusal> {
    let named = |groups: Vec<&GroupKey>| {
        let shown: Vec<String> = groups
            .iter()
            .take(NAMED_GROUPS_IN_A_REFUSAL)
            .map(|group| format!("[{group}]"))
            .collect();
        match groups.len() > NAMED_GROUPS_IN_A_REFUSAL {
            true => format!(
                "{} and {} more",
                shown.join(", "),
                groups.len() - NAMED_GROUPS_IN_A_REFUSAL
            ),
            false => shown.join(", "),
        }
    };
    let only_approximate: Vec<&GroupKey> = approximate
        .keys()
        .filter(|group| !truth.contains_key(*group))
        .collect();
    let only_truth: Vec<&GroupKey> = truth
        .keys()
        .filter(|group| !approximate.contains_key(*group))
        .collect();
    if !only_approximate.is_empty() || !only_truth.is_empty() {
        return Err(Refusal::no_constructor(
            REFUSED_ANSWER,
            format!(
                "the two arms group the rows differently: {} group(s) only the approximate arm \
                 answers ({}), {} group(s) only the exact arm answers ({})",
                only_approximate.len(),
                named(only_approximate),
                only_truth.len(),
                named(only_truth)
            ),
        ));
    }
    Ok(())
}

fn compare_answers(
    plan: &Plan,
    summary: &PostAsapAnswer,
    exact: Option<&[RecordBatch]>,
) -> Result<Vec<Readout>, Refusal> {
    let shape = answer_shape(plan)?;
    if shape.value_owners.is_empty() {
        return Ok(Vec::new());
    }
    let layout = answer_layout(summary.physical.schema().as_ref(), &shape)?;
    let approximate = keyed_rows(&summary.batches, &layout, "approximate")?;

    let truth = match exact {
        Some(batches) => {
            let exact_layout = match batches.first() {
                Some(batch) => answer_layout(batch.schema().as_ref(), &shape)?,
                None => layout.clone(),
            };
            let named = |layout: &AnswerLayout| -> Vec<String> {
                layout
                    .values
                    .iter()
                    .map(|value| value.name.clone())
                    .collect()
            };
            if named(&exact_layout) != named(&layout)
                || exact_layout.keys.len() != layout.keys.len()
            {
                return Err(Refusal::no_constructor(
                    REFUSED_ANSWER,
                    format!(
                        "the approximate arm answers {:?} and the exact arm answers {:?}, so the \
                         two are not the same columns",
                        named(&layout),
                        named(&exact_layout)
                    ),
                ));
            }
            let truth = keyed_rows(batches, &exact_layout, "exact")?;
            refuse_unless_the_group_sets_match(&approximate, &truth)?;
            Some(truth)
        }
        None => None,
    };

    let mut readouts = Vec::new();
    for (group, values) in approximate {
        let against = truth.as_ref().and_then(|held| held.get(&group));
        for (position, column) in layout.values.iter().enumerate() {
            let estimate = values.get(position).copied().flatten();
            let truth = against
                .and_then(|held| held.get(position))
                .copied()
                .flatten();
            readouts.push(Readout {
                node: column.node,
                producer: column.node,
                group: group.clone(),
                query: column.query.clone(),
                approximate: Answer::Scalar(estimate.unwrap_or(f64::NAN)),
                exact: truth.map(Answer::Scalar),
                observed_error: observed_error(estimate, truth, against.is_some()),
                guarantee: None,
                observations: 0,
            });
        }
    }
    Ok(readouts)
}

fn observed_error(estimate: Option<f64>, truth: Option<f64>, verified: bool) -> ObservedError {
    if !verified {
        return ObservedError::NotVerified;
    }
    match (estimate, truth) {
        (Some(estimate), Some(truth)) => ObservedError::Measured {
            metric: ANSWER_ERROR_METRIC.to_string(),
            error: relative_error(estimate, truth),
        },
        _ => ObservedError::Unevaluatable {
            metric: ANSWER_ERROR_METRIC.to_string(),
            reason: crate::score::UnevaluatableReason::ErrorIsNotANumber,
        },
    }
}

fn relative_error(estimate: f64, truth: f64) -> f64 {
    let difference = (estimate - truth).abs();
    if truth == 0.0 {
        difference
    } else {
        difference / truth.abs()
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

#[cfg(test)]
mod tests {
    use super::*;
    use datafusion::arrow::array::{ArrayRef, Float64Array, Int64Array, StringArray};
    use datafusion::arrow::datatypes::{DataType as ArrowDataType, Field};

    fn batch(keys: Vec<i64>, values: Vec<f64>) -> RecordBatch {
        let schema = Arc::new(ArrowSchema::new(vec![
            Field::new("bytes", ArrowDataType::Int64, false),
            Field::new("held", ArrowDataType::Float64, false),
        ]));
        RecordBatch::try_new(
            schema,
            vec![
                Arc::new(Int64Array::from(keys)) as ArrayRef,
                Arc::new(Float64Array::from(values)) as ArrayRef,
            ],
        )
        .expect("two columns of the same length are a batch")
    }

    fn int_keyed_layout() -> AnswerLayout {
        AnswerLayout {
            keys: vec![0],
            values: vec![ValueColumn {
                position: 1,
                name: "held".to_owned(),
                node: PostAsapNodeId(2),
                query: Some(SketchQuery::Quantile { q: 0.5 }),
            }],
        }
    }

    #[test]
    fn an_integer_key_is_read_as_the_integer_and_not_as_a_value() {
        let rows = keyed_rows(
            &[batch(vec![7, 11], vec![1.5, 2.5])],
            &int_keyed_layout(),
            "a",
        )
        .expect("the rows key on the integer column");
        assert_eq!(
            rows.keys().cloned().collect::<Vec<GroupKey>>(),
            vec!["11".to_owned(), "7".to_owned()]
        );
        assert_eq!(rows["7"], vec![Some(1.5)]);
    }

    #[test]
    fn two_rows_that_render_to_one_key_are_refused_rather_than_merged() {
        let refused = keyed_rows(
            &[batch(vec![7, 7], vec![1.5, 2.5])],
            &int_keyed_layout(),
            "approximate",
        )
        .unwrap_err();
        assert_eq!(refused.variant, REFUSED_ANSWER);
        assert!(refused.to_string().contains("twice"), "{refused}");
    }

    #[test]
    fn a_group_one_arm_answers_and_the_other_does_not_is_refused() {
        let layout = int_keyed_layout();
        let approximate = keyed_rows(&[batch(vec![7, 11], vec![1.5, 2.5])], &layout, "a").unwrap();
        let truth = keyed_rows(&[batch(vec![7], vec![1.5])], &layout, "b").unwrap();
        refuse_unless_the_group_sets_match(&approximate, &approximate)
            .expect("a group set agrees with itself");
        let refused = refuse_unless_the_group_sets_match(&approximate, &truth).unwrap_err();
        assert_eq!(refused.variant, REFUSED_ANSWER);
        assert!(refused.to_string().contains("[11]"), "{refused}");
    }

    #[test]
    fn a_column_that_scores_no_number_is_refused_rather_than_taken_as_a_key() {
        let schema = Arc::new(ArrowSchema::new(vec![Field::new(
            "held",
            ArrowDataType::Utf8,
            false,
        )]));
        let held = RecordBatch::try_new(
            schema,
            vec![Arc::new(StringArray::from(vec!["a", "b"])) as ArrayRef],
        )
        .unwrap();
        let layout = AnswerLayout {
            keys: Vec::new(),
            values: vec![ValueColumn {
                position: 0,
                name: "held".to_owned(),
                node: PostAsapNodeId(2),
                query: None,
            }],
        };
        let refused = keyed_rows(&[held], &layout, "approximate").unwrap_err();
        assert_eq!(refused.variant, REFUSED_ANSWER);
    }
}
