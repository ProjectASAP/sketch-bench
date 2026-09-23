//! Executes a post-ASAP plan over rows, holding one summary per
//! `(plan, node, group)`, and runs the other two arms over the same rows so
//! every comparison comes out of one pass.
//!
//! The exact arm is not a second implementation of the query: a `Fallback`
//! leaf is a bare `Scan`, so "execute exactly" is "keep the column the sketch
//! was fed and compute the statistic on it". That is why no query engine is
//! needed here, and why the two arms cannot silently disagree about which rows
//! they saw. It is also why it is ground truth and not a baseline: it measures
//! a push into a `Vec`, not the query anyone would have run. The baseline the
//! ratios divide by is the third arm, the pre-ASAP tree in `exact.rs`, timed
//! here by the same phase machinery.

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
    DataPrimitive, EdgeRole, EntityIdentity, ExecutableDag, ExecutableDagEdge, ExecutableDagNode,
    ExecutableOperatorPayload, ExecutionDataState, GroupingStrategy, PostAsapNodeId,
    ResultGuarantee, SketchQuery, SummaryFamilyType, SummaryInputExpr, SummarySchema,
    SummaryUpdate, ValueOperation, WindowEdgeCompatibility,
};
use asap_types::pre_asap::{ColumnRef, DataType, QueryExpr, Reduction};

use crate::df::RefusalCounts;
use crate::exact;
use crate::handle::{bind, SummaryHandle};
use crate::plan::{Plan, TimeRangeOrigin};
use crate::rows;
use crate::rows::{check_predicate, resolve_column, variant_name};
use crate::score;
use crate::score::ObservedError;
use crate::types::{Answer, EvalError, GroupKey, ItemKey, Refusal, Retained, Row, Value};
use crate::value;

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
    pub pre_asap: bool,
    pub per_node_time: bool,
    pub timed_runs: usize,
    pub warmup_runs: usize,
}

