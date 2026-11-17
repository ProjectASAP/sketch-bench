//! A single-node VictoriaMetrics server as an engine under test: each arm is
//! a plan file translated by [`crate::victoriametrics::translate`], its
//! stream aggregation loaded into the server, its data fed in real time with
//! wall-clock timestamps, and its queries run over HTTP.
//!
//! Stream aggregation runs on wall-clock time (see the capability README's
//! run contract), so an arm maps data time to wall time by an offset: a
//! sample at data time `ts` is sent with timestamp `ts + offset`, and an
//! evaluation at data time `t` queries `t + offset`. The offset is a whole
//! number of every window width and of the evaluation interval, so window
//! ends stay window ends. Example: windows of 10 s, evaluations every 10 s,
//! data from 0 ms, installed at wall time 1,794,932,831.4 s: offset
//! 1,794,932,840,000 ms, and the sample at data time 5,000 ms is sent with
//! timestamp 1,794,932,845,000 at wall time 1,794,932,840.5 s (the slice
//! `(4 s, 5 s]` sent 4.5 s early: `h` = 4.5 s).
//!
//! Feeding: each refresh chunk is cut into 1 s slices, and each slice is
//! sent a lead early: `max(1 s, h)`, `h = (w − 1 s) / 2` the narrowest
//! window's read shift. So no sample lags behind the wall clock even when
//! the harness's own work between steps delays a slice (a lag would delay
//! the server's flush), and every sample still lands in its own window
//! (the lead stays below the width). A slice sent at or after its window's
//! end fails the step. The ingest of the chunk ending at `t` then waits
//! until every stream aggregation with a window ending at `t` has flushed
//! (failing if a flush lags by `h` or more) and makes the new rows
//! searchable, so this work is ingest CPU and an evaluation runs only its
//! queries. One arm at a time per engine: the server's stream-aggregation
//! config is one file, and wall-clock data cannot be replayed into a second
//! arm, so run the lane with no warm-up and one measured run.
//!
//! Series: a per-series answer is keyed by its labels with `__name__` set
//! to the input metric (as the harness keys answers); an aggregated answer
//! by its labels without `__name__`.
//!
//! CPU: the server runs in its own cgroup, read through the cgroup agent,
//! which also holds the server's `-streamAggr.config` file. VictoriaMetrics
//! keeps no per-query CPU counter, so [`StepReport`] carries none.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use aqpbm_core::harness::cgroup::{put_server_config, CgroupSource};
use aqpbm_core::harness::data::Sample;
use aqpbm_core::harness::engine::{
    Answers, ArmSpec, CpuAccounting, EngineDescription, EngineError, EngineUnderTest, Footprint,
    InstalledArm, PlanSummary, SampleChunk, SeriesLabels, StepReport, WorkloadSpec,
};
use serde_json::Value;

use crate::plan_file::{PlanFile, PlanOperator};
use crate::victoriametrics::{
    translate, MetricsQlQuery, StreamAggregation, VictoriaMetricsDeployment,
};

/// How to reach the server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VictoriaMetricsServer {
    /// The HTTP interface, e.g. `http://10.0.0.1:8428`.
    pub url: String,
    /// The cgroup agent beside the server, started with `--server-config`
    /// on the server's `-streamAggr.config` file, e.g. `http://10.0.0.1:9101`.
    pub cgroup_agent_url: String,
    /// How long one request may take, e.g. 600 s.
    pub request_timeout: Duration,
}

/// A VictoriaMetrics server; see the module docs.
pub struct VictoriaMetricsEngine {
    server: VictoriaMetricsServer,
    http: Http,
    description: EngineDescription,
    /// Set while an arm is installed.
    arm_installed: Arc<AtomicBool>,
}

