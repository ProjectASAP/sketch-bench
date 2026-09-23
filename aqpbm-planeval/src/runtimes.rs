use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::df::run::{run as run_datafusion, DataFusionRunConfig};
use crate::plan::Plan;
use crate::record::{AnswerRecord, Arm, PlanEvalRecord, ReadoutRecord};
use crate::run::{run as run_interpreter, RunConfig};
use crate::types::EvalError;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeRecords {
    pub interp: PlanEvalRecord,
    pub datafusion: PlanEvalRecord,
}

pub fn records_per_runtime(
    query: &str,
    plan: &Plan,
    cfg: &RunConfig,
    engine: &DataFusionRunConfig,
) -> Result<RuntimeRecords, EvalError> {
    let interpreted = run_interpreter(plan, cfg)?;
    let executed = run_datafusion(plan, cfg, engine)?;
    Ok(RuntimeRecords {
        interp: PlanEvalRecord::from_run(query, plan, &interpreted),
        datafusion: PlanEvalRecord::from_run(query, plan, &executed),
    })
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReadoutPair {
    pub readout: String,
    pub interp: Vec<ReadoutRecord>,
    pub datafusion: Vec<ReadoutRecord>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "difference", rename_all = "snake_case")]
pub enum ReadoutDifference {
    ReadoutCount {
        readout: String,
        interp: usize,
        datafusion: usize,
    },
    RepeatedIdentity {
        readout: String,
        runtime: String,
        count: usize,
    },
    ApproximateValue {
        readout: String,
        interp: AnswerRecord,
        datafusion: AnswerRecord,
    },
    ExactValue {
        readout: String,
        interp: Option<AnswerRecord>,
        datafusion: Option<AnswerRecord>,
    },
}

impl std::fmt::Display for ReadoutDifference {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ReadoutDifference::ReadoutCount {
                readout,
                interp,
                datafusion,
            } => write!(
                f,
                "{readout}: interp produced {interp} of it and datafusion {datafusion}"
            ),
            ReadoutDifference::RepeatedIdentity {
                readout,
                runtime,
                count,
            } => write!(
                f,
                "{readout}: {runtime} gave {count} readouts this one name, so no readout of it \
                 can be paired with the other runtime's"
            ),
            ReadoutDifference::ApproximateValue {
                readout,
                interp,
                datafusion,
            } => write!(
                f,
                "{readout}: approximate interp {interp} vs datafusion {datafusion}"
            ),
            ReadoutDifference::ExactValue {
                readout,
                interp,
                datafusion,
            } => write!(
                f,
                "{readout}: exact interp {} vs datafusion {}",
                answer_or_absent(interp.as_ref()),
                answer_or_absent(datafusion.as_ref())
            ),
        }
    }
}

impl ReadoutPair {
    pub fn differences(&self) -> Vec<ReadoutDifference> {
        if self.interp.len() != self.datafusion.len() {
            return vec![ReadoutDifference::ReadoutCount {
                readout: self.readout.clone(),
                interp: self.interp.len(),
                datafusion: self.datafusion.len(),
            }];
        }
        if self.interp.len() > 1 {
            return ["interp", "datafusion"]
                .into_iter()
                .map(|runtime| ReadoutDifference::RepeatedIdentity {
                    readout: self.readout.clone(),
                    runtime: runtime.to_string(),
                    count: self.interp.len(),
                })
                .collect();
        }
        let (Some(interp), Some(datafusion)) = (self.interp.first(), self.datafusion.first())
        else {
            return Vec::new();
        };
        let mut held = Vec::new();
        if !same_bits(&interp.approximate, &datafusion.approximate) {
            held.push(ReadoutDifference::ApproximateValue {
                readout: self.readout.clone(),
                interp: interp.approximate.clone(),
                datafusion: datafusion.approximate.clone(),
            });
        }
        if !same_optional_bits(interp.exact.as_ref(), datafusion.exact.as_ref()) {
            held.push(ReadoutDifference::ExactValue {
                readout: self.readout.clone(),
                interp: interp.exact.clone(),
                datafusion: datafusion.exact.clone(),
            });
        }
        held
    }
}

pub fn readout_pairs(records: &RuntimeRecords) -> Vec<ReadoutPair> {
    let mut held: BTreeMap<String, ReadoutPair> = BTreeMap::new();
    for readout in &records.interp.readouts {
        pair_named(&mut held, readout_identity(readout))
            .interp
            .push(readout.clone());
    }
    for readout in &records.datafusion.readouts {
        pair_named(&mut held, readout_identity(readout))
            .datafusion
            .push(readout.clone());
    }
    held.into_values().collect()
}

fn pair_named(held: &mut BTreeMap<String, ReadoutPair>, key: String) -> &mut ReadoutPair {
    held.entry(key.clone()).or_insert_with(|| ReadoutPair {
        readout: key,
        interp: Vec::new(),
        datafusion: Vec::new(),
    })
}

