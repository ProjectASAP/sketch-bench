use std::collections::HashMap;

use asap_types::post_asap::{
    EdgeRole, ExactKind, ExactOperation, ExecutableDag, ExecutableDagEdge, ExecutableDagNode,
    ExecutableOperatorPayload, ExecutionDataState, GroupingEdgeCompatibility, GroupingStrategy,
    PostAsapNodeId, ResultGuarantee, SummaryFamilyType, SummarySchema, ValueOperation,
    WindowEdgeCompatibility,
};
use asap_types::pre_asap::Reduction;
use datafusion::common::TableReference;
use datafusion::error::DataFusionError;
use datafusion::execution::context::SessionState;
use datafusion::logical_expr::{Expr, LogicalPlan, LogicalPlanBuilder, SortExpr};

use crate::df::estimate_udf::readout_call;
use crate::df::metrics::node_alias;
use crate::df::pre_asap::{lower as lower_pre_asap, reduce_measures, TableSources};
use crate::df::refusal::Refusal;
use crate::df::scalar::{lower_scalar, ColumnScope};
use crate::df::sketch_udaf::{lower_weight, sketch_aggregate_call};

pub fn payload_name(payload: &ExecutableOperatorPayload) -> &'static str {
    match payload {
        ExecutableOperatorPayload::Fallback { .. } => "Fallback",
        ExecutableOperatorPayload::Binary { .. } => "Binary",
        ExecutableOperatorPayload::CandidateTopK { .. } => "CandidateTopK",
        ExecutableOperatorPayload::Value { .. } => "Value",
        ExecutableOperatorPayload::RelationalJoin { .. } => "RelationalJoin",
        ExecutableOperatorPayload::SummaryAgg { .. } => "SummaryAgg",
        ExecutableOperatorPayload::SummaryJoin { .. } => "SummaryJoin",
        ExecutableOperatorPayload::SummarySubtract => "SummarySubtract",
        ExecutableOperatorPayload::SummaryDelete { .. } => "SummaryDelete",
        ExecutableOperatorPayload::SummaryEstimate { .. } => "SummaryEstimate",
        ExecutableOperatorPayload::SummaryMerge => "SummaryMerge",
    }
}

pub fn value_operation_name(operation: &ValueOperation) -> &'static str {
    match operation {
        ValueOperation::MaintainPopulation { .. } => "MaintainPopulation",
        ValueOperation::ReadPopulation { .. } => "ReadPopulation",
        ValueOperation::Exact(_) => "Exact",
        ValueOperation::FinalizeExactAccumulator => "FinalizeExactAccumulator",
        ValueOperation::Project { .. } => "Project",
        ValueOperation::Filter { .. } => "Filter",
        ValueOperation::Sort { .. } => "Sort",
        ValueOperation::Limit { .. } => "Limit",
        ValueOperation::Extension { .. } => "Extension",
        _ => "Unnamed",
    }
}

#[derive(Debug, Default)]
pub struct DagPlans {
    plans: HashMap<PostAsapNodeId, LogicalPlan>,
    readout_guarantees: Vec<(PostAsapNodeId, ResultGuarantee)>,
}

impl DagPlans {
    pub fn plan(&self, node: PostAsapNodeId) -> Result<&LogicalPlan, Refusal> {
        self.plans.get(&node).ok_or_else(|| {
            Refusal::no_constructor(
                "ExecutableDag",
                format!("{node:?} has no plan in this fold; the fold did not reach it"),
            )
        })
    }

    pub fn readout_guarantees(&self) -> &[(PostAsapNodeId, ResultGuarantee)] {
        &self.readout_guarantees
    }

    pub fn len(&self) -> usize {
        self.plans.len()
    }

    pub fn is_empty(&self) -> bool {
        self.plans.is_empty()
    }
}

pub fn lower(
    dag: &ExecutableDag,
    tables: &TableSources,
    state: &SessionState,
) -> Result<DagPlans, Refusal> {
    let order = fold_order(dag)?;
    lower_nodes(dag, &order, tables, state, &HashMap::new())
}

pub fn fold_order(dag: &ExecutableDag) -> Result<Vec<PostAsapNodeId>, Refusal> {
    crate::plan::topological_order(dag).map_err(|error| {
        Refusal::no_constructor(
            "ExecutableDag",
            format!("the DAG has no fold order: {error}"),
        )
    })
}

pub fn lower_nodes(
    dag: &ExecutableDag,
    order: &[PostAsapNodeId],
    tables: &TableSources,
    state: &SessionState,
    scans: &HashMap<PostAsapNodeId, LogicalPlan>,
) -> Result<DagPlans, Refusal> {
    let nodes: HashMap<PostAsapNodeId, &ExecutableDagNode> =
        dag.nodes.iter().map(|node| (node.id, node)).collect();
    let mut held = DagPlans::default();

    for id in order {
        if let Some(scanned) = scans.get(id) {
            held.plans.insert(*id, scanned.clone());
            continue;
        }
        let node = *nodes.get(id).ok_or_else(|| {
            Refusal::no_constructor("ExecutableDag", format!("{id:?} names no node"))
        })?;
        let inputs = collect_inputs(dag, node, &nodes, &held)?;
        let plan = lower_node(dag, node, &inputs, tables, state)?;
        if let (ExecutableOperatorPayload::SummaryEstimate { .. }, Some(guarantee)) =
            (&node.payload, node.guarantee.as_ref())
        {
            held.readout_guarantees.push((node.id, guarantee.clone()));
        }
        held.plans.insert(*id, plan);
    }
    Ok(held)
}

struct RoleInputs<'a> {
    input: Vec<&'a LogicalPlan>,
    left: Option<&'a LogicalPlan>,
    right: Option<&'a LogicalPlan>,
    candidate_membership: Option<&'a LogicalPlan>,
    authoritative_values: Option<&'a LogicalPlan>,
    producer_schemas: Vec<&'a SummarySchema>,
}

