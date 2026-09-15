//! Admission — everything this evaluator declines, decided before the first
//! row of data is read.
//!
//! `admit` turns "this plan is not supported" from a panic somewhere in the
//! middle of a corpus sweep into a typed, serializable [`Refusal`] carrying the
//! node it is about. Running it over the corpus produces a coverage table
//! rather than prose (PLAN.md §1.5).
//!
//! # Order, and why every refusal is collected
//!
//! The phases run edge roles → operators → payloads → column resolution →
//! parameter bounds, and **every** node is checked: returning the first refusal
//! would make a sweep report one problem per plan and hide the rest. Within a
//! single node the first fault wins — a `Binary` node refused on its edge role
//! must not also report a payload it was never going to run — so a plan with
//! *n* unsupported nodes yields exactly *n* refusals.
//!
//! # The window gate
//!
//! A `RequiresAlignedPanePhaseOrExactBoundaryResidual` edge is refused **only**
//! when producer and consumer are both `MAINTENANCE_SUMMARY`. The same stamp
//! lands on the ROWS→SUMMARY edge of every bound plan (it is on the 0→1 edge of
//! the three-node minimal document), where the run manifest's
//! `evaluation = "one_shot_whole_input"` discharges it: one pane covers the
//! whole file, so pane phase is vacuous. A blanket rule here would refuse 100%
//! of sketch plans (PLAN.md §1.6, last paragraph).
//!
//! # What is *not* decided here
//!
//! Binding a `SummaryFamilyType` to a concrete constructor, and the parameter
//! ranges that go with it (KLL `k`, HLL `precision`, CMS power-of-two widths —
//! PLAN.md §1.7), belong to the state-binding table. This module checks shape,
//! grouping, readout kind, column resolution, and the parameters carried on the
//! *readout* rather than on the family.

use std::collections::HashMap;

use asap_types::post_asap::{
    DataPrimitive, EdgeRole, ExecutableDag, ExecutableDagEdge, ExecutableDagNode,
    ExecutableOperatorPayload, ExecutionDataState, PostAsapNodeId, SketchAlgorithm, SketchCategory,
    SketchKind, SketchParams, SketchQuery, SummaryFamilyType, SummaryInputExpr, SummarySchema,
    SummaryUpdate, WindowEdgeCompatibility,
};
use asap_types::post_asap::{ExactOperation, GroupingStrategy, ValueOperation};
use asap_types::pre_asap::{ColumnRef, ProjectItem, QueryExpr, Reduction};

use crate::rows::{check_predicate, resolve_column, variant_name};
use crate::types::Refusal;

// ── The admitted plan ────────────────────────────────────────────────────────

/// A plan every node of which this evaluator can run, plus what admission
/// resolved along the way.
///
/// The decisions are kept because they were computed from the schemas anyway:
/// re-resolving a weight column at execution time would be a second chance to
/// resolve it differently.
#[derive(Debug, Clone)]
pub struct AdmittedPlan {
    dag: ExecutableDag,
    decisions: HashMap<PostAsapNodeId, NodeDecision>,
}

impl AdmittedPlan {
    pub fn dag(&self) -> &ExecutableDag {
        &self.dag
    }

    /// What admission resolved for one node. Every node of an `AdmittedPlan`
    /// has exactly one.
    pub fn decision(&self, node: PostAsapNodeId) -> Option<&NodeDecision> {
        self.decisions.get(&node)
    }

    pub fn decisions(&self) -> &HashMap<PostAsapNodeId, NodeDecision> {
        &self.decisions
    }
}

/// What admission resolved for one node.
#[derive(Debug, Clone, PartialEq)]
pub enum NodeDecision {
    /// `Fallback` carrying a bare `Scan` — the plan's row source.
    RowSource,
    /// `SummaryAgg` — the update's resolved column positions in the producer's
    /// output schema, and the grouping columns taken from `reduction`.
    Aggregate {
        item: Option<ResolvedInput>,
        weight: ResolvedInput,
        group_columns: Vec<usize>,
    },
    /// `SummaryEstimate` — the readout this node performs.
    Readout(SketchQuery),
    /// `Value` — an accepted read-time row operation.
    RowOperation,
}

/// A `SummaryInputExpr` with every column reference resolved to a position.
#[derive(Debug, Clone, PartialEq)]
pub enum ResolvedInput {
    Constant(f64),
    /// Position in the producer's output schema.
    Column(usize),
    Tuple(Vec<ResolvedInput>),
}

// ── Entry point ──────────────────────────────────────────────────────────────

/// Decide whether every node of `dag` can be run, before any data is read.
///
/// On refusal the whole list is returned, one refusal per refused node.
pub fn admit(dag: &ExecutableDag) -> Result<AdmittedPlan, Vec<Refusal>> {
    let nodes: HashMap<PostAsapNodeId, &ExecutableDagNode> =
        dag.nodes.iter().map(|node| (node.id, node)).collect();

    let mut refusals: Vec<Refusal> = Vec::new();
    // At most one refusal per node: later phases skip a node an earlier phase
    // already declined, so the count of refusals is the count of bad nodes.
    let mut refused: Vec<PostAsapNodeId> = Vec::new();
    let push = |refusals: &mut Vec<Refusal>,
                refused: &mut Vec<PostAsapNodeId>,
                node: PostAsapNodeId,
                refusal: Refusal| {
        if refused.contains(&node) {
            return;
        }
        refused.push(node);
        refusals.push(refusal);
    };

    // ── Phase 1: edge roles ──────────────────────────────────────────────────
    //
    // `Binary` and `CandidateTopK` are refused on the edge, before anything
    // looks at a payload (PLAN.md §1.6).
    for edge in &dag.edges {
        if let Some(refusal) = edge_role_refusal(edge) {
            push(&mut refusals, &mut refused, edge.consumer, refusal);
            continue;
        }
        if let Some(refusal) = window_refusal(edge, &nodes) {
            push(&mut refusals, &mut refused, edge.consumer, refusal);
        }
    }

    // ── Phase 2: operators ───────────────────────────────────────────────────
    for node in &dag.nodes {
        if let Some(refusal) = operator_refusal(node) {
            push(&mut refusals, &mut refused, node.id, refusal);
        }
    }

    // ── Phases 3-5: payloads, column resolution, parameter bounds ────────────
    //
    // Run per node so one node's payload fault does not hide the next node's.
    let mut decisions: HashMap<PostAsapNodeId, NodeDecision> = HashMap::new();
    for node in &dag.nodes {
        if refused.contains(&node.id) {
            continue;
        }
        match payload_decision(node, dag) {
            Ok(decision) => {
                decisions.insert(node.id, decision);
            }
            Err(refusal) => push(&mut refusals, &mut refused, node.id, refusal),
        }
    }

    if !refusals.is_empty() {
        return Err(refusals);
    }
    Ok(AdmittedPlan {
        dag: dag.clone(),
        decisions,
    })
}

// ── Phase 1: edge roles and the window gate ──────────────────────────────────

fn edge_role_refusal(edge: &ExecutableDagEdge) -> Option<Refusal> {
    let operator = match edge.role {
        EdgeRole::Input => return None,
        // Only `Binary` produces these, and it is refused in v0: nothing in the
        // corpus emits one.
        EdgeRole::Left | EdgeRole::Right => "Binary",
        // Only `CandidateTopK` produces these, and it is unreachable: the
        // accuracy gate always declines `CmsWithHeap`/`CountSketchWithHeap`.
        EdgeRole::CandidateMembership | EdgeRole::AuthoritativeValues => "CandidateTopK",
    };
    Some(Refusal::UnsupportedOperator {
        node: edge.consumer,
        operator: format!("{operator} (edge role {:?})", edge.role),
    })
}