impl RunConfig {
    pub fn new(rows: RowsFrom, seed: u64, verify: bool) -> Self {
        Self {
            rows,
            seed,
            verify,
            pre_asap: true,
            per_node_time: false,
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
    pub query: Option<SketchQuery>,
    pub approximate: Answer,
    /// `None` when `verify` was off — never 0.0, which would read as "exact
    /// and the sketch was perfect".
    pub exact: Option<Answer>,
    pub observed_error: ObservedError,
    pub guarantee: Option<ResultGuarantee>,
    pub observations: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Runtime {
    #[default]
    Interpreter,
    DataFusion,
}

impl Runtime {
    pub fn tag(&self) -> &'static str {
        match self {
            Runtime::Interpreter => "interp",
            Runtime::DataFusion => "datafusion",
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct ArmTiming {
    pub build: Vec<RunMetrics>,
    pub update: Vec<RunMetrics>,
    pub readout: Vec<RunMetrics>,
    pub evaluate: Vec<RunMetrics>,
    pub maintenance: Vec<RunMetrics>,
    pub read: Vec<RunMetrics>,
    pub engine_overhead_ns: Option<u64>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct NodeTiming {
    pub build_ns: Option<u64>,
    pub update_ns: Option<u64>,
    pub readout_ns: Option<u64>,
    pub elapsed_compute_ns: Option<u64>,
}

#[derive(Debug, Clone)]
pub struct RunOutcome {
    pub runtime: Runtime,
    pub refusals: RefusalCounts,
    pub rows_scanned: u64,
    pub rows_emitted: Option<u64>,
    /// Rows the root node produced, for a plan whose answer is rows rather
    /// than a readout. `None` when the root is not a row-producing node.
    pub root_rows: Option<usize>,
    pub verified: bool,
    pub no_summary_in_plan: bool,
    /// Values the exact arm held, summed over every `(node, group)`. Counted
    /// here rather than derived from `readouts`, which are empty for a plan
    /// whose state *is* its answer (an exact accumulator has no
    /// `SummaryEstimate`) even though the exact arm still retained a column.
    pub retained_values: usize,
    pub retained_bytes: usize,
    pub readouts: Vec<Readout>,
    /// Summary state held per node, summed over that node's groups.
    pub node_footprints: Vec<(PostAsapNodeId, usize)>,
    pub node_times: Vec<(PostAsapNodeId, NodeTiming)>,
    pub approximate: ArmTiming,
    pub pre_asap: ArmTiming,
    pub pre_asap_bytes: usize,
    pub pre_asap_node_times: Vec<exact::NodeTime>,
    pub pre_asap_answer: Option<exact::Data>,
    pub maintenance_peak_bytes: Option<usize>,
    pub read_peak_bytes: Option<usize>,
    pub pre_asap_evaluate_peak_bytes: Option<usize>,
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
}

type SummaryState = Vec<Box<dyn SummaryHandle>>;
type RetainedState = Vec<Retained>;
type Fault = Rc<RefCell<Option<EvalError>>>;
type Kept<T> = Rc<RefCell<Vec<T>>>;
type PerSlotNs = Vec<Vec<u64>>;
type UpdateTiming = (Vec<RunMetrics>, Vec<SummaryState>, PerSlotNs);
type EstimateTiming = (Vec<RunMetrics>, Vec<Answer>, PerSlotNs);

/// Run one plan: resolve every node on its own payload, then execute.
///
/// The row-producing nodes are materialized first, in topological order, so a
/// `SummaryAgg` reads the rows its own producer emitted rather than the plan's
/// row source.
pub fn run(plan: &Plan, cfg: &RunConfig) -> Result<RunOutcome, EvalError> {
    let dag = &plan.dag;
    let resolved = resolve(dag, &plan.order, plan.time_range_origin).map_err(EvalError::Refused)?;
    if resolved.nodes() != plan.order.len() {
        return Err(EvalError::Validation(format!(
            "{} of the plan's {} nodes resolved into something to execute",
            resolved.nodes(),
            plan.order.len()
        )));
    }

    let materialized = materialize(&resolved, cfg)?;

    // ── the aggregates, grouped by the rows they consume ────────────────────
    //
    // Row-major within a group, as one shared source used to be: the update
    // order is what the timed insert phase replays.
    let mut consumers: Vec<(PostAsapNodeId, Vec<&ResolvedAggregate<'_>>)> = Vec::new();
    for aggregate in &resolved.aggregates {
        if !materialized.rows.contains_key(&aggregate.producer) {
            return Err(EvalError::Validation(format!(
                "node {:?} consumes {:?}, which produced no rows",
                aggregate.node, aggregate.producer
            )));
        }
        match consumers
            .iter_mut()
            .find(|(held, _)| *held == aggregate.producer)
        {
            Some((_, specs)) => specs.push(aggregate),
            None => consumers.push((aggregate.producer, vec![aggregate])),
        }
    }

    let drained = drain(&consumers, &materialized)?;

    let mut probes: Vec<Probe> = Vec::new();
    for readout in &resolved.readouts {
        let mut of_producer: Vec<(u32, &Slot)> = drained
            .slots
            .iter()
            .enumerate()
            .filter(|(_, slot)| slot.node == readout.producer)
            .map(|(i, slot)| (i as u32, slot))
            .collect();
        of_producer.sort_by(|a, b| a.1.group.cmp(&b.1.group));

        for (slot, held) in of_producer {
            probes.push(Probe {
                node: readout.node,
                producer: readout.producer,
                slot,
                group: held.group.clone(),
                query: readout.query.clone(),
                guarantee: readout.guarantee.clone(),
            });
        }
    }
    let probes = Rc::new(probes);

    let (build, build_ns) = time_binds(&drained.slots, cfg)?;
    let (update, mut states, update_ns) = time_updates(&drained.slots, &drained.updates, cfg)?;

    let mut node_footprints: HashMap<PostAsapNodeId, usize> = HashMap::new();
    if let Some(last) = states.last() {
        for (slot, handle) in drained.slots.iter().zip(last.iter()) {
            *node_footprints.entry(slot.node).or_insert(0) += handle.footprint_bytes();
        }
    }
    let mut node_footprints: Vec<_> = node_footprints.into_iter().collect();
    node_footprints.sort_by_key(|(id, _)| id.0);

    let (readout, answers, readout_ns) = time_estimates(&probes, std::mem::take(&mut states), cfg)?;
    let node_times = node_times(&drained.slots, &build_ns, &update_ns, &readout_ns);

    let mut retained_values = 0usize;
    let mut retained_bytes_held = 0usize;
    let mut truths: Vec<Answer> = Vec::new();
    let mut retained: RetainedState = Vec::new();
    if cfg.verify {
        let columns = retain(drained.slots.len(), &drained.updates);
        retained_values = columns.iter().map(Retained::len).sum();
        retained_bytes_held = retained_bytes(&columns) as usize;
        let (answered, columns) = exact_answers(&probes, columns)?;
        truths = answered;
        retained = columns;
    }

    let mut pre_asap = ArmTiming::default();
    let mut pre_asap_bytes = 0usize;
    let mut pre_asap_node_times = Vec::new();
    let mut pre_asap_answer = None;
    if let (true, Some(root)) = (cfg.pre_asap, plan.pre_asap.as_ref()) {
        let arm = exact::time_pre_asap(root, cfg, plan.time_range_origin)?;
        pre_asap.evaluate = arm.evaluate;
        pre_asap_bytes = arm.retained_bytes;
        pre_asap_node_times = arm.node_times;
        pre_asap_answer = Some(arm.answer);
    }

    // ── read every estimate out, and answer the same question exactly ───────
    let mut readouts = Vec::with_capacity(probes.len());
    for (i, probe) in probes.iter().enumerate() {
        let approximate = answers.get(i).cloned().ok_or_else(|| {
            EvalError::Validation(format!("readout {:?} produced no answer", probe.node))
        })?;
        let (exact_value, observed_error) = match truths.get(i) {
            Some(truth) => {
                let empty = Retained::default();
                let column = retained.get(probe.slot as usize).unwrap_or(&empty);
                let err = score::observed_error(
                    probe.guarantee.as_ref(),
                    column,
                    &probe.query,
                    &approximate,
                    truth,
                );
                (Some(truth.clone()), err)
            }
            None => (None, ObservedError::NotVerified),
        };

        readouts.push(Readout {
            node: probe.node,
            producer: probe.producer,
            group: probe.group.clone(),
            query: Some(probe.query.clone()),
            approximate,
            exact: exact_value,
            observed_error,
            guarantee: probe.guarantee.clone(),
            observations: drained.slots[probe.slot as usize].observations,
        });
    }

    Ok(RunOutcome {
        runtime: Runtime::Interpreter,
        refusals: RefusalCounts::default(),
        rows_scanned: materialized.scanned,
        rows_emitted: Some(materialized.emitted),
        root_rows: materialized.rows.get(&dag.root).map(|rows| rows.len()),
        verified: cfg.verify,
        no_summary_in_plan: crate::df::split::no_summary_in_plan(dag),
        retained_values,
        retained_bytes: retained_bytes_held,
        readouts,
        node_footprints,
        node_times,
        approximate: ArmTiming {
            build,
            update,
            readout,
            ..ArmTiming::default()
        },
        pre_asap,
        pre_asap_bytes,
        pre_asap_node_times,
        pre_asap_answer,
        maintenance_peak_bytes: None,
        read_peak_bytes: None,
        pre_asap_evaluate_peak_bytes: None,
    })
}

fn drain(
    consumers: &[(PostAsapNodeId, Vec<&ResolvedAggregate<'_>>)],
    materialized: &Materialized,
) -> Result<Drained, EvalError> {
    let mut slots: Vec<Slot> = Vec::new();
    let mut index: HashMap<(PostAsapNodeId, GroupKey), u32> = HashMap::new();
    let mut updates: Vec<Update> = Vec::new();

    for (producer, specs) in consumers {
        let rows = &materialized.rows[producer];
        for row in rows.iter() {
            for spec in specs {
                let group = group_key(row, &spec.group_columns)?;
                let slot = match index.entry((spec.node, group)) {
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
                let weight = resolve_weight(&spec.weight, row)?;
                let item = match &spec.item {
                    Some(input) => Some(resolve_item(input, row)?),
                    None => None,
                };
                slots[slot as usize].observations += 1;
                updates.push(Update { slot, item, weight });
            }
        }
    }

    Ok(Drained {
        slots,
        updates: Rc::new(updates),
    })
}

struct Materialized {
    rows: HashMap<PostAsapNodeId, Rc<Vec<Row>>>,
    scanned: u64,
    emitted: u64,
}

/// Every node that produces rows, in topological order.
///
/// `Fallback` opens the run's row set; a `Value` transforms the rows its own
/// producer emitted. Both timings are materialized here: with the manifest's
/// one-shot whole-input evaluation there is one pass, so a maintenance-time
/// row operation and a read-time one see the same rows.
fn materialize(resolved: &Resolved<'_>, cfg: &RunConfig) -> Result<Materialized, EvalError> {
    let mut rows: HashMap<PostAsapNodeId, Rc<Vec<Row>>> = HashMap::new();
    let mut scanned = 0;
    let mut emitted = 0;
    let mut source: Option<PostAsapNodeId> = None;

    for step in &resolved.rows {
        match step {
            RowStep::Source { node, expression } => {
                if let Some(held) = source {
                    return Err(EvalError::RowSource(format!(
                        "the run manifest names one row set, and this plan reads two: \
                         {held:?} and {:?}",
                        node.id
                    )));
                }
                source = Some(node.id);
                let mut reader = match &cfg.rows {
                    RowsFrom::Csv(path) => rows::open(expression, path, &node.output_schema)?,
                    RowsFrom::Generated(table) => {
                        rows::open_generated(expression, Rc::clone(table), &node.output_schema)?
                    }
                };
                let mut held = Vec::new();
                for row in reader.by_ref() {
                    held.push(row?);
                }
                if let Some(metric) = reader.metric_absent() {
                    return Err(EvalError::Refused(vec![Refusal::MetricAbsentFromRows {
                        node: node.id,
                        metric: metric.to_string(),
                        detail: format!("{} names other series", reader.origin()),
                    }]));
                }
                scanned = reader.scanned();
                emitted = reader.emitted();
                rows.insert(node.id, Rc::new(held));
            }
            RowStep::Operation {
                node,
                producer,
                producer_columns,
                operation,
            } => {
                let Some(input) = rows.get(producer) else {
                    value::identity_over(*node, operation, *producer_columns)
                        .map_err(|refusal| EvalError::Refused(vec![refusal]))?;
                    continue;
                };
                let produced = value::apply(*node, operation, input)?;
                rows.insert(*node, produced);
            }
        }
    }

    Ok(Materialized {
        rows,
        scanned,
        emitted,
    })
}

pub(crate) fn phase_config(
    metric: Metric,
    extra: MetricsMask,
    cfg: &RunConfig,
) -> (usize, MeasureConfig) {
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

struct SlotClock {
    ns: Option<Vec<u64>>,
}

impl SlotClock {
    fn new(slots: usize, armed: bool) -> Self {
        Self {
            ns: armed.then(|| vec![0u64; slots]),
        }
    }

    fn armed(&self) -> bool {
        self.ns.is_some()
    }

    fn charge(&mut self, slot: usize, elapsed_ns: u64) {
        if let Some(held) = self.ns.as_mut() {
            if let Some(total) = held.get_mut(slot) {
                *total += elapsed_ns;
            }
        }
    }

    fn into_totals(self) -> Vec<u64> {
        self.ns.unwrap_or_default()
    }
}

fn timed_calls(
    recorder: &mut LatencyRecorder,
    clock: &mut SlotClock,
    slot_of: impl Fn(usize) -> usize,
    steps: usize,
    mut call: impl FnMut(usize),
) {
    for i in 0..steps {
        let watch = WallClock::start();
        call(i);
        let elapsed_ns = watch.elapsed_ns();
        recorder.record_ns(elapsed_ns);
        clock.charge(slot_of(i), elapsed_ns);
    }
}

fn node_times(
    slots: &[Slot],
    build: &[Vec<u64>],
    update: &[Vec<u64>],
    readout: &[Vec<u64>],
) -> Vec<(PostAsapNodeId, NodeTiming)> {
    let build = per_node(slots, build);
    let update = per_node(slots, update);
    let readout = per_node(slots, readout);

    let mut ids: Vec<PostAsapNodeId> = slots.iter().map(|slot| slot.node).collect();
    ids.sort_by_key(|id| id.0);
    ids.dedup();

    ids.into_iter()
        .map(|id| {
            (
                id,
                NodeTiming {
                    build_ns: build.get(&id).copied(),
                    update_ns: update.get(&id).copied(),
                    readout_ns: readout.get(&id).copied(),
                    elapsed_compute_ns: None,
                },
            )
        })
        .filter(|(_, timing)| *timing != NodeTiming::default())
        .collect()
}

fn per_node(slots: &[Slot], passes: &[Vec<u64>]) -> HashMap<PostAsapNodeId, u64> {
    let mut totals: HashMap<PostAsapNodeId, u64> = HashMap::new();
    if passes.is_empty() {
        return totals;
    }
    for pass in passes {
        for (slot, elapsed_ns) in slots.iter().zip(pass) {
            *totals.entry(slot.node).or_insert(0) += elapsed_ns;
        }
    }
    for total in totals.values_mut() {
        *total /= passes.len() as u64;
    }
    totals
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
    columns.iter().map(|column| column.bytes() as u64).sum()
}

fn bind_all(slots: &[Slot], seed: u64) -> Result<SummaryState, EvalError> {
    slots
        .iter()
        .map(|slot| {
            bind(&slot.family, slot.node, seed).map_err(|refusal| EvalError::Refused(vec![refusal]))
        })
        .collect()
}

fn time_binds(slots: &[Slot], cfg: &RunConfig) -> Result<(Vec<RunMetrics>, PerSlotNs), EvalError> {
    if slots.is_empty() {
        return Ok((Vec::new(), Vec::new()));
    }
    let fault: Fault = Rc::new(RefCell::new(None));
    let kept: Kept<(SummaryState, LatencyRecorder, Vec<u64>)> = Rc::new(RefCell::new(Vec::new()));
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
        let clock = SlotClock::new(recipes.len(), cfg.per_node_time);
        let pass: Pass = Box::new(move || {
            let mut built = built;
            let mut recorder = recorder;
            let mut clock = clock;
            let mut failure: Option<EvalError> = None;
            timed_calls(
                &mut recorder,
                &mut clock,
                |i| i,
                recipes.len(),
                |i| match bind(&recipes[i].0, recipes[i].1, seed) {
                    Ok(handle) => built.push(handle),
                    Err(refusal) => {
                        if failure.is_none() {
                            failure = Some(EvalError::Refused(vec![refusal]));
                        }
                    }
                },
            );
            let work = recipes.len() as u64;
            let report: Report = Box::new(move || {
                if let Some(err) = failure {
                    hold(&fault, err);
                }
                kept.borrow_mut()
                    .push((built, recorder, clock.into_totals()));
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
    let mut per_slot = Vec::with_capacity(measured);
    for (metrics, (built, recorder, slot_ns)) in runs.iter_mut().zip(measured_tail(&kept, measured))
    {
        metrics.memory_bytes = Some(state_bytes(built));
        metrics.latency_ns = Some(recorder.snapshot());
        if !slot_ns.is_empty() {
            per_slot.push(slot_ns.clone());
        }
    }
    Ok((runs, per_slot))
}

fn time_updates(
    slots: &[Slot],
    updates: &Rc<Vec<Update>>,
    cfg: &RunConfig,
) -> Result<UpdateTiming, EvalError> {
    if slots.is_empty() {
        return Ok((Vec::new(), Vec::new(), Vec::new()));
    }
    let fault: Fault = Rc::new(RefCell::new(None));
    let filled: Kept<(SummaryState, Vec<u64>)> = Rc::new(RefCell::new(Vec::new()));

    let (total, config) = phase_config(Metric::Throughput, MetricsMask::empty(), cfg);
    let mut passes: Measurement = Vec::with_capacity(total);
    for _ in 0..total {
        let handles = bind_all(slots, cfg.seed)?;
        let updates = Rc::clone(updates);
        let fault = Rc::clone(&fault);
        let filled = Rc::clone(&filled);
        let clock = SlotClock::new(slots.len(), cfg.per_node_time);
        let pass: Pass = Box::new(move || {
            let mut handles = handles;
            let mut clock = clock;
            let mut failure: Option<EvalError> = None;
            if clock.armed() {
                for update in updates.iter() {
                    let watch = WallClock::start();
                    let outcome =
                        handles[update.slot as usize].update(update.item.as_ref(), update.weight);
                    clock.charge(update.slot as usize, watch.elapsed_ns());
                    if let Err(err) = outcome {
                        failure = Some(err);
                        break;
                    }
                }
            } else {
                for update in updates.iter() {
                    if let Err(err) =
                        handles[update.slot as usize].update(update.item.as_ref(), update.weight)
                    {
                        failure = Some(err);
                        break;
                    }
                }
            }
            let work = updates.len() as u64;
            let report: Report = Box::new(move || {
                if let Some(err) = failure {
                    hold(&fault, err);
                }
                filled.borrow_mut().push((handles, clock.into_totals()));
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
    let kept = std::mem::take(&mut *filled.borrow_mut());
    let measured = runs.len();
    let mut per_slot = Vec::with_capacity(measured);
    for (metrics, (handles, slot_ns)) in runs.iter_mut().zip(measured_tail(&kept, measured)) {
        metrics.memory_bytes = Some(state_bytes(handles));
        if !slot_ns.is_empty() {
            per_slot.push(slot_ns.clone());
        }
    }
    let states = kept.into_iter().map(|(handles, _)| handles).collect();
    Ok((runs, states, per_slot))
}

fn time_estimates(
    probes: &Rc<Vec<Probe>>,
    states: Vec<SummaryState>,
    cfg: &RunConfig,
) -> Result<EstimateTiming, EvalError> {
    if probes.is_empty() || states.is_empty() {
        return Ok((Vec::new(), Vec::new(), Vec::new()));
    }
    let fault: Fault = Rc::new(RefCell::new(None));
    let kept: Kept<(Vec<Answer>, SummaryState, LatencyRecorder, Vec<u64>)> =
        Rc::new(RefCell::new(Vec::new()));

    let (_, config) = phase_config(Metric::Throughput, MetricsMask::LATENCY, cfg);
    let mut passes: Measurement = Vec::with_capacity(states.len());
    let slots: Rc<Vec<usize>> = Rc::new(probes.iter().map(|probe| probe.slot as usize).collect());
    for state in states {
        let probes = Rc::clone(probes);
        let fault = Rc::clone(&fault);
        let kept = Rc::clone(&kept);
        let slots = Rc::clone(&slots);
        let produced: Vec<Answer> = Vec::with_capacity(probes.len());
        let recorder = LatencyRecorder::new();
        let clock = SlotClock::new(state.len(), cfg.per_node_time);
        let pass: Pass = Box::new(move || {
            let mut handles = state;
            let mut produced = produced;
            let mut recorder = recorder;
            let mut clock = clock;
            let mut failure: Option<EvalError> = None;
            timed_calls(
                &mut recorder,
                &mut clock,
                |i| slots[i],
                probes.len(),
                |i| {
                    let probe = &probes[i];
                    match handles[probe.slot as usize].estimate(&probe.query) {
                        Ok(answer) => produced.push(answer),
                        Err(err) => {
                            if failure.is_none() {
                                failure = Some(err);
                            }
                        }
                    }
                },
            );
            let work = probes.len() as u64;
            let report: Report = Box::new(move || {
                if let Some(err) = failure {
                    hold(&fault, err);
                }
                kept.borrow_mut()
                    .push((produced, handles, recorder, clock.into_totals()));
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
    let mut per_slot = Vec::with_capacity(measured);
    for (metrics, (_, handles, recorder, slot_ns)) in
        runs.iter_mut().zip(measured_tail(&kept, measured))
    {
        metrics.memory_bytes = Some(state_bytes(handles));
        metrics.latency_ns = Some(recorder.snapshot());
        if !slot_ns.is_empty() {
            per_slot.push(slot_ns.clone());
        }
    }
    let answers = kept
        .pop()
        .map(|(answers, _, _, _)| answers)
        .unwrap_or_default();
    Ok((runs, answers, per_slot))
}

fn retain(slot_count: usize, updates: &Rc<Vec<Update>>) -> RetainedState {
    let mut columns: RetainedState = vec![Retained::default(); slot_count];
    for update in updates.iter() {
        columns[update.slot as usize].push(update.item.as_ref(), update.weight);
    }
    columns
}

fn exact_answers(
    probes: &Rc<Vec<Probe>>,
    mut columns: RetainedState,
) -> Result<(Vec<Answer>, RetainedState), EvalError> {
    let mut answered = Vec::with_capacity(probes.len());
    for probe in probes.iter() {
        let column = &mut columns[probe.slot as usize];
        if score::needs_a_sorted_column(&probe.query) {
            column.weights.sort_by(f64::total_cmp);
        }
        answered.push(score::exact_answer(column, &probe.query)?);
    }
    Ok((answered, columns))
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
        ResolvedInput::Tuple(parts) => {
            let mut key = String::new();
            for (i, part) in parts.iter().enumerate() {
                if i > 0 {
                    key.push(';');
                }
                key.push_str(&resolve_item(part, row)?.rendered());
            }
            Ok(ItemKey::Str(key))
        }
        other => Err(EvalError::RowSource(format!(
            "item must be a column reference, found {other:?}"
        ))),
    }
}

/// The operators a v0 run can encounter, for a caller that wants to report
/// coverage without re-deriving it from the payloads.
pub fn operators(dag: &ExecutableDag) -> Vec<(PostAsapNodeId, &'static str)> {
    dag.nodes
        .iter()
        .map(|node| (node.id, operator_name(&node.payload)))
        .collect()
}

pub fn operator_name(payload: &ExecutableOperatorPayload) -> &'static str {
    match payload {
        ExecutableOperatorPayload::Fallback { .. } => "Fallback",
        ExecutableOperatorPayload::Binary { .. } => "Binary",
        ExecutableOperatorPayload::CandidateTopK { .. } => "CandidateTopK",
        ExecutableOperatorPayload::Value { .. } => "Value",
        ExecutableOperatorPayload::RelationalJoin { .. } => "RelationalJoin",
        ExecutableOperatorPayload::SummaryAgg { .. } => "SummaryAgg",
        ExecutableOperatorPayload::SummaryJoin { .. } => "SummaryJoin",
        ExecutableOperatorPayload::SummarySubtract => "SummarySubtract",
        ExecutableOperatorPayload::SummaryDelete { .. } => "SummaryDelete",
        ExecutableOperatorPayload::SummaryEstimate { .. } => "SummaryEstimate",
        ExecutableOperatorPayload::SummaryMerge => "SummaryMerge",
    }
}

// ── Resolution ───────────────────────────────────────────────────────────────
//
// One walk, one arm per payload: either the node resolves into what the run
// executes, or the arm says why it cannot. Every node of the plan lands in
// exactly one of the three lists below, so a payload that grew an arm and a
// payload that did not are the same kind of answer.

/// A `SummaryInputExpr` with every column reference resolved to a position.
#[derive(Debug, Clone, PartialEq)]
pub enum ResolvedInput {
    Constant(f64),
    /// Position in the producer's output schema.
    Column(usize),
    Tuple(Vec<ResolvedInput>),
}

#[derive(Debug)]
pub(crate) struct Resolved<'a> {
    rows: Vec<RowStep<'a>>,
    aggregates: Vec<ResolvedAggregate<'a>>,
    readouts: Vec<ResolvedReadout>,
}

impl Resolved<'_> {
    fn nodes(&self) -> usize {
        self.rows.len() + self.aggregates.len() + self.readouts.len()
    }
}

#[derive(Debug)]
enum RowStep<'a> {
    Source {
        node: &'a ExecutableDagNode,
        expression: &'a QueryExpr,
    },
    Operation {
        node: PostAsapNodeId,
        producer: PostAsapNodeId,
        producer_columns: usize,
        operation: &'a ValueOperation,
    },
}

#[derive(Debug)]
pub(crate) struct ResolvedAggregate<'a> {
    node: PostAsapNodeId,
    producer: PostAsapNodeId,
    family: &'a SummaryFamilyType,
    item: Option<ResolvedInput>,
    weight: ResolvedInput,
    group_columns: Vec<usize>,
}

#[derive(Debug)]
struct ResolvedReadout {
    node: PostAsapNodeId,
    producer: PostAsapNodeId,
    query: SketchQuery,
    guarantee: Option<ResultGuarantee>,
}

/// Resolve every node of `dag`, in `order`, into what the run executes.
///
/// Refusals are collected rather than returned on the first one: a node that
/// cannot run is one fact about the plan, and a sweep wants all of them.
pub(crate) fn resolve<'a>(
    dag: &'a ExecutableDag,
    order: &[PostAsapNodeId],
    origin: TimeRangeOrigin,
) -> Result<Resolved<'a>, Vec<Refusal>> {
    let by_id: HashMap<PostAsapNodeId, &ExecutableDagNode> =
        dag.nodes.iter().map(|node| (node.id, node)).collect();
    let mut refusals: Vec<Refusal> = Vec::new();
    let mut resolved = Resolved {
        rows: Vec::new(),
        aggregates: Vec::new(),
        readouts: Vec::new(),
    };

    for id in order {
        let Some(node) = by_id.get(id).copied() else {
            refusals.push(Refusal::UnsupportedOperator {
                node: *id,
                operator: "the execution order names a node the DAG does not carry".to_string(),
            });
            continue;
        };
        match &node.payload {
            ExecutableOperatorPayload::Fallback { expression } => {
                let expression = match crate::plan::ingestion_horizon_child(expression, origin) {
                    Some(child) => child.as_ref(),
                    None => expression,
                };
                match resolve_fallback(node, expression) {
                    Ok(()) => resolved.rows.push(RowStep::Source { node, expression }),
                    Err(refusal) => refusals.push(refusal),
                }
            }
            ExecutableOperatorPayload::Value { operation, .. } => {
                match input_edge(node, dag).and_then(|(producer, schema)| {
                    value::check(node.id, operation, schema.fields.len())
                        .map(|()| (producer.id, schema.fields.len()))
                }) {
                    Ok((producer, producer_columns)) => resolved.rows.push(RowStep::Operation {
                        node: node.id,
                        producer,
                        producer_columns,
                        operation,
                    }),
                    Err(refusal) => refusals.push(refusal),
                }
            }
            ExecutableOperatorPayload::SummaryAgg {
                family,
                input,
                reduction,
                grouping,
            } => match resolve_aggregate(node, dag, family, input, reduction, grouping) {
                Ok(aggregate) => resolved.aggregates.push(aggregate),
                Err(refusal) => refusals.push(refusal),
            },
            ExecutableOperatorPayload::SummaryEstimate { query } => {
                match resolve_readout(node, dag, query) {
                    Ok(readout) => resolved.readouts.push(readout),
                    Err(refusal) => refusals.push(refusal),
                }
            }
            other => refusals.push(operator_refusal(node.id, other)),
        }
    }

    if refusals.is_empty() {
        Ok(resolved)
    } else {
        Err(refusals)
    }
}

/// The payloads with no execution arm, refused by name.
///
/// Driven off the payload, which upstream made the sole operator identity: a
/// payload is what would actually be executed.
fn operator_refusal(node: PostAsapNodeId, payload: &ExecutableOperatorPayload) -> Refusal {
    let operator = match payload {
        ExecutableOperatorPayload::Fallback { .. }
        | ExecutableOperatorPayload::Value { .. }
        | ExecutableOperatorPayload::SummaryAgg { .. }
        | ExecutableOperatorPayload::SummaryEstimate { .. } => {
            unreachable!("every executed payload has its own arm")
        }

        // No sketch in asap_sketchlib has an inverse. CMS could technically
        // carry negative counters, but its min estimator does not hold once it
        // does — a semantics problem, not a wiring problem. Permanent.
        ExecutableOperatorPayload::SummarySubtract => {
            return Refusal::NoInverseOperation {
                node,
                operator: "SummarySubtract".to_string(),
            }
        }
        // Same missing capability, plus `key: ColumnRef` carries no weight, so
        // the magnitude of a deletion is undefined. Permanent.
        ExecutableOperatorPayload::SummaryDelete { .. } => {
            return Refusal::NoInverseOperation {
                node,
                operator: "SummaryDelete".to_string(),
            }
        }

        // Theta/KMV join cardinality; asap_sketchlib has no Theta at all.
        ExecutableOperatorPayload::SummaryJoin { .. } => "SummaryJoin",
        ExecutableOperatorPayload::CandidateTopK { .. } => "CandidateTopK",
        ExecutableOperatorPayload::Binary { .. } => "Binary",
        ExecutableOperatorPayload::RelationalJoin { .. } => "RelationalJoin",
        ExecutableOperatorPayload::SummaryMerge => "SummaryMerge",
    };
    Refusal::UnsupportedOperator {
        node,
        operator: operator.to_string(),
    }
}

// ── Fallback ─────────────────────────────────────────────────────────────────

fn resolve_fallback(node: &ExecutableDagNode, expression: &QueryExpr) -> Result<(), Refusal> {
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

fn resolve_aggregate<'a>(
    node: &ExecutableDagNode,
    dag: &'a ExecutableDag,
    family: &'a SummaryFamilyType,
    input: &SummaryUpdate,
    reduction: &Reduction,
    grouping: &GroupingStrategy,
) -> Result<ResolvedAggregate<'a>, Refusal> {
    let (producer, input_schema) = input_edge(node, dag)?;

    // Ask the state binding table before any data is read. Without this a
    // `Kll{k:65535}`, a `Theta`, or the planner's own `Cms{width:272,depth:5}`
    // resolves clean, the CSV is opened, and the refusal arrives at row
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

    let item = match &input.item {
        Some(item) => Some(resolve_input(node, item, input_schema)?),
        None => None,
    };
    let weight = resolve_input(node, &input.weight, input_schema)?;

    Ok(ResolvedAggregate {
        node: node.id,
        producer: producer.id,
        family,
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
        SummaryInputExpr::EntityIdentity(EntityIdentity::PromqlLabelSet { excluding }) => {
            Ok(ResolvedInput::Tuple(
                label_columns(node, schema, excluding)?
                    .into_iter()
                    .map(ResolvedInput::Column)
                    .collect(),
            ))
        }
        // Needs the previous sample for the same series, which a one-shot scan
        // of an unordered file does not have.
        SummaryInputExpr::ResetAwareCounterDelta { .. } => Err(Refusal::UnsupportedUpdate {
            node: node.id,
            detail: "ResetAwareCounterDelta needs per-series carry-over state".to_string(),
        }),
    }
}

fn label_columns(
    node: &ExecutableDagNode,
    schema: &SummarySchema,
    excluding: &[ColumnRef],
) -> Result<Vec<usize>, Refusal> {
    let sample_value = resolve_column(&ColumnRef::SampleValue, schema);
    let mut excluded: Vec<usize> = Vec::with_capacity(excluding.len());
    for column in excluding {
        match resolve_column(column, schema) {
            Some(position) => excluded.push(position),
            None => {
                return Err(Refusal::UnresolvableColumn {
                    node: node.id,
                    column: column_ref_name(column),
                    detail: "EntityIdentity excludes a column the producer's schema lacks"
                        .to_string(),
                })
            }
        }
    }

    let labels: Vec<usize> = schema
        .fields
        .iter()
        .enumerate()
        .filter(|(position, field)| {
            !matches!(field.dtype, SummaryFamilyType::Plain(DataType::Timestamp))
                && Some(*position) != sample_value
                && !excluded.contains(position)
        })
        .map(|(position, _)| position)
        .collect();

    if labels.is_empty() {
        return Err(Refusal::UnsupportedUpdate {
            node: node.id,
            detail: format!(
                "EntityIdentity over a schema with no label column: fields {:?}, excluding {:?}",
                schema
                    .fields
                    .iter()
                    .map(|field| field.name.as_str())
                    .collect::<Vec<_>>(),
                excluding
            ),
        });
    }
    Ok(labels)
}

fn column_ref_name(column: &ColumnRef) -> String {
    match column {
        ColumnRef::Named(name) => name.clone(),
        ColumnRef::Qualified { table, name } => format!("{table}.{name}"),
        ColumnRef::SampleValue => "SampleValue".to_string(),
        ColumnRef::Wildcard => "Wildcard".to_string(),
    }
}

// ── SummaryEstimate ──────────────────────────────────────────────────────────

fn resolve_readout(
    node: &ExecutableDagNode,
    dag: &ExecutableDag,
    query: &SketchQuery,
) -> Result<ResolvedReadout, Refusal> {
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
        // The bare bucket total, a per-item point lookup, the distinct count,
        // the two frequency moments and the top-k list are all answered
        // exactly from the retained `(item, weight)` stream, so the shape of
        // the readout is the exact arm's business rather than a reason to
        // decline the plan.
        SketchQuery::PointCount { .. }
        | SketchQuery::Cardinality
        | SketchQuery::FrequencyL2
        | SketchQuery::FrequencyEntropy
        | SketchQuery::TopK { .. } => {}
    }
    let producer = input_edge(node, dag)?.0;
    match &producer.payload {
        ExecutableOperatorPayload::SummaryAgg { family, .. } => {
            crate::handle::check_readout(family, query, node.id)?;
            Ok(ResolvedReadout {
                node: node.id,
                producer: producer.id,
                query: query.clone(),
                guarantee: node.guarantee.clone(),
            })
        }
        // Waving an unrecognized producer through would accept a readout that
        // `run` then finds no handle for, and resolved-but-unexecuted is
        // strictly worse than refused.
        other => Err(Refusal::UnsupportedOperator {
            node: node.id,
            operator: format!(
                "SummaryEstimate reading from a {}, which builds no summary",
                operator_name(other)
            ),
        }),
    }
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
            detail: format!("{} has no Input edge", operator_name(&node.payload)),
        })?;
    let producer = dag
        .nodes
        .iter()
        .find(|candidate| candidate.id == edge.producer)
        .ok_or_else(|| Refusal::UnsupportedUpdate {
            node: node.id,
            detail: format!("Input edge names {:?}, which is not a node", edge.producer),
        })?;
    if let Some(refusal) = window_refusal(edge, producer, node) {
        return Err(refusal);
    }
    // `edge.intermediate_schema == producer.output_schema` is one of
    // `validate()`'s checks; the producer's own copy is read here so execution
    // does not depend on having been handed a validated document.
    Ok((producer, &producer.output_schema))
}

fn window_refusal(
    edge: &ExecutableDagEdge,
    producer: &ExecutableDagNode,
    consumer: &ExecutableDagNode,
) -> Option<Refusal> {
    if edge.window != WindowEdgeCompatibility::RequiresAlignedPanePhaseOrExactWindowEdgeResidual {
        return None;
    }
    // SUMMARY -> SUMMARY only. On a ROWS -> SUMMARY edge the same stamp is
    // discharged by the manifest's one-shot evaluation: one pane covers the
    // whole input, so there is no pane phase to align.
    if producer.output_state == ExecutionDataState::MAINTENANCE_SUMMARY
        && consumer.output_state == ExecutionDataState::MAINTENANCE_SUMMARY
    {
        return Some(Refusal::UnmetWindowObligation {
            node: consumer.id,
            producer: producer.id,
        });
    }
    None
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
    fn a_topk_readout_is_scored_against_the_true_ranked_set() {
        use asap_types::post_asap::SummaryInputExpr;
        use asap_types::pre_asap::ColumnRef;

        let mut plan =
            plan_promql("quantile(0.5, cpu_cores)", AccuracyTarget::Epsilon(0.01)).expect("plan");
        for node in &mut plan.dag.nodes {
            match &mut node.payload {
                ExecutableOperatorPayload::SummaryAgg { family, input, .. } => {
                    *family = SummaryFamilyType::Sketch(
                        SketchKind::new(
                            SketchAlgorithm::CmsWithHeap,
                            SketchParams::CmsWithHeap {
                                width: 2048,
                                depth: 5,
                                heap_size: 32,
                            },
                        ),
                        GroupingStrategy::PerSubpopulationInstance,
                    );
                    input.item = Some(SummaryInputExpr::Column(ColumnRef::SampleValue));
                    input.weight = SummaryInputExpr::Constant(1.0);
                }
                ExecutableOperatorPayload::SummaryEstimate { query } => {
                    *query = SketchQuery::TopK { k: 5 };
                }
                _ => {}
            }
        }

        let mut values: Vec<f64> = Vec::new();
        for (value, count) in [(0.0, 100), (1.0, 90), (2.0, 80), (3.0, 70), (4.0, 60)] {
            values.extend(std::iter::repeat_n(value, count));
        }
        values.extend((0..200).map(|i| 10.0 + i as f64));

        let csv = csv_with(&values);
        let outcome = run(&plan, &csv_config(csv.path(), 5, true)).expect("run");

        let readout = &outcome.readouts[0];
        assert_eq!(
            readout.exact,
            Some(Answer::Ranked(vec![
                (ItemKey::Float(0.0), 100),
                (ItemKey::Float(1.0), 90),
                (ItemKey::Float(2.0), 80),
                (ItemKey::Float(3.0), 70),
                (ItemKey::Float(4.0), 60),
            ]))
        );
        let Answer::Ranked(estimated) = &readout.approximate else {
            panic!(
                "a heap family answers a ranked list: {:?}",
                readout.approximate
            );
        };
        let named: Vec<f64> = estimated
            .iter()
            .map(|(key, _)| match key {
                ItemKey::Float(value) => *value,
                other => panic!("{other:?}"),
            })
            .collect();
        assert_eq!(named, vec![0.0, 1.0, 2.0, 3.0, 4.0]);
    }

    #[test]
    fn the_topk_plan_runs_as_a_row_pipeline_instead_of_being_refused() {
        for (query, expected) in [("topk(5, cpu_cores)", 5usize), ("bottomk(3, cpu_cores)", 3)] {
            let plan = plan_promql(query, AccuracyTarget::Epsilon(0.01)).expect("plan");
            assert_eq!(
                plan.dag
                    .nodes
                    .iter()
                    .filter(|node| operator_name(&node.payload) == "Value")
                    .count(),
                2,
                "{query} compiles to Sort + Limit"
            );

            let values: Vec<f64> = (0..600).map(|i| (i % 97) as f64).collect();
            let csv = csv_with(&values);

            let outcome = run(&plan, &csv_config(csv.path(), 0, true)).expect("run");
            assert_eq!(outcome.root_rows, Some(expected), "{query}");
            assert_eq!(outcome.rows_scanned, 600, "{query}");
            assert!(
                outcome.readouts.is_empty(),
                "{query} holds no summary to read out"
            );
        }
    }

    #[test]
    fn an_entity_identity_item_counts_the_distinct_series_in_the_rows() {
        use asap_types::post_asap::{EntityIdentity, SummaryField, SummaryInputExpr};
        use asap_types::pre_asap::{Column, ColumnRef, DataType};

        let mut plan =
            plan_promql("quantile(0.5, cpu_cores)", AccuracyTarget::Epsilon(0.01)).expect("plan");
        let leaf = plan.dag.nodes[0].id;
        for node in &mut plan.dag.nodes {
            if node.id == leaf {
                node.output_schema.fields.push(SummaryField {
                    name: "cluster".to_string(),
                    dtype: SummaryFamilyType::Plain(DataType::Utf8),
                    nullable: false,
                });
                let asap_types::pre_asap::QueryExpr::Scan { schema, .. } =
                    crate::rows::tests::fallback_scan_mut(
                        &mut node.payload,
                        plan.time_range_origin,
                    )
                else {
                    panic!("the Fallback node lost its Scan");
                };
                schema.columns.push(Column {
                    name: "cluster".to_string(),
                    dtype: DataType::Utf8,
                    nullable: false,
                    table: None,
                });
                continue;
            }
            match &mut node.payload {
                ExecutableOperatorPayload::SummaryAgg { family, input, .. } => {
                    *family = SummaryFamilyType::Sketch(
                        SketchKind::new(SketchAlgorithm::Hll, SketchParams::Hll { precision: 14 }),
                        GroupingStrategy::PerSubpopulationInstance,
                    );
                    input.item = Some(SummaryInputExpr::EntityIdentity(
                        EntityIdentity::PromqlLabelSet { excluding: vec![] },
                    ));
                    input.weight = SummaryInputExpr::Column(ColumnRef::SampleValue);
                }
                ExecutableOperatorPayload::SummaryEstimate { query } => {
                    *query = SketchQuery::Cardinality;
                }
                _ => {}
            }
        }
        let schema = plan.dag.nodes[0].output_schema.clone();
        for edge in &mut plan.dag.edges {
            if edge.producer == leaf {
                edge.intermediate_schema = schema.clone();
            }
        }

        let csv = grouped_csv(12, 600);
        let outcome = run(&plan, &csv_config(csv.path(), 3, true)).expect("run");

        assert_eq!(
            outcome.readouts[0].exact,
            Some(Answer::Scalar(12.0)),
            "the identity is the cluster label, so the distinct count is the cluster count"
        );
    }

    #[test]
    fn a_cardinality_readout_is_scored_against_the_true_distinct_count() {
        use asap_types::post_asap::SummaryInputExpr;
        use asap_types::pre_asap::ColumnRef;

        let mut plan =
            plan_promql("quantile(0.5, cpu_cores)", AccuracyTarget::Epsilon(0.01)).expect("plan");
        for node in &mut plan.dag.nodes {
            match &mut node.payload {
                ExecutableOperatorPayload::SummaryAgg { family, input, .. } => {
                    *family = SummaryFamilyType::Sketch(
                        SketchKind::new(SketchAlgorithm::Hll, SketchParams::Hll { precision: 14 }),
                        GroupingStrategy::PerSubpopulationInstance,
                    );
                    input.item = Some(SummaryInputExpr::Column(ColumnRef::SampleValue));
                }
                ExecutableOperatorPayload::SummaryEstimate { query } => {
                    *query = SketchQuery::Cardinality;
                }
                _ => {}
            }
        }

        let values: Vec<f64> = (0..600).map(|i| (i % 97) as f64).collect();
        let csv = csv_with(&values);

        let outcome = run(&plan, &csv_config(csv.path(), 7, true)).expect("run");
        let readout = &outcome.readouts[0];
        assert_eq!(readout.exact, Some(Answer::Scalar(97.0)));
        let Answer::Scalar(estimate) = readout.approximate else {
            panic!("HLL answers a scalar: {:?}", readout.approximate);
        };
        assert!(
            (estimate - 97.0).abs() / 97.0 < 0.05,
            "estimate {estimate} is nowhere near the 97 distinct values"
        );
        assert!(
            outcome.retained_bytes > outcome.retained_values * std::mem::size_of::<f64>(),
            "the retained item keys are not in the reported footprint"
        );
    }

    #[test]
    fn readouts_come_out_in_the_same_order_every_run() {
        let plan = plan_promql(
            "quantile by (cluster) (0.5, cpu_cores)",
            AccuracyTarget::Epsilon(0.01),
        )
        .expect("plan");
        let csv = grouped_csv(12, 600);

        let groups = |seed: u64| -> Vec<String> {
            run(&plan, &csv_config(csv.path(), seed, true))
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

    #[test]
    fn a_null_weight_is_skipped_by_the_pre_asap_arm_and_refused_by_this_one() {
        use asap_types::pre_asap::{
            AggIntent, Column, DataType, QueryExpr, Reduction, Schema, Source,
        };
        use asap_types::types::AccuracyTarget;

        let rows = Rc::new(vec![
            Row(vec![Value::Timestamp(1), Value::Float(10.0)]),
            Row(vec![Value::Timestamp(2), Value::Null]),
            Row(vec![Value::Timestamp(3), Value::Float(30.0)]),
        ]);

        let tree = Rc::new(QueryExpr::Aggregate {
            reduction: Reduction::by(Vec::new()),
            measures: vec![
                AggIntent::Count {
                    accuracy: AccuracyTarget::Epsilon(0.01),
                },
                AggIntent::Sum { col: None },
            ],
            output_names: Vec::new(),
            having: None,
            child: Rc::new(QueryExpr::Scan {
                source: Source::TimeSeries {
                    metric: "cpu_cores".into(),
                },
                predicates: Vec::new(),
                schema: Schema {
                    columns: vec![
                        Column::new("ts", DataType::Timestamp, false),
                        Column::new("value", DataType::Float64, true),
                    ],
                    time_index: Some(0),
                    unique_keys: Vec::new(),
                    closed: false,
                },
            }),
        });

        let (answered, _) =
            crate::exact::evaluate(PostAsapNodeId(1), &tree, &rows, TimeRangeOrigin::Unknown)
                .expect("evaluates");
        match answered {
            crate::exact::Data::Rows(emitted) => {
                assert_eq!(
                    emitted,
                    Rc::new(vec![Row(vec![Value::Int(3), Value::Float(40.0)])])
                )
            }
            other => panic!("{other:?}"),
        }

        let refused = resolve_weight(&ResolvedInput::Column(1), &rows[1]).expect_err("refuses");
        assert!(
            refused
                .to_string()
                .contains("weight column 1 is not numeric"),
            "{refused}"
        );

        for kept in [&rows[0], &rows[2]] {
            resolve_weight(&ResolvedInput::Column(1), kept).expect("a numeric weight resolves");
        }
    }

    use super::*;
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
            Ok((runs, _, _)) => {
                panic!("a discarded pass swallowed the fault, and reported {runs:?}")
            }
        };
        assert!(matches!(err, EvalError::Refused(_)), "{err:?}");
    }

    #[test]
    fn a_fault_is_raised_when_more_than_one_pass_is_timed() {
        let slots = vec![cms_slot()];
        let err = match time_updates(&slots, &overflowing_updates(), &passes_config(0, 3)) {
            Err(err) => err,
            Ok((runs, _, _)) => panic!("three timed passes faulted and reported {runs:?}"),
        };
        assert!(matches!(err, EvalError::Refused(_)), "{err:?}");
    }

    #[test]
    fn a_faulted_phase_reports_no_timing_at_all() {
        let slots = vec![cms_slot()];
        let config = passes_config(1, 3);

        let clean = Rc::new(vec![one_update(1.0)]);
        let (runs, states, _) = match time_updates(&slots, &clean, &config) {
            Ok(measured) => measured,
            Err(err) => panic!("a clean pass must not fault: {err:?}"),
        };
        assert_eq!(runs.len(), 3, "the same shape does report timing");
        assert_eq!(states.len(), 4, "warm-up included");
        assert!(runs.iter().all(|run| run.elapsed_ns > 0));

        assert!(time_updates(&slots, &overflowing_updates(), &config).is_err());
    }

    #[test]
    fn a_sql_plan_runs_end_to_end_through_the_projection_its_front_end_adds() {
        use asap_types::pre_asap::schema::{Column, Schema};

        let catalog = asap_frontend_sql::SqlCatalog::new().with_table(
            "metrics",
            Schema::new(vec![
                Column::new("service", DataType::Utf8, false),
                Column::new("latency", DataType::Float64, false),
            ]),
        );
        let plan = crate::plan::plan_sql(
            "SELECT approx_percentile_cont(latency, 0.99) FROM metrics",
            &catalog,
            AccuracyTarget::Epsilon(0.01),
        )
        .expect("planning");
        let root = plan
            .dag
            .nodes
            .iter()
            .find(|node| node.id == plan.dag.root)
            .expect("the root is a node");
        assert_eq!(operator_name(&root.payload), "Value");

        let mut file = tempfile::NamedTempFile::new().expect("temp file");
        writeln!(file, "service,latency").unwrap();
        for i in 0..1_000 {
            writeln!(file, "s{},{}", i % 4, i as f64).unwrap();
        }
        file.flush().unwrap();

        let outcome = run(&plan, &csv_config(file.path(), 7, true)).expect("run");
        assert_eq!(outcome.rows_scanned, 1_000);
        assert_eq!(outcome.readouts.len(), 1);
        assert!(
            matches!(outcome.readouts[0].query, Some(SketchQuery::Quantile { q }) if q == 0.99),
            "{:?}",
            outcome.readouts[0].query
        );
        assert!(outcome.readouts[0].exact.is_some());
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

        let values: Vec<f64> = (0..10_000).map(|i| i as f64).collect();
        let csv = csv_with(&values);
        let outcome = run(&plan, &csv_config(csv.path(), 42, true)).expect("run");

        assert_eq!(outcome.rows_scanned, 10_000);
        assert_eq!(outcome.rows_emitted, Some(10_000));
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
        let csv = csv_with(&(0..1_000).map(|i| (i % 91) as f64).collect::<Vec<_>>());

        let mut config = csv_config(csv.path(), 5, true);
        config.timed_runs = 4;
        config.warmup_runs = 2;
        let outcome = run(&plan, &config).expect("run");

        assert_eq!(outcome.rows_scanned, 1_000);
        assert_eq!(outcome.rows_emitted, Some(1_000));
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
    }

    #[test]
    fn a_timed_phase_reports_a_population_and_an_untimed_one_reports_nothing() {
        let plan =
            plan_promql("quantile(0.5, cpu_cores)", AccuracyTarget::Epsilon(0.01)).expect("plan");
        let csv = csv_with(&(0..2_000).map(|i| (i % 313) as f64).collect::<Vec<_>>());

        let mut config = csv_config(csv.path(), 1, true);
        config.timed_runs = 3;
        let outcome = run(&plan, &config).expect("run");

        for phase in [
            &outcome.approximate.build,
            &outcome.approximate.update,
            &outcome.approximate.readout,
            &outcome.pre_asap.evaluate,
        ] {
            assert_eq!(phase.len(), 3, "one draw is not a distribution");
        }
        assert!(
            outcome.approximate.evaluate.is_empty(),
            "the post-ASAP arm walks no tree, which is not the same as walking one instantly"
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
        let csv = csv_with(&(0..1_000).map(|i| i as f64).collect::<Vec<_>>());

        let outcome = run(&plan, &csv_config(csv.path(), 0, false)).expect("run");

        assert!(!outcome.verified);
        let readout = &outcome.readouts[0];
        // None, not 0.0 — "not computed" must never read as "exact".
        assert!(readout.exact.is_none());
        assert!(readout.observed_error.measured().is_none());
        assert_eq!(outcome.retained_values, 0);
        assert_eq!(outcome.retained_bytes, 0);
    }

    #[test]
    fn an_exact_accumulator_plan_needs_no_sketch_and_still_reports_both_arms() {
        let plan = plan_promql("sum(cpu_cores)", AccuracyTarget::Epsilon(0.01)).expect("plan");
        assert_eq!(
            plan.dag.nodes.len(),
            2,
            "Fallback -> SummaryAgg, no estimate"
        );

        let values: Vec<f64> = (1..=100).map(|i| i as f64).collect();
        let csv = csv_with(&values);
        let outcome = run(&plan, &csv_config(csv.path(), 0, true)).expect("run");

        assert_eq!(outcome.rows_emitted, Some(100));
        // No SummaryEstimate node, so no readout — the accumulator's state is
        // the answer, reached through FinalizeExactAccumulator.
        assert!(outcome.readouts.is_empty());
        assert_eq!(outcome.node_footprints.len(), 1);
        assert_eq!(outcome.retained_values, 100);
    }

    /// `count(...)` is one of the most basic queries in the corpus, and the
    /// planner compiles it to a CMS read through a bare-bucket-total readout
    /// rather than to an exact accumulator.
    #[test]
    fn the_count_plan_resolves_runs_and_agrees_with_the_exact_arm() {
        let plan = plan_promql("count(cpu_cores)", AccuracyTarget::Epsilon(0.01)).expect("plan");

        let values: Vec<f64> = (0..5_000).map(|i| (i % 37) as f64).collect();
        let csv = csv_with(&values);
        let outcome = run(&plan, &csv_config(csv.path(), 7, true)).expect("run");

        assert_eq!(outcome.readouts.len(), 1);
        let readout = &outcome.readouts[0];
        assert_eq!(
            readout.query,
            Some(SketchQuery::PointCount {
                key: asap_types::pre_asap::ColumnRef::SampleValue,
                value: None,
            })
        );
        assert_eq!(readout.approximate, Answer::Scalar(5_000.0));
        assert_eq!(readout.exact, Some(Answer::Scalar(5_000.0)));
        // The state it cost, and that it is nothing like the retained column.
        assert_eq!(outcome.node_footprints.len(), 1);
        assert!(outcome.node_footprints[0].1 > 0);
    }

    #[test]
    fn a_bare_selector_plan_has_no_summary_at_all() {
        let plan = plan_promql("cpu_cores", AccuracyTarget::Epsilon(0.01)).expect("plan");
        assert_eq!(plan.dag.nodes.len(), 1);
        assert!(plan.dag.edges.is_empty());

        let csv = csv_with(&[1.0, 2.0, 3.0]);
        let outcome = run(&plan, &csv_config(csv.path(), 0, true)).expect("run");

        assert_eq!(outcome.rows_emitted, Some(3));
        assert!(outcome.readouts.is_empty());
        assert!(outcome.node_footprints.is_empty(), "nothing was summarized");
        assert!(outcome.approximate.update.is_empty());
    }

    #[test]
    fn the_claimed_bound_holds_across_many_seeds() {
        let plan =
            plan_promql("quantile(0.9, cpu_cores)", AccuracyTarget::Epsilon(0.01)).expect("plan");
        let values: Vec<f64> = (0..20_000).map(|i| (i % 997) as f64).collect();
        let csv = csv_with(&values);

        let mut violations = 0;
        let mut observed_errors = Vec::new();
        let seeds = 32;
        for seed in 0..seeds {
            let outcome = run(&plan, &csv_config(csv.path(), seed, true)).expect("run");
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
#[cfg(test)]
mod resolution_tests {
    use super::*;
    use std::rc::Rc;

    use asap_types::post_asap::{
        ExactKind, ExactParams, ExecutionTiming, GroupingEdgeCompatibility, SketchAlgorithm,
        SketchKind, SketchParams, SummaryField,
    };
    use asap_types::pre_asap::{
        CompareOpKind, GroupKeys, Predicate, ProjectItem, ScalarValue, SortKey, Source,
    };

    use crate::rows::tests::{node_of, plan};

    /// Node ids come out of the compiler producer-before-consumer, which is a
    /// property of the compiler rather than one the document carries — enough
    /// for a hand-built fixture, where the whole DAG is three nodes.
    fn resolved(dag: &ExecutableDag) -> Result<Resolved<'_>, Vec<Refusal>> {
        let order: Vec<PostAsapNodeId> = dag.nodes.iter().map(|node| node.id).collect();
        resolve(dag, &order, TimeRangeOrigin::InjectedIngestionHorizon)
    }

    // ── The planner's own plans ──────────────────────────────────────────────

    #[test]
    fn the_three_node_quantile_plan_resolves_with_zero_refusals() {
        let dag = plan("quantile(0.5, cpu_cores)");
        assert_eq!(dag.nodes.len(), 3);

        let held = match resolved(&dag) {
            Ok(held) => held,
            Err(refusals) => panic!("refused: {refusals:?}"),
        };

        assert_eq!(held.nodes(), 3);
        assert_eq!(held.rows.len(), 1);
        assert_eq!(held.aggregates.len(), 1);
        assert_eq!(held.readouts.len(), 1);

        let aggregate = &held.aggregates[0];
        assert_eq!(aggregate.node, node_of(&dag, "SummaryAgg").id);
        assert_eq!(aggregate.item, None);
        // `weight: Column(SampleValue)` resolved to the `value` field, and
        // `Reduce([])` is a genuine global reduction, not "no grouping".
        assert_eq!(aggregate.weight, ResolvedInput::Column(1));
        assert!(aggregate.group_columns.is_empty());

        assert_eq!(held.readouts[0].query, SketchQuery::Quantile { q: 0.5 });
        assert_eq!(held.readouts[0].producer, node_of(&dag, "SummaryAgg").id);
    }

    #[test]
    fn the_rows_to_summary_window_stamp_does_not_refuse_a_sketch_plan() {
        let dag = plan("quantile(0.5, cpu_cores)");
        // The stamp really is on the 0 -> 1 edge; if it ever stops being, this
        // test stops proving anything and should be revisited.
        assert!(dag.edges.iter().any(|edge| {
            edge.window
                == WindowEdgeCompatibility::RequiresAlignedPanePhaseOrExactWindowEdgeResidual
        }));
        assert!(
            resolved(&dag).is_ok(),
            "a blanket window gate refuses every sketch plan"
        );
    }

    #[test]
    fn the_exact_accumulator_and_fallback_only_plans_are_resolved() {
        for query in ["cpu_cores", "sum(cpu_cores)"] {
            let dag = plan(query);
            if let Err(refusals) = resolved(&dag) {
                panic!("{query} was refused: {refusals:?}");
            }
        }
    }

    // ── Permanent refusals ───────────────────────────────────────────────────

    #[test]
    fn a_summary_subtract_node_is_refused_with_no_inverse_operation() {
        let mut dag = plan("quantile(0.5, cpu_cores)");
        let agg = node_of(&dag, "SummaryAgg").id;
        // Hand-built: nothing in the planner emits one, which is the point.
        let victim = dag
            .nodes
            .iter_mut()
            .find(|node| node.id == agg)
            .expect("the SummaryAgg node");
        victim.payload = ExecutableOperatorPayload::SummarySubtract;

        let estimate = node_of(&dag, "SummaryEstimate").id;
        let refusals = resolved(&dag).expect_err("is refused");
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
        let agg = node_of(&dag, "SummaryAgg").id;
        let victim = dag
            .nodes
            .iter_mut()
            .find(|node| node.id == agg)
            .expect("the SummaryAgg node");
        victim.payload = ExecutableOperatorPayload::SummaryDelete {
            key: ColumnRef::Named("cluster".into()),
        };

        let refusals = resolved(&dag).expect_err("is refused");
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
        let agg = node_of(&dag, "SummaryAgg").id;
        let victim = dag
            .nodes
            .iter_mut()
            .find(|node| node.id == agg)
            .expect("the SummaryAgg node");
        victim.payload = ExecutableOperatorPayload::SummaryMerge;

        let refusals = resolved(&dag).expect_err("is refused");
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
    /// shape that survived: resolved unchecked, and then `run` finds no handle
    /// for a `Fallback` and the readout produces nothing at all.
    #[test]
    fn a_readout_whose_producer_builds_no_summary_is_refused_rather_than_waved_through() {
        let mut dag = plan("quantile(0.5, cpu_cores)");
        let fallback = node_of(&dag, "Fallback").id;
        let estimate = node_of(&dag, "SummaryEstimate").id;
        let edge = dag
            .edges
            .iter_mut()
            .find(|edge| edge.consumer == estimate)
            .expect("the readout's input edge");
        edge.producer = fallback;

        let refusals = resolved(&dag).expect_err("is refused");
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
        let fallback = node_of(&dag, "Fallback").id;
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

        let refusals = resolved(&dag).expect_err("is refused");
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
        let fallback = node_of(&dag, "Fallback").id;
        let victim = dag
            .nodes
            .iter_mut()
            .find(|node| node.id == fallback)
            .expect("the Fallback node");
        let QueryExpr::Scan { predicates, .. } = crate::rows::tests::fallback_scan_mut(
            &mut victim.payload,
            TimeRangeOrigin::InjectedIngestionHorizon,
        ) else {
            panic!("the Fallback node lost its Scan");
        };
        predicates.push(Predicate(Rc::new(QueryExpr::FunctionCall {
            name: "lower".into(),
            args: vec![QueryExpr::Column(1)],
        })));

        let refusals = resolved(&dag).expect_err("is refused");
        assert!(
            matches!(refusals.as_slice(), [Refusal::FallbackIsAProgram { variant, .. }] if variant.contains("FunctionCall")),
            "{refusals:?}"
        );
    }

    #[test]
    fn a_label_matcher_on_the_fallback_is_resolved() {
        // A `Scan` with predicates says where rows come from, not what to
        // compute: resolved, deliberately.
        let mut dag = plan("quantile(0.5, cpu_cores)");
        let fallback = node_of(&dag, "Fallback").id;
        let victim = dag
            .nodes
            .iter_mut()
            .find(|node| node.id == fallback)
            .expect("the Fallback node");
        let QueryExpr::Scan { predicates, .. } = crate::rows::tests::fallback_scan_mut(
            &mut victim.payload,
            TimeRangeOrigin::InjectedIngestionHorizon,
        ) else {
            panic!("the Fallback node lost its Scan");
        };
        predicates.push(Predicate(Rc::new(QueryExpr::Compare {
            left: Rc::new(QueryExpr::Column(1)),
            op: CompareOpKind::Gt,
            right: Rc::new(QueryExpr::Literal(ScalarValue::Float64(0.0))),
        })));

        assert!(resolved(&dag).is_ok());
    }

    // ── SummaryAgg ───────────────────────────────────────────────────────────

    #[test]
    fn an_unresolvable_weight_column_names_the_column() {
        let mut dag = plan("quantile(0.5, cpu_cores)");
        let agg = node_of(&dag, "SummaryAgg").id;
        let victim = dag
            .nodes
            .iter_mut()
            .find(|node| node.id == agg)
            .expect("the SummaryAgg node");
        let ExecutableOperatorPayload::SummaryAgg { input, .. } = &mut victim.payload else {
            panic!("the SummaryAgg node lost its payload");
        };
        input.weight = SummaryInputExpr::Column(ColumnRef::Named("absent".into()));

        let refusals = resolved(&dag).expect_err("is refused");
        assert!(
            matches!(refusals.as_slice(), [Refusal::UnresolvableColumn { column, .. }] if column == "absent"),
            "{refusals:?}"
        );
    }

    #[test]
    fn a_wildcard_weight_is_refused() {
        let mut dag = plan("quantile(0.5, cpu_cores)");
        let agg = node_of(&dag, "SummaryAgg").id;
        let victim = dag
            .nodes
            .iter_mut()
            .find(|node| node.id == agg)
            .expect("the SummaryAgg node");
        let ExecutableOperatorPayload::SummaryAgg { input, .. } = &mut victim.payload else {
            panic!("the SummaryAgg node lost its payload");
        };
        input.weight = SummaryInputExpr::Column(ColumnRef::Wildcard);

        let refusals = resolved(&dag).expect_err("is refused");
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
        let agg = node_of(&dag, "SummaryAgg").id;
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

        let refusals = resolved(&dag).expect_err("is refused");
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
            let agg = node_of(&dag, "SummaryAgg").id;
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

            let refusals = resolved(&dag).expect_err("is refused");
            assert!(
                matches!(refusals.as_slice(), [Refusal::UnsupportedGrouping { .. }]),
                "{reduction:?}: {refusals:?}"
            );
        }
    }

    #[test]
    fn a_by_column_past_the_producers_schema_is_refused() {
        let mut dag = plan("quantile(0.5, cpu_cores)");
        let agg = node_of(&dag, "SummaryAgg").id;
        let victim = dag
            .nodes
            .iter_mut()
            .find(|node| node.id == agg)
            .expect("the SummaryAgg node");
        let ExecutableOperatorPayload::SummaryAgg { reduction, .. } = &mut victim.payload else {
            panic!("the SummaryAgg node lost its payload");
        };
        *reduction = Reduction::by(vec![9]);

        let refusals = resolved(&dag).expect_err("is refused");
        assert!(
            matches!(refusals.as_slice(), [Refusal::UnresolvableColumn { column, .. }] if column.contains('9')),
            "{refusals:?}"
        );
    }

    #[test]
    fn an_entity_identity_over_a_schema_with_no_label_column_is_refused() {
        let mut dag = plan("quantile(0.5, cpu_cores)");
        let agg = node_of(&dag, "SummaryAgg").id;
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

        let refusals = resolved(&dag).expect_err("is refused");
        assert!(
            matches!(refusals.as_slice(), [Refusal::UnsupportedUpdate { .. }]),
            "{refusals:?}"
        );
    }

    #[test]
    fn an_entity_identity_resolves_to_the_label_columns_of_the_producer() {
        for (excluding, expected) in [
            (vec![], vec![1usize, 2]),
            (vec![ColumnRef::Named("cluster".into())], vec![2usize]),
        ] {
            let mut dag = with_labels(plan("quantile(0.5, cpu_cores)"));
            let agg = node_of(&dag, "SummaryAgg").id;
            let victim = dag
                .nodes
                .iter_mut()
                .find(|node| node.id == agg)
                .expect("the SummaryAgg node");
            let ExecutableOperatorPayload::SummaryAgg { input, .. } = &mut victim.payload else {
                panic!("the SummaryAgg node lost its payload");
            };
            input.item = Some(SummaryInputExpr::EntityIdentity(
                EntityIdentity::PromqlLabelSet {
                    excluding: excluding.clone(),
                },
            ));

            let held = match resolved(&dag) {
                Ok(held) => held,
                Err(refusals) => panic!("{excluding:?}: refused: {refusals:?}"),
            };
            let aggregate = held
                .aggregates
                .iter()
                .find(|aggregate| aggregate.node == agg)
                .expect("the SummaryAgg node resolved");
            let (item, weight) = (&aggregate.item, &aggregate.weight);
            assert_eq!(
                item.as_ref(),
                Some(&ResolvedInput::Tuple(
                    expected
                        .iter()
                        .copied()
                        .map(ResolvedInput::Column)
                        .collect()
                )),
                "{excluding:?}"
            );
            assert_eq!(
                weight,
                &ResolvedInput::Column(3),
                "the sample value is the weight, never part of the identity"
            );
        }
    }

    fn with_labels(mut dag: ExecutableDag) -> ExecutableDag {
        use asap_types::pre_asap::Column;

        let fallback = node_of(&dag, "Fallback").id;
        let labels = ["cluster", "task"];
        for node in &mut dag.nodes {
            if node.id != fallback {
                continue;
            }
            for (offset, label) in labels.iter().enumerate() {
                node.output_schema.fields.insert(
                    1 + offset,
                    SummaryField {
                        name: (*label).to_string(),
                        dtype: SummaryFamilyType::Plain(DataType::Utf8),
                        nullable: false,
                    },
                );
            }
            let QueryExpr::Scan { schema, .. } = crate::rows::tests::fallback_scan_mut(
                &mut node.payload,
                TimeRangeOrigin::InjectedIngestionHorizon,
            ) else {
                panic!("the Fallback node lost its Scan");
            };
            for (offset, label) in labels.iter().enumerate() {
                schema.columns.insert(
                    1 + offset,
                    Column {
                        name: (*label).to_string(),
                        dtype: DataType::Utf8,
                        nullable: false,
                        table: None,
                    },
                );
            }
        }
        let schema = node_of(&dag, "Fallback").output_schema.clone();
        for edge in &mut dag.edges {
            if edge.producer == fallback {
                edge.intermediate_schema = schema.clone();
            }
        }
        dag
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
        let agg = node_of(&dag, "SummaryAgg").id;
        let victim = dag
            .nodes
            .iter_mut()
            .find(|node| node.id == agg)
            .expect("the SummaryAgg node");
        let ExecutableOperatorPayload::SummaryAgg { family, .. } = &mut victim.payload else {
            panic!("the SummaryAgg node lost its payload");
        };
        *family = SummaryFamilyType::Sketch(kind, GroupingStrategy::PerSubpopulationInstance);

        let refusals = resolved(&dag).expect_err("is refused");
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
    fn every_readout_shape_is_resolved_when_its_family_answers_it() {
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
            (
                SketchQuery::Cardinality,
                Some(sketch(
                    SketchAlgorithm::Hll,
                    SketchParams::Hll { precision: 14 },
                )),
            ),
            (SketchQuery::FrequencyL2, Some(univmon())),
            (SketchQuery::FrequencyEntropy, Some(univmon())),
            (
                SketchQuery::PointCount {
                    key: ColumnRef::Named("service".into()),
                    value: Some("api".into()),
                },
                Some(sketch(
                    SketchAlgorithm::Cms,
                    SketchParams::Cms {
                        width: 256,
                        depth: 5,
                    },
                )),
            ),
            (
                SketchQuery::TopK { k: 10 },
                Some(sketch(
                    SketchAlgorithm::CmsWithHeap,
                    SketchParams::CmsWithHeap {
                        width: 256,
                        depth: 5,
                        heap_size: 32,
                    },
                )),
            ),
        ];

        for (query, family) in accepted {
            let dag = match family {
                Some(family) => with_family(with_readout(&query), family),
                None => with_readout(&query),
            };
            assert!(resolved(&dag).is_ok(), "{query:?} should be resolved");
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
            let refusals = resolved(&dag).expect_err("is refused");
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
    fn the_pairings_that_do_name_the_same_question_are_resolved() {
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
                resolved(&dag).is_ok(),
                "{family:?} / {query:?} should be resolved"
            );
        }
    }

    fn sketch(algorithm: SketchAlgorithm, params: SketchParams) -> SummaryFamilyType {
        SummaryFamilyType::Sketch(
            SketchKind::new(algorithm, params),
            GroupingStrategy::PerSubpopulationInstance,
        )
    }

    fn univmon() -> SummaryFamilyType {
        sketch(
            SketchAlgorithm::UnivMon,
            SketchParams::UnivMon {
                heap_size: 1000,
                sketch_rows: 5,
                sketch_cols: 256,
                layers: 16,
            },
        )
    }

    fn with_family(mut dag: ExecutableDag, family: SummaryFamilyType) -> ExecutableDag {
        let agg = node_of(&dag, "SummaryAgg").id;
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
            let refusals = resolved(&dag).expect_err("is refused");
            assert!(
                matches!(refusals.as_slice(), [Refusal::ParameterOutOfBounds { .. }]),
                "q = {q}: {refusals:?}"
            );
        }
    }

    fn with_readout(query: &SketchQuery) -> ExecutableDag {
        let mut dag = plan("quantile(0.5, cpu_cores)");
        let estimate = node_of(&dag, "SummaryEstimate").id;
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
        let estimate = node_of(&dag, "SummaryEstimate").clone();
        let id = PostAsapNodeId(dag.nodes.len() as u32);
        dag.nodes.push(ExecutableDagNode {
            id,
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
    fn the_row_operations_run_executes_are_resolved_and_the_rest_are_refused_by_name() {
        for operation in [
            ValueOperation::Limit { n: 10, offset: 0 },
            ValueOperation::Sort {
                keys: vec![SortKey {
                    expr: QueryExpr::Column(0),
                    ascending: false,
                    nulls_first: false,
                }],
                partition_by: GroupKeys::default(),
            },
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
            if let Err(refusals) = resolved(&with_value(&operation)) {
                panic!("{operation:?}: {refusals:?}");
            }
        }

        let outside = [
            ValueOperation::FinalizeExactAccumulator,
            ValueOperation::Project {
                cols: vec![ProjectItem {
                    alias: None,
                    expr: QueryExpr::EvalTimestamp,
                }],
                qualifier: None,
            },
            ValueOperation::Filter {
                pred: Predicate(Rc::new(QueryExpr::FunctionCall {
                    name: "lower".into(),
                    args: vec![QueryExpr::Column(0)],
                })),
            },
        ];
        for operation in outside {
            let refusals = resolved(&with_value(&operation)).expect_err("is refused");
            assert!(
                matches!(
                    refusals.as_slice(),
                    [Refusal::UnsupportedValueOperation { .. }]
                ),
                "{operation:?}: {refusals:?}"
            );
        }

        let refusals = resolved(&with_value(&ValueOperation::Extension {
            name: "promql_histogram_quantile".into(),
        }))
        .expect_err("is refused");
        assert!(
            matches!(refusals.as_slice(), [Refusal::UnregisteredExtension { name, .. }]
                if name == "promql_histogram_quantile"),
            "{refusals:?}"
        );
    }

    #[test]
    fn a_summary_to_summary_window_obligation_is_refused() {
        // Both endpoints MAINTENANCE_SUMMARY: the only shape the gate catches.
        let mut dag = plan("quantile(0.5, cpu_cores)");
        let agg = node_of(&dag, "SummaryAgg");
        let agg_id = agg.id;
        let agg_schema = agg.output_schema.clone();
        let estimate = node_of(&dag, "SummaryEstimate").id;

        dag.nodes
            .iter_mut()
            .find(|node| node.id == estimate)
            .expect("the SummaryEstimate node")
            .output_state = ExecutionDataState::MAINTENANCE_SUMMARY;
        for edge in &mut dag.edges {
            if edge.producer == agg_id && edge.consumer == estimate {
                edge.window =
                    WindowEdgeCompatibility::RequiresAlignedPanePhaseOrExactWindowEdgeResidual;
                edge.intermediate_schema = agg_schema.clone();
                edge.data_state = ExecutionDataState::MAINTENANCE_SUMMARY;
                edge.grouping = GroupingEdgeCompatibility::NotApplicable;
            }
        }

        let refusals = resolved(&dag).expect_err("is refused");
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
        let fallback = node_of(&dag, "Fallback").id;
        let agg = node_of(&dag, "SummaryAgg").id;
        let estimate = node_of(&dag, "SummaryEstimate").id;

        for node in &mut dag.nodes {
            if node.id == fallback {
                node.payload = ExecutableOperatorPayload::Fallback {
                    expression: QueryExpr::EvalTimestamp,
                };
            } else if node.id == agg {
                node.payload = ExecutableOperatorPayload::SummarySubtract;
            } else if node.id == estimate {
                node.payload = ExecutableOperatorPayload::SummaryEstimate {
                    query: SketchQuery::Quantile { q: 1.5 },
                };
            }
        }

        let refusals = resolved(&dag).expect_err("is refused");
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
            .any(|refusal| matches!(refusal, Refusal::ParameterOutOfBounds { .. })));
    }

    #[test]
    fn one_bad_node_yields_exactly_one_refusal() {
        // A `Binary` node is catchable on both its edge role and its operator;
        // it must be reported once.
        let mut dag = plan("quantile(0.5, cpu_cores)");
        let estimate = node_of(&dag, "SummaryEstimate").id;
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
        victim.payload = ExecutableOperatorPayload::Binary {
            timing: ExecutionTiming::ReadTime,
            operator: asap_types::post_asap::BinaryOperator {
                checked_relative_division: false,
                checked_finite_division: false,
                kind: asap_types::pre_asap::BinaryOpKind::Compare(CompareOpKind::Gt),
                vector_match: None,
            },
        };

        let refusals = resolved(&dag).expect_err("is refused");
        assert_eq!(refusals.len(), 1, "{refusals:?}");
    }

    // ── Naming ───────────────────────────────────────────────────────────────

    #[test]
    fn a_fallback_whose_output_is_summary_state_is_refused() {
        let mut dag = plan("quantile(0.5, cpu_cores)");
        let fallback = node_of(&dag, "Fallback").id;
        let agg_schema = node_of(&dag, "SummaryAgg").output_schema.clone();
        let victim = dag
            .nodes
            .iter_mut()
            .find(|node| node.id == fallback)
            .expect("the Fallback node");
        victim.output_state = ExecutionDataState::MAINTENANCE_SUMMARY;
        victim.output_schema = agg_schema;

        let refusals = resolved(&dag).expect_err("is refused");
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
        let fallback = node_of(&dag, "Fallback").id;
        let victim = dag
            .nodes
            .iter_mut()
            .find(|node| node.id == fallback)
            .expect("the Fallback node");
        let QueryExpr::Scan { source, .. } = crate::rows::tests::fallback_scan_mut(
            &mut victim.payload,
            TimeRangeOrigin::InjectedIngestionHorizon,
        ) else {
            panic!("the Fallback node lost its Scan");
        };
        *source = Source::Table {
            table_ref: "metrics".into(),
        };

        assert!(resolved(&dag).is_ok());
    }

    #[test]
    fn a_node_with_no_input_edge_where_one_is_required_is_refused() {
        let mut dag = plan("quantile(0.5, cpu_cores)");
        let agg = node_of(&dag, "SummaryAgg").id;
        dag.edges.retain(|edge| edge.consumer != agg);

        let refusals = resolved(&dag).expect_err("is refused");
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
        let field: &SummaryField = &node_of(&dag, "Fallback").output_schema.fields[1];
        assert_eq!(field.name, "value");
        assert_eq!(field.dtype, SummaryFamilyType::Plain(DataType::Float64));
    }
}