impl<'a> RoleInputs<'a> {
    fn single(&self, variant: &str) -> Result<&'a LogicalPlan, Refusal> {
        match self.input.as_slice() {
            [only] => Ok(only),
            other => Err(Refusal::no_constructor(
                format!("ExecutableOperatorPayload::{variant}"),
                format!(
                    "a unary operator reads one Input edge and this node has {}",
                    other.len()
                ),
            )),
        }
    }

    fn producer_schema(&self, variant: &str) -> Result<&'a SummarySchema, Refusal> {
        match self.producer_schemas.as_slice() {
            [only] => Ok(only),
            other => Err(Refusal::no_constructor(
                format!("ExecutableOperatorPayload::{variant}"),
                format!(
                    "a unary operator reads one producer's schema and this node has {}",
                    other.len()
                ),
            )),
        }
    }

    fn leaf(&self, variant: &str) -> Result<(), Refusal> {
        let held = self.input.len()
            + usize::from(self.left.is_some())
            + usize::from(self.right.is_some())
            + usize::from(self.candidate_membership.is_some())
            + usize::from(self.authoritative_values.is_some());
        if held == 0 {
            return Ok(());
        }
        Err(Refusal::no_constructor(
            format!("ExecutableOperatorPayload::{variant}"),
            format!("this operator is a leaf and the DAG gives it {held} inputs"),
        ))
    }
}

fn collect_inputs<'a>(
    dag: &'a ExecutableDag,
    consumer: &'a ExecutableDagNode,
    nodes: &HashMap<PostAsapNodeId, &'a ExecutableDagNode>,
    held: &'a DagPlans,
) -> Result<RoleInputs<'a>, Refusal> {
    let mut inputs = RoleInputs {
        input: Vec::new(),
        left: None,
        right: None,
        candidate_membership: None,
        authoritative_values: None,
        producer_schemas: Vec::new(),
    };

    for edge in dag.edges.iter().filter(|edge| edge.consumer == consumer.id) {
        let producer = *nodes.get(&edge.producer).ok_or_else(|| {
            Refusal::no_constructor(
                "ExecutableDagEdge",
                format!("{:?} names no node", edge.producer),
            )
        })?;
        refuse_unless_edge_composes(edge, producer, consumer)?;
        let plan = held.plan(edge.producer)?;
        match edge.role {
            EdgeRole::Input => {
                inputs.input.push(plan);
                inputs.producer_schemas.push(&producer.output_schema);
            }
            EdgeRole::Left => inputs.left = Some(plan),
            EdgeRole::Right => inputs.right = Some(plan),
            EdgeRole::CandidateMembership => inputs.candidate_membership = Some(plan),
            EdgeRole::AuthoritativeValues => inputs.authoritative_values = Some(plan),
        }
    }
    Ok(inputs)
}

pub fn refuse_unless_edge_composes(
    edge: &ExecutableDagEdge,
    producer: &ExecutableDagNode,
    consumer: &ExecutableDagNode,
) -> Result<(), Refusal> {
    match edge.grouping {
        GroupingEdgeCompatibility::Identical | GroupingEdgeCompatibility::NotApplicable => {}
        GroupingEdgeCompatibility::Incompatible => {
            return Err(Refusal::no_constructor(
                "ExecutableDagEdge(grouping = Incompatible)",
                format!(
                    "{:?} groups its output on keys {:?} cannot read, and no regrouping recovers \
                     one from the other",
                    producer.id, consumer.id
                ),
            ))
        }
        GroupingEdgeCompatibility::ConsumerCoarsensProducer => {
            if !matches!(consumer.payload, ExecutableOperatorPayload::SummaryMerge) {
                return Err(Refusal::deferred(
                    "ExecutableDagEdge(grouping = ConsumerCoarsensProducer)",
                    "coarsening-without-merge",
                    format!(
                        "{:?} reads coarser groups than {:?} produces, which needs a merge, and \
                         the DAG carries no SummaryMerge node on this edge",
                        consumer.id, producer.id
                    ),
                ));
            }
        }
    }

    match edge.window {
        WindowEdgeCompatibility::NotApplicable => {}
        WindowEdgeCompatibility::RequiresAlignedPanePhaseOrExactWindowEdgeResidual => {
            if producer.output_state == ExecutionDataState::MAINTENANCE_SUMMARY
                && consumer.output_state == ExecutionDataState::MAINTENANCE_SUMMARY
            {
                return Err(Refusal::time_axis(
                    "ExecutableDagEdge(window = RequiresAlignedPanePhaseOrExactWindowEdgeResidual)",
                    format!(
                        "{:?} hands summary state to {:?} across a window edge whose pane phase \
                         the DAG does not state, and the DAG carries no evaluation instant to \
                         align it against",
                        producer.id, consumer.id
                    ),
                ));
            }
        }
    }
    Ok(())
}

fn lower_node(
    dag: &ExecutableDag,
    node: &ExecutableDagNode,
    inputs: &RoleInputs<'_>,
    tables: &TableSources,
    state: &SessionState,
) -> Result<LogicalPlan, Refusal> {
    match &node.payload {
        ExecutableOperatorPayload::Fallback { expression } => {
            inputs.leaf("Fallback")?;
            lower_pre_asap(expression, tables)
        }

        ExecutableOperatorPayload::Value { operation, .. } => lower_value(node, operation, inputs),

        ExecutableOperatorPayload::SummaryAgg {
            family,
            input,
            reduction,
            grouping,
        } => lower_summary_agg(node, family, input, reduction, grouping, inputs, state),

        ExecutableOperatorPayload::SummaryEstimate { query } => {
            lower_summary_estimate(dag, node, query, inputs, state)
        }

        ExecutableOperatorPayload::SummaryMerge => Err(Refusal::no_constructor(
            "ExecutableOperatorPayload::SummaryMerge",
            "merging two summary states of one family has no producer-side construction point \
             upstream, so no corpus plan reaches this node and no merge aggregate is registered",
        )),

        ExecutableOperatorPayload::SummarySubtract => Err(Refusal::no_constructor(
            "ExecutableOperatorPayload::SummarySubtract",
            "no sketch in asap_sketchlib subtracts one state from another, and subtracting the \
             estimates instead would report a difference the summary never held",
        )),

        ExecutableOperatorPayload::SummaryDelete { key } => Err(Refusal::no_constructor(
            "ExecutableOperatorPayload::SummaryDelete",
            format!(
                "the key {key:?} names what to remove but carries no weight, so the magnitude to \
                 remove from the state is undefined"
            ),
        )),

        ExecutableOperatorPayload::SummaryJoin { key, family } => Err(Refusal::no_constructor(
            "ExecutableOperatorPayload::SummaryJoin",
            format!(
                "joining a {family:?} state on {key:?} has no producer-side construction point \
                 upstream, so no corpus plan reaches this node"
            ),
        )),

        ExecutableOperatorPayload::CandidateTopK { k, .. } => Err(Refusal::deferred(
            "ExecutableOperatorPayload::CandidateTopK",
            "candidate-topk-corpus",
            format!(
                "ranking a candidate set of {k} against authoritative values is a join, a row \
                 number window and a rank filter, and no corpus query has asked for one yet"
            ),
        )),

        ExecutableOperatorPayload::Binary { operator, .. } => Err(Refusal::deferred(
            "ExecutableOperatorPayload::Binary",
            "binary-operand-alignment",
            format!(
                "{:?} over two branches is a join on the group columns followed by the \
                 arithmetic, and no corpus query has asked for one yet",
                operator.kind
            ),
        )),

        ExecutableOperatorPayload::RelationalJoin { join_kind, .. } => Err(Refusal::deferred(
            "ExecutableOperatorPayload::RelationalJoin",
            "post-asap-join-corpus",
            format!(
                "a {join_kind:?} join between two post-ASAP branches reuses the pre-ASAP join \
                 lowering, and no corpus query has asked for one yet"
            ),
        )),
    }
}

