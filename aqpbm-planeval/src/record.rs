//! The output shape: one record per plan run, carrying both arms.
//!
//! Not `aqpbm_core::MergedRecord`. That record's identity is one
//! `(sketch, library, sketch_config)` triple, it has no node list, and — the
//! reason a new shape exists at all — it has nowhere to put the exact-execution
//! arm. The advantage this crate reports is a ratio between two measurements,
//! so both have to live in one record or the ratio is assembled by whoever
//! reads the file, differently each time.

use aqpbm_core::benchmark_result::fold;
use aqpbm_core::benchmark_result::{CpuTime, LatencySummary, RunStats};
use aqpbm_core::metrics::RunMetrics;
use serde::{Deserialize, Serialize};

use asap_types::post_asap::{ExecutableOperator, PostAsapNodeId, SummaryFamilyType};

use crate::plan::Plan;
use crate::run::{ArmTiming, RunOutcome};
use crate::score::ObservedError;
use crate::types::{Answer, PlanId};

pub const PLANEVAL_SCHEMA_VERSION: u32 = 4;

#[derive(Debug, Clone, Serialize, Deserialize)]
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Arm {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub build: Option<Phase>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub update: Option<Phase>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub readout: Option<Phase>,
    /// Summary state held, summed over every node and group. `0` for the exact
    /// arm, which holds the retained column instead — reported separately so a
    /// reader is not invited to compare a sketch against nothing.
    pub state_bytes: usize,
    pub retained_bytes: usize,
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
pub struct ReadoutRecord {
    pub node: u32,
    pub group: String,
    pub query: String,
    #[serde(with = "crate::score::json_f64")]
    pub approximate: f64,
    /// `None` when `verify` was off. Never 0.0, which would read as "the
    /// sketch was exactly right".
    pub exact: Option<f64>,
    pub observed_error: ObservedError,
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
        let retained_bytes = outcome.retained_bytes;

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
        Advantage {
            memory: ratio(
                self.exact.retained_bytes as f64,
                self.approximate.state_bytes as f64,
            ),
            readout_latency: ratio(
                phase_mean(self.exact.readout.as_ref()),
                phase_mean(self.approximate.readout.as_ref()),
            ),
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
        build: phase(&timing.build),
        update: phase(&timing.update),
        readout: phase(&timing.readout),
        state_bytes,
        retained_bytes,
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
    use crate::admit::admit;
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
        let admitted = admit(&plan.dag).unwrap();
        let file = csv(values);
        let outcome = run(
            &plan,
            &admitted,
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
        assert!(readout.observed_error.measured().unwrap() <= readout.claimed_bound.unwrap());
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
            record.readouts[0].approximate = value;
            record.readouts[0].observed_error = ObservedError::Measured {
                metric: "rank".to_string(),
                error: value,
            };

            let line = record.to_jsonl();
            assert!(
                line.contains(&format!("\"approximate\":{spelling}")),
                "{line}"
            );
            assert!(line.contains(&format!("\"error\":{spelling}")), "{line}");

            let back: PlanEvalRecord = serde_json::from_str(&line)
                .unwrap_or_else(|e| panic!("{line} must read back, got {e}"));
            let readout = &back.readouts[0];
            assert_eq!(
                readout.approximate.total_cmp(&value),
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
        assert_eq!(PLANEVAL_SCHEMA_VERSION, 4);
        let record = record_of("quantile(0.5, cpu_cores)", &[1.0, 2.0, 3.0], true);
        assert!(
            record.to_jsonl().contains("\"schema_version\":4"),
            "the stream has to say which encoding it is in"
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
            ("exact.update", record.exact.update.as_ref()),
            ("exact.readout", record.exact.readout.as_ref()),
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
        assert!(record.exact.readout.as_ref().unwrap().latency_ns.is_some());
        assert!(back.exact.build.is_none());
    }

    #[test]
    fn every_timing_field_carries_a_population_and_its_samples() {
        let values: Vec<f64> = (0..2_000).map(|i| (i % 211) as f64).collect();
        let record = record_of("quantile(0.5, cpu_cores)", &values, true);

        for (name, phase) in [
            ("approximate.build", record.approximate.build.as_ref()),
            ("approximate.update", record.approximate.update.as_ref()),
            ("approximate.readout", record.approximate.readout.as_ref()),
            ("exact.update", record.exact.update.as_ref()),
            ("exact.readout", record.exact.readout.as_ref()),
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
    fn the_readout_advantage_is_a_ratio_of_two_measured_regions() {
        let values: Vec<f64> = (0..20_000).map(|i| (i % 977) as f64).collect();
        let record = record_of("quantile(0.5, cpu_cores)", &values, true);
        let ratio = record
            .advantage()
            .readout_latency
            .expect("both arms read out");
        assert!(
            ratio > 1.0,
            "reading a quantile off a KLL should beat sorting 20k f64s, got {ratio}x"
        );
    }
}
