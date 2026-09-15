//! Executes an admitted plan over rows, holding one summary per
//! `(plan, node, group)`, and computes the exact answer from the same rows so
//! both arms of the comparison come out of one pass.
//!
//! The exact arm is not a second implementation of the query: a `Fallback`
//! leaf is a bare `Scan`, so "execute exactly" is "keep the column the sketch
//! was fed and compute the statistic on it". That is why no query engine is
//! needed here, and why the two arms cannot silently disagree about which rows
//! they saw.

use aqpbm_core::measure::{
    measure, runs_for, MeasureConfig, Measurement, Pass, Report, RunOutcome as MeasuredRun,
};
use aqpbm_core::metrics::{LatencyRecorder, Metric, MetricsMask, RunMetrics, WallClock};
use aqpbm_datagen::table::GeneratedTable;
use std::cell::RefCell;
use std::collections::hash_map::Entry;
use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::Rc;

use asap_types::post_asap::{
    ExecutableOperator, ExecutableOperatorPayload, PostAsapNodeId, ResultGuarantee, SketchQuery,
    SummaryFamilyType,
};

use crate::admit::{AdmittedPlan, NodeDecision, ResolvedInput};
use crate::handle::{bind, SummaryHandle};
use crate::plan::Plan;
use crate::rows;
use crate::rows::RowSource;
use crate::score;
use crate::score::ObservedError;
use crate::types::{Answer, EvalError, GroupKey, ItemKey, Row, Value};

pub const DEFAULT_TIMED_RUNS: usize = 3;
pub const DEFAULT_WARMUP_RUNS: usize = 1;

/// Where the rows come from. `Source::Table{table_ref}` / `TimeSeries{metric}`
/// carries only the leaf's *identity*; which bytes that identity names is the
/// run's business, not the plan's.
#[derive(Debug, Clone)]
pub enum RowsFrom {
    Csv(PathBuf),
    /// Generated in process from a `TableDescription`. Shared, not cloned: the
    /// table is the same every seed, so a difference between seeds is the
    /// sketch's and never the data's.
    Generated(Rc<GeneratedTable>),
}

/// Everything the DAG cannot carry: where the rows are, the seed, whether the
/// exact arm is computed at all, and how many passes each timed phase gets.
#[derive(Debug, Clone)]
pub struct RunConfig {
    pub rows: RowsFrom,
    pub seed: u64,
    /// Retain the summarized column so the exact answer and the error under the
    /// guarantee's metric can be computed. O(n) memory. The outcome records
    /// whether it ran.
    pub verify: bool,
    pub timed_runs: usize,
    pub warmup_runs: usize,
}

impl RunConfig {
    pub fn new(rows: RowsFrom, seed: u64, verify: bool) -> Self {
        Self {
            rows,
            seed,
            verify,
            timed_runs: DEFAULT_TIMED_RUNS,
            warmup_runs: DEFAULT_WARMUP_RUNS,
        }
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
    pub observed_error: ObservedError,
    pub guarantee: Option<ResultGuarantee>,
    pub observations: u64,
}

#[derive(Debug, Clone, Default)]
pub struct ArmTiming {
    pub build: Vec<RunMetrics>,
    pub update: Vec<RunMetrics>,
    pub readout: Vec<RunMetrics>,
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

struct Slot {
    node: PostAsapNodeId,
    family: SummaryFamilyType,
    group: GroupKey,
    observations: u64,
}

struct Update {
    slot: u32,
    item: Option<ItemKey>,
    weight: f64,
}

struct Probe {
    node: PostAsapNodeId,
    producer: PostAsapNodeId,
    slot: u32,
    group: GroupKey,
    query: SketchQuery,
    guarantee: Option<ResultGuarantee>,
}

struct Drained {
    slots: Vec<Slot>,
    updates: Rc<Vec<Update>>,
    scanned: u64,
    emitted: u64,
}

struct AggregateSpec<'a> {
    node: PostAsapNodeId,
    family: &'a SummaryFamilyType,
    item: &'a Option<ResolvedInput>,
    weight: &'a ResolvedInput,
    group_columns: &'a [usize],
}

type SummaryState = Vec<Box<dyn SummaryHandle>>;
type RetainedState = Vec<Vec<f64>>;
type Fault = Rc<RefCell<Option<EvalError>>>;
type Kept<T> = Rc<RefCell<Vec<T>>>;

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