fn lower_value(
    node: &ExecutableDagNode,
    operation: &ValueOperation,
    inputs: &RoleInputs<'_>,
) -> Result<LogicalPlan, Refusal> {
    let variant = format!("Value::{}", value_operation_name(operation));
    match operation {
        ValueOperation::Project { cols, qualifier } => {
            let input = inputs.single(&variant)?.clone();
            let scope = ColumnScope::of_plan(&input);
            let names = output_names(node);
            if names.len() != cols.len() {
                return Err(Refusal::no_constructor(
                    &variant,
                    format!(
                        "the node projects {} items and declares {} output columns",
                        cols.len(),
                        names.len()
                    ),
                ));
            }
            let mut projection = Vec::with_capacity(cols.len());
            for (item, name) in cols.iter().zip(names) {
                projection.push(lower_scalar(&item.expr, &scope)?.alias(name));
            }
            let projected = build(
                LogicalPlanBuilder::from(input).project(projection),
                &variant,
            )?;
            match qualifier {
                None => Ok(projected),
                Some(alias) => build(
                    LogicalPlanBuilder::from(projected).alias(TableReference::bare(alias.clone())),
                    &variant,
                ),
            }
        }

        ValueOperation::Filter { pred } => {
            let input = inputs.single(&variant)?.clone();
            let scope = ColumnScope::of_plan(&input);
            let pred = lower_scalar(&pred.0, &scope)?;
            build(LogicalPlanBuilder::from(input).filter(pred), &variant)
        }

        ValueOperation::Sort { keys, partition_by } => {
            if !partition_by.is_empty() {
                return Err(Refusal::deferred(
                    &variant,
                    "sort-partition-limit-coupling",
                    format!(
                        "an order partitioned by {:?} only means anything together with the Limit \
                         above it, and the IR carries no coupling between the two; translating \
                         the order alone reports a plausible wrong answer",
                        partition_by.keys()
                    ),
                ));
            }
            let input = inputs.single(&variant)?.clone();
            let scope = ColumnScope::of_plan(&input);
            let mut sorts = Vec::with_capacity(keys.len());
            for key in keys {
                sorts.push(SortExpr::new(
                    lower_scalar(&key.expr, &scope)?,
                    key.ascending,
                    key.nulls_first,
                ));
            }
            build(LogicalPlanBuilder::from(input).sort(sorts), &variant)
        }

        ValueOperation::Limit { n, offset } => {
            let input = inputs.single(&variant)?.clone();
            build(
                LogicalPlanBuilder::from(input).limit(*offset, Some(*n)),
                &variant,
            )
        }

        ValueOperation::Exact(exact) => match exact {
            ExactOperation::Aggregate {
                reduction,
                measures,
                output_names,
                having,
            } => {
                let input = inputs.single(&variant)?.clone();
                let declared = node.output_schema.fields.len();
                if output_names.len() != declared {
                    return Err(Refusal::no_constructor(
                        "Value::Exact(Aggregate)",
                        format!(
                            "the payload names {} output columns and the node declares {}",
                            output_names.len(),
                            declared
                        ),
                    ));
                }
                reduce_measures(
                    input,
                    reduction,
                    measures,
                    having.as_ref(),
                    &crate::df::post_asap::output_names(node),
                    "Value::Exact(Aggregate)",
                )
            }
            other => Err(Refusal::deferred(
                format!("ValueOperation::Exact({other:?})"),
                "upstream-exact-operation",
                "ExactOperation grew a variant this translator has no arm for",
            )),
        },

        ValueOperation::FinalizeExactAccumulator => {
            let input = inputs.single(&variant)?.clone();
            let produced = inputs.producer_schema(&variant)?;
            for field in &produced.fields {
                accumulator_holds_its_value(&field.dtype, &variant)?;
            }
            let scope = ColumnScope::of_plan(&input);
            let names = output_names(node);
            if names.len() != scope.width() {
                return Err(Refusal::no_constructor(
                    &variant,
                    format!(
                        "the accumulator hands up {} columns and the node declares {}",
                        scope.width(),
                        names.len()
                    ),
                ));
            }
            let mut projection = Vec::with_capacity(names.len());
            for (position, name) in names.into_iter().enumerate() {
                projection.push(scope.expr(position)?.alias(name));
            }
            build(
                LogicalPlanBuilder::from(input).project(projection),
                &variant,
            )
        }

        ValueOperation::MaintainPopulation { population } => Err(Refusal::deferred(
            &variant,
            "maintained-population-operator",
            format!(
                "holding the top {} of a population so a removal can promote its successor is a \
                 custom physical operator, and this session registers none",
                population.max_k
            ),
        )),

        ValueOperation::ReadPopulation { readout } => Err(Refusal::deferred(
            &variant,
            "maintained-population-operator",
            format!(
                "reading {readout:?} out of a maintained population is a custom physical \
                 operator, and this session registers none"
            ),
        )),

        ValueOperation::Extension { name } => Err(Refusal::deferred(
            &variant,
            "dialect-extension",
            format!("{name} is a deployment-specific operation with no registry entry (G10)"),
        )),

        other => Err(Refusal::deferred(
            format!("ValueOperation::{other:?}"),
            "upstream-value-operation",
            "ValueOperation grew a variant this translator has no arm for",
        )),
    }
}

