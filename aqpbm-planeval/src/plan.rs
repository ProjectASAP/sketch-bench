//! Getting a plan into main memory and preparing it for execution.
//!
//! The chain is PromQL -> pre-ASAP `QueryExpr` -> plan space -> global
//! selection -> materialized `SummaryNode` -> `ExecutableDag`, all in process.
//! `compile_executable_dag` is always called, even though nothing is written to
//! a wire: the `ExecutionDataState` (timing + primitive) assignment is computed
//! during compilation and does not exist on the `SummaryNode` tree, and the ten
//! checks in `validate()` only exist on `ExecutableDag`.
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
use asap_frontend_promql::lower_promql;
use asap_types::post_asap::{
    compile_executable_dag, ExecutableDag, PostAsapDagDocument, PostAsapNodeId,
};
use asap_types::types::AccuracyTarget;

use crate::types::{EvalError, PlanId};

/// A decoded, validated plan plus the order its nodes must run in.
#[derive(Debug, Clone)]
pub struct Plan {
    /// blake3 of the canonical JSON of the `PostAsapDagDocument`.
    pub id: PlanId,
    pub dag: ExecutableDag,
    /// Topological, producers first. Every node appears exactly once.
    pub order: Vec<PostAsapNodeId>,
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
        return Err(EvalError::Planning(format!(
            "planning {query:?} produced {} roots, expected 1",
            planned.len()
        )));
    }
    Ok(planned.remove(0).1)
}

/// Same, for a whole workload planned together, so cross-root CSE has a chance.
///
/// `queries` is `(name, query)`; the returned names are the ones passed in.
pub fn plan_promql_workload(
    queries: &[(&str, &str)],
    accuracy: AccuracyTarget,
) -> Result<Vec<(String, Plan)>, EvalError> {
    let mut roots = Vec::with_capacity(queries.len());
    for (name, query) in queries {
        let expr = lower_promql(query, accuracy.clone())
            .map_err(|err| EvalError::Planning(format!("lower {name:?} ({query:?}): {err:?}")))?;
        roots.push(((*name).to_string(), Rc::new(expr)));
    }

    // `search_workload` runs CSE over the roots and may hand back different
    // `Rc`s than the ones passed in, so materialization targets are read back
    // off the space rather than reused from `roots`.
    let space = search_workload(roots);
    let selection = space.global_selection(&DefaultCostModel);

    let mut planned = Vec::with_capacity(space.roots.len());
    for (name, root) in &space.roots {
        let materialized = match selection.materialize(root) {
            Ok(Some(node)) => node,
            Ok(None) => {
                return Err(EvalError::Planning(format!(
                    "root {name:?} was not discovered by the plan space"
                )))
            }
            Err(err) => {
                return Err(EvalError::Planning(format!(
                    "materialize {name:?}: {err:?}"
                )))
            }
        };
        let dag = compile_executable_dag(&materialized)
            .map_err(|err| EvalError::Planning(format!("compile {name:?}: {err:?}")))?;
        planned.push((name.clone(), from_dag(dag)?));
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
fn from_dag(dag: ExecutableDag) -> Result<Plan, EvalError> {
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
    })
}

/// Kahn's algorithm over `dag.edges`.
///
/// Ready nodes are drained smallest id first purely so the order is
/// deterministic across runs; correctness does not depend on it, and no edge
/// direction is inferred from the ids.
fn topological_order(dag: &ExecutableDag) -> Result<Vec<PostAsapNodeId>, EvalError> {
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
        from_dag(document.dag)
    }

    use super::*;
    use asap_types::post_asap::ExecutableOperator;

    const ACCURACY: AccuracyTarget = AccuracyTarget::Epsilon(0.01);

    fn operators(plan: &Plan) -> Vec<ExecutableOperator> {
        plan.order
            .iter()
            .map(|id| {
                plan.dag
                    .nodes
                    .iter()
                    .find(|node| node.id == *id)
                    .expect("ordered id names a node")
                    .operator
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
            vec![
                ExecutableOperator::Fallback,
                ExecutableOperator::SummaryAgg,
                ExecutableOperator::SummaryEstimate,
            ]
        );
        assert_eq!(plan.dag.root, PostAsapNodeId(2));
    }

    #[test]
    fn sum_plans_two_nodes() {
        let plan = plan_promql("sum(cpu_cores)", ACCURACY).expect("plans");
        assert_eq!(plan.dag.nodes.len(), 2);
        assert_eq!(plan.order.len(), 2);
        assert_eq!(
            operators(&plan),
            vec![ExecutableOperator::Fallback, ExecutableOperator::SummaryAgg]
        );
    }

    #[test]
    fn bare_metric_plans_one_node_and_no_edges() {
        let plan = plan_promql("cpu_cores", ACCURACY).expect("plans");
        assert_eq!(plan.dag.nodes.len(), 1);
        assert!(plan.dag.edges.is_empty());
        assert_eq!(ids(&plan), vec![0]);
        assert_eq!(operators(&plan), vec![ExecutableOperator::Fallback]);
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
        assert!(json.starts_with("{\n  \"schema_version\": 1,\n  \"dag\": {\n"));
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