    let mut specs = Vec::with_capacity(aggregates.len());
    for agg in &aggregates {
        let (item, weight, group_columns) = match admitted.decision(*agg) {
            Some(NodeDecision::Aggregate {
                item,
                weight,
                group_columns,
            }) => (item, weight, group_columns),
            _ => unreachable!("filtered to Aggregate above"),
        };
        specs.push(AggregateSpec {
            node: *agg,
            family: agg_family(&by_id[agg].payload)?,
            item,
            weight,
            group_columns,
        });
    }

    // ── one pass over the rows, resolved into what the passes replay ────────
    let mut source = match &cfg.rows {
        RowsFrom::Csv(path) => rows::open(scan, path, &source_node.output_schema)?,
        RowsFrom::Generated(table) => {
            rows::open_generated(scan, Rc::clone(table), &source_node.output_schema)?
        }
    };
    let drained = drain(&specs, &mut source)?;

    let mut probes: Vec<Probe> = Vec::new();
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

        let mut of_producer: Vec<(u32, &Slot)> = drained
            .slots
            .iter()
            .enumerate()
            .filter(|(_, slot)| slot.node == producer)
            .map(|(i, slot)| (i as u32, slot))
            .collect();
        of_producer.sort_by(|a, b| a.1.group.cmp(&b.1.group));

        for (slot, held) in of_producer {
            probes.push(Probe {
                node: *id,
                producer,
                slot,
                group: held.group.clone(),
                query: query.clone(),
                guarantee: by_id[id].guarantee.clone(),
            });
        }
    }
    let probes = Rc::new(probes);

    let build = time_binds(&drained.slots, cfg)?;
    let (update, mut states) = time_updates(&drained.slots, &drained.updates, cfg)?;

    let mut node_footprints: HashMap<PostAsapNodeId, usize> = HashMap::new();
    if let Some(last) = states.last() {
        for (slot, handle) in drained.slots.iter().zip(last.iter()) {
            *node_footprints.entry(slot.node).or_insert(0) += handle.footprint_bytes();
        }
    }
    let mut node_footprints: Vec<_> = node_footprints.into_iter().collect();
    node_footprints.sort_by_key(|(id, _)| id.0);

    let (readout, answers) = time_estimates(&probes, std::mem::take(&mut states), cfg)?;

    let mut exact = ArmTiming::default();
    let mut retained_values = 0usize;
    let mut truths: Vec<f64> = Vec::new();
    let mut retained: RetainedState = Vec::new();
    if cfg.verify {
        let (timing, columns) = time_retains(drained.slots.len(), &drained.updates, cfg);
        exact.update = timing;
        retained_values = columns
            .last()
            .map(|set| set.iter().map(Vec::len).sum())
            .unwrap_or(0);
        let (timing, answered, columns) = time_exact_estimates(&probes, columns, cfg)?;
        exact.readout = timing;
        truths = answered;
        retained = columns;
    }

    // ── read every estimate out, and answer the same question exactly ───────
    let mut readouts = Vec::with_capacity(probes.len());
    for (i, probe) in probes.iter().enumerate() {
        let approximate = answers.get(i).cloned().ok_or_else(|| {
            EvalError::Validation(format!("readout {:?} produced no answer", probe.node))
        })?;
        let (exact_value, observed_error) = match truths.get(i) {
            Some(truth) => {
                let column = retained
                    .get(probe.slot as usize)
                    .map(Vec::as_slice)
                    .unwrap_or(&[]);
                let err = score::observed_error(
                    probe.guarantee.as_ref(),
                    column,
                    &probe.query,
                    &approximate,
                    *truth,
                );
                (Some(*truth), err)
            }
            None => (None, ObservedError::NotVerified),
        };

        readouts.push(Readout {
            node: probe.node,
            producer: probe.producer,
            group: probe.group.clone(),
            query: probe.query.clone(),
            approximate,
            exact: exact_value,
            observed_error,
            guarantee: probe.guarantee.clone(),
            observations: drained.slots[probe.slot as usize].observations,
        });
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

    Ok(RunOutcome {
        rows_scanned: drained.scanned,
        rows_emitted: drained.emitted,
        verified: cfg.verify,
        retained_values,
        readouts,
        node_footprints,
        approximate: ArmTiming {
            build,
            update,
            readout,
        },
        exact,
    })
}

