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
//! window's read shift. Every sample still lands in its own window (the
//! lead stays below the width), and a sample lags behind the wall clock
//! (which delays the server's flush by the lag) only when the harness's own
//! work between two chunks (the evaluation, the idle probe) takes longer
//! than the gap to the next chunk's first sample; the flush-lag check below
//! catches that. A slice that reaches the server less than 100 ms before
//! its window's end fails the step. The ingest of the chunk ending at `t`
//! then waits until `t`, then until every stream aggregation flushed once
//! for each of its ticks in the chunk (failing if that lags by `h` or more,
//! or if the server dropped a late sample), and makes the new rows
//! searchable, so this work is ingest CPU and an evaluation runs only its
//! queries. An arm that reads counters starts its data at least 10 minutes
//! plus its longest range after the previous arm's newest sample, so
//! MetricsQL's look-back for the sample before a range (`rate`, `increase`)
//! never reaches another arm's data; other arms start right away, since a
//! window rollup reads only its window, which holds only the arm's own data. One arm at a time per engine: the server's stream-aggregation
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
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use aqpbm_core::harness::cgroup::{put_server_config, CgroupSource};
use aqpbm_core::harness::data::Sample;
use aqpbm_core::harness::engine::{
    Answers, ArmSpec, CpuAccounting, EngineDescription, EngineError, EngineUnderTest, Footprint,
    InstalledArm, PlanSummary, SampleChunk, SeriesLabels, StepReport, WorkloadSpec,
};
use serde_json::Value;

use crate::plan_file::{PlanFile, PlanOperator, SummaryFamily};
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
    /// The wall time of the newest sample any arm of this engine sent, in
    /// ms (0 before any).
    newest_sample_wall_ms: Arc<AtomicI64>,
}

/// Installs so far in this process, for each install's config nonce.
static INSTALLS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// How long to wait for the server to apply a reloaded config.
const RELOAD_TIMEOUT: Duration = Duration::from_secs(10);
/// How often a wait polls the server's `/metrics`.
const POLL_INTERVAL: Duration = Duration::from_millis(50);
/// The least time a slice reaches the server before its window ends.
const BEFORE_WINDOW_END_MARGIN_MS: i64 = 100;
/// How far before a range MetricsQL reads a series for an instant query:
/// its silence interval (5 min) plus the larger of the range and the step
/// (5 min by default), where it finds the sample before the range that
/// `rate` and `increase` start from. An arm's data starts at least this
/// plus its longest range after the previous arm's newest sample, so no arm
/// reads another arm's samples of the same series.
const PREVIOUS_SAMPLE_LOOKBACK_MS: i64 = 600_000;
/// The least time between installing an arm and its first data, so the
/// data starts after the reloaded config's first tick.
const START_MARGIN_MS: i64 = 2_000;

impl VictoriaMetricsEngine {
    /// Connects to the server and reads its version.
    pub fn new(server: VictoriaMetricsServer) -> Result<VictoriaMetricsEngine, EngineError> {
        let http = Http::new(&server.url, server.request_timeout);
        let metrics = ServerMetrics::parse(&http.get("/metrics", &[])?, "");
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
            newest_sample_wall_ms: Arc::new(AtomicI64::new(0)),
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
            newest_sample_wall_ms: Arc::clone(&self.newest_sample_wall_ms),
            aggregation_name: String::new(),
            flush_counts: (0, BTreeMap::new()),
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
        // Each install names its aggregators, so its config always differs
        // from the last one (a reload with unchanged bytes keeps the old
        // aggregators) and its metrics are told apart from earlier ones.
        installed.aggregation_name = format!(
            "install-{}-{}",
            std::process::id(),
            INSTALLS.fetch_add(1, Ordering::Relaxed)
        );
        let config = deployment.stream_aggregation_config_named(&installed.aggregation_name);
        installed.load_config(&config, deployment.stream_aggregations.len())?;
        installed.http.get("/internal/force_flush", &[])?;
        let metrics = installed.metrics()?;
        installed.too_old_at_install = metrics.too_old_samples;
        installed.data_bytes_at_install = metrics.data_size_bytes;
        let flushes_at_install = metrics.flushes_by_position;
        let step_ms = deployment
            .stream_aggregations
            .iter()
            .fold(lcm(1000, interval_ms), |step, a| lcm(step, a.interval_ms));
        let data_start = data_start_ms(workload);
        let earliest_start = now_ms()
            .max(self.newest_sample_wall_ms.load(Ordering::SeqCst) + previous_arm_gap_ms(&plan));
        installed.offset_ms = wall_offset_ms(earliest_start, data_start, step_ms);
        // Flushes are counted for the ticks after the data start; earlier
        // ones (empty windows) only add to the counts.
        installed.flush_counts = (data_start + installed.offset_ms, flushes_at_install);
        // The wait for the data start is the install's, not the first
        // chunk's.
        sleep_until_wall_ms(data_start + installed.offset_ms - installed.lead_ms);
        Ok(Box::new(installed))
    }
}

