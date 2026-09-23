//! Getting a plan into main memory and preparing it for execution.
//!
//! The chain is PromQL -> pre-ASAP `QueryExpr` -> plan space -> global
//! selection -> assembled `SummaryNode` -> `ExecutableDag`, all in process.
//! `compile_executable_dag_with_node_ids` is always called, even though nothing
//! is written to a wire: the `ExecutionDataState` (timing + primitive)
//! assignment is computed during compilation and does not exist on the
//! `SummaryNode` tree, and the ten checks in `validate()` only exist on
//! `ExecutableDag`.
//!
//! The execution order is computed here by Kahn's algorithm over `dag.edges`.
//! Node ids happen to come out of the compiler in producer-before-consumer
//! order, but that is a property of the compiler, not an invariant the
//! validated document enforces (PLAN.md 1.1, 2.2), so the ordering is derived
//! from the edges and the id monotonicity is only checked, never relied upon.

use std::cmp::Reverse;
use std::collections::{BinaryHeap, HashMap};
use std::rc::Rc;
use std::time::Duration;

use asap_aware_mapping::{search_workload, DefaultCostModel};
use asap_frontend_promql::{lower_promql_workload, PromqlError};
use asap_frontend_sql::SqlCatalog;
use asap_types::post_asap::{
    compile_executable_dag_with_node_ids, ExecutableDag, ExecutableNodeIdentityMap,
    PostAsapDagDocument, PostAsapNodeId,
};
use asap_types::pre_asap::QueryExpr;
use asap_types::types::AccuracyTarget;
use asap_types::workload::{
    AccuracyRequirement, BatchEntry, DataWorkload, DurationMs, Evidence, PlanningWorkload,
    Predictability, Query, QueryLanguage, QueryRequirements, QueryWorkload, TimeSelection,
};

use crate::types::{EvalError, PlanId, PlanningStage};

const DATA_INGESTION_INTERVAL: DurationMs = DurationMs(1_000);

const SECOND_INGESTION_INTERVAL: DurationMs = DurationMs(2_000);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TimeRangeOrigin {
    #[default]
    Unknown,
    InjectedIngestionHorizon,
}

pub fn lower_promql(query: &str, accuracy: AccuracyTarget) -> Result<QueryExpr, PromqlError> {
    lower_promql_declaring(query, accuracy, DATA_INGESTION_INTERVAL)
}

fn lower_promql_declaring(
    query: &str,
    accuracy: AccuracyTarget,
    interval: DurationMs,
) -> Result<QueryExpr, PromqlError> {
    let workload = PlanningWorkload {
        query_workload: QueryWorkload {
            language: QueryLanguage::PromQL,
            query_batch: Some(vec![BatchEntry {
                query: Query(query.to_string()),
                requirements: QueryRequirements {
                    accuracy: AccuracyRequirement::Explicit(accuracy),
                    ..Default::default()
                },
                predictability: Predictability::Unknown,
                invocations: 1,
                execute_at: None,
                time_selection: TimeSelection::default(),
            }]),
            repeating_queries: None,
        },
        data_workload: Some(DataWorkload {
            data_ingestion_interval: Evidence {
                value: Some(interval),
                ..Default::default()
            },
            ..Default::default()
        }),
    };
    let mut lowered = lower_promql_workload(&workload, 0)?;
    Ok(lowered.remove(0))
}

fn time_range_origin(
    query: &str,
    accuracy: &AccuracyTarget,
    lowered: &QueryExpr,
) -> Result<TimeRangeOrigin, PromqlError> {
    if !every_range_selects(lowered, DATA_INGESTION_INTERVAL) {
        return Ok(TimeRangeOrigin::Unknown);
    }
    let second = lower_promql_declaring(query, accuracy.clone(), SECOND_INGESTION_INTERVAL)?;
    Ok(if every_range_selects(&second, SECOND_INGESTION_INTERVAL) {
        TimeRangeOrigin::InjectedIngestionHorizon
    } else {
        TimeRangeOrigin::Unknown
    })
}

fn every_range_selects(expr: &QueryExpr, interval: DurationMs) -> bool {
    let mut selected = Vec::new();
    selected_ranges(expr, &mut selected);
    selected
        .iter()
        .all(|range| range.as_millis() == u128::from(interval.0))
}