/// Installs so far in this process, for each install's config nonce.
static INSTALLS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// How long to wait for the server to apply a reloaded config.
const RELOAD_TIMEOUT: Duration = Duration::from_secs(10);
/// How often a wait polls the server's `/metrics`.
const POLL_INTERVAL: Duration = Duration::from_millis(50);
/// The latest a chunk's flush counts may be read before the tick at its end.
const BEFORE_TICK_MARGIN_MS: i64 = 50;
/// The least time between installing an arm and its first data, so the
/// data starts after the reloaded config's first tick.
const START_MARGIN_MS: i64 = 2_000;

impl VictoriaMetricsEngine {
    /// Connects to the server and reads its version.
    pub fn new(server: VictoriaMetricsServer) -> Result<VictoriaMetricsEngine, EngineError> {
        let http = Http::new(&server.url, server.request_timeout);
        let metrics = ServerMetrics::parse(&http.get("/metrics", &[])?);
        let version = metrics
            .app_version
            .ok_or_else(|| EngineError("the server's /metrics has no vm_app_version".into()))?;
        Ok(VictoriaMetricsEngine {
            description: EngineDescription {
                id: "victoriametrics".into(),
                version,
                version_dirty: false,
                build_profile: "single-node".into(),
            },
            server,
            http,
            arm_installed: Arc::new(AtomicBool::new(false)),
        })
    }
}

impl EngineUnderTest for VictoriaMetricsEngine {
    fn description(&self) -> EngineDescription {
        self.description.clone()
    }

    fn cpu_accounting(&self) -> CpuAccounting {
        CpuAccounting::Cgroup(CgroupSource::Agent {
            url: self.server.cgroup_agent_url.clone(),
        })
    }

    fn install(
        &self,
        workload: &WorkloadSpec,
        arm: &ArmSpec,
    ) -> Result<Box<dyn InstalledArm>, EngineError> {
        let path = arm
            .engine_options
            .get("plan_file")
            .and_then(|p| p.as_str())
            .ok_or_else(|| EngineError("engine_options has no plan_file".into()))?;
        let text = std::fs::read_to_string(path)
            .map_err(|error| EngineError(format!("plan file {path}: {error}")))?;
        let plan = PlanFile::from_json(&text)
            .map_err(|error| EngineError(format!("plan file {path}: {error}")))?;
        let query_ids = query_ids_of(&plan, workload)?;
        let deployment =
            translate(&plan).map_err(|error| EngineError(format!("plan file {path}: {error}")))?;
        let interval_ms = evaluation_interval_ms(workload)?;
        check_time_steps(workload, &deployment.queries, interval_ms)?;
        if self
            .arm_installed
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_err()
        {
            return Err(EngineError(
                "a VictoriaMetrics arm is already installed: wall-clock data runs one arm at a \
                 time, so run this lane with --warmup 0 --runs 1"
                    .into(),
            ));
        }
        // From here on, dropping the arm unloads its config and frees the engine.
        let mut installed = VictoriaMetricsArm {
            http: self.http.clone(),
            agent_url: self.server.cgroup_agent_url.clone(),
            arm_installed: Arc::clone(&self.arm_installed),
            queries: query_ids
                .into_iter()
                .zip(deployment.queries.clone())
                .collect(),
            aggregations: deployment.stream_aggregations.clone(),
            offset_ms: 0,
            lead_ms: lead_ms(&deployment.stream_aggregations),
            too_old_at_install: 0,
            data_bytes_at_install: 0,
            summary: plan_summary(&plan, &deployment),
        };
        // A reload whose config bytes are unchanged keeps the old
        // aggregators and their state, so each install's config differs.
        let nonce = format!(
            "# install {} {}\n",
            std::process::id(),
            INSTALLS.fetch_add(1, Ordering::Relaxed)
        );
        installed.load_config(&(nonce + &deployment.stream_aggregation_config()))?;
        installed.http.get("/internal/force_flush", &[])?;
        let metrics = installed.metrics()?;
        installed.too_old_at_install = metrics.too_old_samples;
        installed.data_bytes_at_install = metrics.data_size_bytes;
        let step_ms = deployment
            .stream_aggregations
            .iter()
            .fold(lcm(1000, interval_ms), |step, a| lcm(step, a.interval_ms));
        let data_start = data_start_ms(workload);
        installed.offset_ms = wall_offset_ms(now_ms(), data_start, step_ms);
        // The wait for the data start is the install's, not the first
        // chunk's.
        sleep_until_wall_ms(data_start + installed.offset_ms - installed.lead_ms);
        Ok(Box::new(installed))
    }
}