fn window_refusal(
    edge: &ExecutableDagEdge,
    nodes: &HashMap<PostAsapNodeId, &ExecutableDagNode>,
) -> Option<Refusal> {
    if edge.window != WindowEdgeCompatibility::RequiresAlignedPanePhaseOrExactBoundaryResidual {
        return None;
    }
    let producer = nodes.get(&edge.producer)?.output_state;
    let consumer = nodes.get(&edge.consumer)?.output_state;
    // SUMMARY -> SUMMARY only. On a ROWS -> SUMMARY edge the same stamp is
    // discharged by the manifest's one-shot evaluation: one pane covers the
    // whole input, so there is no pane phase to align.
    if producer == ExecutionDataState::MAINTENANCE_SUMMARY
        && consumer == ExecutionDataState::MAINTENANCE_SUMMARY
    {
        return Some(Refusal::UnmetWindowObligation {
            node: edge.consumer,
            producer: edge.producer,
        });
    }
    None
}

// ── Phase 2: operators ───────────────────────────────────────────────────────

/// Refuse a node on its operator alone, before its payload is inspected.
///
/// Driven off the payload rather than `node.operator`: the two agreeing is one
/// of `ExecutableDag::validate`'s checks, and a payload is what would actually
/// be executed.
fn operator_refusal(node: &ExecutableDagNode) -> Option<Refusal> {
    let operator = match &node.payload {
        ExecutableOperatorPayload::Fallback { .. }
        | ExecutableOperatorPayload::Value { .. }
        | ExecutableOperatorPayload::SummaryAgg { .. }
        | ExecutableOperatorPayload::SummaryEstimate { .. } => return None,

        // No sketch in asap_sketchlib has an inverse. CMS could technically
        // carry negative counters, but its min estimator does not hold once it
        // does — a semantics problem, not a wiring problem. Permanent.
        ExecutableOperatorPayload::SummarySubtract => {
            return Some(Refusal::NoInverseOperation {
                node: node.id,
                operator: "SummarySubtract".to_string(),
            })
        }
        // Same missing capability, plus `key: ColumnRef` carries no weight, so
        // the magnitude of a deletion is undefined. Permanent.
        ExecutableOperatorPayload::SummaryDelete { .. } => {
            return Some(Refusal::NoInverseOperation {
                node: node.id,
                operator: "SummaryDelete".to_string(),
            })
        }

        // Theta/KMV join cardinality; asap_sketchlib has no Theta at all.
        ExecutableOperatorPayload::SummaryJoin { .. } => "SummaryJoin",
        ExecutableOperatorPayload::CandidateTopK { .. } => "CandidateTopK",
        ExecutableOperatorPayload::Binary { .. } => "Binary",
        ExecutableOperatorPayload::RelationalJoin { .. } => "RelationalJoin",
        ExecutableOperatorPayload::SummaryMerge => "SummaryMerge",
    };
    Some(Refusal::UnsupportedOperator {
        node: node.id,
        operator: operator.to_string(),
    })
}

// ── Phases 3-5: payload, column resolution, parameter bounds ─────────────────

fn payload_decision(
    node: &ExecutableDagNode,
    dag: &ExecutableDag,
) -> Result<NodeDecision, Refusal> {
    match &node.payload {
        ExecutableOperatorPayload::Fallback { expression } => {
            admit_fallback(node, expression).map(|()| NodeDecision::RowSource)
        }
        ExecutableOperatorPayload::SummaryAgg {
            family,
            input,
            reduction,
            grouping,
        } => admit_summary_agg(node, dag, family, input, reduction, grouping),
        ExecutableOperatorPayload::SummaryEstimate { query } => {
            admit_summary_estimate(node, dag, query).map(|()| NodeDecision::Readout(query.clone()))
        }
        // Admitted-but-unexecuted is strictly worse than refused: a read-time
        // `Filter` that is ignored reports the unfiltered readout as the
        // answer, and a `Value` between the row source and an aggregate makes
        // that aggregate vanish from the run with no error. v0 refuses until
        // `run` executes it.
        ExecutableOperatorPayload::Value { operation, .. } => {
            let _ = admit_value(node, dag, operation);
            Err(Refusal::UnsupportedValueOperation {
                node: node.id,
                detail: format!(
                    "{}: v0 admits no Value node, because nothing executes one yet",
                    debug_variant(operation)
                ),
            })
        }
        // Every other payload was refused in phase 2; a node that reaches here
        // carrying one means the phases have drifted apart.
        other => Err(Refusal::UnsupportedOperator {
            node: node.id,
            operator: format!("{:?}", other.operator()),
        }),
    }
}

// ── Fallback ─────────────────────────────────────────────────────────────────

/// A `Fallback` is the only leaf a plan can have, and the only payload accepted
/// there is a bare `Scan`.
///
/// The line is "a `Fallback` that is itself a *program* is not accepted". A
/// `Scan` with predicates is not a program — it is a statement about where rows
/// come from and which ones count. Moving this line is a specification change
/// that needs a fixture and a planner that actually emits the new shape, never
/// just "the variant exists".
fn admit_fallback(node: &ExecutableDagNode, expression: &QueryExpr) -> Result<(), Refusal> {
    let QueryExpr::Scan {
        predicates, schema, ..
    } = expression
    else {
        return Err(Refusal::FallbackIsAProgram {
            node: node.id,
            variant: variant_name(expression).to_string(),
        });
    };

    // Rows, at either timing. `MAINTENANCE_ROWS` and `READ_ROWS` differ only in
    // *when*, which is a scheduler's concern; the bare `cpu_cores` plan's only
    // node is a `READ_ROWS` Fallback and PLAN.md §1.9 admits it explicitly.
    if node.output_state.primitive != DataPrimitive::Raw {
        return Err(Refusal::UnsupportedOperator {
            node: node.id,
            operator: format!("Fallback producing {}", node.output_state),
        });
    }

    for predicate in predicates {
        // Positional `ColumnId`s index the scan's own binding schema.
        check_predicate(&predicate.0, schema.columns.len()).map_err(|fault| {
            Refusal::FallbackIsAProgram {
                node: node.id,
                variant: format!("Scan.predicates: {}", fault.detail()),
            }
        })?;
    }
    Ok(())
}

// ── SummaryAgg ───────────────────────────────────────────────────────────────