/// How long after the previous arm's newest sample this arm's data must
/// start, in ms: 0 for a plan that reads only its windows, and
/// [`PREVIOUS_SAMPLE_LOOKBACK_MS`] plus the longest range for one that reads
/// counters (`rate` and `increase` start from the sample before their
/// range, which MetricsQL looks for that far back). Example: `sum by (job)
/// (rate(m[1m]))`: 660,000 ms; `max_over_time(m[1m])`: 0.
pub fn previous_arm_gap_ms(plan: &PlanFile) -> i64 {
    let reads_counters = plan.nodes.iter().any(|node| match &node.operator {
        PlanOperator::SummaryTumblingWindowChain { family, .. }
        | PlanOperator::SummaryAgg { family, .. } => matches!(
            family,
            SummaryFamily::Exact(kind) if matches!(kind.as_str(), "Rate" | "Increase" | "IRate")
        ),
        _ => false,
    });
    if !reads_counters {
        return 0;
    }
    let longest_range_ms = plan
        .nodes
        .iter()
        .filter_map(|node| match node.operator {
            PlanOperator::TimeRange { range_ms, .. } => Some(range_ms as i64),
            _ => None,
        })
        .max()
        .unwrap_or(0);
    PREVIOUS_SAMPLE_LOOKBACK_MS + longest_range_ms
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
    /// `vminsert_streamagg_config_reloads_errors_total`.
    pub config_reload_errors: u64,
    /// The positions of the aggregations named `aggregation_name` the server
    /// runs (they have an ignored-samples counter from their start).
    pub running_positions: std::collections::BTreeSet<u32>,
    /// Per stream aggregation named `aggregation_name`, by its 1-based
    /// `position` in the config: `vm_streamaggr_flush_duration_seconds_count`,
    /// the flushes so far.
    pub flushes_by_position: BTreeMap<u32, u64>,
    /// `vm_streamaggr_ignored_samples_total{reason="too_old"}` of the
    /// aggregations named `aggregation_name`, summed: samples that arrived
    /// after their window had flushed.
    pub too_old_samples: u64,
    /// `vm_data_size_bytes`, summed over every type: the server's whole data.
    pub data_size_bytes: u64,
}

impl ServerMetrics {
    /// The metrics of `/metrics` text; the stream-aggregation counters of the
    /// aggregations named `aggregation_name` only.
    pub fn parse(text: &str, aggregation_name: &str) -> ServerMetrics {
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
            let ours = || label(labels, "name").as_deref() == Some(aggregation_name);
            match name {
                "vm_app_version" => metrics.app_version = label(labels, "version"),
                "vminsert_streamagg_config_reloads_total" => metrics.config_reloads = count(),
                "vminsert_streamagg_config_reloads_errors_total" => {
                    metrics.config_reload_errors = count()
                }
                "vm_streamaggr_flush_duration_seconds_count" if ours() => {
                    if let Some(position) = label(labels, "position").and_then(|p| p.parse().ok()) {
                        *metrics.flushes_by_position.entry(position).or_default() += count();
                    }
                }
                "vm_streamaggr_ignored_samples_total" if ours() => {
                    if let Some(position) = label(labels, "position").and_then(|p| p.parse().ok()) {
                        metrics.running_positions.insert(position);
                    }
                    if label(labels, "reason").as_deref() == Some("too_old") {
                        metrics.too_old_samples += count();
                    }
                }
                "vm_data_size_bytes" => metrics.data_size_bytes += count(),
                _ => {}
            }
        }
        metrics
    }
}