/// The one evaluation interval of `workload`'s queries.
fn evaluation_interval_ms(workload: &WorkloadSpec) -> Result<u64, EngineError> {
    let mut intervals = workload.queries.iter().map(|q| q.evaluation_interval_ms);
    let first = intervals
        .next()
        .ok_or_else(|| EngineError(format!("workload {} has no queries", workload.name)))?;
    if first == 0 || intervals.any(|i| i != first) {
        return Err(EngineError(format!(
            "workload {}: one evaluation interval, above 0, for every query",
            workload.name
        )));
    }
    Ok(first)
}

/// A query that reads stream outputs answers right only at a window end, so
/// its step must divide the evaluation interval (the data start is moved to
/// a window end by the wall offset).
fn check_time_steps(
    workload: &WorkloadSpec,
    queries: &[MetricsQlQuery],
    interval_ms: u64,
) -> Result<(), EngineError> {
    for (query, plan_query) in workload.queries.iter().zip(queries) {
        if !interval_ms.is_multiple_of(plan_query.time_step_ms) {
            return Err(EngineError(format!(
                "query {} reads stream outputs of {} ms windows, which do not divide its \
                 evaluation interval ({interval_ms} ms)",
                query.id, plan_query.time_step_ms
            )));
        }
    }
    Ok(())
}

/// The workload's data start, or 0 when the spec carries no data (the
/// samples then come from the caller).
fn data_start_ms(workload: &WorkloadSpec) -> i64 {
    workload
        .data
        .iter()
        .map(|record| record.description.start_ms)
        .min()
        .unwrap_or(0)
}

/// The offset from data time to wall time: the smallest multiple of
/// `step_ms` that puts the data start's step boundary (the data start
/// rounded down to a multiple of `step_ms`) at least [`START_MARGIN_MS`]
/// after `now_ms`. So the data's first window starts on a tick after the
/// config's load, even for a data start that is not a window end. Example:
/// now 1,000,000,500 ms, data from 10,000 ms, step 60,000 ms: 1,000,020,000
/// ms (the data starts at wall time 1,000,030,000 ms, after the tick at
/// 1,000,020,000 ms).
pub fn wall_offset_ms(now_ms: i64, data_start_ms: i64, step_ms: u64) -> i64 {
    let step = step_ms as i64;
    let boundary = data_start_ms.div_euclid(step) * step;
    let earliest = now_ms + START_MARGIN_MS - boundary;
    earliest.div_euclid(step) * step
        + if earliest.rem_euclid(step) == 0 {
            0
        } else {
            step
        }
}

/// How early the feeder sends a slice: `max(1 s, h)` with `h = (w − 1 s) /
/// 2` the read shift of the narrowest window; 1 s with no stream
/// aggregation. Below the width, so a sample still lands in its own window.
/// Example: windows of 60 s and 10 s: 4,500 ms.
pub fn lead_ms(aggregations: &[StreamAggregation]) -> i64 {
    aggregations
        .iter()
        .map(|a| (a.interval_ms as i64 - 1000) / 2)
        .min()
        .unwrap_or(0)
        .max(1000)
}

/// The feeder's slice length.
pub const SLICE_MS: i64 = 1000;