fn accumulator_holds_its_value(family: &SummaryFamilyType, variant: &str) -> Result<(), Refusal> {
    match family {
        SummaryFamilyType::Plain(_) => Ok(()),
        SummaryFamilyType::ExactAggregate(
            ExactKind::Sum | ExactKind::Count | ExactKind::Min | ExactKind::Max,
            _,
        ) => Ok(()),
        SummaryFamilyType::ExactAggregate(
            kind @ (ExactKind::Increase | ExactKind::Rate | ExactKind::IRate),
            _,
        ) => Err(Refusal::deferred(
            variant,
            "order-sensitive",
            format!(
                "a {kind:?} accumulator holds a first sample, a last sample and a reset count \
                 rather than the value, and recovering the value reads its input in timestamp \
                 order"
            ),
        )),
        SummaryFamilyType::Sketch(kind, _) => Err(Refusal::no_constructor(
            variant,
            format!(
                "{:?} state is not a value; a SummaryEstimate readout recovers one from it",
                kind.algorithm()
            ),
        )),
        other => Err(Refusal::no_constructor(
            variant,
            format!("{other:?} state has no finalized value in the type table"),
        )),
    }
}

#[allow(clippy::too_many_arguments)]
fn lower_summary_agg(
    node: &ExecutableDagNode,
    family: &SummaryFamilyType,
    update: &asap_types::post_asap::SummaryUpdate,
    reduction: &Reduction,
    grouping: &GroupingStrategy,
    inputs: &RoleInputs<'_>,
    state: &SessionState,
) -> Result<LogicalPlan, Refusal> {
    let variant = "SummaryAgg";
    let by =
        match reduction {
            Reduction::PerEntity => return Err(Refusal::time_axis(
                "ExecutableOperatorPayload::SummaryAgg",
                "a per-entity reduction builds one summary per series along that series' own time \
                 axis, and the DAG carries no evaluation instant",
            )),
            Reduction::Reduce(by) if by.is_without() => return Err(Refusal::promql_only(
                "ExecutableOperatorPayload::SummaryAgg",
                "GROUP BY every column except these has no SQL spelling, and the SQL front end \
                 never emits it",
            )),
            Reduction::Reduce(by) => by,
        };
    refuse_shared_layout(grouping)?;
    if let SummaryFamilyType::Sketch(_, family_grouping) = family {
        refuse_shared_layout(family_grouping)?;
    }

    let input = inputs.single(variant)?.clone();
    let scope = ColumnScope::of_plan(&input);
    let state_position = by.keys().len();
    if node.output_schema.fields.len() != state_position + 1 {
        return Err(Refusal::no_constructor(
            "ExecutableOperatorPayload::SummaryAgg",
            format!(
                "this node reduces {state_position} keys into one state column and declares {} \
                 output columns",
                node.output_schema.fields.len()
            ),
        ));
    }
    if node.output_schema.fields[state_position].dtype != *family {
        return Err(Refusal::no_constructor(
            "ExecutableOperatorPayload::SummaryAgg",
            format!(
                "column {state_position} of the output schema is {:?} and the payload builds a \
                 {family:?}",
                node.output_schema.fields[state_position].dtype
            ),
        ));
    }
    let state_name = node_alias(node.id, &node.output_schema.fields[state_position].name);

    let aggregated =
        match family {
            SummaryFamilyType::Sketch(..) => {
                let call = sketch_aggregate_call(family, update, node.id)?;
                let udaf = state
                    .aggregate_functions()
                    .get(call.function)
                    .cloned()
                    .ok_or_else(|| {
                        Refusal::no_constructor(
                            "ExecutableOperatorPayload::SummaryAgg",
                            format!("{} is not registered in this session", call.function),
                        )
                    })?;
                udaf.call(call.arguments)
            }
            SummaryFamilyType::ExactAggregate(kind, _) => {
                let weight = lower_weight(&update.weight)?;
                exact_accumulator(kind, weight)?
            }
            SummaryFamilyType::Plain(_)
            | SummaryFamilyType::Sample(..)
            | SummaryFamilyType::Wavelet(..)
            | SummaryFamilyType::StatModel(..) => return Err(Refusal::no_constructor(
                format!("ExecutableOperatorPayload::SummaryAgg({family:?})"),
                "the planner produces no summary of this family today, so nothing constructs its \
                 state",
            )),
        };

    let mut group = Vec::with_capacity(state_position);
    for key in by.keys() {
        group.push(scope.expr(*key)?);
    }
    let reduced = build(
        LogicalPlanBuilder::from(input).aggregate(group, vec![aggregated.alias(&state_name)]),
        variant,
    )?;

    let reduced_scope = ColumnScope::of_plan(&reduced);
    let mut projection = Vec::with_capacity(node.output_schema.fields.len());
    for (position, name) in output_names(node).into_iter().enumerate() {
        projection.push(reduced_scope.expr(position)?.alias(name));
    }
    build(
        LogicalPlanBuilder::from(reduced).project(projection),
        variant,
    )
}

fn refuse_shared_layout(grouping: &GroupingStrategy) -> Result<(), Refusal> {
    match grouping {
        GroupingStrategy::PerSubpopulationInstance => Ok(()),
        GroupingStrategy::SharedMultiSubpopulation { kind, .. } => Err(Refusal::deferred(
            "GroupingStrategy::SharedMultiSubpopulation",
            "hydra-layout",
            format!(
                "a {kind:?} layout shares one state across subpopulations and the IR gives no \
                 mapping from the group keys to its buckets"
            ),
        )),
    }
}

fn exact_accumulator(kind: &ExactKind, weight: Expr) -> Result<Expr, Refusal> {
    use datafusion::functions_aggregate::expr_fn::{count, max, min, sum};
    match kind {
        ExactKind::Sum => Ok(sum(weight)),
        ExactKind::Count => Ok(count(weight)),
        ExactKind::Min => Ok(min(weight)),
        ExactKind::Max => Ok(max(weight)),
        ExactKind::Increase | ExactKind::Rate | ExactKind::IRate => Err(Refusal::deferred(
            format!("ExactAggregate({kind:?})"),
            "order-sensitive",
            "a counter-reset-aware accumulator reads its input in timestamp order, which needs an \
             aggregate that declares a beneficial ordering",
        )),
    }
}

