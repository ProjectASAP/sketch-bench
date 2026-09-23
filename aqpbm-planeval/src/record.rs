//! The output shape: one record per plan run, carrying all three arms — the
//! post-ASAP plan, the retained column it is scored against, and the pre-ASAP
//! tree the ratios divide by.
//!
//! Not `aqpbm_core::MergedRecord`. That record's identity is one
//! `(sketch, library, sketch_config)` triple, it has no node list, and — the
//! reason a new shape exists at all — it has nowhere to put the arms that do
//! not use a sketch. The advantage this crate reports is a ratio between two
//! measurements, so both have to live in one record or the ratio is assembled
//! by whoever reads the file, differently each time.

use aqpbm_core::benchmark_result::fold;
use aqpbm_core::benchmark_result::{CpuTime, LatencySummary, RunStats};
use aqpbm_core::metrics::RunMetrics;
use serde::{Deserialize, Serialize};

use asap_types::post_asap::{ExecutableOperatorPayload, PostAsapNodeId, SummaryFamilyType};

use crate::df::RefusalCounts;
use crate::plan::Plan;
use crate::run::{operator_name, ArmTiming, NodeTiming, RunOutcome};
use crate::score::ObservedError;
use crate::types::{Answer, PlanId};

pub const PLANEVAL_SCHEMA_VERSION: u32 = 7;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanEvalRecord {
    pub schema_version: u32,
    pub runtime: String,
    pub refusals: RefusalCounts,
    pub plan: PlanIdentity,
    pub rows_scanned: u64,
    pub rows_emitted: u64,
    /// Rows the root node produced, for a plan whose answer is rows rather
    /// than a readout. `null` when the root holds summary state.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub root_rows: Option<usize>,
    /// Whether the exact arm ran at all. A record with `verified: false` has
    /// no ground truth in it, and says so rather than leaving the reader to
    /// infer it from absent fields. The arm carries no timing: it recomputes
    /// the statistic off the column the summary consumed, which is ground
    /// truth and not a query anyone would run.
    pub verified: bool,
    pub nodes: Vec<NodeCost>,
    pub approximate: Arm,
    pub exact: Arm,
    pub pre_asap: Arm,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pre_asap_nodes: Vec<TreeNodeCost>,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub build_ns: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub update_ns: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub readout_ns: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub elapsed_compute_ns: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TreeNodeCost {
    pub node: u32,
    pub operator: String,
    pub elapsed_ns: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Arm {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub build: Option<Phase>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub update: Option<Phase>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub readout: Option<Phase>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evaluate: Option<Phase>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub maintenance: Option<Phase>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub read: Option<Phase>,
    /// Summary state held, summed over every node and group. `0` for the exact
    /// arm, which holds the retained column instead — reported separately so a
    /// reader is not invited to compare a sketch against nothing.
    pub state_bytes: usize,
    pub retained_bytes: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub peak_reserved_bytes: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub engine_overhead_ns: Option<u64>,
}

impl Arm {
    pub fn aggregate_ms(&self) -> f64 {
        phase_mean(self.build.as_ref())
            + phase_mean(self.update.as_ref())
            + phase_mean(self.maintenance.as_ref())
    }

    pub fn query_ms(&self) -> f64 {
        phase_mean(self.readout.as_ref())
            + phase_mean(self.read.as_ref())
            + phase_mean(self.evaluate.as_ref())
    }

    pub fn memory_bytes(&self) -> usize {
        self.state_bytes + self.retained_bytes + self.peak_reserved_bytes.unwrap_or(0)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Phase {
    pub work: u64,
    pub elapsed_ms: RunStats,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub per_sec: Option<RunStats>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cpu_ms: Option<CpuTime>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub latency_ns: Option<LatencySummary>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "shape", rename_all = "snake_case")]
pub enum AnswerRecord {
    Scalar {
        #[serde(with = "crate::score::json_f64")]
        value: f64,
    },
    Ranked {
        entries: Vec<RankedEntry>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RankedEntry {
    pub key: String,
    pub count: u64,
}

impl AnswerRecord {
    pub fn of(answer: &Answer) -> Self {
        match answer {
            Answer::Scalar(value) => AnswerRecord::Scalar { value: *value },
            Answer::Ranked(entries) => AnswerRecord::Ranked {
                entries: entries
                    .iter()
                    .map(|(key, count)| RankedEntry {
                        key: key.rendered(),
                        count: *count,
                    })
                    .collect(),
            },
        }
    }

    pub fn scalar(&self) -> Option<f64> {
        match self {
            AnswerRecord::Scalar { value } => Some(*value),
            AnswerRecord::Ranked { .. } => None,
        }
    }
}

impl std::fmt::Display for AnswerRecord {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AnswerRecord::Scalar { value } => match f.precision() {
                Some(places) => write!(f, "{value:.places$}"),
                None => write!(f, "{value}"),
            },
            AnswerRecord::Ranked { entries } => {
                write!(f, "[")?;
                for (index, entry) in entries.iter().enumerate() {
                    if index > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{}={}", entry.key, entry.count)?;
                }
                write!(f, "]")
            }
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ReadoutRecord {
    pub node: u32,
    pub group: String,
    pub query: String,
    pub approximate: AnswerRecord,
    /// `None` when `verify` was off. Never 0.0, which would read as "the
    /// sketch was exactly right".
    pub exact: Option<AnswerRecord>,
    pub observed_error: ObservedError,
    pub claimed_bound: Option<f64>,
    pub failure_probability: Option<f64>,
    pub observations: u64,
}

/// What the plan bought, as ratios of the pre-ASAP arm over the approximate
/// one, plus what it cost. Each is `None` when the quantity was not measured
/// on both arms — a ratio nothing measured is not `1.0`. Both time ratios
/// divide the same tree walk, because without approximation maintaining the
/// answer and asking for it are the same work. `accuracy` is not a ratio: it
/// is the largest error any readout showed, in that readout's own metric.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Advantage {
    pub aggregate_time: Option<f64>,
    pub query_time: Option<f64>,
    pub memory: Option<f64>,
    pub accuracy: Option<f64>,
}

impl PlanEvalRecord {
    pub fn from_run(query: &str, plan: &Plan, outcome: &RunOutcome) -> Self {
        let document = plan.document();
        let footprints: std::collections::HashMap<PostAsapNodeId, usize> =
            outcome.node_footprints.iter().copied().collect();

        let times: std::collections::HashMap<PostAsapNodeId, &NodeTiming> = outcome
            .node_times
            .iter()
            .map(|(id, timing)| (*id, timing))
            .collect();

        let nodes = plan
            .dag
            .nodes
            .iter()
            .map(|node| NodeCost {
                node: node.id.0,
                operator: operator_name(&node.payload).to_string(),
                family: match node.payload {
                    ExecutableOperatorPayload::SummaryAgg { .. } => node
                        .output_schema
                        .fields
                        .first()
                        .map(|field| family_label(&field.dtype)),
                    ExecutableOperatorPayload::Fallback { .. }
                    | ExecutableOperatorPayload::Binary { .. }
                    | ExecutableOperatorPayload::CandidateTopK { .. }
                    | ExecutableOperatorPayload::Value { .. }
                    | ExecutableOperatorPayload::RelationalJoin { .. }
                    | ExecutableOperatorPayload::SummaryJoin { .. }
                    | ExecutableOperatorPayload::SummarySubtract
                    | ExecutableOperatorPayload::SummaryDelete { .. }
                    | ExecutableOperatorPayload::SummaryEstimate { .. }
                    | ExecutableOperatorPayload::SummaryMerge => None,
                },
                state_bytes: footprints.get(&node.id).copied(),
                build_ns: times.get(&node.id).and_then(|timing| timing.build_ns),
                update_ns: times.get(&node.id).and_then(|timing| timing.update_ns),
                readout_ns: times.get(&node.id).and_then(|timing| timing.readout_ns),
                elapsed_compute_ns: times
                    .get(&node.id)
                    .and_then(|timing| timing.elapsed_compute_ns),
            })
            .collect();

        let state_bytes: usize = outcome.node_footprints.iter().map(|(_, b)| b).sum();
        // The exact arm's cost is the column it had to keep.
        let retained_bytes = outcome.retained_bytes;

        Self {
            schema_version: PLANEVAL_SCHEMA_VERSION,
            runtime: outcome.runtime.tag().to_string(),
            refusals: outcome.refusals.clone(),
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
            root_rows: outcome.root_rows,
            verified: outcome.verified,
            nodes,
            approximate: arm(
                &outcome.approximate,
                state_bytes,
                0,
                outcome.approximate_peak_bytes,
            ),
            exact: Arm {
                build: None,
                update: None,
                readout: None,
                evaluate: None,
                maintenance: None,
                read: None,
                state_bytes: 0,
                retained_bytes,
                peak_reserved_bytes: None,
                engine_overhead_ns: None,
            },
            pre_asap: arm(
                &outcome.pre_asap,
                0,
                outcome.pre_asap_bytes,
                outcome.pre_asap_peak_bytes,
            ),
            pre_asap_nodes: outcome
                .pre_asap_node_times
                .iter()
                .map(|node| TreeNodeCost {
                    node: node.node,
                    operator: node.operator.to_string(),
                    elapsed_ns: node.elapsed_ns,
                })
                .collect(),
            readouts: outcome
                .readouts
                .iter()
                .map(|r| ReadoutRecord {
                    node: r.node.0,
                    group: r.group.clone(),
                    query: format!("{:?}", r.query),
                    approximate: AnswerRecord::of(&r.approximate),
                    exact: r.exact.as_ref().map(AnswerRecord::of),
                    observed_error: r.observed_error.clone(),
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
        let without_approximation = self.pre_asap.query_ms();
        Advantage {
            aggregate_time: ratio(without_approximation, self.approximate.aggregate_ms()),
            query_time: ratio(without_approximation, self.approximate.query_ms()),
            memory: ratio(
                self.pre_asap.memory_bytes() as f64,
                self.approximate.memory_bytes() as f64,
            ),
            accuracy: self
                .readouts
                .iter()
                .filter_map(|readout| readout.observed_error.measured())
                .fold(None, |worst: Option<f64>, error| match worst {
                    Some(held) if held.total_cmp(&error).is_ge() => Some(held),
                    _ => Some(error),
                }),
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

fn arm(
    timing: &ArmTiming,
    state_bytes: usize,
    retained_bytes: usize,
    peak_reserved_bytes: Option<usize>,
) -> Arm {
    Arm {
        build: phase(&timing.build),
        update: phase(&timing.update),
        readout: phase(&timing.readout),
        evaluate: phase(&timing.evaluate),
        maintenance: phase(&timing.maintenance),
        read: phase(&timing.read),
        state_bytes,
        retained_bytes,
        peak_reserved_bytes,
        engine_overhead_ns: timing.engine_overhead_ns,
    }
}

fn phase(runs: &[RunMetrics]) -> Option<Phase> {
    let elapsed_ms = fold::elapsed_ms(runs)?;
    Some(Phase {
        work: runs.last().map(|run| run.work).unwrap_or(0),
        elapsed_ms,
        per_sec: fold::rate(runs),
        cpu_ms: fold::cpu_time_ms(runs),
        latency_ns: fold::latency(runs),
    })
}

fn phase_mean(phase: Option<&Phase>) -> f64 {
    phase.map(|p| p.elapsed_ms.mean).unwrap_or(0.0)
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
    use crate::plan::plan_promql;
    use crate::run::{run, RowsFrom, RunConfig, DEFAULT_TIMED_RUNS};
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

        let file = csv(values);
        let outcome = run(
            &plan,
            &RunConfig::new(RowsFrom::Csv(file.path().to_path_buf()), 7, verify),
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
        assert_eq!(
            record.exact.retained_bytes,
            16_384 * 8,
            "the column's allocation, not its length: the summary side is an allocation too"
        );
        let advantage = record.advantage();
        let memory = advantage.memory.expect("both arms measured");
        assert!(
            memory > 1.0,
            "a k=269 sketch should hold less than 10k retained f64s, got {memory}x"
        );

        // The readout carries the planner's claim next to what was observed.
        let readout = &record.readouts[0];
        assert!(readout.exact.is_some());
        assert!(readout.observed_error.measured().unwrap() <= readout.claimed_bound.unwrap());
        assert_eq!(readout.failure_probability, Some(0.01));
    }

    #[test]
    fn without_verify_there_is_no_ground_truth_and_no_accuracy() {
        let record = record_of("quantile(0.5, cpu_cores)", &[1.0, 2.0, 3.0, 4.0], false);
        assert!(!record.verified);
        assert_eq!(record.exact.retained_bytes, 0);
        assert!(record.readouts[0].exact.is_none());
        // Not 0.0 — no error was measured, rather than none being made.
        assert_eq!(record.advantage().accuracy, None);
        assert!(record.advantage().memory.is_some());
    }

    #[test]
    fn a_record_carrying_non_finite_numbers_still_round_trips() {
        let values: Vec<f64> = (0..500).map(|i| i as f64).collect();
        let base = record_of("quantile(0.5, cpu_cores)", &values, true);

        for (value, spelling) in [
            (12.0_f64, "12.0"),
            (f64::INFINITY, "\"inf\""),
            (f64::NEG_INFINITY, "\"-inf\""),
            (f64::NAN, "null"),
        ] {
            let mut record = base.clone();
            record.readouts[0].approximate = AnswerRecord::Scalar { value };
            record.readouts[0].observed_error = ObservedError::Measured {
                metric: "rank".to_string(),
                error: value,
            };

            let line = record.to_jsonl();
            assert!(
                line.contains(&format!(
                    "\"approximate\":{{\"shape\":\"scalar\",\"value\":{spelling}}}"
                )),
                "{line}"
            );
            assert!(line.contains(&format!("\"error\":{spelling}")), "{line}");

            let back: PlanEvalRecord = serde_json::from_str(&line)
                .unwrap_or_else(|e| panic!("{line} must read back, got {e}"));
            let readout = &back.readouts[0];
            assert_eq!(
                readout
                    .approximate
                    .scalar()
                    .expect("a scalar")
                    .total_cmp(&value),
                std::cmp::Ordering::Equal,
                "{line}"
            );
            match &readout.observed_error {
                ObservedError::Measured { error, .. } => {
                    assert_eq!(error.total_cmp(&value), std::cmp::Ordering::Equal, "{line}");
                }
                other => panic!("{other:?} is not the measurement that went in"),
            }
        }
    }

    #[test]
    fn the_encoding_change_moved_the_record_version() {
        assert_eq!(PLANEVAL_SCHEMA_VERSION, 7);
        let record = record_of("quantile(0.5, cpu_cores)", &[1.0, 2.0, 3.0], true);
        let line = record.to_jsonl();
        assert!(
            line.contains("\"schema_version\":7"),
            "the stream has to say which encoding it is in"
        );
        assert!(line.contains("\"runtime\":\"interp\""), "{line}");
        assert!(
            line.contains(
                "\"refusals\":{\"promql_only\":0,\"time_axis\":0,\"no_constructor\":0,\
                 \"deferred\":0,\"unclassified\":0}"
            ),
            "{line}"
        );
    }

    fn same_f64(read: f64, wrote: f64, field: &str) {
        if read.total_cmp(&wrote) == std::cmp::Ordering::Equal {
            return;
        }
        assert!(
            (read - wrote).abs() <= f64::EPSILON * wrote.abs(),
            "{field}: {read} is not {wrote}"
        );
    }

    fn same_run_stats(read: &RunStats, wrote: &RunStats, field: &str) {
        assert_eq!(read.n, wrote.n, "{field}.n");
        same_f64(read.mean, wrote.mean, &format!("{field}.mean"));
        same_f64(read.stddev, wrote.stddev, &format!("{field}.stddev"));
        match (read.ci95, wrote.ci95) {
            (Some(read), Some(wrote)) => {
                same_f64(read[0], wrote[0], &format!("{field}.ci95[0]"));
                same_f64(read[1], wrote[1], &format!("{field}.ci95[1]"));
            }
            (None, None) => {}
            (read, wrote) => panic!("{field}.ci95: {read:?} is not {wrote:?}"),
        }
        assert_eq!(
            read.samples.len(),
            wrote.samples.len(),
            "{field}.samples length"
        );
        for (i, (read, wrote)) in read.samples.iter().zip(&wrote.samples).enumerate() {
            same_f64(*read, *wrote, &format!("{field}.samples[{i}]"));
        }
    }

    fn same_cpu_time(read: &CpuTime, wrote: &CpuTime, field: &str) {
        same_run_stats(&read.user_ms, &wrote.user_ms, &format!("{field}.user_ms"));
        same_run_stats(&read.sys_ms, &wrote.sys_ms, &format!("{field}.sys_ms"));
    }

    fn same_latency(read: &LatencySummary, wrote: &LatencySummary, field: &str) {
        assert_eq!(read.p50, wrote.p50, "{field}.p50");
        assert_eq!(read.p95, wrote.p95, "{field}.p95");
        assert_eq!(read.p99, wrote.p99, "{field}.p99");
        assert_eq!(read.p999, wrote.p999, "{field}.p999");
        assert_eq!(read.max, wrote.max, "{field}.max");
        assert_eq!(read.count, wrote.count, "{field}.count");
    }

    fn same_phase(read: Option<&Phase>, wrote: Option<&Phase>, field: &str) {
        let (read, wrote) = match (read, wrote) {
            (Some(read), Some(wrote)) => (read, wrote),
            (None, None) => return,
            (read, wrote) => panic!(
                "{field}: measured={} came back measured={}",
                wrote.is_some(),
                read.is_some()
            ),
        };
        assert_eq!(read.work, wrote.work, "{field}.work");
        same_run_stats(
            &read.elapsed_ms,
            &wrote.elapsed_ms,
            &format!("{field}.elapsed_ms"),
        );
        match (&read.per_sec, &wrote.per_sec) {
            (Some(read), Some(wrote)) => same_run_stats(read, wrote, &format!("{field}.per_sec")),
            (None, None) => {}
            (read, wrote) => panic!("{field}.per_sec: {read:?} is not {wrote:?}"),
        }
        match (&read.cpu_ms, &wrote.cpu_ms) {
            (Some(read), Some(wrote)) => same_cpu_time(read, wrote, &format!("{field}.cpu_ms")),
            (None, None) => {}
            (read, wrote) => panic!("{field}.cpu_ms: {read:?} is not {wrote:?}"),
        }
        match (&read.latency_ns, &wrote.latency_ns) {
            (Some(read), Some(wrote)) => same_latency(read, wrote, &format!("{field}.latency_ns")),
            (None, None) => {}
            (read, wrote) => panic!("{field}.latency_ns: {read:?} is not {wrote:?}"),
        }
    }

    fn same_arm(read: &Arm, wrote: &Arm, field: &str) {
        same_phase(
            read.build.as_ref(),
            wrote.build.as_ref(),
            &format!("{field}.build"),
        );
        same_phase(
            read.update.as_ref(),
            wrote.update.as_ref(),
            &format!("{field}.update"),
        );
        same_phase(
            read.readout.as_ref(),
            wrote.readout.as_ref(),
            &format!("{field}.readout"),
        );
        assert_eq!(read.state_bytes, wrote.state_bytes, "{field}.state_bytes");
        assert_eq!(
            read.retained_bytes, wrote.retained_bytes,
            "{field}.retained_bytes"
        );
    }

    #[test]
    fn the_record_round_trips_as_jsonl() {
        let values: Vec<f64> = (0..500).map(|i| i as f64).collect();
        let record = record_of("quantile(0.9, cpu_cores)", &values, true);
        let line = record.to_jsonl();
        assert!(!line.contains('\n'), "one record, one line");
        let back: PlanEvalRecord = serde_json::from_str(&line).unwrap();

        assert_eq!(back.schema_version, record.schema_version);
        assert_eq!(back.plan, record.plan);
        assert_eq!(back.nodes, record.nodes);
        assert_eq!(back.rows_scanned, record.rows_scanned);
        assert_eq!(back.rows_emitted, record.rows_emitted);
        assert_eq!(back.verified, record.verified);
        assert_eq!(back.readouts, record.readouts);

        same_arm(&back.approximate, &record.approximate, "approximate");
        same_arm(&back.exact, &record.exact, "exact");

        for (name, phase) in [
            ("approximate.build", record.approximate.build.as_ref()),
            ("approximate.update", record.approximate.update.as_ref()),
            ("approximate.readout", record.approximate.readout.as_ref()),
            ("pre_asap.evaluate", record.pre_asap.evaluate.as_ref()),
        ] {
            let phase = phase.unwrap_or_else(|| panic!("{name} was not measured"));
            assert!(phase.per_sec.is_some(), "{name}.per_sec");
            assert!(phase.cpu_ms.is_some(), "{name}.cpu_ms");
        }
        assert!(record
            .approximate
            .build
            .as_ref()
            .unwrap()
            .latency_ns
            .is_some());
        assert!(record
            .approximate
            .readout
            .as_ref()
            .unwrap()
            .latency_ns
            .is_some());
        assert!(back.exact.build.is_none());
        assert!(back.exact.update.is_none(), "the exact arm is not timed");
        assert!(back.exact.readout.is_none(), "the exact arm is not timed");
        assert_eq!(back.exact.retained_bytes, record.exact.retained_bytes);
    }

    #[test]
    fn every_timing_field_carries_a_population_and_its_samples() {
        let values: Vec<f64> = (0..2_000).map(|i| (i % 211) as f64).collect();
        let record = record_of("quantile(0.5, cpu_cores)", &values, true);

        for (name, phase) in [
            ("approximate.build", record.approximate.build.as_ref()),
            ("approximate.update", record.approximate.update.as_ref()),
            ("approximate.readout", record.approximate.readout.as_ref()),
            ("pre_asap.evaluate", record.pre_asap.evaluate.as_ref()),
        ] {
            let phase = phase.unwrap_or_else(|| panic!("{name} was not measured"));
            assert_eq!(phase.elapsed_ms.n, DEFAULT_TIMED_RUNS, "{name}");
            assert_eq!(phase.elapsed_ms.samples.len(), DEFAULT_TIMED_RUNS, "{name}");
            assert!(
                phase.elapsed_ms.ci95.is_none(),
                "{name}: iterations of one process are not independent samples"
            );
            assert!(phase.per_sec.is_some(), "{name} covered units of work");
        }

        assert!(record.exact.build.is_none());
        assert!(record.exact.update.is_none());
        assert!(record.exact.readout.is_none());

        assert_eq!(record.approximate.update.as_ref().unwrap().work, 2_000);
        assert_eq!(record.approximate.readout.as_ref().unwrap().work, 1);
    }

    #[test]
    fn the_per_call_percentiles_are_real_and_not_a_counter_only_shim() {
        let values: Vec<f64> = (0..2_000).map(|i| (i % 211) as f64).collect();
        let record = record_of("quantile(0.5, cpu_cores)", &values, true);

        let readout = record.approximate.readout.as_ref().unwrap();
        let latency = readout
            .latency_ns
            .expect("the readout phase times each probe");
        assert_eq!(latency.count, 1, "one ungrouped probe");
        assert!(latency.p50 > 0, "hdrhist is off");
        assert!(latency.max >= latency.p50);

        assert!(record
            .approximate
            .update
            .as_ref()
            .unwrap()
            .latency_ns
            .is_none());
    }

    #[test]
    fn the_time_advantages_are_ratios_over_the_pre_asap_arm() {
        let values: Vec<f64> = (0..20_000).map(|i| (i % 977) as f64).collect();
        let record = record_of("quantile(0.5, cpu_cores)", &values, true);

        let tree = record
            .pre_asap
            .evaluate
            .as_ref()
            .expect("the pre-ASAP arm is timed");
        assert_eq!(tree.work, 20_000, "one unit of work per row it reads");
        assert!(tree.elapsed_ms.mean > 0.0);
        assert!(record.pre_asap.build.is_none(), "a tree binds nothing");

        let advantage = record.advantage();
        let query = advantage.query_time.expect("both arms answered");
        assert!(
            query > 1.0,
            "reading a quantile off a KLL should beat walking the tree over 20k rows, got {query}x"
        );
        assert!(advantage.aggregate_time.expect("both arms aggregated") > 0.0);

        let readout = record.approximate.readout.as_ref().unwrap();
        let maintain = record.approximate.build.as_ref().unwrap().elapsed_ms.mean
            + record.approximate.update.as_ref().unwrap().elapsed_ms.mean;
        assert!((query - tree.elapsed_ms.mean / readout.elapsed_ms.mean).abs() < 1e-9);
        assert!((advantage.aggregate_time.unwrap() - tree.elapsed_ms.mean / maintain).abs() < 1e-9);
    }

    #[test]
    fn the_memory_advantage_is_taken_against_the_pre_asap_arm() {
        let values: Vec<f64> = (0..10_000).map(|i| i as f64).collect();
        let record = record_of("quantile(0.5, cpu_cores)", &values, true);

        assert!(
            record.pre_asap.retained_bytes > record.exact.retained_bytes,
            "the tree holds whole rows, the retained column holds one value each"
        );
        let memory = record.advantage().memory.expect("both arms held something");
        assert!(
            (memory
                - record.pre_asap.retained_bytes as f64 / record.approximate.state_bytes as f64)
                .abs()
                < 1e-9
        );
        assert!(memory > 1.0, "got {memory}x");
    }

    #[test]
    fn the_accuracy_term_is_the_worst_error_any_readout_showed() {
        let values: Vec<f64> = (0..5_000).map(|i| (i % 313) as f64).collect();
        let record = record_of("quantile(0.5, cpu_cores)", &values, true);
        let worst = record
            .readouts
            .iter()
            .filter_map(|readout| readout.observed_error.measured())
            .fold(f64::NEG_INFINITY, f64::max);
        assert_eq!(record.advantage().accuracy, Some(worst));

        let unverified = record_of("quantile(0.5, cpu_cores)", &values, false);
        assert_eq!(unverified.advantage().accuracy, None);
    }

    #[test]
    fn per_node_time_is_recorded_only_when_it_is_asked_for() {
        let values: Vec<f64> = (0..4_000).map(|i| (i % 211) as f64).collect();
        let plan = plan_promql("quantile(0.5, cpu_cores)", AccuracyTarget::Epsilon(0.01)).unwrap();
        let file = csv(&values);

        let mut config = RunConfig::new(RowsFrom::Csv(file.path().to_path_buf()), 3, true);
        let quiet = PlanEvalRecord::from_run(
            "quantile(0.5, cpu_cores)",
            &plan,
            &run(&plan, &config).unwrap(),
        );
        assert!(quiet.nodes.iter().all(|node| node.update_ns.is_none()));
        assert!(quiet.pre_asap_nodes.is_empty());

        config.per_node_time = true;
        let timed = PlanEvalRecord::from_run(
            "quantile(0.5, cpu_cores)",
            &plan,
            &run(&plan, &config).unwrap(),
        );

        let aggregate = timed
            .nodes
            .iter()
            .find(|node| node.operator == "SummaryAgg")
            .expect("the plan holds one");
        assert!(aggregate.build_ns.unwrap() > 0, "bind is per slot");
        assert!(aggregate.update_ns.unwrap() > 0, "insert is per update");
        assert!(aggregate.readout_ns.unwrap() > 0, "readout is per probe");

        let fallback = timed
            .nodes
            .iter()
            .find(|node| node.operator == "Fallback")
            .expect("the plan holds one");
        assert_eq!(fallback.update_ns, None);

        let operators: Vec<&str> = timed
            .pre_asap_nodes
            .iter()
            .map(|node| node.operator.as_str())
            .collect();
        assert_eq!(
            operators,
            vec!["Scan", "Aggregate"],
            "children charged first"
        );
        let tree_total: u64 = timed
            .pre_asap_nodes
            .iter()
            .map(|node| node.elapsed_ns)
            .sum();
        let phase_ns = timed.pre_asap.evaluate.as_ref().unwrap().elapsed_ms.mean * 1e6;
        assert!(
            (tree_total as f64) <= phase_ns * 1.5,
            "the doc allows a small mismatch, not a different measurement: \
             {tree_total} ns of nodes against {phase_ns} ns of phase"
        );
    }

    #[test]
    fn a_plan_with_no_pre_asap_tree_reports_no_ratio_rather_than_one() {
        let values: Vec<f64> = (0..500).map(|i| i as f64).collect();
        let plan = plan_promql("quantile(0.5, cpu_cores)", AccuracyTarget::Epsilon(0.01)).unwrap();
        let file = csv(&values);

        let mut config = RunConfig::new(RowsFrom::Csv(file.path().to_path_buf()), 1, true);
        config.pre_asap = false;
        let record = PlanEvalRecord::from_run(
            "quantile(0.5, cpu_cores)",
            &plan,
            &run(&plan, &config).unwrap(),
        );

        assert!(record.pre_asap.evaluate.is_none());
        let advantage = record.advantage();
        assert_eq!(advantage.aggregate_time, None);
        assert_eq!(advantage.query_time, None);
        assert_eq!(advantage.memory, None);
        assert!(advantage.accuracy.is_some());
    }
}