/// One slice of a chunk: the samples in `(end − slice, end]` (data time),
/// to be sent at wall time `send_at_wall_ms`.
#[derive(Debug, Clone, PartialEq)]
pub struct Slice<'a> {
    pub end_ms: i64,
    pub send_at_wall_ms: i64,
    pub samples: Vec<&'a Sample>,
}

/// `chunk`'s samples cut into slices of [`SLICE_MS`] ending at its
/// multiples, in time order, each to be sent `lead_ms` before its end in
/// wall time. Example: 1 Hz samples at 1,000..3,000 ms, offset 10,000, lead
/// 1,000: three slices sent at 10,000, 11,000 and 12,000.
pub fn slices(chunk: &SampleChunk, offset_ms: i64, lead_ms: i64) -> Vec<Slice<'_>> {
    let mut by_end: BTreeMap<i64, Vec<&Sample>> = BTreeMap::new();
    for sample in &chunk.samples {
        let end = (sample.ts_ms + SLICE_MS - 1).div_euclid(SLICE_MS) * SLICE_MS;
        by_end.entry(end).or_default().push(sample);
    }
    by_end
        .into_iter()
        .map(|(end_ms, samples)| Slice {
            end_ms,
            send_at_wall_ms: end_ms + offset_ms - lead_ms,
            samples,
        })
        .collect()
}

/// `/api/v1/import` JSON lines: one line per series, its values and
/// timestamps (data time + `offset_ms`) in order. NaN and ±Inf are written
/// as VictoriaMetrics reads them (`NaN`, `Inf`, `-Inf`, as JSON strings
/// would not be read).
pub fn import_body(samples: &[&Sample], offset_ms: i64) -> String {
    let mut series: BTreeMap<&SeriesLabels, (Vec<String>, Vec<i64>)> = BTreeMap::new();
    for sample in samples {
        let (values, timestamps) = series.entry(&*sample.labels).or_default();
        values.push(number_text(sample.value));
        timestamps.push(sample.ts_ms + offset_ms);
    }
    let mut body = String::new();
    for (labels, (values, timestamps)) in series {
        let timestamps: Vec<String> = timestamps.iter().map(i64::to_string).collect();
        body.push_str(&format!(
            "{{\"metric\":{},\"values\":[{}],\"timestamps\":[{}]}}\n",
            serde_json::to_string(labels).expect("a string map serializes"),
            values.join(","),
            timestamps.join(",")
        ));
    }
    body
}

fn number_text(value: f64) -> String {
    if value.is_nan() {
        "NaN".into()
    } else if value == f64::INFINITY {
        "Inf".into()
    } else if value == f64::NEG_INFINITY {
        "-Inf".into()
    } else {
        value.to_string()
    }
}

/// What the adapter reads from the server's `/metrics`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ServerMetrics {
    /// `vm_app_version`'s `version` label.
    pub app_version: Option<String>,
    /// `vminsert_streamagg_config_reloads_total`.
    pub config_reloads: u64,
    /// `vminsert_streamagg_config_last_reload_successful == 1`.
    pub last_reload_successful: bool,
    /// Per stream aggregation, by its 1-based `position` in the config:
    /// `vm_streamaggr_flush_duration_seconds_count`, the flushes so far.
    pub flushes_by_position: BTreeMap<u32, u64>,
    /// `vm_streamaggr_ignored_samples_total{reason="too_old"}`, summed: samples
    /// that arrived after their window had flushed.
    pub too_old_samples: u64,
    /// `vm_data_size_bytes`, summed over every type: the server's whole data.
    pub data_size_bytes: u64,
}

