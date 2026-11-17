//! A ClickHouse server as an engine under test: each arm is a plan file
//! translated by [`crate::clickhouse::translate`] and installed in a fresh
//! database of the server over HTTP.
//!
//! Example: an arm whose `engine_options` are `{"plan_file": "q4c.plan.json"}`
//! installs `src_latency` in database `bench_4711_0`. Each refresh chunk is
//! one `INSERT` per source table, and each evaluation at `t` runs the plan's
//! queries with `param_t=<t>`.
//!
//! Series: a sample's series identity is its full label map as compact
//! JSON with sorted keys, e.g. `{"__name__":"latency","job":"a"}`. A
//! per-series answer is keyed by that map (`__name__` kept, as the harness
//! keys answers). An aggregated answer is keyed by its label columns, with
//! empty values left out, as PromQL leaves out an empty label.
//!
//! CPU: the server runs in its own cgroup, read through the cgroup agent.
//! The server's HTTP summary header has no CPU counter, so
//! [`StepReport::engine_reported_cpu_ns`] is `None`; `scanned_bytes` is the
//! summed `read_bytes` of the step's statements.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use aqpbm_core::harness::cgroup::CgroupSource;
use aqpbm_core::harness::engine::{
    Answers, ArmSpec, CpuAccounting, EngineDescription, EngineError, EngineUnderTest, Footprint,
    InstalledArm, PlanSummary, SampleChunk, SeriesLabels, StepReport, WorkloadSpec,
};

use crate::clickhouse::{translate, SourceTable, INSERT_SETTINGS, SERIES_IDENTITY};
use crate::plan_file::{PlanFile, PlanOperator};

/// How to reach the server and how to run its queries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClickHouseServer {
    /// The HTTP interface, e.g. `http://10.0.0.1:8123`.
    pub url: String,
    /// The cgroup agent beside the server, e.g. `http://10.0.0.1:9101`.
    pub cgroup_agent_url: String,
    /// `max_threads` of every statement (1 for the main figures).
    pub max_threads: u32,
    /// How long one statement may take before it fails, e.g. 600 s.
    pub statement_timeout: Duration,
}

/// A ClickHouse server; see the module docs.
pub struct ClickHouseEngine {
    server: ClickHouseServer,
    http: Http,
    description: EngineDescription,
    /// Each install's database is `<database_prefix>_<n>`.
    database_prefix: String,
    installs: AtomicU64,
}

impl ClickHouseEngine {
    /// Connects to the server and reads its version and git hash. A
    /// database of an earlier run (`bench_*`) is refused: dropping it, or its
    /// background merges, would be charged to this run's measurements.
    pub fn new(server: ClickHouseServer) -> Result<ClickHouseEngine, EngineError> {
        let http = Http::new(&server.url, server.statement_timeout);
        let (leftovers, _) = http.run(
            "SELECT name FROM system.databases WHERE startsWith(name, 'bench_') FORMAT TabSeparated",
            &[],
            None,
        )?;
        if !leftovers.trim().is_empty() {
            return Err(EngineError(format!(
                "the server holds databases of an earlier run: {}; start it from an empty data dir",
                leftovers.trim().replace('\n', ", ")
            )));
        }
        let (text, _) = http.run(
            "SELECT version(), (SELECT value FROM system.build_options \
             WHERE name = 'VERSION_GITHASH') FORMAT TabSeparated",
            &[],
            None,
        )?;
        let (version, git_hash) = text
            .trim_end()
            .split_once('\t')
            .ok_or_else(|| EngineError(format!("unexpected version answer '{text}'")))?;
        let description = EngineDescription {
            id: "clickhouse".into(),
            version: format!("{version} ({git_hash})"),
            version_dirty: false,
            build_profile: format!("max_threads={}", server.max_threads),
        };
        Ok(ClickHouseEngine {
            database_prefix: format!("bench_{}", std::process::id()),
            server,
            http,
            description,
            installs: AtomicU64::new(0),
        })
    }
}

impl EngineUnderTest for ClickHouseEngine {
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
        check_evaluation_times(workload, &deployment.query_time_steps_ms)?;
        if let Some(source) = deployment.sources.iter().find(|source| {
            source
                .label_columns
                .iter()
                .any(|label| label == "ts" || label == "value")
        }) {
            return Err(EngineError(format!(
                "metric {} has a label named ts or value, the names of its sample columns",
                source.metric
            )));
        }