pub fn readout_differences(records: &RuntimeRecords) -> Vec<ReadoutDifference> {
    readout_pairs(records)
        .iter()
        .flat_map(ReadoutPair::differences)
        .collect()
}

pub fn readout_identity(readout: &ReadoutRecord) -> String {
    format!(
        "node {} [{}] {}",
        readout.node, readout.group, readout.query
    )
}

fn answer_or_absent(answer: Option<&AnswerRecord>) -> String {
    match answer {
        Some(answer) => answer.to_string(),
        None => "not computed".to_string(),
    }
}

fn same_optional_bits(left: Option<&AnswerRecord>, right: Option<&AnswerRecord>) -> bool {
    match (left, right) {
        (None, None) => true,
        (Some(left), Some(right)) => same_bits(left, right),
        _ => false,
    }
}

fn same_bits(left: &AnswerRecord, right: &AnswerRecord) -> bool {
    match (left, right) {
        (AnswerRecord::Scalar { value: left }, AnswerRecord::Scalar { value: right }) => {
            left.to_bits() == right.to_bits()
        }
        (AnswerRecord::Ranked { entries: left }, AnswerRecord::Ranked { entries: right }) => {
            left == right
        }
        _ => false,
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RuntimeQuantity {
    pub arm: &'static str,
    pub name: &'static str,
    pub interp: Option<String>,
    pub datafusion: Option<String>,
    pub same_measurement: bool,
    pub reason: &'static str,
}

const SUMMARY_ARM: &str = "post-ASAP";
const PRE_ASAP_ARM: &str = "pre-ASAP";

pub fn quantities(records: &RuntimeRecords) -> Vec<RuntimeQuantity> {
    let interp = &records.interp;
    let datafusion = &records.datafusion;
    vec![
        RuntimeQuantity {
            arm: SUMMARY_ARM,
            name: "aggregate time",
            interp: milliseconds(interp.approximate.aggregate_ms()),
            datafusion: milliseconds(datafusion.approximate.aggregate_ms()),
            same_measurement: true,
            reason: "wall clock to turn the same rows into the same plan's summary state; \
                     interp sums its bind and insert phases, DataFusion times the maintenance \
                     query including the serialization into the state table",
        },
        RuntimeQuantity {
            arm: SUMMARY_ARM,
            name: "query time",
            interp: milliseconds(interp.approximate.query_ms()),
            datafusion: milliseconds(datafusion.approximate.query_ms()),
            same_measurement: false,
            reason: "both are wall clock to get the answer out of the summary, but interp \
                     times a bare in-process probe of a sketch it still holds while DataFusion \
                     times a whole read query, scanning and deserializing the state table; the \
                     difference between them is mostly that definition, not the engine",
        },
        RuntimeQuantity {
            arm: SUMMARY_ARM,
            name: "state bytes",
            interp: bytes(interp.approximate.state_bytes),
            datafusion: bytes(datafusion.approximate.state_bytes),
            same_measurement: false,
            reason: "interp sums each summary's in-process footprint_bytes(); DataFusion \
                     measures the Arrow state table the maintenance query wrote, which holds \
                     the serialized sketch. This row is the persistent state on its own and is \
                     never added to the pool peak below",
        },
        RuntimeQuantity {
            arm: SUMMARY_ARM,
            name: "maintenance pool peak bytes",
            interp: interp
                .approximate
                .maintenance_peak_reserved_bytes
                .map(render_bytes),
            datafusion: datafusion
                .approximate
                .maintenance_peak_reserved_bytes
                .map(render_bytes),
            same_measurement: false,
            reason: "interp has no memory pool to read; DataFusion 43 grows an ungrouped \
                     reservation by the delta in Accumulator::size(), which a preallocated \
                     sketch never moves, so 0 on an ungrouped aggregate is an unaccounted \
                     sketch and not an absent one, while a grouped aggregate does account for \
                     its hash table. It is a transient working set, not state anyone keeps",
        },
        RuntimeQuantity {
            arm: SUMMARY_ARM,
            name: "engine overhead",
            interp: interp.approximate.engine_overhead_ns.map(render_ns),
            datafusion: datafusion.approximate.engine_overhead_ns.map(render_ns),
            same_measurement: false,
            reason: "only DataFusion has operators outside the plan's nodes to charge, and its \
                     MemoryExec reports no MetricsSet, so the number it does report is a floor",
        },
        RuntimeQuantity {
            arm: PRE_ASAP_ARM,
            name: "query time",
            interp: phase_milliseconds(&interp.pre_asap),
            datafusion: phase_milliseconds(&datafusion.pre_asap),
            same_measurement: true,
            reason: "wall clock for the whole pre-ASAP tree over the same rows; this pair is \
                     the interpreter dimension the milestone asks for",
        },
        RuntimeQuantity {
            arm: PRE_ASAP_ARM,
            name: "input bytes held",
            interp: bytes(interp.pre_asap.retained_bytes),
            datafusion: bytes(datafusion.pre_asap.retained_bytes),
            same_measurement: false,
            reason: "interp counts the one retained column of its row-of-enum scan and \
                     DataFusion counts the whole four-column Arrow MemTable, so this pair \
                     diffs two definitions before it diffs two engines",
        },
        RuntimeQuantity {
            arm: PRE_ASAP_ARM,
            name: "evaluate pool peak bytes",
            interp: interp
                .pre_asap
                .evaluate_peak_reserved_bytes
                .map(render_bytes),
            datafusion: datafusion
                .pre_asap
                .evaluate_peak_reserved_bytes
                .map(render_bytes),
            same_measurement: false,
            reason: "interp has no memory pool to read",
        },
        RuntimeQuantity {
            arm: "ground truth",
            name: "retained bytes",
            interp: bytes(interp.exact.retained_bytes),
            datafusion: bytes(datafusion.exact.retained_bytes),
            same_measurement: false,
            reason: "interp keeps the summarized column to recompute the statistic; DataFusion \
                     takes its truth from the pre-ASAP arm's own query and retains nothing past \
                     it",
        },
    ]
}

fn milliseconds(value: f64) -> Option<String> {
    (value > 0.0).then(|| format!("{value:.4} ms"))
}

fn phase_milliseconds(arm: &Arm) -> Option<String> {
    arm.evaluate
        .as_ref()
        .map(|phase| format!("{:.4} ms", phase.elapsed_ms.mean))
}

fn bytes(value: usize) -> Option<String> {
    (value > 0).then(|| render_bytes(value))
}

fn render_bytes(value: usize) -> String {
    format!("{value} B")
}

fn render_ns(value: u64) -> String {
    format!("{:.4} ms", value as f64 / 1e6)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::score::ObservedError;

    fn scalar(value: f64) -> AnswerRecord {
        AnswerRecord::Scalar { value }
    }

    fn readout(approximate: f64, exact: Option<f64>) -> ReadoutRecord {
        ReadoutRecord {
            node: 2,
            group: String::new(),
            query: "Quantile { q: 0.99 }".to_string(),
            approximate: scalar(approximate),
            exact: exact.map(scalar),
            observed_error: ObservedError::NotVerified,
            claimed_bound: None,
            failure_probability: None,
            observations: 0,
        }
    }

    #[test]
    fn two_answers_agree_only_when_every_bit_of_both_agrees() {
        assert!(same_bits(&scalar(870.0), &scalar(870.0)));
        assert!(!same_bits(&scalar(870.0), &scalar(870.000_000_000_000_1)));
        assert!(
            same_bits(&scalar(f64::NAN), &scalar(f64::NAN)),
            "two runtimes that both produced no number agree"
        );
        assert!(!same_bits(&scalar(0.0), &scalar(-0.0)));
    }

    const NAME: &str = "node 2 [] Quantile { q: 0.99 }";

    fn pair(interp: Vec<ReadoutRecord>, datafusion: Vec<ReadoutRecord>) -> ReadoutPair {
        ReadoutPair {
            readout: NAME.to_string(),
            interp,
            datafusion,
        }
    }

    #[test]
    fn a_pair_reports_the_approximate_and_the_exact_side_separately() {
        let agreeing = pair(
            vec![readout(870.0, Some(899.0))],
            vec![readout(870.0, Some(899.0))],
        );
        assert!(agreeing.differences().is_empty());

        let differing = pair(
            vec![readout(870.0, Some(899.0))],
            vec![readout(871.0, Some(900.0))],
        );
        assert_eq!(differing.differences().len(), 2);
        assert!(matches!(
            differing.differences()[0],
            ReadoutDifference::ApproximateValue { .. }
        ));
        assert!(matches!(
            differing.differences()[1],
            ReadoutDifference::ExactValue { .. }
        ));
    }

    #[test]
    fn a_readout_only_one_runtime_produced_is_a_difference() {
        let alone = pair(vec![readout(870.0, None)], Vec::new());
        assert_eq!(
            alone.differences(),
            vec![ReadoutDifference::ReadoutCount {
                readout: NAME.to_string(),
                interp: 1,
                datafusion: 0,
            }]
        );
    }

    #[test]
    fn readouts_one_runtime_gave_the_same_name_are_reported_rather_than_overwritten() {
        let repeated = pair(
            vec![readout(870.0, Some(899.0)), readout(871.0, Some(899.0))],
            vec![readout(870.0, Some(899.0)), readout(871.0, Some(899.0))],
        );
        let differences = repeated.differences();
        assert_eq!(
            differences,
            vec![
                ReadoutDifference::RepeatedIdentity {
                    readout: NAME.to_string(),
                    runtime: "interp".to_string(),
                    count: 2,
                },
                ReadoutDifference::RepeatedIdentity {
                    readout: NAME.to_string(),
                    runtime: "datafusion".to_string(),
                    count: 2,
                },
            ],
            "two readouts under one name cannot be paired, and keeping the last of them would \
             score an answer against a truth that belongs to a different group"
        );
    }
}
