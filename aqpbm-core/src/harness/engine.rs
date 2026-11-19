//! The engine under test: the traits every engine implements, and the
//! records it exchanges with the harness.
//!
//! An engine never times itself. [`InstalledArm::ingest`] and
//! [`InstalledArm::evaluate`] return a [`StepReport`] with the counters the
//! engine keeps (scanned bytes, a per-query CPU counter of its own), and the
//! harness builds the [`StepCost`] of the step from its own clock.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

pub use crate::harness::data::{DataRecord, SampleChunk, SeriesLabels};

/// An engine the harness can measure: the Planner's executor, a fake, or an
/// external server such as ClickHouse.
pub trait EngineUnderTest {
    /// Id, version and build profile, recorded in every run.
    fn description(&self) -> EngineDescription;
    /// Where the harness reads this engine's CPU time.
    fn cpu_accounting(&self) -> CpuAccounting;
    /// Plan (if the engine plans) and install one arm of one workload.
    fn install(
        &self,
        workload: &WorkloadSpec,
        arm: &ArmSpec,
    ) -> Result<Box<dyn InstalledArm>, EngineError>;
    /// The engine's cost calibration, when it has a cost model to calibrate.
    fn calibration(&mut self) -> Option<&mut dyn Calibratable> {
        None
    }
}

/// Which engine and which build a measurement ran on.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EngineDescription {
    /// Short id, such as `fake`, `asap` or `clickhouse`.
    pub id: String,
    /// A commit for an in-tree engine; a server version for an external one.
    pub version: String,
    /// The working tree had uncommitted changes when the engine was built.
    pub version_dirty: bool,
    /// Cargo profile or server build, such as `release`.
    pub build_profile: String,
}

/// Where an engine's CPU time is read.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum CpuAccounting {
    /// In-process: the work of a step runs on the thread that calls it, and
    /// the harness reads that thread's CPU clock.
    CallingThread,
    /// External servers: the harness reads the CPU of these processes
    /// around each step (their own cgroup's `cpu.stat`, or their threads'
    /// `schedstat`; see [`crate::harness::protocol::ProcessesCpuClock`]).
    Processes(Vec<u32>),
    /// An external server in its own cgroup v2 group (a container), whose
    /// `cpu.stat` and memory files the harness reads, on this host or over
    /// SSH on the server host ([`crate::harness::cgroup::CgroupSource`]).
    Cgroup(crate::harness::cgroup::CgroupSource),
}

/// One arm of a workload: how the engine is asked to run it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArmSpec {
    /// `exact`, `planned`, `noraw`, `stored`, ...
    pub name: String,
    /// Whether the engine keeps raw samples it could answer exactly from.
    pub raw_data_retained: bool,
    /// Opaque to the harness; read only by the engine adapter.
    pub engine_options: serde_json::Value,
}

/// The queries of a workload in every language it is written in, and the
/// data they run over; an engine picks the language it reads.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkloadSpec {
    pub name: String,
    pub queries: Vec<WorkloadQuery>,
    pub data: Vec<DataRecord>,
}

/// One recurring query: evaluated every `evaluation_interval_ms` over the
/// left-open window `(t - window_ms, t]`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkloadQuery {
    pub id: String,
    pub texts: Vec<QueryText>,
    pub window_ms: u64,
    pub evaluation_interval_ms: u64,
}

/// A query text in one language.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QueryText {
    pub language: QueryLanguage,
    pub text: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum QueryLanguage {
    PromQl,
    Sql,
    /// MetricsQL: read natively by VictoriaMetrics and planned by
    /// ASAPPlanner's MetricsQL frontend.
    MetricsQl,
}

/// One installed arm. The harness calls `ingest` per refresh chunk and
/// `evaluate` per evaluation time, and times each call itself.
pub trait InstalledArm {
    /// What the engine installed: plan text, node kinds, stored outputs.
    fn plan_summary(&self) -> PlanSummary;
    /// The engine's own predicted cost; `None` for an engine that does not plan.
    fn predicted(&self) -> Option<PredictedCost> {
        None
    }
    /// One refresh chunk; the harness times the call.
    fn ingest(&mut self, chunk: &SampleChunk) -> Result<StepReport, EngineError>;
    /// Answers of every query at `at_ms`; the harness times the call.
    fn evaluate(&mut self, at_ms: i64) -> Result<(Answers, StepReport), EngineError>;
    /// Bytes held after the run.
    fn footprint(&self) -> Result<Footprint, EngineError>;
    /// Per-operator timing hook: per node, per batch, the CPU the node itself
    /// used during the evaluations since the last call.
    fn node_profiles(&mut self) -> Option<Vec<NodeProfile>> {
        None
    }
    /// Measured usage priced at the engine's own prices (the Planner: cost
    /// model 1). `None` for an engine without a cost model.
    fn price(&self, usage: &MeasuredUsage) -> Option<f64> {
        let _ = usage;
        None
    }
}

/// What an engine reports about one step besides time. It never reports its
/// own CPU time, only counters it keeps itself, such as ClickHouse's
/// per-query CPU counter.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StepReport {
    /// Bytes the step scanned, when the engine counts them.
    pub scanned_bytes: Option<u64>,
    /// The engine's own CPU counter for the step, in ns, when it keeps one.
    pub engine_reported_cpu_ns: Option<u64>,
}

/// What the harness measured for one step.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StepCost {
    /// CPU of the calling thread, in ns; `None` for a `Processes` engine.
    pub thread_cpu_ns: Option<u64>,
    /// CPU of the measured processes (this process for `CallingThread`), in ns.
    pub process_cpu_ns: u64,
    /// Wall time of the call, in ns.
    pub wall_ns: u64,
    /// The engine's own counters for the step.
    pub report: StepReport,
}