fn admit_summary_agg(
    node: &ExecutableDagNode,
    dag: &ExecutableDag,
    family: &SummaryFamilyType,
    input: &SummaryUpdate,
    reduction: &Reduction,
    grouping: &GroupingStrategy,
) -> Result<NodeDecision, Refusal> {
    let (producer, input_schema) = input_edge(node, dag)?;

    // Ask the state binding table before any data is read. Without this a
    // `Kll{k:65535}`, a `Theta`, or the planner's own `Cms{width:272,depth:5}`
    // is admitted clean, the CSV is opened, and the refusal arrives at row
    // zero — so a corpus sweep reports it as a run failure, and the coverage
    // table is missing exactly the SBT-1 rows that matter most.
    crate::handle::check_bindable(family, node.id)?;

    // The second legal child state — a `SummaryAgg` stacked on an exact
    // accumulator's `MAINTENANCE_SUMMARY` — is refused in v0.
    if producer.output_state.primitive == DataPrimitive::SummaryState {
        return Err(Refusal::UnsupportedOperator {
            node: node.id,
            operator: "SummaryAgg over summary state".to_string(),
        });
    }

    // The IR gives no mapping from `by` keys to a Hydra's internal layout.
    if let GroupingStrategy::SharedMultiSubpopulation { kind, .. } = grouping {
        return Err(Refusal::UnsupportedGrouping {
            node: node.id,
            detail: format!("SharedMultiSubpopulation({kind:?}) has no layout mapping in the IR"),
        });
    }

    // Grouping columns come from `reduction`, never from `grouping`:
    // `PerSubpopulationInstance` is the `Default` and lands on ungrouped
    // aggregates too, so reading key columns off it would be backwards.
    let group_columns = match reduction {
        Reduction::PerEntity => {
            return Err(Refusal::UnsupportedGrouping {
                node: node.id,
                detail: "Reduction::PerEntity has no entity concept over CSV rows".to_string(),
            })
        }
        Reduction::Reduce(keys) if keys.is_without() => {
            return Err(Refusal::UnsupportedGrouping {
                node: node.id,
                detail: "GroupKeys::without needs a closed label set, which a CSV leaf lacks"
                    .to_string(),
            })
        }
        Reduction::Reduce(keys) => {
            let mut columns = Vec::with_capacity(keys.keys().len());
            for key in keys.keys() {
                if *key >= input_schema.fields.len() {
                    return Err(Refusal::UnresolvableColumn {
                        node: node.id,
                        column: format!("by column {key}"),
                        detail: format!(
                            "the producer's schema has {} fields",
                            input_schema.fields.len()
                        ),
                    });
                }
                columns.push(*key);
            }
            columns
        }
    };

    // The family's own parameters are the binding table's business (§1.7); what
    // is checked here is that the committed `SketchKind` is not internally
    // inconsistent, which only a decoded document can be — `SketchKind::new`
    // panics rather than build one, but serde reaches the private fields.
    if let SummaryFamilyType::Sketch(kind, _) = family {
        if let Some(detail) = sketch_kind_inconsistency(kind) {
            return Err(Refusal::InconsistentSketchKind {
                node: node.id,
                detail,
            });
        }
    }

    let item = match &input.item {
        Some(item) => Some(resolve_input(node, item, input_schema)?),
        None => None,
    };
    let weight = resolve_input(node, &input.weight, input_schema)?;

    Ok(NodeDecision::Aggregate {
        item,
        weight,
        group_columns,
    })
}

/// Resolve one `SummaryInputExpr` against the producer's output schema.
fn resolve_input(
    node: &ExecutableDagNode,
    input: &SummaryInputExpr,
    schema: &SummarySchema,
) -> Result<ResolvedInput, Refusal> {
    match input {
        SummaryInputExpr::Constant(value) => Ok(ResolvedInput::Constant(*value)),
        SummaryInputExpr::Column(column) => match resolve_column(column, schema) {
            Some(position) => Ok(ResolvedInput::Column(position)),
            None => Err(Refusal::UnresolvableColumn {
                node: node.id,
                column: column_ref_name(column),
                detail: format!(
                    "the producer's schema has fields {:?}",
                    schema
                        .fields
                        .iter()
                        .map(|field| field.name.as_str())
                        .collect::<Vec<_>>()
                ),
            }),
        },
        SummaryInputExpr::Tuple(parts) => {
            let mut resolved = Vec::with_capacity(parts.len());
            for part in parts {
                resolved.push(resolve_input(node, part, schema)?);
            }
            Ok(ResolvedInput::Tuple(resolved))
        }
        // A PromQL label set is not a column of the CSV row schema; there is no
        // series identity to reconstruct from one flat file.
        SummaryInputExpr::EntityIdentity(_) => Err(Refusal::UnsupportedUpdate {
            node: node.id,
            detail: "EntityIdentity has no representation over flat rows".to_string(),
        }),
        // Needs the previous sample for the same series, which a one-shot scan
        // of an unordered file does not have.
        SummaryInputExpr::ResetAwareCounterDelta { .. } => Err(Refusal::UnsupportedUpdate {
            node: node.id,
            detail: "ResetAwareCounterDelta needs per-series carry-over state".to_string(),
        }),
    }
}

fn column_ref_name(column: &ColumnRef) -> String {
    match column {
        ColumnRef::Named(name) => name.clone(),
        ColumnRef::Qualified { table, name } => format!("{table}.{name}"),
        ColumnRef::SampleValue => "SampleValue".to_string(),
        ColumnRef::Wildcard => "Wildcard".to_string(),
    }
}

/// Re-derive a `SketchKind`'s classification and report any disagreement.
///
/// `SketchKind::new` is the one place `(algorithm, params)` is classified, and
/// it panics on a mismatch — so an inconsistent value can only arrive by
/// deserialization, which reaches the private fields directly. That is exactly
/// the case a cross-process evaluator has to handle.
fn sketch_kind_inconsistency(kind: &SketchKind) -> Option<String> {
    let algorithm = kind.algorithm();
    let params_match = matches!(
        (algorithm, kind.params()),
        (SketchAlgorithm::UnivMon, SketchParams::UnivMon { .. })
            | (SketchAlgorithm::Kll, SketchParams::Kll { .. })
            | (SketchAlgorithm::Cms, SketchParams::Cms { .. })
            | (SketchAlgorithm::Hll, SketchParams::Hll { .. })
            | (SketchAlgorithm::DDSketch, SketchParams::DDSketch { .. })
            | (
                SketchAlgorithm::CmsWithHeap,
                SketchParams::CmsWithHeap { .. }
            )
            | (SketchAlgorithm::Kmv, SketchParams::Kmv { .. })
            | (SketchAlgorithm::Theta, SketchParams::Theta { .. })
            | (
                SketchAlgorithm::CountSketch,
                SketchParams::CountSketch { .. }
            )
            | (
                SketchAlgorithm::CountSketchWithHeap,
                SketchParams::CountSketchWithHeap { .. }
            )
    );
    if !params_match {
        return Some(format!(
            "algorithm {:?} carries {:?} parameters",
            algorithm,
            kind.params()
        ));
    }
    let expected = match algorithm {
        SketchAlgorithm::UnivMon => SketchCategory::Universal,
        SketchAlgorithm::Kll | SketchAlgorithm::DDSketch => SketchCategory::Quantile,
        SketchAlgorithm::Hll | SketchAlgorithm::Theta | SketchAlgorithm::Kmv => {
            SketchCategory::Cardinality
        }
        SketchAlgorithm::Cms | SketchAlgorithm::CountSketch => SketchCategory::Frequency,
        SketchAlgorithm::CmsWithHeap | SketchAlgorithm::CountSketchWithHeap => SketchCategory::TopK,
    };
    if kind.category() != expected {
        return Some(format!(
            "algorithm {algorithm:?} is a {expected:?} sketch but the kind says {:?}",
            kind.category()
        ));
    }
    None
}

// ── SummaryEstimate ──────────────────────────────────────────────────────────