fn selected_ranges(expr: &QueryExpr, found: &mut Vec<Duration>) {
    match expr {
        QueryExpr::TimeRange { range, child } => {
            found.push(*range);
            selected_ranges(child, found);
        }
        QueryExpr::Scan { predicates, .. } => {
            for predicate in predicates {
                selected_ranges(&predicate.0, found);
            }
        }
        QueryExpr::EvalTimestamp
        | QueryExpr::CurrentTimestamp
        | QueryExpr::Column(_)
        | QueryExpr::Literal(_) => {}
        QueryExpr::PromqlScalarBridge(child)
        | QueryExpr::PromqlVectorFromScalar(child)
        | QueryExpr::PromqlScalarFromVector(child)
        | QueryExpr::Not(child)
        | QueryExpr::IsNull(child)
        | QueryExpr::IsNotNull(child) => selected_ranges(child, found),
        QueryExpr::PromqlRelabel { value, child, .. } => {
            selected_ranges(value, found);
            selected_ranges(child, found);
        }
        QueryExpr::PromqlInfoEnrich { child, .. }
        | QueryExpr::PromqlSeriesSample { child, .. }
        | QueryExpr::Dedup { child, .. }
        | QueryExpr::Limit { child, .. }
        | QueryExpr::PromqlSubquery { child, .. }
        | QueryExpr::TimeShift { child, .. } => selected_ranges(child, found),
        QueryExpr::Filter { pred, child } => {
            selected_ranges(&pred.0, found);
            selected_ranges(child, found);
        }
        QueryExpr::Project { cols, child, .. } => {
            for item in cols {
                selected_ranges(&item.expr, found);
            }
            selected_ranges(child, found);
        }
        QueryExpr::Aggregate { having, child, .. } => {
            if let Some(pred) = having {
                selected_ranges(&pred.0, found);
            }
            selected_ranges(child, found);
        }
        QueryExpr::Concat { children, .. } => {
            for branch in children {
                selected_ranges(branch, found);
            }
        }
        QueryExpr::Join {
            pred, left, right, ..
        } => {
            selected_ranges(&pred.0, found);
            selected_ranges(left, found);
            selected_ranges(right, found);
        }
        QueryExpr::SetOp { left, right, .. } => {
            selected_ranges(left, found);
            selected_ranges(right, found);
        }
        QueryExpr::Sort { keys, child, .. } => {
            for key in keys {
                selected_ranges(&key.expr, found);
            }
            selected_ranges(child, found);
        }
        QueryExpr::SQLWindowFunc {
            args,
            order_by,
            child,
            ..
        } => {
            for arg in args {
                selected_ranges(arg, found);
            }
            for key in order_by {
                selected_ranges(&key.expr, found);
            }
            selected_ranges(child, found);
        }
        QueryExpr::BinaryOp { lhs, rhs, .. } => {
            selected_ranges(lhs, found);
            selected_ranges(rhs, found);
        }
        QueryExpr::Compare { left, right, .. } | QueryExpr::Arithmetic { left, right, .. } => {
            selected_ranges(left, found);
            selected_ranges(right, found);
        }
        QueryExpr::BoolAnd(items) | QueryExpr::BoolOr(items) => {
            for item in items {
                selected_ranges(item, found);
            }
        }
        QueryExpr::Cast { expr, .. } => selected_ranges(expr, found),
        QueryExpr::InList { expr, list, .. } => {
            selected_ranges(expr, found);
            for item in list {
                selected_ranges(item, found);
            }
        }
        QueryExpr::FunctionCall { args, .. } => {
            for arg in args {
                selected_ranges(arg, found);
            }
        }
        QueryExpr::Case {
            operand,
            branches,
            else_expr,
        } => {
            if let Some(operand) = operand {
                selected_ranges(operand, found);
            }
            for (when, then) in branches {
                selected_ranges(when, found);
                selected_ranges(then, found);
            }
            if let Some(else_expr) = else_expr {
                selected_ranges(else_expr, found);
            }
        }
    }
}

pub(crate) fn ingestion_horizon_child(
    expr: &QueryExpr,
    origin: TimeRangeOrigin,
) -> Option<&Rc<QueryExpr>> {
    match (origin, expr) {
        (TimeRangeOrigin::InjectedIngestionHorizon, QueryExpr::TimeRange { child, .. }) => {
            Some(child)
        }
        _ => None,
    }
}