        let database = format!(
            "{}_{}",
            self.database_prefix,
            self.installs.fetch_add(1, Ordering::Relaxed)
        );
        self.http
            .run(&format!("CREATE DATABASE {database}"), &[], None)?;
        // From here on, dropping the arm drops the database.
        let installed = ClickHouseArm {
            http: self.http.clone(),
            database,
            max_threads: self.server.max_threads,
            sources: deployment.sources.clone(),
            queries: query_ids
                .into_iter()
                .zip(deployment.queries.clone())
                .collect(),
            stored_tables: deployment.stored_tables.clone(),
            summary: plan_summary(&plan, &deployment),
        };
        // TTL statements are never applied: the harness replays data, and a
        // TTL deletes by wall-clock time.
        for statement in deployment
            .tables
            .iter()
            .chain(&deployment.materialized_views)
        {
            installed.run(statement, &[], None)?;
        }
        Ok(Box::new(installed))
    }
}

/// Every evaluation time is a multiple of the evaluation interval from an
/// aligned data start ([`aqpbm_core::harness::arm_run::ArmSchedule`]), so a query
/// that reads stored windows answers right only when its step divides the
/// interval and the data start. Refused at install, not at the first
/// evaluation.
fn check_evaluation_times(workload: &WorkloadSpec, steps_ms: &[u64]) -> Result<(), EngineError> {
    let data_start_ms = workload
        .data
        .iter()
        .map(|record| record.description.start_ms)
        .min()
        .unwrap_or(0);
    for (query, &step_ms) in workload.queries.iter().zip(steps_ms) {
        let step = step_ms as i64;
        if (query.evaluation_interval_ms as i64) % step != 0 || data_start_ms % step != 0 {
            return Err(EngineError(format!(
                "query {} reads stored windows of {step_ms} ms, which do not divide its \
                 evaluation interval ({} ms) and the data start ({data_start_ms} ms)",
                query.id, query.evaluation_interval_ms
            )));
        }
    }
    Ok(())
}

/// The workload query id of each plan root: root i answers query i, whose
/// texts (in any language) must include the plan's text of root i.
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