impl ServerMetrics {
    pub fn parse(text: &str) -> ServerMetrics {
        let mut metrics = ServerMetrics::default();
        for line in text.lines() {
            let (name_and_labels, value) = match line.rsplit_once(' ') {
                Some(pair) if !line.starts_with('#') => pair,
                _ => continue,
            };
            let (name, labels) = match name_and_labels.split_once('{') {
                Some((name, labels)) => (name, labels.trim_end_matches('}')),
                None => (name_and_labels, ""),
            };
            let count = || value.parse::<f64>().map(|v| v as u64).unwrap_or(0);
            match name {
                "vm_app_version" => metrics.app_version = label(labels, "version"),
                "vminsert_streamagg_config_reloads_total" => metrics.config_reloads = count(),
                "vminsert_streamagg_config_last_reload_successful" => {
                    metrics.last_reload_successful = value == "1"
                }
                "vm_streamaggr_flush_duration_seconds_count" => {
                    if let Some(position) = label(labels, "position").and_then(|p| p.parse().ok()) {
                        *metrics.flushes_by_position.entry(position).or_default() += count();
                    }
                }
                "vm_streamaggr_ignored_samples_total"
                    if label(labels, "reason").as_deref() == Some("too_old") =>
                {
                    metrics.too_old_samples += count()
                }
                "vm_data_size_bytes" => metrics.data_size_bytes += count(),
                _ => {}
            }
        }
        metrics
    }
}

/// The value of `name="value"` in a Prometheus label list.
fn label(labels: &str, name: &str) -> Option<String> {
    let start = labels.find(&format!("{name}=\""))? + name.len() + 2;
    let end = labels[start..].find('"')? + start;
    Some(labels[start..end].to_string())
}

/// The series of one `/api/v1/query` answer: labels without `__name__`,
/// plus `__name__ = metric` for a per-series answer (`per_series_of`).
pub fn parse_answer(
    body: &str,
    per_series_of: Option<&str>,
) -> Result<BTreeMap<SeriesLabels, f64>, String> {
    let answer: Value = serde_json::from_str(body).map_err(|e| format!("answer: {e}"))?;
    if answer["status"] != "success" {
        return Err(format!("answer: {body}"));
    }
    if answer["data"]["resultType"] != "vector" {
        return Err(format!("answer of type {}", answer["data"]["resultType"]));
    }
    let mut series = BTreeMap::new();
    for result in answer["data"]["result"].as_array().into_iter().flatten() {
        let mut labels: SeriesLabels = result["metric"]
            .as_object()
            .ok_or_else(|| format!("result without labels: {result}"))?
            .iter()
            .filter(|(name, _)| name.as_str() != "__name__")
            .map(|(name, value)| (name.clone(), value.as_str().unwrap_or("").to_string()))
            .collect();
        if let Some(metric) = per_series_of {
            labels.insert("__name__".into(), metric.into());
        }
        let text = result["value"][1]
            .as_str()
            .ok_or_else(|| format!("result without a value: {result}"))?;
        let value = match text {
            "NaN" => f64::NAN,
            "+Inf" | "Inf" => f64::INFINITY,
            "-Inf" => f64::NEG_INFINITY,
            _ => text
                .parse()
                .map_err(|_| format!("value '{text}' is not a number"))?,
        };
        if series.insert(labels.clone(), value).is_some() {
            return Err(format!("two series with the labels {labels:?}"));
        }
    }
    Ok(series)
}

/// The workload query id of each plan root: root i answers query i, whose
/// texts must include the plan's text of root i.
fn query_ids_of(plan: &PlanFile, workload: &WorkloadSpec) -> Result<Vec<String>, EngineError> {
    if plan.query_texts.len() != workload.queries.len() {
        return Err(EngineError(format!(
            "the plan has {} queries but workload {} has {}",
            plan.query_texts.len(),
            workload.name,
            workload.queries.len()
        )));
    }
    plan.query_texts
        .iter()
        .zip(&workload.queries)
        .map(|(plan_text, query)| {
            if query.texts.iter().any(|text| &text.text == plan_text) {
                Ok(query.id.clone())
            } else {
                let texts: Vec<&str> = query.texts.iter().map(|t| t.text.as_str()).collect();
                Err(EngineError(format!(
                    "query {}: the plan answers '{plan_text}', but the workload's texts are {texts:?}",
                    query.id
                )))
            }
        })
        .collect()
}