fn lower_summary_estimate(
    dag: &ExecutableDag,
    node: &ExecutableDagNode,
    query: &asap_types::post_asap::SketchQuery,
    inputs: &RoleInputs<'_>,
    state: &SessionState,
) -> Result<LogicalPlan, Refusal> {
    let variant = "SummaryEstimate";
    let input = inputs.single(variant)?.clone();
    let producer = dag
        .edges
        .iter()
        .find(|edge| edge.consumer == node.id && edge.role == EdgeRole::Input)
        .and_then(|edge| dag.nodes.iter().find(|held| held.id == edge.producer));
    if let Some(detail) =
        producer.and_then(|producer| crate::run::point_lookup_key_mismatch(producer, dag, query))
    {
        return Err(Refusal::deferred(
            "SketchQuery::PointCount",
            "point-lookup-key-type",
            detail,
        ));
    }
    let produced = inputs.producer_schema(variant)?;
    let state_position = produced
        .fields
        .iter()
        .position(|field| !matches!(field.dtype, SummaryFamilyType::Plain(_)))
        .ok_or_else(|| {
            Refusal::no_constructor(
                "ExecutableOperatorPayload::SummaryEstimate",
                "every column the producer declares is a plain value, so none of them carries the \
                 state this readout reads",
            )
        })?;
    let family = &produced.fields[state_position].dtype;
    if node.output_schema.fields.len() != produced.fields.len() {
        return Err(Refusal::no_constructor(
            "ExecutableOperatorPayload::SummaryEstimate",
            format!(
                "the producer hands up {} columns and this readout declares {}",
                produced.fields.len(),
                node.output_schema.fields.len()
            ),
        ));
    }

    let readout = readout_call(family, query, node.id)?;
    let udf = state
        .scalar_functions()
        .get(readout.function)
        .cloned()
        .ok_or_else(|| {
            Refusal::no_constructor(
                "ExecutableOperatorPayload::SummaryEstimate",
                format!("{} is not registered in this session", readout.function),
            )
        })?;

    let scope = ColumnScope::of_plan(&input);
    let mut projection = Vec::with_capacity(node.output_schema.fields.len());
    for (position, name) in output_names(node).into_iter().enumerate() {
        let value = if position == state_position {
            udf.call(readout.arguments(scope.expr(position)?))
        } else {
            scope.expr(position)?
        };
        projection.push(value.alias(name));
    }
    build(LogicalPlanBuilder::from(input).project(projection), variant)
}

fn output_names(node: &ExecutableDagNode) -> Vec<String> {
    node.output_schema
        .fields
        .iter()
        .map(|field| node_alias(node.id, &field.name))
        .collect()
}