/// The answers of one evaluation: per query id, one value per output series.
/// An output series carries PromQL's output labels: a function or an
/// aggregation drops `__name__`, and only a bare selector keeps it; every
/// engine uses this convention so the report can join engine runs by label set. The
/// query id is the outer key, so two queries over one metric never share a
/// key and no label name is reserved. Example: `sum_over_time(probe[10s])`
/// with id `sum` over the series `probe{instance="instance_0"}` answers
/// under `"sum" -> {instance="instance_0"}`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Answers(pub BTreeMap<String, BTreeMap<SeriesLabels, f64>>);

impl Answers {
    /// Every answer as `(query id, series, value)`, by query id, then series.
    pub fn iter(&self) -> impl Iterator<Item = (&str, &SeriesLabels, f64)> {
        self.0.iter().flat_map(|(query, series)| {
            series
                .iter()
                .map(move |(labels, value)| (query.as_str(), labels, *value))
        })
    }

    /// The answer of `query` for `series`, if any.
    pub fn get(&self, query: &str, series: &SeriesLabels) -> Option<f64> {
        self.0.get(query)?.get(series).copied()
    }
}

/// Serialized as `{query id: [[labels, value], ...]}`: JSON object keys must
/// be strings, and a series is a label map.
impl Serialize for Answers {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_map(
            self.0
                .iter()
                .map(|(query, series)| (query, series.iter().collect::<Vec<_>>())),
        )
    }
}

impl<'de> Deserialize<'de> for Answers {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let per_query = BTreeMap::<String, Vec<(SeriesLabels, f64)>>::deserialize(deserializer)?;
        Ok(Answers(
            per_query
                .into_iter()
                .map(|(query, pairs)| (query, pairs.into_iter().collect()))
                .collect(),
        ))
    }
}

/// `name{a="1",b="2"}`: a series as PromQL writes it, `__name__` in front.
pub fn format_series(labels: &SeriesLabels) -> String {
    let name = labels.get("__name__").map(String::as_str).unwrap_or("");
    let rest: Vec<String> = labels
        .iter()
        .filter(|(label, _)| label.as_str() != "__name__")
        .map(|(label, value)| format!("{label}={value:?}"))
        .collect();
    format!("{name}{{{}}}", rest.join(","))
}

/// What an engine installed for one arm.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlanSummary {
    /// The engine's own plan text.
    pub text: String,
    /// The kind of each node, such as `scan` or `sum`.
    pub node_kinds: Vec<String>,
    /// Outputs the arm stores between evaluations (views, sketches).
    pub stored_outputs: Vec<String>,
    /// Rules of a translating engine whose answers differ from the plan's
    /// guarantee, such as `quantileDD for Kll at node 3`.
    #[serde(default)]
    pub substitutions: Vec<String>,
    /// Shared query-time nodes the engine recomputes for each reader, such
    /// as `node 4 for nodes 6, 7`.
    #[serde(default)]
    pub sharing_lost: Vec<String>,
}

/// The engine's own predicted cost of an arm, recorded in each
/// [`crate::harness::records::ArmResult`] beside the measured cost so a sweep can
/// rank arms by both ([`crate::harness::sweep::ranking_agreement`]).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PredictedCost {
    /// In the engine's cost units per second of data (the Planner: cost model 1).
    pub engine_cost_per_second: f64,
}

/// Bytes an arm holds.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Footprint {
    /// Bytes of stored outputs (views, sketches).
    pub view_bytes: u64,
    /// Bytes of raw samples kept.
    pub raw_retained_bytes: u64,
    /// Peak bytes during the run, when the engine can tell.
    pub peak_bytes: Option<u64>,
}

/// What the harness measured over a run, per second of data, for
/// [`InstalledArm::price`].
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MeasuredUsage {
    pub ingest_cpu_ns_per_second: f64,
    pub query_cpu_ns_per_second: f64,
    /// Bytes the arm holds after the run: stored outputs (views, sketches)
    /// plus raw samples kept, [`Footprint::view_bytes`] +
    /// [`Footprint::raw_retained_bytes`].
    pub held_bytes: u64,
}

/// One node's own CPU per batch, from the per-operator timing hook (the
/// executor times each operator per batch).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NodeProfile {
    pub node: String,
    /// The node's own CPU in each batch, in ns. A node shared by two
    /// consumers is timed once per batch.
    pub cpu_ns_per_batch: Vec<u64>,
}

/// An engine whose cost constants the harness can calibrate.
pub trait Calibratable {
    /// What to measure: one target per constant and operator family.
    fn cost_targets(&self) -> Vec<CostTarget>;
    /// The harness pins, warms up and times each call; the closure returns the
    /// units it processed.
    #[allow(clippy::type_complexity)]
    fn prepare_cost_target(
        &mut self,
        target: &CostTarget,
        size: u64,
    ) -> Result<Box<dyn FnMut() -> Result<u64, EngineError>>, EngineError>;
    /// The units the engine's own cost model charges `target.constant` at `size`.
    fn charged_units(&self, target: &CostTarget, size: u64) -> f64;
}

/// One calibration target.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CostTarget {
    /// Opaque to the harness; the engine maps it back to its operator.
    pub key: serde_json::Value,
    /// The name of the engine's own cost constant this target measures, such
    /// as `ns_per_unit`; [`Calibratable::charged_units`] gives the units the
    /// engine's cost model charges it.
    pub constant: String,
    /// What one unit is, such as `samples`.
    pub unit: String,
    /// The sizes to run, in units.
    pub sizes: Vec<u64>,
}

/// An engine failure, with the engine's own message.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct EngineError(pub String);
