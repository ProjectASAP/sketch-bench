//! Executes an admitted plan over rows, holding one summary per
//! `(plan, node, group)`, and computes the exact answer from the same rows so
//! both arms of the comparison come out of one pass.
//!
//! The exact arm is not a second implementation of the query: a `Fallback`
//! leaf is a bare `Scan`, so "execute exactly" is "keep the column the sketch
//! was fed and compute the statistic on it". That is why no query engine is
//! needed here, and why the two arms cannot silently disagree about which rows
//! they saw.

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use asap_types::post_asap::{
    ExecutableOperator, ExecutableOperatorPayload, PostAsapNodeId, ResultGuarantee, SketchQuery,
};

use crate::admit::{AdmittedPlan, NodeDecision, ResolvedInput};
use crate::handle::{bind, SummaryHandle};
use crate::plan::Plan;
use crate::rows;
use crate::score;
use crate::types::{Answer, EvalError, GroupKey, ItemKey, Row, StateKey, Value};

/// Everything the DAG cannot carry: which bytes the leaf's identity names, the
/// seed, and whether the exact arm is computed at all.
#[derive(Debug, Clone)]
pub struct RunConfig {
    pub csv: PathBuf,
    pub seed: u64,
    /// Retain the summarized column so the exact answer and the rank error can
    /// be computed. O(n) memory. The outcome records whether it ran.
    pub verify: bool,
}

impl RunConfig {
    pub fn new(csv: impl Into<PathBuf>) -> Self {
        Self {
            csv: csv.into(),
            seed: 0,
            verify: true,
        }
    }

    pub fn seed(mut self, seed: u64) -> Self {
        self.seed = seed;
        self
    }

    pub fn verify(mut self, verify: bool) -> Self {
        self.verify = verify;
        self
    }
}

/// One readout, both arms.
#[derive(Debug, Clone)]
pub struct Readout {
    pub node: PostAsapNodeId,
    pub producer: PostAsapNodeId,
    pub group: GroupKey,
    pub query: SketchQuery,
    pub approximate: Answer,
    /// `None` when `verify` was off — never 0.0, which would read as "exact
    /// and the sketch was perfect".
    pub exact: Option<f64>,
    pub rank_error: Option<f64>,
    pub guarantee: Option<ResultGuarantee>,
    pub observations: u64,
}

#[derive(Debug, Clone, Default)]
pub struct ArmTiming {
    pub build: Duration,
    pub update: Duration,
    pub readout: Duration,
}

#[derive(Debug, Clone)]
pub struct RunOutcome {
    pub rows_scanned: u64,
    pub rows_emitted: u64,
    pub verified: bool,
    /// Values the exact arm held, summed over every `(node, group)`. Counted
    /// here rather than derived from `readouts`, which are empty for a plan
    /// whose state *is* its answer (an exact accumulator has no
    /// `SummaryEstimate`) even though the exact arm still retained a column.
    pub retained_values: usize,
    pub readouts: Vec<Readout>,
    /// Summary state held per node, summed over that node's groups.
    pub node_footprints: Vec<(PostAsapNodeId, usize)>,
    pub approximate: ArmTiming,
    pub exact: ArmTiming,
}