#[cfg(test)]
pub(crate) fn ingestion_horizon_child_mut(
    expr: &mut QueryExpr,
    origin: TimeRangeOrigin,
) -> Option<&mut QueryExpr> {
    match (origin, expr) {
        (TimeRangeOrigin::InjectedIngestionHorizon, QueryExpr::TimeRange { child, .. }) => {
            Some(Rc::make_mut(child))
        }
        _ => None,
    }
}

/// A decoded, validated plan, the order its nodes must run in, and the
/// pre-ASAP tree the same lowering produced — the plan's own baseline, which
/// the wire document does not carry and a plan decoded from one therefore has
/// no copy of.
#[derive(Debug, Clone)]
pub struct Plan {
    /// blake3 of the canonical JSON of the `PostAsapDagDocument`.
    pub id: PlanId,
    pub dag: ExecutableDag,
    /// Topological, producers first. Every node appears exactly once.
    pub order: Vec<PostAsapNodeId>,
    pub pre_asap: Option<Rc<QueryExpr>>,
    pub node_ids: Option<ExecutableNodeIdentityMap>,
    pub time_range_origin: TimeRangeOrigin,
}

impl Plan {
    /// The document envelope this plan's `id` is computed over.
    pub fn document(&self) -> PostAsapDagDocument {
        PostAsapDagDocument::new(self.dag.clone())
    }
}

/// PromQL -> pre-ASAP -> post-ASAP -> `ExecutableDag`, all in memory.
pub fn plan_promql(query: &str, accuracy: AccuracyTarget) -> Result<Plan, EvalError> {
    let mut planned = plan_promql_workload(&[(query, query)], accuracy)?;
    if planned.len() != 1 {
        return Err(EvalError::Planning {
            stage: PlanningStage::Search,
            detail: format!(
                "planning {query:?} produced {} roots, expected 1",
                planned.len()
            ),
        });
    }
    Ok(planned.remove(0).1)
}

pub fn lower_promql_root(
    query: &str,
    accuracy: AccuracyTarget,
) -> Result<(Rc<QueryExpr>, TimeRangeOrigin), EvalError> {
    let planning = |err| EvalError::Planning {
        stage: PlanningStage::Lower,
        detail: format!("{query:?}: {err:?}"),
    };
    let expr = lower_promql(query, accuracy.clone()).map_err(planning)?;
    let origin = time_range_origin(query, &accuracy, &expr).map_err(planning)?;
    Ok((Rc::new(expr), origin))
}

/// Same, for a whole workload planned together, so cross-root CSE has a chance.
///
/// `queries` is `(name, query)`; the returned names are the ones passed in.
pub fn plan_promql_workload(
    queries: &[(&str, &str)],
    accuracy: AccuracyTarget,
) -> Result<Vec<(String, Plan)>, EvalError> {
    let mut roots = Vec::with_capacity(queries.len());
    let mut origin = TimeRangeOrigin::InjectedIngestionHorizon;
    for (name, query) in queries {
        let planning = |err| EvalError::Planning {
            stage: PlanningStage::Lower,
            detail: format!("{name:?} ({query:?}): {err:?}"),
        };
        let expr = lower_promql(query, accuracy.clone()).map_err(planning)?;
        if time_range_origin(query, &accuracy, &expr).map_err(planning)? == TimeRangeOrigin::Unknown
        {
            origin = TimeRangeOrigin::Unknown;
        }
        roots.push(((*name).to_string(), Rc::new(expr)));
    }
    plan_roots(roots, origin)
}

pub fn plan_sql(
    sql: &str,
    catalog: &SqlCatalog,
    accuracy: AccuracyTarget,
) -> Result<Plan, EvalError> {
    plan_sql_root(sql, crate::sql::lower_sql_root(sql, catalog, accuracy)?)
}

pub fn plan_sql_root(sql: &str, root: Rc<QueryExpr>) -> Result<Plan, EvalError> {
    let mut planned = plan_roots(vec![(sql.to_string(), root)], TimeRangeOrigin::Unknown)?;
    if planned.len() != 1 {
        return Err(EvalError::Planning {
            stage: PlanningStage::Search,
            detail: format!(
                "planning {sql:?} produced {} roots, expected 1",
                planned.len()
            ),
        });
    }
    Ok(planned.remove(0).1)
}