fn plan_summary(plan: &PlanFile, deployment: &VictoriaMetricsDeployment) -> PlanSummary {
    let queries: Vec<&str> = deployment.queries.iter().map(|q| q.expr.as_str()).collect();
    PlanSummary {
        text: format!(
            "# -streamAggr.config\n{}# queries\n{}\n",
            deployment.stream_aggregation_config(),
            queries.join("\n")
        ),
        node_kinds: plan
            .nodes
            .iter()
            .map(|node| operator_kind(&node.operator))
            .collect(),
        stored_outputs: deployment
            .stream_aggregations
            .iter()
            .flat_map(|a| a.outputs.iter().map(|&o| a.output_metric(o)))
            .collect(),
        substitutions: deployment
            .substitutions
            .iter()
            .map(|s| format!("{} at node {}: {}", s.rule, s.node, s.guarantee_difference))
            .collect(),
        sharing_lost: deployment
            .sharing_lost
            .iter()
            .map(|lost| {
                let readers: Vec<String> = lost.readers.iter().map(|r| r.to_string()).collect();
                format!("node {} for nodes {}", lost.node, readers.join(", "))
            })
            .collect(),
    }
}

/// The planner's operator name of `operator`, e.g. `SummaryAgg`.
fn operator_kind(operator: &PlanOperator) -> String {
    match operator {
        PlanOperator::Scan { .. } => "Scan",
        PlanOperator::TimeRange { .. } => "TimeRange",
        PlanOperator::SummaryTumblingWindowChain { .. } => "SummaryTumblingWindowChain",
        PlanOperator::SummaryAgg { .. } => "SummaryAgg",
        PlanOperator::SummaryMerge { .. } => "SummaryMerge",
        PlanOperator::FinalizeExactAccumulator { .. } => "FinalizeExactAccumulator",
        PlanOperator::SummaryEstimate { .. } => "SummaryEstimate",
        PlanOperator::Aggregate { .. } => "Aggregate",
        PlanOperator::Project { .. } => "Project",
        PlanOperator::BinaryOp { .. } => "BinaryOp",
        PlanOperator::Other { kind } => kind,
    }
    .to_string()
}

/// The installed arm: its stream aggregations, queries and wall offset.
struct VictoriaMetricsArm {
    http: Http,
    agent_url: String,
    arm_installed: Arc<AtomicBool>,
    /// Per plan root: the workload query id and its query.
    queries: Vec<(String, MetricsQlQuery)>,
    /// In config order: aggregation `i` has `position` `i + 1`.
    aggregations: Vec<StreamAggregation>,
    offset_ms: i64,
    lead_ms: i64,
    /// The server's too-old count after the config loaded.
    too_old_at_install: u64,
    /// The server's data size after the config loaded.
    data_bytes_at_install: u64,
    summary: PlanSummary,
}

impl VictoriaMetricsArm {
    fn metrics(&self) -> Result<ServerMetrics, EngineError> {
        Ok(ServerMetrics::parse(&self.http.get("/metrics", &[])?))
    }

    /// Writes `config` as the server's `-streamAggr.config` and reloads it.
    fn load_config(&self, config: &str) -> Result<(), EngineError> {
        let before = self.metrics()?.config_reloads;
        put_server_config(&self.agent_url, config).map_err(|error| {
            EngineError(format!("writing the stream-aggregation config: {error}"))
        })?;
        self.http.post("/-/reload")?;
        let deadline = Instant::now() + RELOAD_TIMEOUT;
        loop {
            let metrics = self.metrics()?;
            if metrics.config_reloads > before {
                if !metrics.last_reload_successful {
                    return Err(EngineError(
                        "the server refused the stream-aggregation config (see its log)".into(),
                    ));
                }
                return Ok(());
            }
            if Instant::now() > deadline {
                return Err(EngineError("the server did not reload its config".into()));
            }
            std::thread::sleep(POLL_INTERVAL);
        }
    }