fn build(
    builder: Result<LogicalPlanBuilder, DataFusionError>,
    variant: &str,
) -> Result<LogicalPlan, Refusal> {
    builder
        .and_then(LogicalPlanBuilder::build)
        .map_err(|error| {
            Refusal::deferred(
                format!("ExecutableOperatorPayload::{variant}"),
                "datafusion-plan-builder",
                format!("DataFusion would not build this node: {error}"),
            )
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::df::refusal::RefusalReason;
    use crate::df::session::{MemoryPoolSettings, SeedSession};
    use crate::df::sketch_udaf::SummaryFunctions;
    use crate::df::variants::{
        every_executable_payload, every_value_operation, kll_family, latency_update,
        leaf_arrow_schema, plain_summary_schema, scan, TABLE,
    };
    use asap_types::post_asap::{
        DataPrimitive, ExactParams, ExecutionTiming, PostAsapDagDocument, SummaryField,
    };
    use asap_types::pre_asap::schema::DataType;
    use datafusion::datasource::MemTable;
    use std::sync::Arc;

    fn session() -> SeedSession {
        SeedSession::new(0, MemoryPoolSettings::default(), &SummaryFunctions)
            .expect("a session needs no OS resources")
    }

    fn sources() -> TableSources {
        let schema = Arc::new(leaf_arrow_schema());
        let table = MemTable::try_new(Arc::clone(&schema), vec![Vec::new()])
            .expect("an empty partition list is a table");
        TableSources::new().with_table(
            TABLE,
            datafusion::datasource::provider_as_source(Arc::new(table)),
        )
    }

    fn leaf_summary_schema() -> SummarySchema {
        plain_summary_schema(&[
            ("ts", DataType::Timestamp),
            ("service", DataType::Utf8),
            ("latency", DataType::Float64),
            ("bytes", DataType::Int64),
        ])
    }

    fn leaf_node() -> ExecutableDagNode {
        ExecutableDagNode {
            id: PostAsapNodeId(0),
            payload: ExecutableOperatorPayload::Fallback {
                expression: (*scan()).clone(),
            },
            output_state: ExecutionDataState::MAINTENANCE_ROWS,
            output_schema: leaf_summary_schema(),
            guarantee: None,
        }
    }

    fn feeding(
        payload: ExecutableOperatorPayload,
        output_schema: SummarySchema,
        output_state: ExecutionDataState,
    ) -> ExecutableDag {
        let leaf = leaf_node();
        let consumer = ExecutableDagNode {
            id: PostAsapNodeId(1),
            payload,
            output_state,
            output_schema,
            guarantee: None,
        };
        let edge = ExecutableDagEdge {
            producer: leaf.id,
            consumer: consumer.id,
            role: EdgeRole::Input,
            intermediate_schema: leaf.output_schema.clone(),
            data_state: leaf.output_state,
            grouping: GroupingEdgeCompatibility::NotApplicable,
            window: WindowEdgeCompatibility::NotApplicable,
        };
        ExecutableDag {
            nodes: vec![leaf, consumer],
            edges: vec![edge],
            root: PostAsapNodeId(1),
        }
    }

    fn fold(dag: &ExecutableDag) -> Result<DagPlans, Refusal> {
        let session = session();
        lower(dag, &sources(), &session.state())
    }

    #[test]
    fn every_payload_is_named_by_its_own_variant() {
        let named: Vec<&'static str> = every_executable_payload()
            .iter()
            .map(|(_, payload)| payload_name(payload))
            .collect();
        assert_eq!(
            named,
            vec![
                "Fallback",
                "Binary",
                "CandidateTopK",
                "Value",
                "RelationalJoin",
                "SummaryAgg",
                "SummaryJoin",
                "SummarySubtract",
                "SummaryDelete",
                "SummaryEstimate",
                "SummaryMerge",
            ],
            "eleven payloads, each with an arm"
        );
    }

    #[test]
    fn every_value_operation_is_named_by_its_own_variant() {
        let named: Vec<&'static str> = every_value_operation()
            .iter()
            .map(|(_, operation)| value_operation_name(operation))
            .collect();
        assert_eq!(
            named,
            vec![
                "MaintainPopulation",
                "ReadPopulation",
                "Exact",
                "FinalizeExactAccumulator",
                "Project",
                "Filter",
                "Sort",
                "Limit",
                "Extension",
            ],
            "the nine ValueOperation variants upstream has today"
        );
    }

    #[test]
    fn a_fallback_hands_its_whole_subtree_to_the_pre_asap_translator() {
        let dag = ExecutableDag {
            nodes: vec![leaf_node()],
            edges: Vec::new(),
            root: PostAsapNodeId(0),
        };
        let plans = fold(&dag).expect("a scan lowers");
        let scanned = plans.plan(PostAsapNodeId(0)).expect("the leaf has a plan");
        assert!(matches!(scanned, LogicalPlan::TableScan(_)), "{scanned:?}");
    }

    #[test]
    fn the_four_row_shaping_value_operations_lower() {
        for (name, operation) in every_value_operation() {
            if !matches!(
                operation,
                ValueOperation::Project { .. }
                    | ValueOperation::Filter { .. }
                    | ValueOperation::Sort { .. }
                    | ValueOperation::Limit { .. }
            ) {
                continue;
            }
            let schema = match &operation {
                ValueOperation::Project { .. } => {
                    plain_summary_schema(&[("latency", DataType::Float64)])
                }
                _ => leaf_summary_schema(),
            };
            let dag = feeding(
                ExecutableOperatorPayload::Value {
                    operation,
                    timing: ExecutionTiming::ReadTime,
                },
                schema,
                ExecutionDataState::READ_ROWS,
            );
            fold(&dag).unwrap_or_else(|refusal| panic!("{name} lowers: {refusal}"));
        }
    }

    #[test]
    fn an_exact_aggregate_residual_takes_the_same_path_as_a_fallback_aggregate() {
        let dag = feeding(
            ExecutableOperatorPayload::Value {
                operation: ValueOperation::Exact(ExactOperation::Aggregate {
                    reduction: Reduction::by(vec![1]),
                    measures: vec![asap_types::pre_asap::agg_intent::AggIntent::Sum {
                        col: Some(3),
                    }],
                    output_names: vec!["service".to_owned(), "total".to_owned()],
                    having: None,
                }),
                timing: ExecutionTiming::ReadTime,
            },
            plain_summary_schema(&[("service", DataType::Utf8), ("total", DataType::Int64)]),
            ExecutionDataState::READ_ROWS,
        );
        let plans = fold(&dag).expect("an exact residual lowers");
        let text = plans
            .plan(PostAsapNodeId(1))
            .expect("the aggregate has a plan")
            .display_indent()
            .to_string();
        assert!(text.contains("sum("), "{text}");
        assert!(text.contains("Aggregate:"), "{text}");
        assert!(
            text.contains(&node_alias(PostAsapNodeId(1), "total")),
            "an exact residual names its node like every other aggregate, or its time lands in \
             engine_overhead_ns: {text}"
        );
    }

    #[test]
    fn an_exact_aggregate_that_names_more_columns_than_its_node_declares_is_refused() {
        for declared in [
            plain_summary_schema(&[("service", DataType::Utf8)]),
            plain_summary_schema(&[
                ("service", DataType::Utf8),
                ("total", DataType::Int64),
                ("spare", DataType::Int64),
            ]),
        ] {
            let width = declared.fields.len();
            let dag = feeding(
                ExecutableOperatorPayload::Value {
                    operation: ValueOperation::Exact(ExactOperation::Aggregate {
                        reduction: Reduction::by(vec![1]),
                        measures: vec![asap_types::pre_asap::agg_intent::AggIntent::Sum {
                            col: Some(3),
                        }],
                        output_names: vec!["service".to_owned(), "total".to_owned()],
                        having: None,
                    }),
                    timing: ExecutionTiming::ReadTime,
                },
                declared,
                ExecutionDataState::READ_ROWS,
            );
            let refused = fold(&dag).unwrap_err();
            assert_eq!(refused.variant, "Value::Exact(Aggregate)", "{refused}");
            assert_eq!(refused.tag(), "no_constructor", "{width}: {refused}");
        }
    }

    #[test]
    fn finalizing_an_exact_accumulator_is_an_identity_projection() {
        let leaf = leaf_node();
        let accumulator = ExecutableDagNode {
            id: PostAsapNodeId(1),
            payload: ExecutableOperatorPayload::SummaryAgg {
                family: SummaryFamilyType::ExactAggregate(ExactKind::Sum, ExactParams::Sum),
                input: latency_update(),
                reduction: Reduction::by(vec![]),
                grouping: GroupingStrategy::PerSubpopulationInstance,
            },
            output_state: ExecutionDataState::MAINTENANCE_SUMMARY,
            output_schema: SummarySchema {
                fields: vec![SummaryField {
                    name: "total".to_owned(),
                    dtype: SummaryFamilyType::ExactAggregate(ExactKind::Sum, ExactParams::Sum),
                    nullable: false,
                }],
                time_index: None,
            },
            guarantee: None,
        };
        let finalize = ExecutableDagNode {
            id: PostAsapNodeId(2),
            payload: ExecutableOperatorPayload::Value {
                operation: ValueOperation::FinalizeExactAccumulator,
                timing: ExecutionTiming::ReadTime,
            },
            output_state: ExecutionDataState::READ_ROWS,
            output_schema: plain_summary_schema(&[("total", DataType::Float64)]),
            guarantee: None,
        };
        let dag = ExecutableDag {
            edges: vec![
                ExecutableDagEdge {
                    producer: leaf.id,
                    consumer: accumulator.id,
                    role: EdgeRole::Input,
                    intermediate_schema: leaf.output_schema.clone(),
                    data_state: leaf.output_state,
                    grouping: GroupingEdgeCompatibility::NotApplicable,
                    window: WindowEdgeCompatibility::NotApplicable,
                },
                ExecutableDagEdge {
                    producer: accumulator.id,
                    consumer: finalize.id,
                    role: EdgeRole::Input,
                    intermediate_schema: accumulator.output_schema.clone(),
                    data_state: accumulator.output_state,
                    grouping: GroupingEdgeCompatibility::NotApplicable,
                    window: WindowEdgeCompatibility::NotApplicable,
                },
            ],
            nodes: vec![leaf, accumulator, finalize],
            root: PostAsapNodeId(2),
        };
        let plans = fold(&dag).expect("an exact accumulator finalizes");
        let text = plans
            .plan(PostAsapNodeId(2))
            .expect("the finalize node has a plan")
            .display_indent()
            .to_string();
        assert!(text.contains("Projection:"), "{text}");
    }

    #[test]
    fn a_counter_rate_accumulator_is_refused_as_order_sensitive() {
        let dag = feeding(
            ExecutableOperatorPayload::SummaryAgg {
                family: SummaryFamilyType::ExactAggregate(ExactKind::Rate, ExactParams::Rate),
                input: latency_update(),
                reduction: Reduction::by(vec![]),
                grouping: GroupingStrategy::PerSubpopulationInstance,
            },
            SummarySchema {
                fields: vec![SummaryField {
                    name: "rate".to_owned(),
                    dtype: SummaryFamilyType::ExactAggregate(ExactKind::Rate, ExactParams::Rate),
                    nullable: false,
                }],
                time_index: None,
            },
            ExecutionDataState::MAINTENANCE_SUMMARY,
        );
        let refused = fold(&dag).unwrap_err();
        assert_eq!(
            refused.reason,
            RefusalReason::Deferred {
                issue: "order-sensitive".into()
            },
            "{refused}"
        );
    }

    #[test]
    fn a_summary_family_the_planner_does_not_build_is_refused_without_a_constructor() {
        for family in [
            SummaryFamilyType::Plain(DataType::Float64),
            SummaryFamilyType::Sample(
                asap_types::post_asap::SamplingKind::Reservoir,
                asap_types::post_asap::SamplingParams::Reservoir { size: 8 },
            ),
        ] {
            let dag = feeding(
                ExecutableOperatorPayload::SummaryAgg {
                    family: family.clone(),
                    input: latency_update(),
                    reduction: Reduction::by(vec![]),
                    grouping: GroupingStrategy::PerSubpopulationInstance,
                },
                SummarySchema {
                    fields: vec![SummaryField {
                        name: "held".to_owned(),
                        dtype: family.clone(),
                        nullable: false,
                    }],
                    time_index: None,
                },
                ExecutionDataState::MAINTENANCE_SUMMARY,
            );
            let refused = fold(&dag).unwrap_err();
            assert_eq!(refused.tag(), "no_constructor", "{family:?}: {refused}");
        }
    }

    #[test]
    fn a_hydra_layout_is_deferred_because_the_ir_maps_no_keys_to_its_buckets() {
        let hydra = GroupingStrategy::SharedMultiSubpopulation {
            kind: asap_types::post_asap::HydraKind::HydraCms,
            params: asap_types::post_asap::HydraParams::HydraCms {
                width: 64,
                depth: 4,
                shared_rows: 2,
                shared_columns: 8,
            },
        };
        let dag = feeding(
            ExecutableOperatorPayload::SummaryAgg {
                family: kll_family(),
                input: latency_update(),
                reduction: Reduction::by(vec![]),
                grouping: hydra,
            },
            SummarySchema {
                fields: vec![SummaryField {
                    name: "held".to_owned(),
                    dtype: kll_family(),
                    nullable: false,
                }],
                time_index: None,
            },
            ExecutionDataState::MAINTENANCE_SUMMARY,
        );
        let refused = fold(&dag).unwrap_err();
        assert_eq!(
            refused.reason,
            RefusalReason::Deferred {
                issue: "hydra-layout".into()
            },
            "{refused}"
        );
    }

    #[test]
    fn a_per_entity_reduction_is_refused_on_the_time_axis() {
        let dag = feeding(
            ExecutableOperatorPayload::SummaryAgg {
                family: kll_family(),
                input: latency_update(),
                reduction: Reduction::PerEntity,
                grouping: GroupingStrategy::PerSubpopulationInstance,
            },
            SummarySchema {
                fields: vec![SummaryField {
                    name: "held".to_owned(),
                    dtype: kll_family(),
                    nullable: false,
                }],
                time_index: None,
            },
            ExecutionDataState::MAINTENANCE_SUMMARY,
        );
        assert_eq!(fold(&dag).unwrap_err().tag(), "time_axis");
    }

    #[test]
    fn the_two_state_edits_are_refused_permanently_and_the_joins_are_deferred() {
        let expected = [
            ("ExecutableOperatorPayload::Binary", "deferred"),
            ("ExecutableOperatorPayload::CandidateTopK", "deferred"),
            ("ExecutableOperatorPayload::RelationalJoin", "deferred"),
            ("ExecutableOperatorPayload::SummaryJoin", "no_constructor"),
            (
                "ExecutableOperatorPayload::SummarySubtract",
                "no_constructor",
            ),
            ("ExecutableOperatorPayload::SummaryDelete", "no_constructor"),
            ("ExecutableOperatorPayload::SummaryMerge", "no_constructor"),
        ];
        for (name, tag) in expected {
            let (_, payload) = every_executable_payload()
                .into_iter()
                .find(|(held, _)| *held == name)
                .expect("the payload list names it");
            let dag = feeding(
                payload,
                leaf_summary_schema(),
                ExecutionDataState::READ_ROWS,
            );
            let refused = fold(&dag).unwrap_err();
            assert_eq!(refused.tag(), tag, "{name}: {refused}");
        }
    }

    #[test]
    fn the_operations_without_a_physical_operator_are_deferred_by_name() {
        for name in [
            "ValueOperation::MaintainPopulation",
            "ValueOperation::ReadPopulation",
            "ValueOperation::Extension",
        ] {
            let (_, operation) = every_value_operation()
                .into_iter()
                .find(|(held, _)| *held == name)
                .expect("the operation list names it");
            let dag = feeding(
                ExecutableOperatorPayload::Value {
                    operation,
                    timing: ExecutionTiming::ReadTime,
                },
                leaf_summary_schema(),
                ExecutionDataState::READ_ROWS,
            );
            let refused = fold(&dag).unwrap_err();
            assert_eq!(refused.tag(), "deferred", "{name}: {refused}");
        }
    }

    #[test]
    fn a_partitioned_order_is_refused_the_same_way_the_pre_asap_translator_refuses_it() {
        let dag = feeding(
            ExecutableOperatorPayload::Value {
                operation: ValueOperation::Sort {
                    keys: Vec::new(),
                    partition_by: asap_types::pre_asap::GroupKeys::by(vec![1]),
                },
                timing: ExecutionTiming::ReadTime,
            },
            leaf_summary_schema(),
            ExecutionDataState::READ_ROWS,
        );
        let refused = fold(&dag).unwrap_err();
        assert_eq!(
            refused.reason,
            RefusalReason::Deferred {
                issue: "sort-partition-limit-coupling".into()
            },
            "{refused}"
        );
    }

    fn gate(
        grouping: GroupingEdgeCompatibility,
        window: WindowEdgeCompatibility,
        producer_state: ExecutionDataState,
        consumer_state: ExecutionDataState,
        consumer_payload: ExecutableOperatorPayload,
    ) -> Result<(), Refusal> {
        let producer = ExecutableDagNode {
            id: PostAsapNodeId(0),
            payload: ExecutableOperatorPayload::SummaryMerge,
            output_state: producer_state,
            output_schema: leaf_summary_schema(),
            guarantee: None,
        };
        let consumer = ExecutableDagNode {
            id: PostAsapNodeId(1),
            payload: consumer_payload,
            output_state: consumer_state,
            output_schema: leaf_summary_schema(),
            guarantee: None,
        };
        let edge = ExecutableDagEdge {
            producer: producer.id,
            consumer: consumer.id,
            role: EdgeRole::Input,
            intermediate_schema: producer.output_schema.clone(),
            data_state: producer.output_state,
            grouping,
            window,
        };
        refuse_unless_edge_composes(&edge, &producer, &consumer)
    }

    #[test]
    fn an_incompatible_grouping_edge_is_refused_and_a_coarsening_one_needs_a_merge() {
        let refused = gate(
            GroupingEdgeCompatibility::Incompatible,
            WindowEdgeCompatibility::NotApplicable,
            ExecutionDataState::MAINTENANCE_SUMMARY,
            ExecutionDataState::READ_ROWS,
            ExecutableOperatorPayload::SummaryEstimate {
                query: asap_types::post_asap::SketchQuery::Cardinality,
            },
        )
        .unwrap_err();
        assert_eq!(refused.tag(), "no_constructor", "{refused}");

        let refused = gate(
            GroupingEdgeCompatibility::ConsumerCoarsensProducer,
            WindowEdgeCompatibility::NotApplicable,
            ExecutionDataState::MAINTENANCE_SUMMARY,
            ExecutionDataState::READ_ROWS,
            ExecutableOperatorPayload::SummaryEstimate {
                query: asap_types::post_asap::SketchQuery::Cardinality,
            },
        )
        .unwrap_err();
        assert_eq!(
            refused.reason,
            RefusalReason::Deferred {
                issue: "coarsening-without-merge".into()
            },
            "{refused}"
        );

        gate(
            GroupingEdgeCompatibility::ConsumerCoarsensProducer,
            WindowEdgeCompatibility::NotApplicable,
            ExecutionDataState::MAINTENANCE_SUMMARY,
            ExecutionDataState::MAINTENANCE_SUMMARY,
            ExecutableOperatorPayload::SummaryMerge,
        )
        .expect("an explicit merge node discharges the coarsening");

        for grouping in [
            GroupingEdgeCompatibility::Identical,
            GroupingEdgeCompatibility::NotApplicable,
        ] {
            gate(
                grouping,
                WindowEdgeCompatibility::NotApplicable,
                ExecutionDataState::MAINTENANCE_SUMMARY,
                ExecutionDataState::READ_ROWS,
                ExecutableOperatorPayload::SummaryEstimate {
                    query: asap_types::post_asap::SketchQuery::Cardinality,
                },
            )
            .expect("these two compose");
        }
    }

    #[test]
    fn a_window_obligation_is_refused_only_between_two_summary_states() {
        let refused = gate(
            GroupingEdgeCompatibility::NotApplicable,
            WindowEdgeCompatibility::RequiresAlignedPanePhaseOrExactWindowEdgeResidual,
            ExecutionDataState::MAINTENANCE_SUMMARY,
            ExecutionDataState::MAINTENANCE_SUMMARY,
            ExecutableOperatorPayload::SummaryMerge,
        )
        .unwrap_err();
        assert_eq!(refused.tag(), "time_axis", "{refused}");

        for (producer, consumer) in [
            (
                ExecutionDataState::MAINTENANCE_ROWS,
                ExecutionDataState::MAINTENANCE_SUMMARY,
            ),
            (
                ExecutionDataState::MAINTENANCE_SUMMARY,
                ExecutionDataState::READ_ROWS,
            ),
        ] {
            gate(
                GroupingEdgeCompatibility::NotApplicable,
                WindowEdgeCompatibility::RequiresAlignedPanePhaseOrExactWindowEdgeResidual,
                producer,
                consumer,
                ExecutableOperatorPayload::SummaryEstimate {
                    query: asap_types::post_asap::SketchQuery::Cardinality,
                },
            )
            .unwrap_or_else(|refusal| {
                panic!("{producer:?} -> {consumer:?} discharges the stamp: {refusal}")
            });
        }
    }

    #[test]
    fn the_fold_order_comes_from_the_edges_and_not_from_the_node_ids() {
        let leaf = leaf_node();
        let consumer = ExecutableDagNode {
            id: PostAsapNodeId(1),
            payload: ExecutableOperatorPayload::Value {
                operation: ValueOperation::Limit { n: 1, offset: 0 },
                timing: ExecutionTiming::ReadTime,
            },
            output_state: ExecutionDataState::READ_ROWS,
            output_schema: leaf_summary_schema(),
            guarantee: None,
        };
        let edge = ExecutableDagEdge {
            producer: leaf.id,
            consumer: consumer.id,
            role: EdgeRole::Input,
            intermediate_schema: leaf.output_schema.clone(),
            data_state: leaf.output_state,
            grouping: GroupingEdgeCompatibility::NotApplicable,
            window: WindowEdgeCompatibility::NotApplicable,
        };
        let dag = ExecutableDag {
            nodes: vec![consumer, leaf],
            edges: vec![edge],
            root: PostAsapNodeId(1),
        };
        PostAsapDagDocument::new(dag.clone())
            .validate()
            .expect("the dag is well formed however its nodes are listed");
        assert_eq!(
            fold_order(&dag).expect("the edges order it"),
            vec![PostAsapNodeId(0), PostAsapNodeId(1)],
            "the producer folds first although it is listed second"
        );
        assert_eq!(fold(&dag).expect("it lowers").len(), 2);
    }

    #[test]
    fn the_three_legal_data_states_are_the_only_ones_the_edges_carry() {
        for state in [
            ExecutionDataState::MAINTENANCE_ROWS,
            ExecutionDataState::MAINTENANCE_SUMMARY,
            ExecutionDataState::READ_ROWS,
        ] {
            assert!(
                state.timing == ExecutionTiming::MaintenanceTime
                    || state.primitive == DataPrimitive::Raw,
                "read-time summary state has no constant: {state:?}"
            );
        }
    }
}