fn plan_summary(
    plan: &PlanFile,
    deployment: &crate::clickhouse::ClickHouseDeployment,
) -> PlanSummary {
    PlanSummary {
        text: deployment.script(),
        node_kinds: plan
            .nodes
            .iter()
            .map(|node| operator_kind(&node.operator))
            .collect(),
        stored_outputs: deployment.stored_tables.clone(),
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

/// One installed arm: its database and the queries it answers.
struct ClickHouseArm {
    http: Http,
    database: String,
    max_threads: u32,
    sources: Vec<SourceTable>,
    /// Per plan root: the workload query id and its SQL.
    queries: Vec<(String, String)>,
    stored_tables: Vec<String>,
    summary: PlanSummary,
}

impl ClickHouseArm {
    /// Runs `sql` in the arm's database with its `max_threads`.
    fn run(
        &self,
        sql: &str,
        extra: &[(&str, &str)],
        insert_rows: Option<&str>,
    ) -> Result<(String, u64), EngineError> {
        let threads = self.max_threads.to_string();
        let mut params = vec![
            ("database", self.database.as_str()),
            ("max_threads", threads.as_str()),
        ];
        params.extend_from_slice(extra);
        self.http.run(sql, &params, insert_rows)
    }
}

impl Drop for ClickHouseArm {
    fn drop(&mut self) {
        // Best effort: a server that is gone keeps nothing anyway.
        let _ = self.http.run(
            &format!("DROP DATABASE IF EXISTS {} SYNC", self.database),
            &[],
            None,
        );
    }
}

impl InstalledArm for ClickHouseArm {
    fn plan_summary(&self) -> PlanSummary {
        self.summary.clone()
    }

    fn ingest(&mut self, chunk: &SampleChunk) -> Result<StepReport, EngineError> {
        let mut scanned_bytes = 0;
        for source in &self.sources {
            let mut rows = String::new();
            for sample in &chunk.samples {
                if sample.labels.get("__name__") != Some(&source.metric) {
                    continue;
                }
                if sample.ts_ms <= 0 {
                    return Err(EngineError(format!(
                        "a sample at {} ms: timestamps must be after 0",
                        sample.ts_ms
                    )));
                }
                rows.push_str(&sample.ts_ms.to_string());
                rows.push('\t');
                rows.push_str(&float_text(sample.value));
                for label in &source.label_columns {
                    rows.push('\t');
                    let value = if label == SERIES_IDENTITY {
                        series_identity(&sample.labels)
                    } else {
                        sample.labels.get(label).cloned().unwrap_or_default()
                    };
                    push_tsv_escaped(&mut rows, &value);
                }
                rows.push('\n');
            }
            if rows.is_empty() {
                continue;
            }
            let (_, read_bytes) = self.run(&insert_statement(source), &[], Some(&rows))?;
            scanned_bytes += read_bytes;
        }
        Ok(StepReport {
            scanned_bytes: Some(scanned_bytes),
            engine_reported_cpu_ns: None,
        })
    }

    fn evaluate(&mut self, at_ms: i64) -> Result<(Answers, StepReport), EngineError> {
        let t = at_ms.to_string();
        let mut answers = BTreeMap::new();
        let mut scanned_bytes = 0;
        for (id, sql) in &self.queries {
            let (text, read_bytes) = self.run(
                &format!("{sql} FORMAT TabSeparatedWithNames"),
                &[("param_t", &t)],
                None,
            )?;
            scanned_bytes += read_bytes;
            answers.insert(
                id.clone(),
                parse_answer(&text).map_err(|e| EngineError(format!("query {id}: {e}")))?,
            );
        }
        Ok((
            Answers(answers),
            StepReport {
                scanned_bytes: Some(scanned_bytes),
                engine_reported_cpu_ns: None,
            },
        ))
    }

    fn footprint(&self) -> Result<Footprint, EngineError> {
        let (text, _) = self.run(
            &format!(
                "SELECT table, sum(bytes_on_disk) FROM system.parts \
                 WHERE database = '{}' AND active GROUP BY table FORMAT TabSeparated",
                self.database
            ),
            &[],
            None,
        )?;
        let mut footprint = Footprint::default();
        for line in text.lines() {
            let (table, bytes) = line
                .split_once('\t')
                .ok_or_else(|| EngineError(format!("unexpected parts row '{line}'")))?;
            let bytes: u64 = bytes
                .parse()
                .map_err(|_| EngineError(format!("unexpected parts row '{line}'")))?;
            if self.stored_tables.iter().any(|t| t == table) {
                footprint.view_bytes += bytes;
            } else if self.sources.iter().any(|s| s.table == table) {
                footprint.raw_retained_bytes += bytes;
            }
        }
        Ok(footprint)
    }
}

/// `INSERT … SELECT … FROM input(…) FORMAT TabSeparated` into `source`: rows
/// of `ts_ms`, `value`, then one column per label column.
fn insert_statement(source: &SourceTable) -> String {
    let inputs: Vec<String> = (0..source.label_columns.len())
        .map(|i| format!("c{i} String"))
        .collect();
    let selected: Vec<String> = (0..source.label_columns.len())
        .map(|i| format!("c{i}"))
        .collect();
    let columns: Vec<String> = source
        .label_columns
        .iter()
        .map(|label| format!("`{}`", label.replace('`', "\\`")))
        .collect();
    format!(
        "INSERT INTO {table} (ts, value, {columns}) SETTINGS {INSERT_SETTINGS} \
         SELECT fromUnixTimestamp64Milli(ts_ms), value, {selected} \
         FROM input('ts_ms Int64, value Float64, {inputs}') FORMAT TabSeparated",
        table = source.table,
        columns = columns.join(", "),
        selected = selected.join(", "),
        inputs = inputs.join(", "),
    )
}

/// A series' identity: its label map as compact JSON, keys sorted.
pub fn series_identity(labels: &SeriesLabels) -> String {
    serde_json::to_string(labels).expect("a string map serializes")
}

/// `value` as TabSeparated reads it: `nan`, `inf`, `-inf`, or the shortest
/// decimal that reads back exactly.
fn float_text(value: f64) -> String {
    if value.is_nan() {
        "nan".into()
    } else {
        value.to_string()
    }
}

fn push_tsv_escaped(out: &mut String, value: &str) {
    for c in value.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '\t' => out.push_str("\\t"),
            '\n' => out.push_str("\\n"),
            c => out.push(c),
        }
    }
}

fn tsv_unescaped(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut chars = value.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('t') => out.push('\t'),
            Some('n') => out.push('\n'),
            Some('0') => out.push('\0'),
            Some(other) => out.push(other),
            None => out.push('\\'),
        }
    }
    out
}