fn admit_summary_estimate(
    node: &ExecutableDagNode,
    dag: &ExecutableDag,
    query: &SketchQuery,
) -> Result<(), Refusal> {
    match query {
        SketchQuery::Quantile { q } => {
            // `q` is documented as (0, 1]; a value outside it is not a readout
            // any estimator can answer.
            if !(q.is_finite() && *q > 0.0 && *q <= 1.0) {
                return Err(Refusal::ParameterOutOfBounds {
                    node: node.id,
                    detail: format!("Quantile q = {q} is outside (0, 1]"),
                });
            }
        }
        // The bare bucket total, which is how an exact accumulator's state is
        // read. A *named* key paired with a value is a per-item point lookup:
        // the exact arm has only the weight column, which carries no item keys
        // to look one up by, so admitting it would produce an approximate
        // number with no ground truth to check it against.
        SketchQuery::PointCount { key, value }
            if matches!(key, ColumnRef::SampleValue | ColumnRef::Wildcard) && value.is_none() => {}
        // Cardinality, the frequency moments and TopK all need a family v0
        // does not bind (see `handle::check_bindable`).
        SketchQuery::Cardinality
        | SketchQuery::PointCount { .. }
        | SketchQuery::TopK { .. }
        | SketchQuery::FrequencyL2
        | SketchQuery::FrequencyEntropy => {
            return Err(Refusal::UnsupportedReadout {
                node: node.id,
                query: Box::new(query.clone()),
            })
        }
    }
    let producer = input_edge(node, dag)?.0;
    match &producer.payload {
        ExecutableOperatorPayload::SummaryAgg { family, .. } => {
            crate::handle::check_readout(family, query, node.id)
        }
        // Waving an unrecognized producer through would admit a readout that
        // `run` then finds no handle for, and admitted-but-unexecuted is
        // strictly worse than refused.
        other => Err(Refusal::UnsupportedOperator {
            node: node.id,
            operator: format!(
                "SummaryEstimate reading from a {:?}, which builds no summary",
                other.operator()
            ),
        }),
    }
}

// ── Value ────────────────────────────────────────────────────────────────────

fn admit_value(
    node: &ExecutableDagNode,
    dag: &ExecutableDag,
    operation: &ValueOperation,
) -> Result<(), Refusal> {
    // `ValueOperation` and `ExactOperation` are `#[non_exhaustive]`, so every
    // match on them here ends in a catch-all that refuses by name. A wildcard
    // that silently accepted (or silently ignored) a new upstream variant would
    // be worse than a compile error.
    match operation {
        ValueOperation::Exact(exact) => match exact {
            // `measures: Vec<AggIntent>` is a whole aggregation engine wearing
            // an enum variant as a coat.
            ExactOperation::Aggregate { .. } => Err(Refusal::UnsupportedValueOperation {
                node: node.id,
                detail: "Exact(Aggregate)".to_string(),
            }),
            other => Err(Refusal::UnsupportedValueOperation {
                node: node.id,
                detail: format!("Exact({})", debug_variant(other)),
            }),
        },
        ValueOperation::FinalizeExactAccumulator => Ok(()),
        ValueOperation::Project { cols, .. } => admit_project(node, cols),
        ValueOperation::Filter { pred } => {
            let columns = input_edge(node, dag)?.1.fields.len();
            check_predicate(&pred.0, columns).map_err(|fault| Refusal::UnsupportedValueOperation {
                node: node.id,
                detail: format!("Filter: {}", fault.detail()),
            })
        }
        // Both are row-local at read time: the corpus `topk` plan compiles to
        // exactly `Sort` + `Limit` over an exact accumulator's readout.
        ValueOperation::Sort { .. } => Ok(()),
        ValueOperation::Limit { .. } => Ok(()),
        ValueOperation::Extension { name } => Err(Refusal::UnregisteredExtension {
            node: node.id,
            name: name.clone(),
        }),
        other => Err(Refusal::UnsupportedValueOperation {
            node: node.id,
            detail: debug_variant(other),
        }),
    }
}

/// `ProjectItem.expr` is a full `QueryExpr`, so a projection can hold anything.
/// Only a rename or a constant is accepted; everything else is refused by the
/// variant's own name.
fn admit_project(node: &ExecutableDagNode, cols: &[ProjectItem]) -> Result<(), Refusal> {
    for (index, item) in cols.iter().enumerate() {
        match &item.expr {
            QueryExpr::Column(_) | QueryExpr::Literal(_) => {}
            other => {
                return Err(Refusal::UnsupportedValueOperation {
                    node: node.id,
                    detail: format!("Project col {index} is a {}", variant_name(other)),
                })
            }
        }
    }
    Ok(())
}

// ── Shared helpers ───────────────────────────────────────────────────────────

/// The node feeding this one on its `Input` edge, and the schema flowing there.
fn input_edge<'a>(
    node: &ExecutableDagNode,
    dag: &'a ExecutableDag,
) -> Result<(&'a ExecutableDagNode, &'a SummarySchema), Refusal> {
    let edge = dag
        .edges
        .iter()
        .find(|edge| edge.consumer == node.id && edge.role == EdgeRole::Input)
        .ok_or_else(|| Refusal::UnsupportedUpdate {
            node: node.id,
            detail: format!("{:?} has no Input edge", node.operator),
        })?;
    let producer = dag
        .nodes
        .iter()
        .find(|candidate| candidate.id == edge.producer)
        .ok_or_else(|| Refusal::UnsupportedUpdate {
            node: node.id,
            detail: format!("Input edge names {:?}, which is not a node", edge.producer),
        })?;
    // `edge.intermediate_schema == producer.output_schema` is one of
    // `validate()`'s checks; the producer's own copy is read here so admission
    // does not depend on having been handed a validated document.
    Ok((producer, &producer.output_schema))
}