/// The value of `name="value"` in a Prometheus label list such as
/// `a="1",name="x"` (not the `name` inside `short_name`).
fn label(labels: &str, name: &str) -> Option<String> {
    let mut rest = labels;
    loop {
        let (key, after) = rest.split_once("=\"")?;
        let end = after.find('"')?;
        if key.trim_start_matches([',', ' ']) == name {
            return Some(after[..end].to_string());
        }
        rest = &after[end + 1..];
    }
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
    newest_sample_wall_ms: Arc<AtomicI64>,
    /// The `name` of this install's stream aggregations, e.g. `install-41-0`.
    aggregation_name: String,
    /// The flush counts per position, and the wall time (ms) whose later
    /// ticks they do not include yet: the data start, then each chunk end.
    flush_counts: (i64, BTreeMap<u32, u64>),
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
        Ok(ServerMetrics::parse(
            &self.http.get("/metrics", &[])?,
            &self.aggregation_name,
        ))
    }

    /// Writes `config` as the server's `-streamAggr.config`, reloads it, and
    /// waits until the server runs its `aggregations` aggregations (named
    /// [`Self::aggregation_name`]); a refused config is an error.
    fn load_config(&self, config: &str, aggregations: usize) -> Result<(), EngineError> {
        let before = self.metrics()?;
        put_server_config(&self.agent_url, config).map_err(|error| {
            EngineError(format!("writing the stream-aggregation config: {error}"))
        })?;
        self.http.post("/-/reload")?;
        let deadline = Instant::now() + RELOAD_TIMEOUT;
        loop {
            let metrics = self.metrics()?;
            if metrics.config_reload_errors > before.config_reload_errors {
                return Err(EngineError(
                    "the server refused the stream-aggregation config (see its log)".into(),
                ));
            }
            // The reload counter grows when a reload starts; the named
            // aggregations' counters appear once it has loaded them.
            if metrics.config_reloads > before.config_reloads
                && metrics.running_positions.len() == aggregations
            {
                if aggregations == 0 {
                    // Nothing to see for an empty config: give the reload
                    // its few milliseconds.
                    std::thread::sleep(POLL_INTERVAL);
                }
                return Ok(());
            }
            if Instant::now() > deadline {
                return Err(EngineError("the server did not reload its config".into()));
            }
            std::thread::sleep(POLL_INTERVAL);
        }
    }

    /// The read shift `h` of the narrowest window, in ms: the most a flush
    /// may lag.
    fn narrowest_shift_ms(&self) -> Option<i64> {
        self.aggregations
            .iter()
            .map(|a| (a.interval_ms as i64 - 1000) / 2)
            .min()
    }

    /// Waits until every aggregation flushed once for each of its ticks in
    /// `(counted up to, tick]`, failing if that takes `h` or more after
    /// `tick`.
    fn wait_for_flushes(&mut self, tick: i64) -> Result<(), EngineError> {
        let Some(shift_ms) = self.narrowest_shift_ms() else {
            return Ok(());
        };
        let (counted_at, before) = self.flush_counts.clone();
        let expected: Vec<(u32, u64)> = self
            .aggregations
            .iter()
            .enumerate()
            .map(|(i, a)| {
                let width = a.interval_ms as i64;
                let ticks = tick.div_euclid(width) - counted_at.div_euclid(width);
                let position = i as u32 + 1;
                (
                    position,
                    before.get(&position).copied().unwrap_or(0) + ticks.max(0) as u64,
                )
            })
            .collect();
        loop {
            let metrics = self.metrics()?;
            let flushed = expected.iter().all(|(position, count)| {
                metrics
                    .flushes_by_position
                    .get(position)
                    .copied()
                    .unwrap_or(0)
                    >= *count
            });
            let lag = now_ms() - tick;
            if lag >= shift_ms {
                return Err(EngineError(format!(
                    "the stream flushes up to wall time {tick} ms took {lag} ms, at least the \
                     read's shift {shift_ms} ms"
                )));
            }
            if flushed {
                if metrics.too_old_samples > self.too_old_at_install {
                    return Err(EngineError(format!(
                        "the server dropped {} samples that arrived after their window",
                        metrics.too_old_samples - self.too_old_at_install
                    )));
                }
                self.flush_counts = (tick, metrics.flushes_by_position);
                return Ok(());
            }
            std::thread::sleep(POLL_INTERVAL);
        }
    }
}

impl Drop for VictoriaMetricsArm {
    fn drop(&mut self) {
        // Best effort: stop the aggregators, so the next arm's server does
        // no work for this one.
        let _ = self.load_config("[]\n", 0);
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
                if now >= window_end - BEFORE_WINDOW_END_MARGIN_MS {
                    return Err(EngineError(format!(
                        "pacing: the slice ending at data time {} ms was sent {} ms before its \
                         {width} ms window ended, under {BEFORE_WINDOW_END_MARGIN_MS} ms",
                        slice.end_ms,
                        window_end - now
                    )));
                }
            }
            if !slice.samples.is_empty() {
                self.http.post_body(
                    "/api/v1/import",
                    &import_body(&slice.samples, self.offset_ms),
                )?;
                self.newest_sample_wall_ms
                    .fetch_max(wall_end, Ordering::SeqCst);
            }
        }
        // The chunk's windows end at its end: wait for it, then for their
        // flushes (the flush is ingest work).
        let tick = chunk.end_ms + self.offset_ms;
        sleep_until_wall_ms(tick);
        self.wait_for_flushes(tick)?;
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