fn plan_roots(
    roots: Vec<(String, Rc<QueryExpr>)>,
    origin: TimeRangeOrigin,
) -> Result<Vec<(String, Plan)>, EvalError> {
    // `search_workload` runs CSE over the roots and may hand back different
    // `Rc`s than the ones passed in, so assembly targets are read back
    // off the space rather than reused from `roots`.
    let space = search_workload(roots);
    let selection = space.global_selection(&DefaultCostModel);

    let mut planned = Vec::with_capacity(space.roots.len());
    for (name, root) in &space.roots {
        let assembled = match selection.assemble_selected_dag(root) {
            Ok(Some(node)) => node,
            Ok(None) => {
                return Err(EvalError::Planning {
                    stage: PlanningStage::Search,
                    detail: format!("root {name:?} was not discovered by the plan space"),
                })
            }
            Err(err) => {
                return Err(EvalError::Planning {
                    stage: PlanningStage::Search,
                    detail: format!("assemble_selected_dag {name:?}: {err:?}"),
                })
            }
        };
        let compilation = compile_executable_dag_with_node_ids(&assembled).map_err(|err| {
            EvalError::Planning {
                stage: PlanningStage::Compile,
                detail: format!("{name:?}: {err:?}"),
            }
        })?;
        planned.push((
            name.clone(),
            from_dag(
                compilation.dag,
                Some(Rc::clone(root)),
                Some(compilation.node_ids),
                origin,
            )?,
        ));
    }
    Ok(planned)
}

/// Canonical JSON: 2-space pretty, LF, no trailing newline.
///
/// This is the exact byte string the `PlanId` hashes, so a fixture written from
/// it decodes back to the same id.
pub fn to_json(plan: &Plan) -> Result<String, EvalError> {
    canonical_json(&plan.dag)
}

fn canonical_json(dag: &ExecutableDag) -> Result<String, EvalError> {
    let document = PostAsapDagDocument::new(dag.clone());
    serde_json::to_string_pretty(&document)
        .map_err(|err| EvalError::Validation(format!("encode post-ASAP DAG document: {err}")))
}

/// Validate, order, and hash one compiled dag.
fn from_dag(
    dag: ExecutableDag,
    pre_asap: Option<Rc<QueryExpr>>,
    node_ids: Option<ExecutableNodeIdentityMap>,
    time_range_origin: TimeRangeOrigin,
) -> Result<Plan, EvalError> {
    let document = PostAsapDagDocument::new(dag);
    document
        .validate()
        .map_err(|err| EvalError::Validation(err.to_string()))?;

    let json = canonical_json(&document.dag)?;
    let id: PlanId = *blake3::hash(json.as_bytes()).as_bytes();
    let order = topological_order(&document.dag)?;

    Ok(Plan {
        id,
        dag: document.dag,
        order,
        pre_asap,
        node_ids,
        time_range_origin,
    })
}

/// Kahn's algorithm over `dag.edges`.
///
/// Ready nodes are drained smallest id first purely so the order is
/// deterministic across runs; correctness does not depend on it, and no edge
/// direction is inferred from the ids.
pub fn topological_order(dag: &ExecutableDag) -> Result<Vec<PostAsapNodeId>, EvalError> {
    let mut indegree: HashMap<PostAsapNodeId, usize> =
        dag.nodes.iter().map(|node| (node.id, 0usize)).collect();
    let mut consumers: HashMap<PostAsapNodeId, Vec<PostAsapNodeId>> = HashMap::new();

    for edge in &dag.edges {
        if !indegree.contains_key(&edge.producer) || !indegree.contains_key(&edge.consumer) {
            return Err(EvalError::Validation(format!(
                "edge {:?}->{:?} names a node that is not in the document",
                edge.producer, edge.consumer
            )));
        }
        // Parallel edges (the same producer feeding one consumer in two roles)
        // are counted once each: the consumer is ready only after every
        // incoming edge has been released.
        *indegree.entry(edge.consumer).or_insert(0) += 1;
        consumers
            .entry(edge.producer)
            .or_default()
            .push(edge.consumer);
    }

    let mut ready: BinaryHeap<Reverse<u32>> = indegree
        .iter()
        .filter(|(_, degree)| **degree == 0)
        .map(|(id, _)| Reverse(id.0))
        .collect();

    let mut order = Vec::with_capacity(dag.nodes.len());
    while let Some(Reverse(raw)) = ready.pop() {
        let id = PostAsapNodeId(raw);
        order.push(id);
        for consumer in consumers.get(&id).into_iter().flatten() {
            let degree = indegree
                .get_mut(consumer)
                .expect("edge endpoints were checked above");
            *degree -= 1;
            if *degree == 0 {
                ready.push(Reverse(consumer.0));
            }
        }
    }

    if order.len() != dag.nodes.len() {
        return Err(EvalError::Validation(format!(
            "post-ASAP DAG contains a cycle: ordered {} of {} nodes",
            order.len(),
            dag.nodes.len()
        )));
    }
    Ok(order)
}