/// Run one admitted plan.
///
/// v0 shape: exactly one row source, and every `SummaryAgg` consumes it
/// directly. That covers every plan the planner produces from the corpus; a
/// second row source is refused rather than guessed at.
pub fn run(plan: &Plan, admitted: &AdmittedPlan, cfg: &RunConfig) -> Result<RunOutcome, EvalError> {
    let dag = admitted.dag();
    let by_id: HashMap<PostAsapNodeId, &_> = dag.nodes.iter().map(|n| (n.id, n)).collect();

    // ── locate the single row source ────────────────────────────────────────
    let sources: Vec<PostAsapNodeId> = plan
        .order
        .iter()
        .copied()
        .filter(|id| matches!(admitted.decision(*id), Some(NodeDecision::RowSource)))
        .collect();
    let source_id = match sources.as_slice() {
        [one] => *one,
        [] => return Err(EvalError::RowSource("plan has no row source".into())),
        many => {
            return Err(EvalError::RowSource(format!(
                "v0 runs a single row source; this plan has {}",
                many.len()
            )))
        }
    };
    let source_node = by_id[&source_id];
    let scan = match &source_node.payload {
        ExecutableOperatorPayload::Fallback { expression } => expression,
        other => {
            return Err(EvalError::RowSource(format!(
                "row source node carries a {other:?} payload"
            )))
        }
    };

    // ── the aggregates that consume it ──────────────────────────────────────
    let aggregates: Vec<PostAsapNodeId> = plan
        .order
        .iter()
        .copied()
        .filter(|id| matches!(admitted.decision(*id), Some(NodeDecision::Aggregate { .. })))
        .filter(|id| {
            dag.edges
                .iter()
                .any(|e| e.consumer == *id && e.producer == source_id)
        })
        .collect();

    // Handles are bound lazily: a group is not known until a row names it, so
    // construction cannot happen before the first row. `build` therefore
    // accumulates the `bind` calls themselves, kept out of `update`, rather
    // than timing a single up-front phase.
    let mut handles: HashMap<StateKey, Box<dyn SummaryHandle>> = HashMap::new();
    let mut retained: HashMap<StateKey, Vec<f64>> = HashMap::new();
    let mut counts: HashMap<StateKey, u64> = HashMap::new();
    let mut build_elapsed = Duration::ZERO;

    // ── one pass over the rows, feeding every aggregate ─────────────────────
    let mut source = rows::open(scan, &cfg.csv, &source_node.output_schema)?;
    let mut update_elapsed = Duration::ZERO;
    let mut exact_elapsed = Duration::ZERO;

    for row in &mut source {
        let row = row?;
        for agg in &aggregates {
            let (item, weight, group_columns) = match admitted.decision(*agg) {
                Some(NodeDecision::Aggregate {
                    item,
                    weight,
                    group_columns,
                }) => (item, weight, group_columns),
                _ => unreachable!("filtered to Aggregate above"),
            };
            let key = StateKey {
                plan: plan.id,
                node: *agg,
                group: group_key(&row, group_columns)?,
            };
            let weight = resolve_weight(weight, &row)?;
            let item = match item {
                Some(input) => Some(resolve_item(input, &row)?),
                None => None,
            };

            if !handles.contains_key(&key) {
                let started = Instant::now();
                let family = agg_family(&by_id[agg].payload)?;
                let fresh =
                    bind(family, *agg, cfg.seed).map_err(|r| EvalError::Refused(vec![r]))?;
                handles.insert(key.clone(), fresh);
                build_elapsed += started.elapsed();
            }
            let started = Instant::now();
            handles
                .get_mut(&key)
                .expect("just inserted if absent")
                .update(item.as_ref(), weight)?;
            update_elapsed += started.elapsed();

            *counts.entry(key.clone()).or_insert(0) += 1;
            if cfg.verify {
                let started = Instant::now();
                retained.entry(key).or_default().push(weight);
                exact_elapsed += started.elapsed();
            }
        }
    }

    // ── read every estimate out, and answer the same question exactly ───────
    let mut readouts = Vec::new();
    let mut readout_elapsed = Duration::ZERO;
    let mut exact_readout = Duration::ZERO;

    for id in &plan.order {
        let query = match admitted.decision(*id) {
            Some(NodeDecision::Readout(query)) => query.clone(),
            _ => continue,
        };
        let producer = dag
            .edges
            .iter()
            .find(|e| e.consumer == *id)
            .map(|e| e.producer)
            .ok_or_else(|| EvalError::Validation(format!("readout {id:?} has no producer")))?;

        let groups: Vec<GroupKey> = handles
            .keys()
            .filter(|k| k.node == producer)
            .map(|k| k.group.clone())
            .collect();

        for group in groups {
            let key = StateKey {
                plan: plan.id,
                node: producer,
                group: group.clone(),
            };
            let started = Instant::now();
            let approximate = handles
                .get_mut(&key)
                .expect("group came from the handle map")
                .estimate(&query)?;
            readout_elapsed += started.elapsed();

            let (exact, rank_error) = match retained.get(&key) {
                Some(values) => {
                    let started = Instant::now();
                    let truth = score::exact_answer(values, &query)?;
                    let err = match (&approximate, &query) {
                        (Answer::Scalar(estimate), SketchQuery::Quantile { q }) => {
                            Some(score::rank_error(values, *estimate, *q))
                        }
                        _ => None,
                    };
                    exact_readout += started.elapsed();
                    (Some(truth), err)
                }
                None => (None, None),
            };

            readouts.push(Readout {
                node: *id,
                producer,
                group: group.clone(),
                query: query.clone(),
                approximate,
                exact,
                rank_error,
                guarantee: by_id[id].guarantee.clone(),
                observations: counts.get(&key).copied().unwrap_or(0),
            });
        }
    }

    // An admitted node that nothing executed is a seam bug, not an empty
    // result: without this check a `Value` between the row source and an
    // aggregate silently drops that aggregate and `run` still returns `Ok`.
    for id in &plan.order {
        let executed = match admitted.decision(*id) {
            Some(NodeDecision::RowSource) => *id == source_id,
            Some(NodeDecision::Aggregate { .. }) => aggregates.contains(id),
            Some(NodeDecision::Readout(_)) => readouts.iter().any(|r| r.node == *id),
            Some(NodeDecision::RowOperation) | None => false,
        };
        if !executed {
            return Err(EvalError::Validation(format!(
                "node {id:?} was admitted but never executed"
            )));
        }
    }

    let mut node_footprints: HashMap<PostAsapNodeId, usize> = HashMap::new();
    for (key, handle) in &handles {
        *node_footprints.entry(key.node).or_insert(0) += handle.footprint_bytes();
    }
    let mut node_footprints: Vec<_> = node_footprints.into_iter().collect();
    node_footprints.sort_by_key(|(id, _)| id.0);

    Ok(RunOutcome {
        rows_scanned: source.scanned(),
        rows_emitted: source.emitted(),
        verified: cfg.verify,
        retained_values: retained.values().map(Vec::len).sum(),
        readouts,
        node_footprints,
        approximate: ArmTiming {
            build: build_elapsed,
            update: update_elapsed,
            readout: readout_elapsed,
        },
        exact: ArmTiming {
            build: Duration::ZERO,
            update: exact_elapsed,
            readout: exact_readout,
        },
    })
}

