//! The output shape: one record per plan run, carrying both arms.
//!
//! Not `aqpbm_core::MergedRecord`. That record's identity is one
//! `(sketch, library, sketch_config)` triple, it has no node list, and — the
//! reason a new shape exists at all — it has nowhere to put the exact-execution
//! arm. The advantage this crate reports is a ratio between two measurements,
//! so both have to live in one record or the ratio is assembled by whoever
//! reads the file, differently each time.

use serde::{Deserialize, Serialize};

use asap_types::post_asap::{ExecutableOperator, PostAsapNodeId, SummaryFamilyType};

use crate::plan::Plan;
use crate::run::{ArmTiming, RunOutcome};
use crate::types::{Answer, PlanId};

pub const PLANEVAL_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PlanEvalRecord {
    pub schema_version: u32,
    pub plan: PlanIdentity,
    pub rows_scanned: u64,
    pub rows_emitted: u64,
    /// Whether the exact arm ran at all. A record with `verified: false` has
    /// no ground truth in it, and says so rather than leaving the reader to
    /// infer it from absent fields.
    pub verified: bool,
    pub nodes: Vec<NodeCost>,
    pub approximate: Arm,
    pub exact: Arm,
    pub readouts: Vec<ReadoutRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PlanIdentity {
    /// blake3 of the canonical document, lowercase hex.
    pub plan_id: String,
    pub query: String,
    pub document_schema_version: u32,
    pub nodes: usize,
    pub edges: usize,
    pub root: u32,
}

/// Per-node attribution. `PostAsapNodeId` is a field rather than a map key
/// because it serializes transparently as a `u32`, and JSON object keys must
/// be strings.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct NodeCost {
    pub node: u32,
    pub operator: String,
    /// The family the plan bound here, as the plan named it. `None` for every
    /// node that holds no summary state.
    pub family: Option<String>,
    pub state_bytes: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Arm {
    pub build_ns: u128,
    pub update_ns: u128,
    pub readout_ns: u128,
    /// Summary state held, summed over every node and group. `0` for the exact
    /// arm, which holds the retained column instead — reported separately so a
    /// reader is not invited to compare a sketch against nothing.
    pub state_bytes: usize,
    pub retained_bytes: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ReadoutRecord {
    pub node: u32,
    pub group: String,
    pub query: String,
    pub approximate: f64,
    /// `None` when `verify` was off. Never 0.0, which would read as "the
    /// sketch was exactly right".
    pub exact: Option<f64>,
    pub rank_error: Option<f64>,
    pub claimed_bound: Option<f64>,
    pub failure_probability: Option<f64>,
    pub observations: u64,
}

/// What the plan bought, as ratios of exact over approximate. Each is `None`
/// when the quantity was not measured on both arms — a ratio nothing measured
/// is not `1.0`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Advantage {
    pub memory: Option<f64>,
    pub readout_latency: Option<f64>,
}

impl PlanEvalRecord {
    pub fn from_run(query: &str, plan: &Plan, outcome: &RunOutcome) -> Self {
        let document = plan.document();
        let footprints: std::collections::HashMap<PostAsapNodeId, usize> =
            outcome.node_footprints.iter().copied().collect();

        let nodes = plan
            .dag
            .nodes
            .iter()
            .map(|node| NodeCost {
                node: node.id.0,
                operator: format!("{:?}", node.operator),
                family: match node.operator {
                    ExecutableOperator::SummaryAgg => node
                        .output_schema
                        .fields
                        .first()
                        .map(|field| family_label(&field.dtype)),
                    _ => None,
                },
                state_bytes: footprints.get(&node.id).copied(),
            })
            .collect();

        let state_bytes: usize = outcome.node_footprints.iter().map(|(_, b)| b).sum();
        // The exact arm's cost is the column it had to keep.
        let retained_bytes = outcome.retained_values * std::mem::size_of::<f64>();

        Self {
            schema_version: PLANEVAL_SCHEMA_VERSION,
            plan: PlanIdentity {
                plan_id: hex(&plan.id),
                query: query.to_string(),
                document_schema_version: document.schema_version,
                nodes: plan.dag.nodes.len(),
                edges: plan.dag.edges.len(),
                root: plan.dag.root.0,
            },
            rows_scanned: outcome.rows_scanned,
            rows_emitted: outcome.rows_emitted,
            verified: outcome.verified,
            nodes,
            approximate: arm(&outcome.approximate, state_bytes, 0),
            exact: arm(&outcome.exact, 0, retained_bytes),
            readouts: outcome
                .readouts
                .iter()
                .map(|r| ReadoutRecord {
                    node: r.node.0,
                    group: r.group.clone(),
                    query: format!("{:?}", r.query),
                    approximate: match &r.approximate {
                        Answer::Scalar(v) => *v,
                        Answer::Ranked(_) => f64::NAN,
                    },
                    exact: r.exact,
                    rank_error: r.rank_error,
                    claimed_bound: r.guarantee.as_ref().and_then(|g| g.bound.evaluate()),
                    failure_probability: r
                        .guarantee
                        .as_ref()
                        .and_then(|g| g.failure_probability.evaluate()),
                    observations: r.observations,
                })
                .collect(),
        }
    }

    /// The headline numbers. The planner supplies none of these — see PLAN.md
    /// §1.10.1 — so they are measured, not checked.
    pub fn advantage(&self) -> Advantage {
        Advantage {
            memory: ratio(self.exact.retained_bytes as f64, self.approximate.state_bytes as f64),
            readout_latency: ratio(self.exact.readout_ns as f64, self.approximate.readout_ns as f64),
        }
    }

    pub fn to_jsonl(&self) -> String {
        serde_json::to_string(self).expect("PlanEvalRecord is serializable")
    }
}

/// `Sketch(Kll{k:269})` rather than the whole `Debug` of a `SketchKind`, whose
/// private fields make its derived form unreadable in a terminal.
fn family_label(family: &SummaryFamilyType) -> String {
    match family {
        SummaryFamilyType::Sketch(kind, _) => {
            format!("Sketch({:?}, {:?})", kind.algorithm(), kind.params())
        }
        SummaryFamilyType::ExactAggregate(kind, _) => format!("Exact({kind:?})"),
        other => format!("{other:?}"),
    }
}

fn arm(timing: &ArmTiming, state_bytes: usize, retained_bytes: usize) -> Arm {
    Arm {
        build_ns: timing.build.as_nanos(),
        update_ns: timing.update.as_nanos(),
        readout_ns: timing.readout.as_nanos(),
        state_bytes,
        retained_bytes,
    }
}

/// `None` rather than a number when the denominator is zero: a ratio against
/// something that was never measured is not a measurement.
fn ratio(exact: f64, approximate: f64) -> Option<f64> {
    if approximate > 0.0 && exact > 0.0 {
        Some(exact / approximate)
    } else {
        None
    }
}

fn hex(bytes: &PlanId) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::admit::admit;
    use crate::plan::plan_promql;
    use crate::run::{run, RunConfig};
    use asap_types::types::AccuracyTarget;
    use std::io::Write;

    fn csv(values: &[f64]) -> tempfile::NamedTempFile {
        let mut file = tempfile::NamedTempFile::new().unwrap();
        writeln!(file, "ts,value").unwrap();
        for (i, v) in values.iter().enumerate() {
            writeln!(file, "{},{}", 1_700_000_000 + i as i64, v).unwrap();
        }
        file.flush().unwrap();
        file
    }

    fn record_of(query: &str, values: &[f64], verify: bool) -> PlanEvalRecord {
        let plan = plan_promql(query, AccuracyTarget::Epsilon(0.01)).unwrap();
        let admitted = admit(&plan.dag).unwrap();
        let file = csv(values);
        let outcome = run(
            &plan,
            &admitted,
            &RunConfig::new(file.path()).seed(7).verify(verify),
        )
        .unwrap();
        PlanEvalRecord::from_run(query, &plan, &outcome)
    }

    #[test]
    fn a_real_run_produces_a_record_with_both_arms_and_an_advantage() {
        let values: Vec<f64> = (0..10_000).map(|i| i as f64).collect();
        let record = record_of("quantile(0.5, cpu_cores)", &values, true);

        assert_eq!(record.schema_version, PLANEVAL_SCHEMA_VERSION);
        assert_eq!(record.plan.nodes, 3);
        assert_eq!(record.plan.plan_id.len(), 64, "blake3 as hex");
        assert_eq!(record.rows_emitted, 10_000);

        // Per-node attribution: only the SummaryAgg holds state.
        let with_state: Vec<_> = record
            .nodes
            .iter()
            .filter(|n| n.state_bytes.is_some())
            .collect();
        assert_eq!(with_state.len(), 1);
        assert_eq!(with_state[0].operator, "SummaryAgg");
        assert!(with_state[0].family.as_ref().unwrap().contains("Kll"));

        // Both arms, and the ratio between them.
        assert!(record.approximate.state_bytes > 0);
        assert_eq!(record.exact.state_bytes, 0);
        assert_eq!(record.exact.retained_bytes, 10_000 * 8);
        let advantage = record.advantage();
        let memory = advantage.memory.expect("both arms measured");
        assert!(
            memory > 1.0,
            "a k=269 sketch should hold less than 10k retained f64s, got {memory}x"
        );

        // The readout carries the planner's claim next to what was observed.
        let readout = &record.readouts[0];
        assert!(readout.exact.is_some());
        assert!(readout.rank_error.unwrap() <= readout.claimed_bound.unwrap());
        assert_eq!(readout.failure_probability, Some(0.01));
    }

    #[test]
    fn without_verify_there_is_no_ground_truth_and_no_advantage() {
        let record = record_of("quantile(0.5, cpu_cores)", &[1.0, 2.0, 3.0, 4.0], false);
        assert!(!record.verified);
        assert_eq!(record.exact.retained_bytes, 0);
        assert!(record.readouts[0].exact.is_none());
        // Not 1.0 — nothing was measured on the exact arm.
        assert_eq!(record.advantage().memory, None);
    }

    #[test]
    fn the_record_round_trips_as_jsonl() {
        let values: Vec<f64> = (0..500).map(|i| i as f64).collect();
        let record = record_of("quantile(0.9, cpu_cores)", &values, true);
        let line = record.to_jsonl();
        assert!(!line.contains('\n'), "one record, one line");
        let back: PlanEvalRecord = serde_json::from_str(&line).unwrap();
        assert_eq!(back, record);
    }
}