#[cfg(test)]
mod tests {
    fn from_json(bytes: &[u8]) -> Result<Plan, EvalError> {
        let document: PostAsapDagDocument = serde_json::from_slice(bytes).map_err(|err| {
            EvalError::Validation(format!("decode post-ASAP DAG document: {err}"))
        })?;
        from_dag(document.dag, None, None, TimeRangeOrigin::Unknown)
    }

    use super::*;
    use crate::rows::variant_name;
    use crate::run::operator_name;
    use asap_types::post_asap::{
        ExactKind, ExactParams, ExecutableOperatorPayload, GroupingStrategy, SketchAlgorithm,
        SketchKind, SketchParams, SummaryFamilyType,
    };

    const ACCURACY: AccuracyTarget = AccuracyTarget::Epsilon(0.01);

    fn operators(plan: &Plan) -> Vec<&'static str> {
        plan.order
            .iter()
            .map(|id| {
                operator_name(
                    &plan
                        .dag
                        .nodes
                        .iter()
                        .find(|node| node.id == *id)
                        .expect("ordered id names a node")
                        .payload,
                )
            })
            .collect()
    }

    fn ids(plan: &Plan) -> Vec<u32> {
        plan.order.iter().map(|id| id.0).collect()
    }

    #[test]
    fn quantile_plans_three_nodes_in_producer_order() {
        let plan = plan_promql("quantile(0.5, cpu_cores)", ACCURACY).expect("plans");
        assert_eq!(plan.dag.nodes.len(), 3);
        assert_eq!(plan.dag.edges.len(), 2);
        assert_eq!(ids(&plan), vec![0, 1, 2]);
        assert_eq!(
            operators(&plan),
            vec!["Fallback", "SummaryAgg", "SummaryEstimate"]
        );
        assert_eq!(plan.dag.root, PostAsapNodeId(2));
    }

    #[test]
    fn sum_plans_two_nodes() {
        let plan = plan_promql("sum(cpu_cores)", ACCURACY).expect("plans");
        assert_eq!(plan.dag.nodes.len(), 2);
        assert_eq!(plan.order.len(), 2);
        assert_eq!(operators(&plan), vec!["Fallback", "SummaryAgg"]);
    }

    #[test]
    fn an_instant_selector_injects_the_horizon_and_a_range_selector_is_the_querys_own() {
        let (instant, instant_origin) =
            lower_promql_root("cpu_cores", ACCURACY).expect("an instant selector lowers");
        assert_eq!(
            instant_origin,
            TimeRangeOrigin::InjectedIngestionHorizon,
            "nothing in `cpu_cores` selects a range, so the TimeRange the lowering wrapped it \
             in is the declared ingestion horizon"
        );
        assert_eq!(variant_name(&instant), "TimeRange");
        assert!(ingestion_horizon_child(&instant, instant_origin).is_some());

        for query in [
            "max_over_time(cpu_cores[1s])",
            "max_over_time(cpu_cores[5s])",
        ] {
            let (root, origin) = lower_promql_root(query, ACCURACY).expect("lowers");
            assert_eq!(
                origin,
                TimeRangeOrigin::Unknown,
                "{query} selects its own range, and a 1s one is the same shape the lowering \
                 injects"
            );
            let QueryExpr::Aggregate { child, .. } = root.as_ref() else {
                panic!("{root:?}");
            };
            assert_eq!(variant_name(child), "TimeRange");
            assert!(
                ingestion_horizon_child(child, origin).is_none(),
                "{query}: peeling the range the query wrote answers over every row"
            );
        }
    }

    #[test]
    fn bare_metric_plans_one_node_and_no_edges() {
        let plan = plan_promql("cpu_cores", ACCURACY).expect("plans");
        assert_eq!(plan.dag.nodes.len(), 1);
        assert!(plan.dag.edges.is_empty());
        assert_eq!(ids(&plan), vec![0]);
        assert_eq!(operators(&plan), vec!["Fallback"]);
    }

    fn metrics_catalog() -> SqlCatalog {
        use asap_types::pre_asap::schema::{Column, DataType, Schema};
        SqlCatalog::new().with_table(
            "metrics",
            Schema::with_time_index(
                vec![
                    Column::new("ts", DataType::Timestamp, false),
                    Column::new("service", DataType::Utf8, false),
                    Column::new("latency", DataType::Float64, false),
                    Column::new("bytes", DataType::Int64, false),
                ],
                0,
                Vec::new(),
            ),
        )
    }

    fn bound_family(plan: &Plan) -> SummaryFamilyType {
        plan.dag
            .nodes
            .iter()
            .find_map(|node| match &node.payload {
                ExecutableOperatorPayload::SummaryAgg { family, .. } => Some(family.clone()),
                _ => None,
            })
            .expect("the plan binds one summary")
    }

    #[test]
    fn sql_quantile_binds_a_kll_sketch_sized_from_epsilon() {
        let plan = plan_sql(
            "SELECT approx_percentile_cont(latency, 0.99) FROM metrics",
            &metrics_catalog(),
            ACCURACY,
        )
        .expect("plans");
        assert_eq!(
            bound_family(&plan),
            SummaryFamilyType::Sketch(
                SketchKind::new(SketchAlgorithm::Kll, SketchParams::Kll { k: 269 }),
                GroupingStrategy::default()
            )
        );
        assert_eq!(
            operators(&plan),
            vec!["Fallback", "SummaryAgg", "SummaryEstimate", "Value"]
        );
    }

    #[test]
    fn sql_sum_group_by_binds_an_exact_accumulator_and_no_sketch() {
        let plan = plan_sql(
            "SELECT service, SUM(bytes) FROM metrics GROUP BY service",
            &metrics_catalog(),
            AccuracyTarget::Exact,
        )
        .expect("plans");
        assert_eq!(
            bound_family(&plan),
            SummaryFamilyType::ExactAggregate(ExactKind::Sum, ExactParams::Sum)
        );
        assert_eq!(operators(&plan), vec!["Fallback", "SummaryAgg", "Value"]);
    }

    #[test]
    fn sql_count_distinct_heads_with_hll_and_an_unranked_selection_lands_on_theta() {
        use asap_aware_mapping::{
            Replacement, ReplacementStrategy, ReplacementSubDAG, SketchAlgorithmStrategy,
            TargetSubDAG,
        };
        use asap_types::post_asap::SummaryExpr;

        const SQL: &str = "SELECT COUNT(DISTINCT service) FROM metrics";

        let plan = plan_sql(SQL, &metrics_catalog(), ACCURACY).expect("plans");
        assert_eq!(
            bound_family(&plan),
            SummaryFamilyType::Sketch(
                SketchKind::new(SketchAlgorithm::Theta, SketchParams::Theta { k: 1_000_002 }),
                GroupingStrategy::default()
            ),
            "`DefaultCostModel::rank_candidates` is the identity and every Sketch family costs              the same, so nothing prefers Theta over the Hll the candidate list heads with:              this pins which one today's unranked selection happens to leave selected"
        );

        let root = crate::sql::lower_sql_root(SQL, &metrics_catalog(), ACCURACY).expect("lowers");
        let aggregate = match root.as_ref() {
            QueryExpr::Project { child, .. } => Rc::new(child.as_ref().clone()),
            other => panic!("expected a Project over the aggregate, got {other:?}"),
        };
        let target = TargetSubDAG::new(&aggregate);
        let head = SketchAlgorithmStrategy::default_cost_model()
            .replacements(&target)
            .into_iter()
            .next()
            .expect("one candidate");
        let ReplacementSubDAG {
            replacement: Replacement::Summary(node),
            ..
        } = head
        else {
            panic!("the head candidate is not a summary");
        };
        let SummaryExpr::SummaryEstimate { summary_input, .. } = &node.expr else {
            panic!("expected a SummaryEstimate, got {:?}", node.expr);
        };
        let SummaryExpr::SummaryAgg { family, .. } = &summary_input.expr else {
            panic!("expected a SummaryAgg, got {:?}", summary_input.expr);
        };
        assert_eq!(
            family,
            &SummaryFamilyType::Sketch(
                SketchKind::new(SketchAlgorithm::Hll, SketchParams::Hll { precision: 14 }),
                GroupingStrategy::default()
            ),
            "the head of the candidate list is Hll, and the cost model leaves that order alone"
        );
    }

    #[test]
    fn json_round_trip_preserves_the_plan_id() {
        let plan = plan_promql("quantile(0.5, cpu_cores)", ACCURACY).expect("plans");
        let json = to_json(&plan).expect("encodes");
        let decoded = from_json(json.as_bytes()).expect("decodes");

        assert_eq!(decoded.id, plan.id);
        assert_eq!(decoded.order, plan.order);
        assert_eq!(decoded.dag, plan.dag);
        assert_eq!(to_json(&decoded).expect("re-encodes"), json);
    }

    #[test]
    fn canonical_json_is_two_space_pretty_lf() {
        let plan = plan_promql("sum(cpu_cores)", ACCURACY).expect("plans");
        let json = to_json(&plan).expect("encodes");
        assert!(json.starts_with("{\n  \"schema_version\": 2,\n  \"dag\": {\n"));
        assert!(!json.contains('\r'));
        assert!(!json.ends_with('\n'));
    }

    #[test]
    fn the_same_query_hashes_the_same_and_different_queries_do_not() {
        let once = plan_promql("quantile(0.5, cpu_cores)", ACCURACY).expect("plans");
        let twice = plan_promql("quantile(0.5, cpu_cores)", ACCURACY).expect("plans");
        let other = plan_promql("quantile(0.9, cpu_cores)", ACCURACY).expect("plans");

        assert_eq!(once.id, twice.id);
        assert_ne!(once.id, other.id);
    }

    #[test]
    fn a_workload_plans_every_root_and_keeps_its_names() {
        let planned = plan_promql_workload(
            &[
                ("bare", "cpu_cores"),
                ("total", "sum(cpu_cores)"),
                ("median", "quantile(0.5, cpu_cores)"),
            ],
            ACCURACY,
        )
        .expect("plans");

        let mut by_name: HashMap<&str, &Plan> = HashMap::new();
        for (name, plan) in &planned {
            by_name.insert(name.as_str(), plan);
        }
        assert_eq!(planned.len(), 3);
        assert_eq!(by_name["bare"].dag.nodes.len(), 1);
        assert_eq!(by_name["total"].dag.nodes.len(), 2);
        assert_eq!(by_name["median"].dag.nodes.len(), 3);

        // Planned together or alone, one root compiles to the same document.
        let alone = plan_promql("quantile(0.5, cpu_cores)", ACCURACY).expect("plans");
        assert_eq!(by_name["median"].id, alone.id);
    }

    #[test]
    fn a_cycle_is_rejected_by_the_ordering_pass() {
        let plan = plan_promql("quantile(0.5, cpu_cores)", ACCURACY).expect("plans");
        let mut dag = plan.dag.clone();
        // Close 0 -> 1 -> 2 into a cycle by pointing the last edge back at 0.
        let mut back = dag.edges[1].clone();
        back.producer = PostAsapNodeId(2);
        back.consumer = PostAsapNodeId(0);
        dag.edges.push(back);

        let err = topological_order(&dag).expect_err("a cycle has no topological order");
        assert!(matches!(err, EvalError::Validation(ref msg) if msg.contains("cycle")));
    }

    #[test]
    fn ordering_does_not_depend_on_node_id_order() {
        let plan = plan_promql("quantile(0.5, cpu_cores)", ACCURACY).expect("plans");
        let mut dag = plan.dag.clone();
        // Renumber so the producer has the largest id: 0 -> 2, 1 -> 1, 2 -> 0.
        let flip = |id: PostAsapNodeId| PostAsapNodeId(2 - id.0);
        for node in &mut dag.nodes {
            node.id = flip(node.id);
        }
        for edge in &mut dag.edges {
            edge.producer = flip(edge.producer);
            edge.consumer = flip(edge.consumer);
        }
        dag.root = flip(dag.root);

        let order = topological_order(&dag).expect("orders");
        assert_eq!(
            order,
            vec![PostAsapNodeId(2), PostAsapNodeId(1), PostAsapNodeId(0)],
            "order must follow the edges, not the ids"
        );
    }
}