    /// The aggregations (1-based positions) with a window ending at wall
    /// time `wall_ms`, and the read shift of the narrowest, in ms.
    fn flushing_at(&self, wall_ms: i64) -> (Vec<u32>, Option<i64>) {
        let mut positions = Vec::new();
        let mut shift = None::<i64>;
        for (i, aggregation) in self.aggregations.iter().enumerate() {
            if wall_ms.rem_euclid(aggregation.interval_ms as i64) == 0 {
                positions.push(i as u32 + 1);
                let h = (aggregation.interval_ms as i64 - 1000) / 2;
                shift = Some(shift.map_or(h, |s| s.min(h)));
            }
        }
        (positions, shift)
    }
}

impl Drop for VictoriaMetricsArm {
    fn drop(&mut self) {
        // Best effort: stop the aggregators, so the next arm's server does
        // no work for this one.
        let _ = self.load_config("[]\n");
        self.arm_installed.store(false, Ordering::SeqCst);
    }
}

impl InstalledArm for VictoriaMetricsArm {
    fn plan_summary(&self) -> PlanSummary {
        self.summary.clone()
    }

    fn ingest(&mut self, chunk: &SampleChunk) -> Result<StepReport, EngineError> {
        for slice in slices(chunk, self.offset_ms, self.lead_ms) {
            sleep_until_wall_ms(slice.send_at_wall_ms);
            // Every sample must arrive before its window's flush: for each
            // stream aggregation, its window ending at or after the slice end.
            let wall_end = slice.end_ms + self.offset_ms;
            let now = now_ms();
            for aggregation in &self.aggregations {
                let width = aggregation.interval_ms as i64;
                let window_end = (wall_end + width - 1).div_euclid(width) * width;
                if now >= window_end {
                    return Err(EngineError(format!(
                        "pacing: the slice ending at data time {} ms was sent {} ms after its \
                         {width} ms window ended",
                        slice.end_ms,
                        now - window_end
                    )));
                }
            }
            if !slice.samples.is_empty() {
                self.http.post_body(
                    "/api/v1/import",
                    &import_body(&slice.samples, self.offset_ms),
                )?;
            }
        }
        let tick = chunk.end_ms + self.offset_ms;
        let (positions, shift_ms) = self.flushing_at(tick);
        if let Some(shift_ms) = shift_ms {
            let before = self.metrics()?.flushes_by_position;
            let read_at = now_ms();
            if read_at >= tick - BEFORE_TICK_MARGIN_MS {
                return Err(EngineError(format!(
                    "pacing: the chunk ending at data time {} ms was sent {} ms before its \
                     windows' end, under {BEFORE_TICK_MARGIN_MS} ms",
                    chunk.end_ms,
                    tick - read_at
                )));
            }
            sleep_until_wall_ms(tick);
            loop {
                let metrics = self.metrics()?;
                let flushed = positions.iter().all(|p| {
                    metrics.flushes_by_position.get(p).copied().unwrap_or(0)
                        > before.get(p).copied().unwrap_or(0)
                });
                let lag = now_ms() - tick;
                if flushed && lag < shift_ms {
                    break;
                }
                if lag >= shift_ms {
                    return Err(EngineError(format!(
                        "the stream flush of the windows ending at data time {} ms lagged \
                         {lag} ms, at least the read's shift {shift_ms} ms",
                        chunk.end_ms
                    )));
                }
                std::thread::sleep(POLL_INTERVAL);
            }
        }
        if !self.aggregations.is_empty() {
            let too_old = self.metrics()?.too_old_samples;
            if too_old > self.too_old_at_install {
                return Err(EngineError(format!(
                    "the server dropped {} samples that arrived after their window",
                    too_old - self.too_old_at_install
                )));
            }
        }
        // Make the new rows and stream outputs searchable.
        self.http.get("/internal/force_flush", &[])?;
        Ok(StepReport::default())
    }