/// The variant name of a `#[non_exhaustive]` enum value, via its `Debug`.
///
/// The only way to name a variant this crate cannot match on: an upstream
/// addition compiles here (it must, or the crate would not build against a new
/// `asap-types`) and lands in a catch-all arm, which still has to say which
/// variant it declined.
fn debug_variant<T: std::fmt::Debug>(value: &T) -> String {
    let rendered = format!("{value:?}");
    rendered
        .split(|c: char| !(c.is_alphanumeric() || c == '_'))
        .find(|piece| !piece.is_empty())
        .unwrap_or("<unnameable>")
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::rc::Rc;

    use asap_types::post_asap::{
        ExactKind, ExactParams, ExecutableOperator, ExecutionTiming, GroupingEdgeCompatibility,
        SummaryField,
    };
    use asap_types::pre_asap::{
        CompareOpKind, DataType, GroupKeys, Predicate, ScalarValue, Source,
    };

    use crate::rows::tests::{node_of, plan};

    // ── The planner's own plans ──────────────────────────────────────────────

    #[test]
    fn the_three_node_quantile_plan_is_admitted_with_zero_refusals() {
        let dag = plan("quantile(0.5, cpu_cores)");
        assert_eq!(dag.nodes.len(), 3);

        let admitted = match admit(&dag) {
            Ok(admitted) => admitted,
            Err(refusals) => panic!("refused: {refusals:?}"),
        };

        assert_eq!(admitted.decisions().len(), 3);
        let fallback = node_of(&dag, ExecutableOperator::Fallback).id;
        let agg = node_of(&dag, ExecutableOperator::SummaryAgg).id;
        let estimate = node_of(&dag, ExecutableOperator::SummaryEstimate).id;

        assert_eq!(admitted.decision(fallback), Some(&NodeDecision::RowSource));
        assert_eq!(
            admitted.decision(agg),
            // `weight: Column(SampleValue)` resolved to the `value` field, and
            // `Reduce([])` is a genuine global reduction, not "no grouping".
            Some(&NodeDecision::Aggregate {
                item: None,
                weight: ResolvedInput::Column(1),
                group_columns: Vec::new(),
            })
        );
        assert_eq!(
            admitted.decision(estimate),
            Some(&NodeDecision::Readout(SketchQuery::Quantile { q: 0.5 }))
        );
    }

    #[test]
    fn the_rows_to_summary_window_stamp_does_not_refuse_a_sketch_plan() {
        let dag = plan("quantile(0.5, cpu_cores)");
        // The stamp really is on the 0 -> 1 edge; if it ever stops being, this
        // test stops proving anything and should be revisited.
        assert!(dag.edges.iter().any(|edge| {
            edge.window == WindowEdgeCompatibility::RequiresAlignedPanePhaseOrExactBoundaryResidual
        }));
        assert!(
            admit(&dag).is_ok(),
            "a blanket window gate refuses every sketch plan"
        );
    }

    #[test]
    fn the_exact_accumulator_and_fallback_only_plans_are_admitted() {
        for query in ["cpu_cores", "sum(cpu_cores)"] {
            let dag = plan(query);
            if let Err(refusals) = admit(&dag) {
                panic!("{query} was refused: {refusals:?}");
            }
        }
    }

    // ── Permanent refusals ───────────────────────────────────────────────────

    #[test]
    fn a_summary_subtract_node_is_refused_with_no_inverse_operation() {
        let mut dag = plan("quantile(0.5, cpu_cores)");
        let agg = node_of(&dag, ExecutableOperator::SummaryAgg).id;
        // Hand-built: nothing in the planner emits one, which is the point.
        let victim = dag
            .nodes
            .iter_mut()
            .find(|node| node.id == agg)
            .expect("the SummaryAgg node");
        victim.operator = ExecutableOperator::SummarySubtract;
        victim.payload = ExecutableOperatorPayload::SummarySubtract;

        let estimate = node_of(&dag, ExecutableOperator::SummaryEstimate).id;
        let refusals = admit(&dag).expect_err("is refused");
        assert_eq!(
            refusals,
            vec![
                Refusal::NoInverseOperation {
                    node: agg,
                    operator: "SummarySubtract".to_string(),
                },
                // The readout downstream of it is refused too: its producer
                // builds no summary, so nothing would execute it.
                Refusal::UnsupportedOperator {
                    node: estimate,
                    operator: "SummaryEstimate reading from a SummarySubtract, which builds no \
                               summary"
                        .to_string(),
                },
            ]
        );
    }

    #[test]
    fn a_summary_delete_node_is_refused_with_no_inverse_operation() {
        let mut dag = plan("quantile(0.5, cpu_cores)");
        let agg = node_of(&dag, ExecutableOperator::SummaryAgg).id;
        let victim = dag
            .nodes
            .iter_mut()
            .find(|node| node.id == agg)
            .expect("the SummaryAgg node");
        victim.operator = ExecutableOperator::SummaryDelete;
        victim.payload = ExecutableOperatorPayload::SummaryDelete {
            key: ColumnRef::Named("cluster".into()),
        };

        let refusals = admit(&dag).expect_err("is refused");
        assert!(
            matches!(
                refusals.as_slice(),
                [
                    Refusal::NoInverseOperation { operator, .. },
                    Refusal::UnsupportedOperator { .. },
                ] if operator == "SummaryDelete"
            ),
            "{refusals:?}"
        );
    }

    #[test]
    fn a_summary_merge_node_is_refused_in_v0() {
        let mut dag = plan("quantile(0.5, cpu_cores)");
        let agg = node_of(&dag, ExecutableOperator::SummaryAgg).id;
        let victim = dag
            .nodes
            .iter_mut()
            .find(|node| node.id == agg)
            .expect("the SummaryAgg node");
        victim.operator = ExecutableOperator::SummaryMerge;
        victim.payload = ExecutableOperatorPayload::SummaryMerge;

        let refusals = admit(&dag).expect_err("is refused");
        assert!(
            matches!(
                refusals.as_slice(),
                [
                    Refusal::UnsupportedOperator { operator, .. },
                    Refusal::UnsupportedOperator { .. },
                ] if operator == "SummaryMerge"
            ),
            "{refusals:?}"
        );
    }

    /// The pairing gate used to run only when the producer was a `SummaryAgg`
    /// and to return `Ok` otherwise. `Fallback -> SummaryEstimate` is the
    /// shape that survived: admitted unchecked, and then `run` finds no handle
    /// for a `Fallback` and the readout produces nothing at all.
    #[test]
    fn a_readout_whose_producer_builds_no_summary_is_refused_rather_than_waved_through() {
        let mut dag = plan("quantile(0.5, cpu_cores)");
        let fallback = node_of(&dag, ExecutableOperator::Fallback).id;
        let estimate = node_of(&dag, ExecutableOperator::SummaryEstimate).id;
        let edge = dag
            .edges
            .iter_mut()
            .find(|edge| edge.consumer == estimate)
            .expect("the readout's input edge");
        edge.producer = fallback;

        let refusals = admit(&dag).expect_err("is refused");
        assert!(
            refusals
                .iter()
                .any(|refusal| matches!(refusal, Refusal::UnsupportedOperator { node, .. } if *node == estimate)),
            "{refusals:?}"
        );
    }

    // ── Fallback ─────────────────────────────────────────────────────────────

    #[test]
    fn a_fallback_that_is_a_program_names_the_variant() {
        let mut dag = plan("quantile(0.5, cpu_cores)");
        let fallback = node_of(&dag, ExecutableOperator::Fallback).id;
        let victim = dag
            .nodes
            .iter_mut()
            .find(|node| node.id == fallback)
            .expect("the Fallback node");
        let scan = match &victim.payload {
            ExecutableOperatorPayload::Fallback { expression } => expression.clone(),
            other => panic!("{other:?}"),
        };
        victim.payload = ExecutableOperatorPayload::Fallback {
            expression: QueryExpr::Limit {
                n: 10,
                offset: 0,
                child: Rc::new(scan),
            },
        };

        let refusals = admit(&dag).expect_err("is refused");
        assert_eq!(
            refusals,
            vec![Refusal::FallbackIsAProgram {
                node: fallback,
                variant: "Limit".to_string(),
            }]
        );
    }

    #[test]
    fn a_fallback_predicate_outside_the_subset_is_refused() {
        let mut dag = plan("quantile(0.5, cpu_cores)");
        let fallback = node_of(&dag, ExecutableOperator::Fallback).id;
        let victim = dag
            .nodes
            .iter_mut()
            .find(|node| node.id == fallback)
            .expect("the Fallback node");
        let ExecutableOperatorPayload::Fallback { expression } = &mut victim.payload else {
            panic!("the Fallback node lost its payload");
        };
        let QueryExpr::Scan { predicates, .. } = expression else {
            panic!("the Fallback node lost its Scan");
        };
        predicates.push(Predicate(Rc::new(QueryExpr::FunctionCall {
            name: "lower".into(),
            args: vec![QueryExpr::Column(1)],
        })));

        let refusals = admit(&dag).expect_err("is refused");
        assert!(
            matches!(refusals.as_slice(), [Refusal::FallbackIsAProgram { variant, .. }] if variant.contains("FunctionCall")),
            "{refusals:?}"
        );
    }

    #[test]
    fn a_label_matcher_on_the_fallback_is_admitted() {
        // A `Scan` with predicates says where rows come from, not what to
        // compute: admitted, deliberately.
        let mut dag = plan("quantile(0.5, cpu_cores)");
        let fallback = node_of(&dag, ExecutableOperator::Fallback).id;
        let victim = dag
            .nodes
            .iter_mut()
            .find(|node| node.id == fallback)
            .expect("the Fallback node");
        let ExecutableOperatorPayload::Fallback { expression } = &mut victim.payload else {
            panic!("the Fallback node lost its payload");
        };
        let QueryExpr::Scan { predicates, .. } = expression else {
            panic!("the Fallback node lost its Scan");
        };
        predicates.push(Predicate(Rc::new(QueryExpr::Compare {
            left: Rc::new(QueryExpr::Column(1)),
            op: CompareOpKind::Gt,
            right: Rc::new(QueryExpr::Literal(ScalarValue::Float64(0.0))),
        })));

        assert!(admit(&dag).is_ok());
    }

    // ── SummaryAgg ───────────────────────────────────────────────────────────

    #[test]
    fn an_unresolvable_weight_column_names_the_column() {
        let mut dag = plan("quantile(0.5, cpu_cores)");
        let agg = node_of(&dag, ExecutableOperator::SummaryAgg).id;
        let victim = dag
            .nodes
            .iter_mut()
            .find(|node| node.id == agg)
            .expect("the SummaryAgg node");
        let ExecutableOperatorPayload::SummaryAgg { input, .. } = &mut victim.payload else {
            panic!("the SummaryAgg node lost its payload");
        };
        input.weight = SummaryInputExpr::Column(ColumnRef::Named("absent".into()));

        let refusals = admit(&dag).expect_err("is refused");
        assert!(
            matches!(refusals.as_slice(), [Refusal::UnresolvableColumn { column, .. }] if column == "absent"),
            "{refusals:?}"
        );
    }

    #[test]
    fn a_wildcard_weight_is_refused() {
        let mut dag = plan("quantile(0.5, cpu_cores)");
        let agg = node_of(&dag, ExecutableOperator::SummaryAgg).id;
        let victim = dag
            .nodes
            .iter_mut()
            .find(|node| node.id == agg)
            .expect("the SummaryAgg node");
        let ExecutableOperatorPayload::SummaryAgg { input, .. } = &mut victim.payload else {
            panic!("the SummaryAgg node lost its payload");
        };
        input.weight = SummaryInputExpr::Column(ColumnRef::Wildcard);

        let refusals = admit(&dag).expect_err("is refused");
        assert!(
            matches!(refusals.as_slice(), [Refusal::UnresolvableColumn { column, .. }] if column == "Wildcard"),
            "{refusals:?}"
        );
    }

    #[test]
    fn a_shared_multi_subpopulation_grouping_is_refused() {
        use asap_types::post_asap::{default_hydra_params, HydraKind};

        let params = default_hydra_params(
            HydraKind::HydraCms,
            &SketchParams::Cms {
                width: 256,
                depth: 5,
            },
        )
        .expect("HydraCms takes Cms params");

        let mut dag = plan("quantile(0.5, cpu_cores)");
        let agg = node_of(&dag, ExecutableOperator::SummaryAgg).id;
        let victim = dag
            .nodes
            .iter_mut()
            .find(|node| node.id == agg)
            .expect("the SummaryAgg node");
        let ExecutableOperatorPayload::SummaryAgg { grouping, .. } = &mut victim.payload else {
            panic!("the SummaryAgg node lost its payload");
        };
        *grouping = GroupingStrategy::SharedMultiSubpopulation {
            kind: HydraKind::HydraCms,
            params,
        };

        let refusals = admit(&dag).expect_err("is refused");
        assert!(
            matches!(refusals.as_slice(), [Refusal::UnsupportedGrouping { .. }]),
            "{refusals:?}"
        );
    }

    #[test]
    fn a_without_grouping_and_a_per_entity_reduction_are_both_refused() {
        for reduction in [
            Reduction::Reduce(GroupKeys::without(vec![1])),
            Reduction::PerEntity,
        ] {
            let mut dag = plan("quantile(0.5, cpu_cores)");
            let agg = node_of(&dag, ExecutableOperator::SummaryAgg).id;
            let victim = dag
                .nodes
                .iter_mut()
                .find(|node| node.id == agg)
                .expect("the SummaryAgg node");
            let ExecutableOperatorPayload::SummaryAgg {
                reduction: slot, ..
            } = &mut victim.payload
            else {
                panic!("the SummaryAgg node lost its payload");
            };
            *slot = reduction.clone();

            let refusals = admit(&dag).expect_err("is refused");
            assert!(
                matches!(refusals.as_slice(), [Refusal::UnsupportedGrouping { .. }]),
                "{reduction:?}: {refusals:?}"
            );
        }
    }

    #[test]
    fn a_by_column_past_the_producers_schema_is_refused() {
        let mut dag = plan("quantile(0.5, cpu_cores)");
        let agg = node_of(&dag, ExecutableOperator::SummaryAgg).id;
        let victim = dag
            .nodes
            .iter_mut()
            .find(|node| node.id == agg)
            .expect("the SummaryAgg node");
        let ExecutableOperatorPayload::SummaryAgg { reduction, .. } = &mut victim.payload else {
            panic!("the SummaryAgg node lost its payload");
        };
        *reduction = Reduction::by(vec![9]);

        let refusals = admit(&dag).expect_err("is refused");
        assert!(
            matches!(refusals.as_slice(), [Refusal::UnresolvableColumn { column, .. }] if column.contains('9')),
            "{refusals:?}"
        );
    }

    #[test]
    fn an_entity_identity_update_is_refused() {
        use asap_types::post_asap::EntityIdentity;

        let mut dag = plan("quantile(0.5, cpu_cores)");
        let agg = node_of(&dag, ExecutableOperator::SummaryAgg).id;
        let victim = dag
            .nodes
            .iter_mut()
            .find(|node| node.id == agg)
            .expect("the SummaryAgg node");
        let ExecutableOperatorPayload::SummaryAgg { input, .. } = &mut victim.payload else {
            panic!("the SummaryAgg node lost its payload");
        };
        input.item = Some(SummaryInputExpr::EntityIdentity(
            EntityIdentity::PromqlLabelSet { excluding: vec![] },
        ));

        let refusals = admit(&dag).expect_err("is refused");
        assert!(
            matches!(refusals.as_slice(), [Refusal::UnsupportedUpdate { .. }]),
            "{refusals:?}"
        );
    }

    #[test]
    fn an_internally_inconsistent_sketch_kind_is_refused() {
        // `SketchKind::new` panics rather than build this, so it is reached the
        // only way it can be in production: through serde.
        let kind: SketchKind = serde_json::from_str(
            r#"{"category":"Cardinality","algorithm":"Kll","params":{"Kll":{"k":269}}}"#,
        )
        .expect("decodes");

        let mut dag = plan("quantile(0.5, cpu_cores)");
        let agg = node_of(&dag, ExecutableOperator::SummaryAgg).id;
        let victim = dag
            .nodes
            .iter_mut()
            .find(|node| node.id == agg)
            .expect("the SummaryAgg node");
        let ExecutableOperatorPayload::SummaryAgg { family, .. } = &mut victim.payload else {
            panic!("the SummaryAgg node lost its payload");
        };
        *family = SummaryFamilyType::Sketch(kind, GroupingStrategy::PerSubpopulationInstance);

        let refusals = admit(&dag).expect_err("is refused");
        assert!(
            matches!(
                refusals.as_slice(),
                [Refusal::InconsistentSketchKind { .. }]
            ),
            "{refusals:?}"
        );
    }

    // ── SummaryEstimate ──────────────────────────────────────────────────────

    #[test]
    fn unsupported_readouts_are_refused_and_supported_ones_are_not() {
        let refused = [
            SketchQuery::TopK { k: 10 },
            SketchQuery::FrequencyL2,
            SketchQuery::FrequencyEntropy,
            // Needs a family v0 does not bind.
            SketchQuery::Cardinality,
            // A per-item lookup: the exact arm has only the weight column, so
            // there would be no ground truth to check the estimate against.
            SketchQuery::PointCount {
                key: ColumnRef::Named("service".into()),
                value: Some("api".into()),
            },
        ];
        let accepted = [
            (SketchQuery::Quantile { q: 0.99 }, None),
            (
                SketchQuery::PointCount {
                    key: ColumnRef::SampleValue,
                    value: None,
                },
                Some(SummaryFamilyType::ExactAggregate(
                    ExactKind::Sum,
                    ExactParams::Sum,
                )),
            ),
        ];

        for query in refused {
            let dag = with_readout(&query);
            let refusals = admit(&dag).expect_err("is refused");
            assert!(
                matches!(refusals.as_slice(), [Refusal::UnsupportedReadout { .. }]),
                "{query:?}: {refusals:?}"
            );
        }
        for (query, family) in accepted {
            let dag = match family {
                Some(family) => with_family(with_readout(&query), family),
                None => with_readout(&query),
            };
            assert!(admit(&dag).is_ok(), "{query:?} should be admitted");
        }
    }

    #[test]
    fn a_family_and_a_readout_that_name_different_questions_are_refused() {
        let mispairings = [
            (
                SummaryFamilyType::ExactAggregate(ExactKind::Count, ExactParams::Count),
                SketchQuery::PointCount {
                    key: ColumnRef::SampleValue,
                    value: None,
                },
            ),
            (
                SummaryFamilyType::ExactAggregate(ExactKind::Sum, ExactParams::Sum),
                SketchQuery::PointCount {
                    key: ColumnRef::Wildcard,
                    value: None,
                },
            ),
            (
                SummaryFamilyType::Sketch(
                    SketchKind::new(SketchAlgorithm::Kll, SketchParams::Kll { k: 269 }),
                    GroupingStrategy::PerSubpopulationInstance,
                ),
                SketchQuery::PointCount {
                    key: ColumnRef::SampleValue,
                    value: None,
                },
            ),
            (
                SummaryFamilyType::ExactAggregate(ExactKind::Sum, ExactParams::Sum),
                SketchQuery::Quantile { q: 0.5 },
            ),
        ];

        for (family, query) in mispairings {
            let dag = with_family(with_readout(&query), family.clone());
            let refusals = admit(&dag).expect_err("is refused");
            assert!(
                matches!(
                    refusals.as_slice(),
                    [Refusal::FamilyDoesNotAnswerReadout { .. }]
                ),
                "{family:?} / {query:?}: {refusals:?}"
            );
        }
    }

    #[test]
    fn the_pairings_that_do_name_the_same_question_are_admitted() {
        let pairings = [
            (
                SummaryFamilyType::ExactAggregate(ExactKind::Count, ExactParams::Count),
                SketchQuery::PointCount {
                    key: ColumnRef::Wildcard,
                    value: None,
                },
            ),
            (
                SummaryFamilyType::ExactAggregate(ExactKind::Max, ExactParams::Max),
                SketchQuery::Quantile { q: 1.0 },
            ),
            (
                SummaryFamilyType::Sketch(
                    SketchKind::new(
                        SketchAlgorithm::DDSketch,
                        SketchParams::DDSketch { alpha: 0.01 },
                    ),
                    GroupingStrategy::PerSubpopulationInstance,
                ),
                SketchQuery::Quantile { q: 0.5 },
            ),
        ];

        for (family, query) in pairings {
            let dag = with_family(with_readout(&query), family.clone());
            assert!(
                admit(&dag).is_ok(),
                "{family:?} / {query:?} should be admitted"
            );
        }
    }

    fn with_family(mut dag: ExecutableDag, family: SummaryFamilyType) -> ExecutableDag {
        let agg = node_of(&dag, ExecutableOperator::SummaryAgg).id;
        let victim = dag
            .nodes
            .iter_mut()
            .find(|node| node.id == agg)
            .expect("the SummaryAgg node");
        let ExecutableOperatorPayload::SummaryAgg { family: slot, .. } = &mut victim.payload else {
            panic!("the SummaryAgg node lost its payload");
        };
        *slot = family;
        dag
    }

    #[test]
    fn a_quantile_outside_zero_to_one_is_a_parameter_bound_refusal() {
        for q in [0.0, 1.5, -0.1, f64::NAN] {
            let dag = with_readout(&SketchQuery::Quantile { q });
            let refusals = admit(&dag).expect_err("is refused");
            assert!(
                matches!(refusals.as_slice(), [Refusal::ParameterOutOfBounds { .. }]),
                "q = {q}: {refusals:?}"
            );
        }
    }

    fn with_readout(query: &SketchQuery) -> ExecutableDag {
        let mut dag = plan("quantile(0.5, cpu_cores)");
        let estimate = node_of(&dag, ExecutableOperator::SummaryEstimate).id;
        let victim = dag
            .nodes
            .iter_mut()
            .find(|node| node.id == estimate)
            .expect("the SummaryEstimate node");
        victim.payload = ExecutableOperatorPayload::SummaryEstimate {
            query: query.clone(),
        };
        dag
    }

    // ── Value ────────────────────────────────────────────────────────────────

    /// The 3-node plan with a `Value` node grafted above its readout.
    fn with_value(operation: &ValueOperation) -> ExecutableDag {
        let mut dag = plan("quantile(0.5, cpu_cores)");
        let estimate = node_of(&dag, ExecutableOperator::SummaryEstimate).clone();
        let id = PostAsapNodeId(dag.nodes.len() as u32);
        dag.nodes.push(ExecutableDagNode {
            id,
            operator: ExecutableOperator::Value,
            payload: ExecutableOperatorPayload::Value {
                operation: operation.clone(),
                timing: ExecutionTiming::ReadTime,
            },
            output_state: estimate.output_state,
            output_schema: estimate.output_schema.clone(),
            guarantee: estimate.guarantee.clone(),
        });
        dag.edges.push(ExecutableDagEdge {
            producer: estimate.id,
            consumer: id,
            role: EdgeRole::Input,
            intermediate_schema: estimate.output_schema.clone(),
            data_state: estimate.output_state,
            grouping: GroupingEdgeCompatibility::NotApplicable,
            window: WindowEdgeCompatibility::NotApplicable,
        });
        dag.root = id;
        dag
    }

    #[test]
    fn every_value_node_is_refused_in_v0_and_says_why() {
        // Admitting a Value node without executing it is the worse failure:
        // an ignored read-time Filter reports the unfiltered readout as the
        // answer. These four are the ones §1.6 intends to support later.
        for operation in [
            ValueOperation::FinalizeExactAccumulator,
            ValueOperation::Limit { n: 10, offset: 0 },
            ValueOperation::Project {
                cols: vec![ProjectItem {
                    alias: None,
                    expr: QueryExpr::Column(0),
                }],
                qualifier: None,
            },
            ValueOperation::Filter {
                pred: Predicate(Rc::new(QueryExpr::Compare {
                    left: Rc::new(QueryExpr::Column(0)),
                    op: CompareOpKind::Gt,
                    right: Rc::new(QueryExpr::Literal(ScalarValue::Float64(0.0))),
                })),
            },
        ] {
            let refusals = admit(&with_value(&operation)).expect_err("v0 refuses every Value");
            assert!(
                matches!(
                    refusals.as_slice(),
                    [Refusal::UnsupportedValueOperation { detail, .. }]
                        if detail.contains("v0 admits no Value node")
                ),
                "{operation:?}: {refusals:?}"
            );
        }
    }

    #[test]
    fn a_summary_to_summary_window_obligation_is_refused() {
        // Both endpoints MAINTENANCE_SUMMARY: the only shape the gate catches.
        let mut dag = plan("quantile(0.5, cpu_cores)");
        let agg = node_of(&dag, ExecutableOperator::SummaryAgg);
        let agg_id = agg.id;
        let agg_schema = agg.output_schema.clone();
        let estimate = node_of(&dag, ExecutableOperator::SummaryEstimate).id;

        dag.nodes
            .iter_mut()
            .find(|node| node.id == estimate)
            .expect("the SummaryEstimate node")
            .output_state = ExecutionDataState::MAINTENANCE_SUMMARY;
        for edge in &mut dag.edges {
            if edge.producer == agg_id && edge.consumer == estimate {
                edge.window =
                    WindowEdgeCompatibility::RequiresAlignedPanePhaseOrExactBoundaryResidual;
                edge.intermediate_schema = agg_schema.clone();
                edge.data_state = ExecutionDataState::MAINTENANCE_SUMMARY;
                edge.grouping = GroupingEdgeCompatibility::NotApplicable;
            }
        }

        let refusals = admit(&dag).expect_err("is refused");
        assert_eq!(
            refusals,
            vec![Refusal::UnmetWindowObligation {
                node: estimate,
                producer: agg_id,
            }]
        );
    }

    // ── Collecting, not short-circuiting ─────────────────────────────────────

    #[test]
    fn every_bad_node_is_reported_not_just_the_first() {
        let mut dag = plan("quantile(0.5, cpu_cores)");
        let fallback = node_of(&dag, ExecutableOperator::Fallback).id;
        let agg = node_of(&dag, ExecutableOperator::SummaryAgg).id;
        let estimate = node_of(&dag, ExecutableOperator::SummaryEstimate).id;

        for node in &mut dag.nodes {
            if node.id == fallback {
                node.payload = ExecutableOperatorPayload::Fallback {
                    expression: QueryExpr::EvalTimestamp,
                };
            } else if node.id == agg {
                node.operator = ExecutableOperator::SummarySubtract;
                node.payload = ExecutableOperatorPayload::SummarySubtract;
            } else if node.id == estimate {
                node.payload = ExecutableOperatorPayload::SummaryEstimate {
                    query: SketchQuery::TopK { k: 5 },
                };
            }
        }

        let refusals = admit(&dag).expect_err("is refused");
        assert_eq!(refusals.len(), 3, "{refusals:?}");
        assert!(refusals.iter().any(|refusal| matches!(
            refusal,
            Refusal::FallbackIsAProgram { variant, .. } if variant == "EvalTimestamp"
        )));
        assert!(refusals
            .iter()
            .any(|refusal| matches!(refusal, Refusal::NoInverseOperation { .. })));
        assert!(refusals
            .iter()
            .any(|refusal| matches!(refusal, Refusal::UnsupportedReadout { .. })));
    }

    #[test]
    fn one_bad_node_yields_exactly_one_refusal() {
        // A `Binary` node is catchable on both its edge role and its operator;
        // it must be reported once.
        let mut dag = plan("quantile(0.5, cpu_cores)");
        let estimate = node_of(&dag, ExecutableOperator::SummaryEstimate).id;
        for edge in &mut dag.edges {
            if edge.consumer == estimate {
                edge.role = EdgeRole::Left;
            }
        }
        let victim = dag
            .nodes
            .iter_mut()
            .find(|node| node.id == estimate)
            .expect("the SummaryEstimate node");
        victim.operator = ExecutableOperator::Binary;
        victim.payload = ExecutableOperatorPayload::Binary {
            timing: ExecutionTiming::ReadTime,
            operator: asap_types::post_asap::BinaryOperator {
                checked_relative_division: false,
                checked_finite_division: false,
                kind: asap_types::pre_asap::BinaryOpKind::Compare(CompareOpKind::Gt),
                vector_match: None,
            },
        };

        let refusals = admit(&dag).expect_err("is refused");
        assert_eq!(refusals.len(), 1, "{refusals:?}");
    }

    // ── Naming ───────────────────────────────────────────────────────────────

    #[test]
    fn debug_variant_names_a_variant_from_any_shape() {
        assert_eq!(
            debug_variant(&ValueOperation::Limit { n: 1, offset: 0 }),
            "Limit"
        );
        assert_eq!(
            debug_variant(&ValueOperation::FinalizeExactAccumulator),
            "FinalizeExactAccumulator"
        );
        assert_eq!(
            debug_variant(&ValueOperation::Extension { name: "x".into() }),
            "Extension"
        );
    }

    #[test]
    fn a_fallback_whose_output_is_summary_state_is_refused() {
        let mut dag = plan("quantile(0.5, cpu_cores)");
        let fallback = node_of(&dag, ExecutableOperator::Fallback).id;
        let agg_schema = node_of(&dag, ExecutableOperator::SummaryAgg)
            .output_schema
            .clone();
        let victim = dag
            .nodes
            .iter_mut()
            .find(|node| node.id == fallback)
            .expect("the Fallback node");
        victim.output_state = ExecutionDataState::MAINTENANCE_SUMMARY;
        victim.output_schema = agg_schema;

        let refusals = admit(&dag).expect_err("is refused");
        assert!(
            refusals.iter().any(
                |refusal| matches!(refusal, Refusal::UnsupportedOperator { operator, .. }
                    if operator.contains("Fallback"))
            ),
            "{refusals:?}"
        );
    }

    #[test]
    fn a_source_table_scan_is_accepted_as_a_row_source() {
        // The SQL front end's leaf shape, reached here by rewriting the PromQL
        // one: `Source` carries identity only, and both identities are leaves.
        let mut dag = plan("quantile(0.5, cpu_cores)");
        let fallback = node_of(&dag, ExecutableOperator::Fallback).id;
        let victim = dag
            .nodes
            .iter_mut()
            .find(|node| node.id == fallback)
            .expect("the Fallback node");
        let ExecutableOperatorPayload::Fallback { expression } = &mut victim.payload else {
            panic!("the Fallback node lost its payload");
        };
        let QueryExpr::Scan { source, .. } = expression else {
            panic!("the Fallback node lost its Scan");
        };
        *source = Source::Table {
            table_ref: "metrics".into(),
        };

        assert!(admit(&dag).is_ok());
    }

    #[test]
    fn a_node_with_no_input_edge_where_one_is_required_is_refused() {
        let mut dag = plan("quantile(0.5, cpu_cores)");
        let agg = node_of(&dag, ExecutableOperator::SummaryAgg).id;
        dag.edges.retain(|edge| edge.consumer != agg);

        let refusals = admit(&dag).expect_err("is refused");
        assert!(
            refusals.iter().any(|refusal| matches!(
                refusal,
                Refusal::UnsupportedUpdate { detail, .. } if detail.contains("no Input edge")
            )),
            "{refusals:?}"
        );
    }

    #[test]
    fn the_resolved_weight_position_names_the_promql_sample_value_column() {
        // The position in `NodeDecision::Aggregate` is an index, so this pins
        // what index 1 of the producer's schema actually is: an off-by-one here
        // would weight the sketch by timestamps and never fail loudly.
        let dag = plan("quantile(0.5, cpu_cores)");
        let field: &SummaryField = &node_of(&dag, ExecutableOperator::Fallback)
            .output_schema
            .fields[1];
        assert_eq!(field.name, "value");
        assert_eq!(field.dtype, SummaryFamilyType::Plain(DataType::Float64));
    }
}
