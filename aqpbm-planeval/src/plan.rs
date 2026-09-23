//! Getting a plan into main memory and preparing it for execution.
//!
//! The chain is SQL -> pre-ASAP `QueryExpr` -> plan space -> global
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

use asap_aware_mapping::{search_workload, DefaultCostModel};
use asap_frontend_sql::SqlCatalog;
use asap_types::post_asap::{
    compile_executable_dag_with_node_ids, ExecutableDag, ExecutableNodeIdentityMap,
    PostAsapDagDocument, PostAsapNodeId,
};
use asap_types::pre_asap::QueryExpr;
use asap_types::types::AccuracyTarget;

use crate::types::{EvalError, PlanId, PlanningStage};

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
}

impl Plan {
    /// The document envelope this plan's `id` is computed over.
    pub fn document(&self) -> PostAsapDagDocument {
        PostAsapDagDocument::new(self.dag.clone())
    }
}

pub fn plan_sql(
    sql: &str,
    catalog: &SqlCatalog,
    accuracy: AccuracyTarget,
) -> Result<Plan, EvalError> {
    plan_sql_root(sql, crate::sql::lower_sql_root(sql, catalog, accuracy)?)
}

pub fn plan_sql_root(sql: &str, root: Rc<QueryExpr>) -> Result<Plan, EvalError> {
    let mut planned = plan_roots(vec![(sql.to_string(), root)])?;
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

fn plan_roots(roots: Vec<(String, Rc<QueryExpr>)>) -> Result<Vec<(String, Plan)>, EvalError> {
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
        from_dag(document.dag, None, None)
    }

    use super::*;
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

    const MEDIAN: &str = "SELECT approx_percentile_cont(latency, 0.5) FROM metrics";

    fn plan_of(sql: &str) -> Plan {
        plan_sql(sql, &metrics_catalog(), ACCURACY).expect("plans")
    }

    #[test]
    fn quantile_plans_four_nodes_in_producer_order() {
        let plan = plan_of(MEDIAN);
        assert_eq!(plan.dag.nodes.len(), 4);
        assert_eq!(plan.dag.edges.len(), 3);
        assert_eq!(ids(&plan), vec![0, 1, 2, 3]);
        assert_eq!(
            operators(&plan),
            vec!["Fallback", "SummaryAgg", "SummaryEstimate", "Value"]
        );
        assert_eq!(plan.dag.root, PostAsapNodeId(3));
    }

    #[test]
    fn sum_plans_an_accumulator_and_its_projection() {
        let plan = plan_of("SELECT SUM(bytes) FROM metrics");
        assert_eq!(plan.dag.nodes.len(), 3);
        assert_eq!(plan.order.len(), 3);
        assert_eq!(operators(&plan), vec!["Fallback", "SummaryAgg", "Value"]);
    }

    #[test]
    fn a_bare_projection_plans_a_fallback_and_no_summary() {
        let plan = plan_of("SELECT latency FROM metrics");
        assert_eq!(plan.dag.nodes.len(), 2);
        assert_eq!(plan.dag.edges.len(), 1);
        assert_eq!(ids(&plan), vec![0, 1]);
        assert_eq!(operators(&plan), vec!["Fallback", "Value"]);
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
        let plan = plan_of(MEDIAN);
        let json = to_json(&plan).expect("encodes");
        let decoded = from_json(json.as_bytes()).expect("decodes");

        assert_eq!(decoded.id, plan.id);
        assert_eq!(decoded.order, plan.order);
        assert_eq!(decoded.dag, plan.dag);
        assert_eq!(to_json(&decoded).expect("re-encodes"), json);
    }

    #[test]
    fn canonical_json_is_two_space_pretty_lf() {
        let plan = plan_of("SELECT SUM(bytes) FROM metrics");
        let json = to_json(&plan).expect("encodes");
        assert!(json.starts_with("{\n  \"schema_version\": 2,\n  \"dag\": {\n"));
        assert!(!json.contains('\r'));
        assert!(!json.ends_with('\n'));
    }

    #[test]
    fn the_same_query_hashes_the_same_and_different_queries_do_not() {
        let once = plan_of(MEDIAN);
        let twice = plan_of(MEDIAN);
        let other = plan_of("SELECT approx_percentile_cont(latency, 0.9) FROM metrics");

        assert_eq!(once.id, twice.id);
        assert_ne!(once.id, other.id);
    }

    #[test]
    fn a_cycle_is_rejected_by_the_ordering_pass() {
        let plan = plan_of(MEDIAN);
        let mut dag = plan.dag.clone();
        // Close the chain into a cycle by pointing an edge from the root back at the source.
        let mut back = dag.edges[0].clone();
        back.producer = dag.root;
        back.consumer = plan.order[0];
        dag.edges.push(back);

        let err = topological_order(&dag).expect_err("a cycle has no topological order");
        assert!(matches!(err, EvalError::Validation(ref msg) if msg.contains("cycle")));
    }

    #[test]
    fn ordering_does_not_depend_on_node_id_order() {
        let plan = plan_of(MEDIAN);
        let mut dag = plan.dag.clone();
        // Renumber so the producer has the largest id and the root the smallest.
        let last = dag.nodes.len() as u32 - 1;
        let flip = |id: PostAsapNodeId| PostAsapNodeId(last - id.0);
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
            plan.order.iter().copied().map(flip).collect::<Vec<_>>(),
            "order must follow the edges, not the ids"
        );
    }
}