/// The rows of one query answer (`TabSeparatedWithNames`, the value
/// column last) as series and values.
fn parse_answer(text: &str) -> Result<BTreeMap<SeriesLabels, f64>, String> {
    let mut lines = text.lines();
    let Some(header) = lines.next() else {
        return Ok(BTreeMap::new());
    };
    let names: Vec<String> = header.split('\t').map(tsv_unescaped).collect();
    let mut series = BTreeMap::new();
    for line in lines {
        let fields: Vec<String> = line.split('\t').map(tsv_unescaped).collect();
        if fields.len() != names.len() {
            return Err(format!("row '{line}' does not match the header '{header}'"));
        }
        let (value, labels) = fields.split_last().expect("at least the value");
        let identity = names.iter().position(|name| name == SERIES_IDENTITY);
        let key: SeriesLabels = match identity {
            Some(at) => serde_json::from_str(&labels[at])
                .map_err(|e| format!("series identity '{}': {e}", labels[at]))?,
            None => names
                .iter()
                .zip(labels)
                .filter(|(_, value)| !value.is_empty())
                .map(|(name, value)| (name.clone(), value.clone()))
                .collect(),
        };
        let value = parse_float(value).ok_or_else(|| format!("value '{value}' is not a number"))?;
        if series.insert(key.clone(), value).is_some() {
            return Err(format!("two rows for the series {key:?}"));
        }
    }
    Ok(series)
}

fn parse_float(text: &str) -> Option<f64> {
    match text {
        "nan" | "-nan" => Some(f64::NAN),
        "inf" | "+inf" => Some(f64::INFINITY),
        "-inf" => Some(f64::NEG_INFINITY),
        _ => text.parse().ok(),
    }
}

/// The server's HTTP interface. A statement is the `query` parameter; an
/// insert's rows are the body.
#[derive(Clone)]
struct Http {
    agent: ureq::Agent,
    url: String,
    /// `max_execution_time` of every statement, in whole seconds.
    max_execution_seconds: String,
}

/// How long to wait to connect.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);

impl Http {
    fn new(url: &str, statement_timeout: Duration) -> Http {
        Http {
            // The client waits a little longer than the server lets a
            // statement run, so the server's own timeout error arrives.
            agent: ureq::AgentBuilder::new()
                .timeout_connect(CONNECT_TIMEOUT)
                .timeout(statement_timeout + Duration::from_secs(5))
                .build(),
            url: url.trim_end_matches('/').to_string(),
            max_execution_seconds: statement_timeout.as_secs().max(1).to_string(),
        }
    }

    /// Runs `sql` with the URL parameters `params`; its output and the
    /// `read_bytes` of its summary header. The statement is the request
    /// body, except for an insert, whose body is its rows.
    fn run(
        &self,
        sql: &str,
        params: &[(&str, &str)],
        insert_rows: Option<&str>,
    ) -> Result<(String, u64), EngineError> {
        // `wait_end_of_query`: the status and the summary header come after
        // the statement ends, so an error raised mid-answer is a status
        // error and `read_bytes` is complete.
        let mut request = self
            .agent
            .post(&format!("{}/", self.url))
            .query("wait_end_of_query", "1")
            .query("max_execution_time", &self.max_execution_seconds);
        for (name, value) in params {
            request = request.query(name, value);
        }
        let sent = match insert_rows {
            Some(rows) => request.query("query", sql).send_string(rows),
            None => request.send_string(sql),
        };
        match sent {
            Ok(response) => {
                if let Some(code) = response.header("X-ClickHouse-Exception-Code") {
                    let code = code.to_string();
                    let text = response.into_string().unwrap_or_default();
                    return Err(EngineError(format!(
                        "ClickHouse exception {code}: {}",
                        text.trim_end()
                    )));
                }
                let read_bytes = response
                    .header("X-ClickHouse-Summary")
                    .and_then(|summary| serde_json::from_str::<serde_json::Value>(summary).ok())
                    .and_then(|summary| summary.get("read_bytes")?.as_str()?.parse().ok())
                    .unwrap_or(0);
                // `into_string` stops at 10 MB; an answer may be larger.
                let mut text = String::new();
                std::io::Read::read_to_string(&mut response.into_reader(), &mut text)
                    .map_err(|error| EngineError(format!("reading the answer: {error}")))?;
                Ok((text, read_bytes))
            }
            Err(ureq::Error::Status(code, response)) => {
                let text = response.into_string().unwrap_or_default();
                Err(EngineError(format!(
                    "ClickHouse answered {code}: {}",
                    text.trim_end()
                )))
            }
            Err(error) => Err(EngineError(format!("ClickHouse at {}: {error}", self.url))),
        }
    }
}