    fn evaluate(&mut self, at_ms: i64) -> Result<(Answers, StepReport), EngineError> {
        let wall_ms = at_ms + self.offset_ms;
        // The chunk ending at `at_ms` was ingested (and flushed) already.
        if now_ms() < wall_ms {
            return Err(EngineError(format!(
                "evaluation at data time {at_ms} ms before its data was ingested"
            )));
        }
        let time = format!(
            "{}.{:03}",
            wall_ms.div_euclid(1000),
            wall_ms.rem_euclid(1000)
        );
        let mut answers = BTreeMap::new();
        for (id, query) in &self.queries {
            let body = self.http.get(
                "/api/v1/query",
                &[("query", &query.expr), ("time", &time), ("nocache", "1")],
            )?;
            let series = parse_answer(&body, query.per_series_of.as_deref())
                .map_err(|e| EngineError(format!("query {id}: {e}")))?;
            answers.insert(id.clone(), series);
        }
        Ok((Answers(answers), StepReport::default()))
    }

    fn footprint(&self) -> Result<Footprint, EngineError> {
        // The server's data growth since this arm's install: raw series and
        // stream outputs together (VictoriaMetrics does not split them).
        self.http.get("/internal/force_flush", &[])?;
        Ok(Footprint {
            view_bytes: 0,
            raw_retained_bytes: self
                .metrics()?
                .data_size_bytes
                .saturating_sub(self.data_bytes_at_install),
            peak_bytes: None,
        })
    }
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("after 1970")
        .as_millis() as i64
}

fn sleep_until_wall_ms(wall_ms: i64) {
    let wait = wall_ms - now_ms();
    if wait > 0 {
        std::thread::sleep(Duration::from_millis(wait as u64));
    }
}

fn lcm(a: u64, b: u64) -> u64 {
    let (mut x, mut y) = (a, b);
    while y != 0 {
        (x, y) = (y, x % y);
    }
    a / x * b
}

/// The server's HTTP interface.
#[derive(Clone)]
struct Http {
    agent: ureq::Agent,
    url: String,
}

impl Http {
    fn new(url: &str, timeout: Duration) -> Http {
        Http {
            agent: ureq::AgentBuilder::new()
                .timeout_connect(Duration::from_secs(5))
                .timeout(timeout)
                .build(),
            url: url.trim_end_matches('/').to_string(),
        }
    }

    fn get(&self, path: &str, params: &[(&str, &str)]) -> Result<String, EngineError> {
        let mut request = self.agent.get(&format!("{}{path}", self.url));
        for (name, value) in params {
            request = request.query(name, value);
        }
        Self::text(request.call(), &self.url)
    }

    fn post(&self, path: &str) -> Result<String, EngineError> {
        Self::text(
            self.agent.post(&format!("{}{path}", self.url)).call(),
            &self.url,
        )
    }

    fn post_body(&self, path: &str, body: &str) -> Result<String, EngineError> {
        Self::text(
            self.agent
                .post(&format!("{}{path}", self.url))
                .send_string(body),
            &self.url,
        )
    }

    fn text(sent: Result<ureq::Response, ureq::Error>, url: &str) -> Result<String, EngineError> {
        match sent {
            Ok(response) => {
                let mut text = String::new();
                std::io::Read::read_to_string(&mut response.into_reader(), &mut text)
                    .map_err(|error| EngineError(format!("reading the answer: {error}")))?;
                Ok(text)
            }
            Err(ureq::Error::Status(code, response)) => Err(EngineError(format!(
                "VictoriaMetrics answered {code}: {}",
                response.into_string().unwrap_or_default().trim_end()
            ))),
            Err(error) => Err(EngineError(format!("VictoriaMetrics at {url}: {error}"))),
        }
    }
}