fn drain(specs: &[AggregateSpec<'_>], source: &mut RowSource) -> Result<Drained, EvalError> {
    let mut slots: Vec<Slot> = Vec::new();
    let mut index: HashMap<(usize, GroupKey), u32> = HashMap::new();
    let mut updates: Vec<Update> = Vec::new();

    for row in source.by_ref() {
        let row = row?;
        for (position, spec) in specs.iter().enumerate() {
            let group = group_key(&row, spec.group_columns)?;
            let slot = match index.entry((position, group)) {
                Entry::Occupied(held) => *held.get(),
                Entry::Vacant(empty) => {
                    let id = u32::try_from(slots.len()).map_err(|_| {
                        EvalError::RowSource("more groups than a u32 can index".into())
                    })?;
                    slots.push(Slot {
                        node: spec.node,
                        family: spec.family.clone(),
                        group: empty.key().1.clone(),
                        observations: 0,
                    });
                    empty.insert(id);
                    id
                }
            };
            let weight = resolve_weight(spec.weight, &row)?;
            let item = match spec.item {
                Some(input) => Some(resolve_item(input, &row)?),
                None => None,
            };
            slots[slot as usize].observations += 1;
            updates.push(Update { slot, item, weight });
        }
    }

    Ok(Drained {
        slots,
        updates: Rc::new(updates),
        scanned: source.scanned(),
        emitted: source.emitted(),
    })
}

fn phase_config(metric: Metric, extra: MetricsMask, cfg: &RunConfig) -> (usize, MeasureConfig) {
    let runs = runs_for(metric, cfg.timed_runs);
    (
        cfg.warmup_runs + runs,
        MeasureConfig {
            runs,
            warmup_runs: cfg.warmup_runs,
            metrics: metric.bit() | extra | MetricsMask::SECONDARY,
        },
    )
}

fn timed_calls(recorder: &mut LatencyRecorder, steps: usize, mut call: impl FnMut(usize)) {
    for i in 0..steps {
        let clock = WallClock::start();
        call(i);
        recorder.record_ns(clock.elapsed_ns());
    }
}

fn hold(cell: &Fault, err: EvalError) {
    let mut held = cell.borrow_mut();
    if held.is_none() {
        *held = Some(err);
    }
}

fn raise(cell: &Fault) -> Result<(), EvalError> {
    match cell.borrow_mut().take() {
        Some(err) => Err(err),
        None => Ok(()),
    }
}

fn measured_tail<T>(kept: &[T], measured: usize) -> &[T] {
    &kept[kept.len().saturating_sub(measured)..]
}

fn state_bytes(handles: &SummaryState) -> u64 {
    handles.iter().map(|h| h.footprint_bytes() as u64).sum()
}

fn retained_bytes(columns: &RetainedState) -> u64 {
    let held: usize = columns.iter().map(Vec::len).sum();
    (held * std::mem::size_of::<f64>()) as u64
}

fn bind_all(slots: &[Slot], seed: u64) -> Result<SummaryState, EvalError> {
    slots
        .iter()
        .map(|slot| {
            bind(&slot.family, slot.node, seed).map_err(|refusal| EvalError::Refused(vec![refusal]))
        })
        .collect()
}

fn time_binds(slots: &[Slot], cfg: &RunConfig) -> Result<Vec<RunMetrics>, EvalError> {
    if slots.is_empty() {
        return Ok(Vec::new());
    }
    let fault: Fault = Rc::new(RefCell::new(None));
    let kept: Kept<(SummaryState, LatencyRecorder)> = Rc::new(RefCell::new(Vec::new()));
    let recipes: Rc<Vec<(SummaryFamilyType, PostAsapNodeId)>> = Rc::new(
        slots
            .iter()
            .map(|slot| (slot.family.clone(), slot.node))
            .collect(),
    );

    let (total, config) = phase_config(Metric::Latency, MetricsMask::empty(), cfg);
    let mut passes: Measurement = Vec::with_capacity(total);
    for _ in 0..total {
        let recipes = Rc::clone(&recipes);
        let fault = Rc::clone(&fault);
        let kept = Rc::clone(&kept);
        let seed = cfg.seed;
        let built: SummaryState = Vec::with_capacity(recipes.len());
        let recorder = LatencyRecorder::new();
        let pass: Pass = Box::new(move || {
            let mut built = built;
            let mut recorder = recorder;
            let mut failure: Option<EvalError> = None;
            timed_calls(&mut recorder, recipes.len(), |i| {
                match bind(&recipes[i].0, recipes[i].1, seed) {
                    Ok(handle) => built.push(handle),
                    Err(refusal) => {
                        if failure.is_none() {
                            failure = Some(EvalError::Refused(vec![refusal]));
                        }
                    }
                }
            });
            let work = recipes.len() as u64;
            let report: Report = Box::new(move || {
                if let Some(err) = failure {
                    hold(&fault, err);
                }
                kept.borrow_mut().push((built, recorder));
                MeasuredRun {
                    work,
                    ..MeasuredRun::default()
                }
            });
            report
        });
        passes.push(pass);
    }

    let mut runs = measure(&config, passes);
    raise(&fault)?;
    let kept = std::mem::take(&mut *kept.borrow_mut());
    let measured = runs.len();
    for (metrics, (built, recorder)) in runs.iter_mut().zip(measured_tail(&kept, measured)) {
        metrics.memory_bytes = Some(state_bytes(built));
        metrics.latency_ns = Some(recorder.snapshot());
    }
    Ok(runs)
}

fn time_updates(
    slots: &[Slot],
    updates: &Rc<Vec<Update>>,
    cfg: &RunConfig,
) -> Result<(Vec<RunMetrics>, Vec<SummaryState>), EvalError> {
    if slots.is_empty() {
        return Ok((Vec::new(), Vec::new()));
    }
    let fault: Fault = Rc::new(RefCell::new(None));
    let filled: Rc<RefCell<Vec<SummaryState>>> = Rc::new(RefCell::new(Vec::new()));

    let (total, config) = phase_config(Metric::Throughput, MetricsMask::empty(), cfg);
    let mut passes: Measurement = Vec::with_capacity(total);
    for _ in 0..total {
        let handles = bind_all(slots, cfg.seed)?;
        let updates = Rc::clone(updates);
        let fault = Rc::clone(&fault);
        let filled = Rc::clone(&filled);
        let pass: Pass = Box::new(move || {
            let mut handles = handles;
            let mut failure: Option<EvalError> = None;
            for update in updates.iter() {
                if let Err(err) =
                    handles[update.slot as usize].update(update.item.as_ref(), update.weight)
                {
                    failure = Some(err);
                    break;
                }
            }
            let work = updates.len() as u64;
            let report: Report = Box::new(move || {
                if let Some(err) = failure {
                    hold(&fault, err);
                }
                filled.borrow_mut().push(handles);
                MeasuredRun {
                    work,
                    ..MeasuredRun::default()
                }
            });
            report
        });
        passes.push(pass);
    }

    let mut runs = measure(&config, passes);
    raise(&fault)?;
    let states = std::mem::take(&mut *filled.borrow_mut());
    let measured = runs.len();
    for (metrics, handles) in runs.iter_mut().zip(measured_tail(&states, measured)) {
        metrics.memory_bytes = Some(state_bytes(handles));
    }
    Ok((runs, states))
}

fn time_estimates(
    probes: &Rc<Vec<Probe>>,
    states: Vec<SummaryState>,
    cfg: &RunConfig,
) -> Result<(Vec<RunMetrics>, Vec<Answer>), EvalError> {
    if probes.is_empty() || states.is_empty() {
        return Ok((Vec::new(), Vec::new()));
    }
    let fault: Fault = Rc::new(RefCell::new(None));
    let kept: Kept<(Vec<Answer>, SummaryState, LatencyRecorder)> =
        Rc::new(RefCell::new(Vec::new()));

    let (_, config) = phase_config(Metric::Throughput, MetricsMask::LATENCY, cfg);
    let mut passes: Measurement = Vec::with_capacity(states.len());
    for state in states {
        let probes = Rc::clone(probes);
        let fault = Rc::clone(&fault);
        let kept = Rc::clone(&kept);
        let produced: Vec<Answer> = Vec::with_capacity(probes.len());
        let recorder = LatencyRecorder::new();
        let pass: Pass = Box::new(move || {
            let mut handles = state;
            let mut produced = produced;
            let mut recorder = recorder;
            let mut failure: Option<EvalError> = None;
            timed_calls(&mut recorder, probes.len(), |i| {
                let probe = &probes[i];
                match handles[probe.slot as usize].estimate(&probe.query) {
                    Ok(answer) => produced.push(answer),
                    Err(err) => {
                        if failure.is_none() {
                            failure = Some(err);
                        }
                    }
                }
            });
            let work = probes.len() as u64;
            let report: Report = Box::new(move || {
                if let Some(err) = failure {
                    hold(&fault, err);
                }
                kept.borrow_mut().push((produced, handles, recorder));
                MeasuredRun {
                    work,
                    ..MeasuredRun::default()
                }
            });
            report
        });
        passes.push(pass);
    }

    let mut runs = measure(&config, passes);
    raise(&fault)?;
    let mut kept = std::mem::take(&mut *kept.borrow_mut());
    let measured = runs.len();
    for (metrics, (_, handles, recorder)) in runs.iter_mut().zip(measured_tail(&kept, measured)) {
        metrics.memory_bytes = Some(state_bytes(handles));
        metrics.latency_ns = Some(recorder.snapshot());
    }
    let answers = kept
        .pop()
        .map(|(answers, _, _)| answers)
        .unwrap_or_default();
    Ok((runs, answers))
}

fn time_retains(
    slot_count: usize,
    updates: &Rc<Vec<Update>>,
    cfg: &RunConfig,
) -> (Vec<RunMetrics>, Vec<RetainedState>) {
    if slot_count == 0 {
        return (Vec::new(), Vec::new());
    }
    let kept: Rc<RefCell<Vec<RetainedState>>> = Rc::new(RefCell::new(Vec::new()));

    let (total, config) = phase_config(Metric::Throughput, MetricsMask::empty(), cfg);
    let mut passes: Measurement = Vec::with_capacity(total);
    for _ in 0..total {
        let updates = Rc::clone(updates);
        let kept = Rc::clone(&kept);
        let columns: RetainedState = vec![Vec::new(); slot_count];
        let pass: Pass = Box::new(move || {
            let mut columns = columns;
            for update in updates.iter() {
                columns[update.slot as usize].push(update.weight);
            }
            let work = updates.len() as u64;
            let report: Report = Box::new(move || {
                kept.borrow_mut().push(columns);
                MeasuredRun {
                    work,
                    ..MeasuredRun::default()
                }
            });
            report
        });
        passes.push(pass);
    }

    let mut runs = measure(&config, passes);
    let columns = std::mem::take(&mut *kept.borrow_mut());
    let measured = runs.len();
    for (metrics, set) in runs.iter_mut().zip(measured_tail(&columns, measured)) {
        metrics.memory_bytes = Some(retained_bytes(set));
    }
    (runs, columns)
}

fn time_exact_estimates(
    probes: &Rc<Vec<Probe>>,
    states: Vec<RetainedState>,
    cfg: &RunConfig,
) -> Result<(Vec<RunMetrics>, Vec<f64>, RetainedState), EvalError> {
    if probes.is_empty() || states.is_empty() {
        return Ok((Vec::new(), Vec::new(), Vec::new()));
    }
    let fault: Fault = Rc::new(RefCell::new(None));
    let kept: Kept<(Vec<f64>, RetainedState, LatencyRecorder)> = Rc::new(RefCell::new(Vec::new()));

    let (_, config) = phase_config(Metric::Throughput, MetricsMask::LATENCY, cfg);
    let mut passes: Measurement = Vec::with_capacity(states.len());
    for state in states {
        let probes = Rc::clone(probes);
        let fault = Rc::clone(&fault);
        let kept = Rc::clone(&kept);
        let answered: Vec<f64> = Vec::with_capacity(probes.len());
        let recorder = LatencyRecorder::new();
        let pass: Pass = Box::new(move || {
            let mut columns = state;
            let mut answered = answered;
            let mut recorder = recorder;
            let mut failure: Option<EvalError> = None;
            timed_calls(&mut recorder, probes.len(), |i| {
                let probe = &probes[i];
                let column = &mut columns[probe.slot as usize];
                if score::needs_a_sorted_column(&probe.query) {
                    column.sort_by(f64::total_cmp);
                }
                match score::exact_answer_sorted(column, &probe.query) {
                    Ok(truth) => answered.push(truth),
                    Err(err) => {
                        if failure.is_none() {
                            failure = Some(err);
                        }
                    }
                }
            });
            let work = probes.len() as u64;
            let report: Report = Box::new(move || {
                if let Some(err) = failure {
                    hold(&fault, err);
                }
                kept.borrow_mut().push((answered, columns, recorder));
                MeasuredRun {
                    work,
                    ..MeasuredRun::default()
                }
            });
            report
        });
        passes.push(pass);
    }

    let mut runs = measure(&config, passes);
    raise(&fault)?;
    let mut kept = std::mem::take(&mut *kept.borrow_mut());
    let measured = runs.len();
    for (metrics, (_, columns, recorder)) in runs.iter_mut().zip(measured_tail(&kept, measured)) {
        metrics.memory_bytes = Some(retained_bytes(columns));
        metrics.latency_ns = Some(recorder.snapshot());
    }
    let (answered, columns) = kept
        .pop()
        .map(|(answered, columns, _)| (answered, columns))
        .unwrap_or_default();
    Ok((runs, answered, columns))
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
        ResolvedInput::Column(position) => {
            row.0.get(*position).and_then(Value::as_f64).ok_or_else(|| {
                EvalError::RowSource(format!("weight column {position} is not numeric"))
            })
        }
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
            Some(Value::Float(v)) => Ok(ItemKey::Float(*v)),
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
    fn grouped_csv(groups: usize, rows: usize) -> tempfile::NamedTempFile {
        let mut file = tempfile::NamedTempFile::new().expect("temp file");
        writeln!(file, "ts,value,cluster").unwrap();
        for i in 0..rows {
            writeln!(
                file,
                "{},{},c{:02}",
                1_700_000_000 + i as i64,
                (i % 97) as f64,
                i % groups
            )
            .unwrap();
        }
        file.flush().unwrap();
        file
    }

    #[test]
    fn readouts_come_out_in_the_same_order_every_run() {
        let plan = plan_promql(
            "quantile by (cluster) (0.5, cpu_cores)",
            AccuracyTarget::Epsilon(0.01),
        )
        .expect("plan");
        let admitted = admit(&plan.dag).expect("admitted");
        let csv = grouped_csv(12, 600);

        let groups = |seed: u64| -> Vec<String> {
            run(&plan, &admitted, &csv_config(csv.path(), seed, true))
                .expect("run")
                .readouts
                .iter()
                .map(|readout| readout.group.clone())
                .collect()
        };

        let first = groups(0);
        assert_eq!(first.len(), 12, "one readout per cluster");
        let mut sorted = first.clone();
        sorted.sort();
        assert_eq!(
            first, sorted,
            "the handle map is a HashMap; its iteration order is not an order"
        );
        for seed in 1..6 {
            assert_eq!(groups(seed), first, "readout order moved between runs");
        }
    }

    fn csv_config(path: &std::path::Path, seed: u64, verify: bool) -> RunConfig {
        RunConfig::new(RowsFrom::Csv(path.to_path_buf()), seed, verify)
    }

    use super::*;
    use crate::admit::admit;
    use crate::plan::plan_promql;
    use asap_types::post_asap::{GroupingStrategy, SketchAlgorithm, SketchKind, SketchParams};
    use asap_types::types::AccuracyTarget;
    use std::io::Write;

    fn cms_slot() -> Slot {
        Slot {
            node: PostAsapNodeId(1),
            family: SummaryFamilyType::Sketch(
                SketchKind::new(
                    SketchAlgorithm::Cms,
                    SketchParams::Cms {
                        width: 272,
                        depth: 5,
                    },
                ),
                GroupingStrategy::PerSubpopulationInstance,
            ),
            group: String::new(),
            observations: 0,
        }
    }

    fn one_update(weight: f64) -> Update {
        Update {
            slot: 0,
            item: Some(ItemKey::Int(1)),
            weight,
        }
    }

    fn overflowing_updates() -> Rc<Vec<Update>> {
        Rc::new(vec![one_update(1.0), one_update(f64::from(i32::MAX))])
    }

    fn passes_config(warmup_runs: usize, timed_runs: usize) -> RunConfig {
        let mut config = RunConfig::new(
            RowsFrom::Csv(PathBuf::from("rows are already drained")),
            3,
            false,
        );
        config.warmup_runs = warmup_runs;
        config.timed_runs = timed_runs;
        config
    }

    #[test]
    fn a_fault_in_a_warm_up_pass_is_raised_even_though_the_pass_is_discarded() {
        let slots = vec![cms_slot()];
        let config = passes_config(2, 0);
        assert_eq!(
            phase_config(Metric::Throughput, MetricsMask::empty(), &config).0,
            2,
            "every pass this phase runs is a warm-up"
        );

        let err = match time_updates(&slots, &overflowing_updates(), &config) {
            Err(err) => err,
            Ok((runs, _)) => panic!("a discarded pass swallowed the fault, and reported {runs:?}"),
        };
        assert!(matches!(err, EvalError::Refused(_)), "{err:?}");
    }

    #[test]
    fn a_fault_is_raised_when_more_than_one_pass_is_timed() {
        let slots = vec![cms_slot()];
        let err = match time_updates(&slots, &overflowing_updates(), &passes_config(0, 3)) {
            Err(err) => err,
            Ok((runs, _)) => panic!("three timed passes faulted and reported {runs:?}"),
        };
        assert!(matches!(err, EvalError::Refused(_)), "{err:?}");
    }

    #[test]
    fn a_faulted_phase_reports_no_timing_at_all() {
        let slots = vec![cms_slot()];
        let config = passes_config(1, 3);

        let clean = Rc::new(vec![one_update(1.0)]);
        let (runs, states) = match time_updates(&slots, &clean, &config) {
            Ok(measured) => measured,
            Err(err) => panic!("a clean pass must not fault: {err:?}"),
        };
        assert_eq!(runs.len(), 3, "the same shape does report timing");
        assert_eq!(states.len(), 4, "warm-up included");
        assert!(runs.iter().all(|run| run.elapsed_ns > 0));

        assert!(time_updates(&slots, &overflowing_updates(), &config).is_err());
    }

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
        let outcome = run(&plan, &admitted, &csv_config(csv.path(), 42, true)).expect("run");

        assert_eq!(outcome.rows_scanned, 10_000);
        assert_eq!(outcome.rows_emitted, 10_000);
        assert_eq!(outcome.readouts.len(), 1, "one ungrouped readout");

        let readout = &outcome.readouts[0];
        assert_eq!(readout.observations, 10_000);
        assert!(readout.exact.is_some(), "verify was on");

        // The planner's own claim, checked against the rows it was fed.
        let guarantee = readout.guarantee.as_ref().expect("readout carries one");
        let claimed = guarantee.bound.evaluate().expect("KLL bound is a constant");
        let observed = readout
            .observed_error
            .measured()
            .expect("a quantile readout has one");
        assert!(
            observed <= claimed,
            "rank error {observed} exceeded the claimed bound {claimed}"
        );

        // And the state it cost to get there.
        assert_eq!(outcome.node_footprints.len(), 1, "one SummaryAgg");
        assert!(outcome.node_footprints[0].1 > 0);
    }

    #[test]
    fn the_rows_are_produced_once_however_many_passes_are_timed() {
        let plan =
            plan_promql("quantile(0.5, cpu_cores)", AccuracyTarget::Epsilon(0.01)).expect("plan");
        let admitted = admit(&plan.dag).expect("admitted");
        let csv = csv_with(&(0..1_000).map(|i| (i % 91) as f64).collect::<Vec<_>>());

        let mut config = csv_config(csv.path(), 5, true);
        config.timed_runs = 4;
        config.warmup_runs = 2;
        let outcome = run(&plan, &admitted, &config).expect("run");

        assert_eq!(outcome.rows_scanned, 1_000);
        assert_eq!(outcome.rows_emitted, 1_000);
        assert_eq!(outcome.readouts[0].observations, 1_000);
        assert_eq!(outcome.retained_values, 1_000);

        assert_eq!(
            outcome.approximate.update.len(),
            4,
            "warm-ups are discarded"
        );
        for pass in &outcome.approximate.update {
            assert_eq!(pass.work, 1_000, "each pass replayed the same updates");
            assert!(pass.elapsed_ns > 0);
        }
        for pass in &outcome.exact.update {
            assert_eq!(pass.work, 1_000);
        }
    }

    #[test]
    fn a_timed_phase_reports_a_population_and_an_untimed_one_reports_nothing() {
        let plan =
            plan_promql("quantile(0.5, cpu_cores)", AccuracyTarget::Epsilon(0.01)).expect("plan");
        let admitted = admit(&plan.dag).expect("admitted");
        let csv = csv_with(&(0..2_000).map(|i| (i % 313) as f64).collect::<Vec<_>>());

        let mut config = csv_config(csv.path(), 1, true);
        config.timed_runs = 3;
        let outcome = run(&plan, &admitted, &config).expect("run");

        for phase in [
            &outcome.approximate.build,
            &outcome.approximate.update,
            &outcome.approximate.readout,
            &outcome.exact.update,
            &outcome.exact.readout,
        ] {
            assert_eq!(phase.len(), 3, "one draw is not a distribution");
        }
        assert!(
            outcome.exact.build.is_empty(),
            "the exact arm binds nothing, which is not the same as binding instantly"
        );
        let binds = outcome.approximate.build[0]
            .latency_ns
            .as_ref()
            .expect("each bind was timed on its own");
        assert_eq!(binds.count, 1, "one group, one bind");
        assert!(
            binds.p50 > 0,
            "hdrhist is off: the percentiles come back zero with no error"
        );
    }

    #[test]
    fn the_exact_arm_is_skipped_when_verify_is_off_and_says_so() {
        let plan =
            plan_promql("quantile(0.5, cpu_cores)", AccuracyTarget::Epsilon(0.01)).expect("plan");
        let admitted = admit(&plan.dag).expect("admitted");
        let csv = csv_with(&(0..1_000).map(|i| i as f64).collect::<Vec<_>>());

        let outcome = run(&plan, &admitted, &csv_config(csv.path(), 0, false)).expect("run");

        assert!(!outcome.verified);
        let readout = &outcome.readouts[0];
        // None, not 0.0 — "not computed" must never read as "exact".
        assert!(readout.exact.is_none());
        assert!(readout.observed_error.measured().is_none());
        assert!(outcome.exact.update.is_empty());
        assert!(outcome.exact.readout.is_empty());
        assert_eq!(outcome.retained_values, 0);
    }

    #[test]
    fn an_exact_accumulator_plan_needs_no_sketch_and_still_reports_both_arms() {
        let plan = plan_promql("sum(cpu_cores)", AccuracyTarget::Epsilon(0.01)).expect("plan");
        assert_eq!(
            plan.dag.nodes.len(),
            2,
            "Fallback -> SummaryAgg, no estimate"
        );

        let admitted = admit(&plan.dag).expect("admitted");
        let values: Vec<f64> = (1..=100).map(|i| i as f64).collect();
        let csv = csv_with(&values);
        let outcome = run(&plan, &admitted, &csv_config(csv.path(), 0, true)).expect("run");

        assert_eq!(outcome.rows_emitted, 100);
        // No SummaryEstimate node, so no readout — the accumulator's state is
        // the answer, reached through FinalizeExactAccumulator.
        assert!(outcome.readouts.is_empty());
        assert_eq!(outcome.node_footprints.len(), 1);
        assert_eq!(outcome.retained_values, 100);
        assert!(outcome.exact.readout.is_empty(), "nothing was read out");
    }

    /// `count(...)` is one of the most basic queries in the corpus, and the
    /// planner compiles it to a CMS read through a bare-bucket-total readout
    /// rather than to an exact accumulator.
    #[test]
    fn the_count_plan_admits_runs_and_agrees_with_the_exact_arm() {
        let plan = plan_promql("count(cpu_cores)", AccuracyTarget::Epsilon(0.01)).expect("plan");
        let admitted = match crate::admit::admit(&plan.dag) {
            Ok(admitted) => admitted,
            Err(refusals) => panic!("count(cpu_cores) was refused: {refusals:?}"),
        };

        let values: Vec<f64> = (0..5_000).map(|i| (i % 37) as f64).collect();
        let csv = csv_with(&values);
        let outcome = run(&plan, &admitted, &csv_config(csv.path(), 7, true)).expect("run");

        assert_eq!(outcome.readouts.len(), 1);
        let readout = &outcome.readouts[0];
        assert_eq!(
            readout.query,
            SketchQuery::PointCount {
                key: asap_types::pre_asap::ColumnRef::SampleValue,
                value: None,
            }
        );
        assert_eq!(readout.approximate, Answer::Scalar(5_000.0));
        assert_eq!(readout.exact, Some(5_000.0));
        // The state it cost, and that it is nothing like the retained column.
        assert_eq!(outcome.node_footprints.len(), 1);
        assert!(outcome.node_footprints[0].1 > 0);
    }

    #[test]
    fn a_bare_selector_plan_has_no_summary_at_all() {
        let plan = plan_promql("cpu_cores", AccuracyTarget::Epsilon(0.01)).expect("plan");
        assert_eq!(plan.dag.nodes.len(), 1);
        assert!(plan.dag.edges.is_empty());

        let admitted = admit(&plan.dag).expect("admitted");
        let csv = csv_with(&[1.0, 2.0, 3.0]);
        let outcome = run(&plan, &admitted, &csv_config(csv.path(), 0, true)).expect("run");

        assert_eq!(outcome.rows_emitted, 3);
        assert!(outcome.readouts.is_empty());
        assert!(outcome.node_footprints.is_empty(), "nothing was summarized");
        assert!(outcome.approximate.update.is_empty());
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
            let outcome = run(&plan, &admitted, &csv_config(csv.path(), seed, true)).expect("run");
            let readout = &outcome.readouts[0];
            let claimed = readout
                .guarantee
                .as_ref()
                .unwrap()
                .bound
                .evaluate()
                .unwrap();
            let observed = readout.observed_error.measured().unwrap();
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