fn agg_family(
    payload: &ExecutableOperatorPayload,
) -> Result<&asap_types::post_asap::SummaryFamilyType, EvalError> {
    match payload {
        ExecutableOperatorPayload::SummaryAgg { family, .. } => Ok(family),
        other => Err(EvalError::Validation(format!(
            "expected a SummaryAgg payload, found {other:?}"
        ))),
    }
}

/// `';'`-joined, matching what `aqpbm-cli` already builds and what the
/// subpopulation ground truths already score against.
fn group_key(row: &Row, columns: &[usize]) -> Result<GroupKey, EvalError> {
    let mut out = String::new();
    for (i, position) in columns.iter().enumerate() {
        let value = row.0.get(*position).ok_or_else(|| {
            EvalError::RowSource(format!("grouping column {position} is past the row"))
        })?;
        if i > 0 {
            out.push(';');
        }
        match value {
            Value::Str(s) => out.push_str(s),
            Value::Int(v) | Value::Timestamp(v) => out.push_str(&v.to_string()),
            Value::Float(v) => out.push_str(&v.to_string()),
            Value::Null => out.push_str("\u{0}null"),
        }
    }
    Ok(out)
}

fn resolve_weight(input: &ResolvedInput, row: &Row) -> Result<f64, EvalError> {
    match input {
        ResolvedInput::Constant(v) => Ok(*v),
        ResolvedInput::Column(position) => row
            .0
            .get(*position)
            .and_then(Value::as_f64)
            .ok_or_else(|| EvalError::RowSource(format!("weight column {position} is not numeric"))),
        ResolvedInput::Tuple(_) => Err(EvalError::RowSource(
            "a tuple weight has no defined magnitude".into(),
        )),
    }
}

fn resolve_item(input: &ResolvedInput, row: &Row) -> Result<ItemKey, EvalError> {
    match input {
        ResolvedInput::Column(position) => match row.0.get(*position) {
            Some(Value::Str(s)) => Ok(ItemKey::Str(s.clone())),
            Some(Value::Int(v)) | Some(Value::Timestamp(v)) => Ok(ItemKey::Int(*v)),
            Some(other) => Err(EvalError::RowSource(format!(
                "item column {position} holds {other:?}, which is not a key"
            ))),
            None => Err(EvalError::RowSource(format!(
                "item column {position} is past the row"
            ))),
        },
        other => Err(EvalError::RowSource(format!(
            "item must be a column reference, found {other:?}"
        ))),
    }
}

/// The operators a v0 run can encounter, for a caller that wants to report
/// coverage without re-deriving it from the payloads.
pub fn operators(admitted: &AdmittedPlan) -> Vec<(PostAsapNodeId, ExecutableOperator)> {
    admitted
        .dag()
        .nodes
        .iter()
        .map(|node| (node.id, node.operator))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::admit::admit;
    use crate::plan::plan_promql;
    use asap_types::types::AccuracyTarget;
    use std::io::Write;

    /// `{ts, value}` — the usage-derived schema a PromQL leaf carries.
    fn csv_with(values: &[f64]) -> tempfile::NamedTempFile {
        let mut file = tempfile::NamedTempFile::new().expect("temp file");
        writeln!(file, "ts,value").unwrap();
        for (i, v) in values.iter().enumerate() {
            writeln!(file, "{},{}", 1_700_000_000 + i as i64, v).unwrap();
        }
        file.flush().unwrap();
        file
    }

    #[test]
    fn three_node_plan_runs_end_to_end_and_the_claimed_bound_holds() {
        let plan = plan_promql("quantile(0.5, cpu_cores)", AccuracyTarget::Epsilon(0.01))
            .expect("planning");
        assert_eq!(plan.dag.nodes.len(), 3);

        let admitted = admit(&plan.dag).expect("admitted with no refusals");

        let values: Vec<f64> = (0..10_000).map(|i| i as f64).collect();
        let csv = csv_with(&values);
        let outcome = run(&plan, &admitted, &RunConfig::new(csv.path()).seed(42))
            .expect("run");

        assert_eq!(outcome.rows_scanned, 10_000);
        assert_eq!(outcome.rows_emitted, 10_000);
        assert_eq!(outcome.readouts.len(), 1, "one ungrouped readout");

        let readout = &outcome.readouts[0];
        assert_eq!(readout.observations, 10_000);
        assert!(readout.exact.is_some(), "verify was on");

        // The planner's own claim, checked against the rows it was fed.
        let guarantee = readout.guarantee.as_ref().expect("readout carries one");
        let claimed = guarantee.bound.evaluate().expect("KLL bound is a constant");
        let observed = readout.rank_error.expect("a quantile readout has one");
        assert!(
            observed <= claimed,
            "rank error {observed} exceeded the claimed bound {claimed}"
        );

        // And the state it cost to get there.
        assert_eq!(outcome.node_footprints.len(), 1, "one SummaryAgg");
        assert!(outcome.node_footprints[0].1 > 0);
    }

    #[test]
    fn the_exact_arm_is_skipped_when_verify_is_off_and_says_so() {
        let plan =
            plan_promql("quantile(0.5, cpu_cores)", AccuracyTarget::Epsilon(0.01)).expect("plan");
        let admitted = admit(&plan.dag).expect("admitted");
        let csv = csv_with(&(0..1_000).map(|i| i as f64).collect::<Vec<_>>());

        let outcome = run(
            &plan,
            &admitted,
            &RunConfig::new(csv.path()).verify(false),
        )
        .expect("run");

        assert!(!outcome.verified);
        let readout = &outcome.readouts[0];
        // None, not 0.0 — "not computed" must never read as "exact".
        assert!(readout.exact.is_none());
        assert!(readout.rank_error.is_none());
    }

    #[test]
    fn an_exact_accumulator_plan_needs_no_sketch_and_still_reports_both_arms() {
        let plan = plan_promql("sum(cpu_cores)", AccuracyTarget::Epsilon(0.01)).expect("plan");
        assert_eq!(plan.dag.nodes.len(), 2, "Fallback -> SummaryAgg, no estimate");

        let admitted = admit(&plan.dag).expect("admitted");
        let values: Vec<f64> = (1..=100).map(|i| i as f64).collect();
        let csv = csv_with(&values);
        let outcome = run(&plan, &admitted, &RunConfig::new(csv.path())).expect("run");

        assert_eq!(outcome.rows_emitted, 100);
        // No SummaryEstimate node, so no readout — the accumulator's state is
        // the answer, reached through FinalizeExactAccumulator.
        assert!(outcome.readouts.is_empty());
        assert_eq!(outcome.node_footprints.len(), 1);
    }

    #[test]
    fn a_bare_selector_plan_has_no_summary_at_all() {
        let plan = plan_promql("cpu_cores", AccuracyTarget::Epsilon(0.01)).expect("plan");
        assert_eq!(plan.dag.nodes.len(), 1);
        assert!(plan.dag.edges.is_empty());

        let admitted = admit(&plan.dag).expect("admitted");
        let csv = csv_with(&[1.0, 2.0, 3.0]);
        let outcome = run(&plan, &admitted, &RunConfig::new(csv.path())).expect("run");

        assert_eq!(outcome.rows_emitted, 3);
        assert!(outcome.readouts.is_empty());
        assert!(outcome.node_footprints.is_empty(), "nothing was summarized");
    }

    #[test]
    fn the_claimed_bound_holds_across_many_seeds() {
        let plan =
            plan_promql("quantile(0.9, cpu_cores)", AccuracyTarget::Epsilon(0.01)).expect("plan");
        let admitted = admit(&plan.dag).expect("admitted");
        let values: Vec<f64> = (0..20_000).map(|i| (i % 997) as f64).collect();
        let csv = csv_with(&values);

        let mut violations = 0;
        let mut observed_errors = Vec::new();
        let seeds = 32;
        for seed in 0..seeds {
            let outcome = run(&plan, &admitted, &RunConfig::new(csv.path()).seed(seed))
                .expect("run");
            let readout = &outcome.readouts[0];
            let claimed = readout
                .guarantee
                .as_ref()
                .unwrap()
                .bound
                .evaluate()
                .unwrap();
            let observed = readout.rank_error.unwrap();
            observed_errors.push(observed);
            if observed > claimed {
                violations += 1;
            }
        }
        // delta = 0.01, so even one violation in 32 seeds is worth looking at.
        assert_eq!(
            violations, 0,
            "seeds violating a bound that claims delta = 0.01"
        );
        // ...and the errors must not be identically zero, or the assertion
        // above would hold for a rank_error that always returned 0.
        assert!(
            observed_errors.iter().any(|e| *e > 0.0),
            "every observed rank error was zero — the check is vacuous"
        );
    }
}
