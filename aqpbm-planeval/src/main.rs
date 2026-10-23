//! `planeval`: a workload of PromQL queries, one dataset, ASAPQuery-backend
//! against Prometheus, end to end. See `aqpbm-planeval/README.md`.
//!
//! For each arm (the `DeploymentCapabilities` the control plane plans with,
//! and whether it ignores accuracy guarantees) the harness first plans every
//! query alone, which tells which queries the backend supports. It then
//! plans the supported ones jointly, in batches of `--joint-size`, and for
//! each batch starts a fresh Prometheus and a fresh data plane, replays the
//! same samples into both over Remote Write in event-time order, closes
//! panes with the data plane's watermark endpoint at each evaluation time,
//! and queries both systems there through `/api/v1/query`, each query at its
//! own interval. Every arm and batch uses the same samples and times.

mod data;
mod plan_info;
mod procs;
mod promapi;
mod queries;
mod remote_write;

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{bail, ensure, Context};
use aqpbm_core::metrics::WallClock;
use clap::Parser;
use serde::Serialize;
use serde_json::{json, Value};

use data::{prometheus_quantile, rank_error, Dataset};
use plan_info::QueryMethod;
use procs::{Managed, ProcSample};
use queries::QuerySpec;

#[derive(Parser)]
#[command(about = "Compare ASAPQuery-backend against Prometheus on a workload of PromQL queries")]
struct Args {
    #[arg(long)]
    prometheus_bin: PathBuf,
    #[arg(long)]
    control_plane_bin: PathBuf,
    #[arg(long)]
    data_plane_bin: PathBuf,
    /// Planner `PlanningWorkload` JSON: repeating queries, each with its own
    /// `demand.fixed_interval`, over the gauge source (and the counter).
    #[arg(long)]
    workload: PathBuf,
    /// Run only these queries (`q01`, ...; workload order), repeatable.
    #[arg(long = "query")]
    only: Vec<String>,
    /// One arm: `NAME[,caps=CAPABILITIES.json][,ignore-accuracy-guarantees]`
    /// (`NAME=CAPABILITIES.json` also works); repeat for several arms.
    /// Without `caps` the control plane uses its default capabilities;
    /// `ignore-accuracy-guarantees` passes that flag to the control plane, so
    /// the planner may select outputs it cannot certify.
    #[arg(long = "arm", required = true)]
    arms: Vec<String>,
    /// Most queries planned together; the supported queries of an arm are
    /// split into batches of this size, in workload order.
    #[arg(long, default_value_t = 6)]
    joint_size: usize,
    /// A control-plane run taking longer is killed and counts as failed.
    #[arg(long, default_value_t = 600)]
    plan_timeout_seconds: u64,
    /// `KEY=VALUE` pins file copied into the summary.
    #[arg(long)]
    pins: Option<PathBuf>,
    #[arg(long)]
    out_dir: PathBuf,
    /// Seed for the value columns.
    #[arg(long, default_value_t = 42)]
    seed: u64,
    /// Evaluations of the query with the longest interval; the others run at
    /// their own intervals over the same span.
    #[arg(long, default_value_t = 6)]
    evaluations: usize,
    /// Times each query is sent to each system at each evaluation time.
    #[arg(long, default_value_t = 5)]
    repeats: usize,
    /// Event-time span of samples per Remote Write request.
    #[arg(long, default_value_t = 60)]
    batch_seconds: i64,
    /// Keep each batch's Prometheus TSDB directory instead of deleting it.
    #[arg(long)]
    keep_tsdb: bool,
    /// The gauge metric; the workload must have a source for it.
    #[arg(long, default_value = "latency_ms")]
    gauge_metric: String,
    /// The series' label hierarchy, coarsest level first, e.g.
    /// `region:2,service:2,instance:4` (16 series). Without it the series
    /// are `instance:N`, N the gauge source's `series_count`.
    #[arg(long)]
    labels: Option<String>,
    /// Total series: sets the last level's fan-out so the hierarchy holds N
    /// series (N must be a multiple of the levels above it).
    #[arg(long)]
    series: Option<usize>,
    /// `LABEL:LO,HI`: every group of hierarchy label LABEL scales its gauge
    /// and its counter increments by its own factor, log-uniform in
    /// `[LO, HI]` (`aqpbm-datagen`'s `scale_by`).
    #[arg(long)]
    scale: Option<String>,
    /// Name of the counter metric written next to the gauge; an empty name
    /// writes no counter.
    #[arg(long, default_value = "requests_total")]
    counter_metric: String,
    /// Seconds each closed-loop query-load step runs.
    #[arg(long, default_value_t = 3)]
    load_seconds: u64,
    /// Concurrent clients of one load step; repeat for several steps. Each
    /// query gets the steps at the last evaluation time, after its normal
    /// queries.
    #[arg(long = "load-concurrency", default_values_t = vec![16])]
    load_concurrency: Vec<usize>,
}

fn gcd(a: i64, b: i64) -> i64 {
    if b == 0 {
        a
    } else {
        gcd(b, a % b)
    }
}

#[derive(Debug, Clone, Serialize)]
struct Timeline {
    /// Pane-grid origin, a multiple of every query's interval; the first
    /// sample is one sample interval after it.
    origin_ms: i64,
    first_sample_ms: i64,
    last_sample_ms: i64,
    /// Query id -> its evaluation times. Every query's last one is
    /// `last_sample_ms`.
    evaluations_ms: BTreeMap<String, Vec<i64>>,
}

impl Timeline {
    fn times_of<'a>(&self, queries: impl IntoIterator<Item = &'a QuerySpec>) -> Vec<i64> {
        let set: BTreeSet<i64> = queries
            .into_iter()
            .flat_map(|q| self.evaluations_ms[&q.id].iter().copied())
            .collect();
        set.into_iter().collect()
    }
}

/// Picks evaluation times on each query's interval grid, all in the past:
/// the data plane closes panes no later than the wall clock and drops samples
/// too far ahead of it. The data plane is told the stream starts at the
/// origin (`--stream-start-ms`), so it builds the first pane too, and each
/// query's first evaluation is the first time on its grid that sees one full
/// range after the origin. The span is the longest range plus
/// `evaluations - 1` of the longest interval, rounded up to the common grid.
fn timeline(queries: &[QuerySpec], evaluations: usize, sample_interval_ms: i64) -> Timeline {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64;
    let grid = queries
        .iter()
        .fold(1, |l, q| l / gcd(l, q.interval_ms) * q.interval_ms);
    let longest_lead = queries.iter().map(QuerySpec::lead_ms).max().unwrap_or(0);
    let longest_interval = queries.iter().map(|q| q.interval_ms).max().unwrap_or(0);
    let span = longest_lead + longest_interval * (evaluations.max(1) as i64 - 1);
    let span = (span + grid - 1).div_euclid(grid) * grid;
    let margin = 5 * 60_000;
    let origin_ms = (now - margin - span).div_euclid(grid) * grid;
    let end = origin_ms + span;
    let evaluations_ms = queries
        .iter()
        .map(|q| {
            let first = (origin_ms + q.lead_ms() + q.interval_ms - 1).div_euclid(q.interval_ms)
                * q.interval_ms;
            let times = (0..)
                .map(|k| first + k * q.interval_ms)
                .take_while(|&t| t <= end)
                .collect();
            (q.id.clone(), times)
        })
        .collect();
    Timeline {
        origin_ms,
        first_sample_ms: origin_ms + sample_interval_ms,
        last_sample_ms: end,
        evaluations_ms,
    }
}

#[derive(Debug, Clone, Serialize)]
struct Planning {
    ok: bool,
    /// Why the control plane wrote no plan: its last log line, or a timeout.
    error: Option<String>,
    wall_ms: f64,
    peak_rss_kb: Option<u64>,
    stdout: String,
    selected_candidate: Value,
    stored_outputs: usize,
    query_time_raw_windows: usize,
    /// Query text -> what the plan does for it.
    methods: BTreeMap<String, QueryMethod>,
}

/// One arm of the run, parsed from `--arm`.
#[derive(Debug, Clone, Serialize)]
struct ArmSpec {
    name: String,
    caps: Option<PathBuf>,
    ignore_accuracy_guarantees: bool,
}

impl std::str::FromStr for ArmSpec {
    type Err = anyhow::Error;

    fn from_str(spec: &str) -> anyhow::Result<Self> {
        let mut parts = spec.split(',');
        let first = parts.next().unwrap_or_default();
        let (name, mut caps) = match first.split_once('=') {
            Some((n, c)) => (n, Some(PathBuf::from(c))),
            None => (first, None),
        };
        ensure!(!name.is_empty(), "arm {spec:?} has no name");
        let mut ignore_accuracy_guarantees = false;
        for part in parts {
            match part.split_once('=') {
                Some(("caps", c)) => caps = Some(PathBuf::from(c)),
                None if part == "ignore-accuracy-guarantees" => ignore_accuracy_guarantees = true,
                _ => bail!("arm {spec:?}: unknown option {part:?}"),
            }
        }
        Ok(Self {
            name: name.to_owned(),
            caps,
            ignore_accuracy_guarantees,
        })
    }
}

/// Runs the control plane on `workload` and reaps it with `wait4`, which
/// gives its own peak RSS rather than the maximum over every child reaped so
/// far. A run that fails or outlasts the timeout gives `ok: false` and the
/// reason; only a harness-side failure is an `Err`.
fn plan(args: &Args, workload: &Path, arm: &ArmSpec, out: &Path) -> anyhow::Result<Planning> {
    let _ = fs::remove_file(out);
    let mut command = Command::new(&args.control_plane_bin);
    command
        .arg("--workload")
        .arg(workload)
        .arg("--out")
        .arg(out);
    if let Some(caps) = &arm.caps {
        command.arg("--deployment-capabilities").arg(caps);
    }
    if arm.ignore_accuracy_guarantees {
        command.arg("--ignore-accuracy-guarantees");
    }
    let log = out.with_extension("log");
    let log_file = fs::File::create(&log)?;
    command.stdout(log_file.try_clone()?).stderr(log_file);
    let timeout = Duration::from_secs(args.plan_timeout_seconds);
    let clock = WallClock::start();
    let started = Instant::now();
    let child = command.spawn().context("starting control_plane")?;
    let pid = child.id() as libc::pid_t;
    let mut status = 0;
    let mut usage = std::mem::MaybeUninit::<libc::rusage>::zeroed();
    let mut timed_out = false;
    let rc = loop {
        // SAFETY: waits for our own child; status and usage are written when
        // it has been reaped (rc > 0).
        let rc = unsafe { libc::wait4(pid, &mut status, libc::WNOHANG, usage.as_mut_ptr()) };
        if rc != 0 {
            break rc;
        }
        if started.elapsed() > timeout {
            timed_out = true;
            // SAFETY: our own child, not yet reaped; then reap it.
            unsafe { libc::kill(pid, libc::SIGKILL) };
            break unsafe { libc::wait4(pid, &mut status, 0, usage.as_mut_ptr()) };
        }
        std::thread::sleep(Duration::from_millis(5));
    };
    let wall_ms = clock.elapsed_ns() as f64 / 1e6;
    ensure!(rc > 0, "wait4 on control_plane failed");
    // SAFETY: rc > 0, so wait4 wrote the struct; ru_maxrss is kB on Linux.
    let peak_rss_kb = Some(unsafe { usage.assume_init() }.ru_maxrss as u64);
    let stdout = fs::read_to_string(&log).unwrap_or_default();
    let exited_ok = libc::WIFEXITED(status) && libc::WEXITSTATUS(status) == 0;
    let error = if timed_out {
        Some(format!("timed out after {} s", args.plan_timeout_seconds))
    } else if !exited_ok {
        Some(
            stdout
                .lines()
                .rev()
                .map(str::trim)
                .find(|l| !l.is_empty())
                .unwrap_or("control_plane failed")
                .to_owned(),
        )
    } else {
        None
    };
    let plan: Value = match error {
        None => serde_json::from_str(&fs::read_to_string(out)?)?,
        Some(_) => Value::Null,
    };
    Ok(Planning {
        ok: error.is_none(),
        error,
        wall_ms,
        peak_rss_kb,
        stdout: stdout.trim().to_owned(),
        selected_candidate: plan["selection"].clone(),
        stored_outputs: plan["stored_outputs"].as_array().map_or(0, Vec::len),
        query_time_raw_windows: plan["query_time_raw_windows"]
            .as_array()
            .map_or(0, Vec::len),
        methods: plan_info::query_methods(&plan),
    })
}

#[derive(Debug, Clone, Default, Serialize)]
struct Ingest {
    requests: u64,
    samples: u64,
    /// Sum of Remote Write round trips, ms.
    send_ms: f64,
    /// Time until the samples were absorbed, summed over the replay
    /// segments, ms. Prometheus: its Remote Write round trips (it answers
    /// after the TSDB append). Data plane: its round trips (it answers once
    /// samples are queued) plus each segment's watermark call, which returns
    /// only after every queued sample is ingested and the panes are built.
    absorb_ms: f64,
    /// `samples / absorb_ms`, from one sequential writer.
    samples_per_second: f64,
}

/// One closed-loop load step: `concurrency` clients, each sending one query
/// back to back for the step's duration.
#[derive(Debug, Clone, Serialize)]
struct LoadStep {
    arm: String,
    batch: String,
    query_id: String,
    system: String,
    concurrency: usize,
    seconds: f64,
    requests: usize,
    ok: usize,
    /// Answers whose `x-asap-execution` is set and is not `plan`.
    not_plan: usize,
    qps: f64,
    p50_ms: Option<f64>,
    p99_ms: Option<f64>,
}

fn percentile(sorted: &[f64], p: f64) -> Option<f64> {
    (!sorted.is_empty()).then(|| sorted[((sorted.len() - 1) as f64 * p).round() as usize])
}

fn load_step(
    base: &str,
    query: &str,
    time_ms: i64,
    concurrency: usize,
    duration: Duration,
) -> LoadStep {
    let clock = WallClock::start();
    let deadline = Instant::now() + duration;
    let results: Vec<(bool, bool, f64)> = std::thread::scope(|scope| {
        let handles: Vec<_> = (0..concurrency)
            .map(|_| {
                scope.spawn(move || {
                    let client = reqwest::blocking::Client::builder()
                        .timeout(Duration::from_secs(300))
                        .build()
                        .expect("http client");
                    let mut out = Vec::new();
                    while Instant::now() < deadline {
                        out.push(
                            match promapi::instant_query(&client, base, query, time_ms) {
                                Ok(a) => (
                                    a.status == 200,
                                    a.execution.as_deref().is_some_and(|e| e != "plan"),
                                    a.latency_ms,
                                ),
                                Err(_) => (false, false, f64::NAN),
                            },
                        );
                    }
                    out
                })
            })
            .collect();
        handles
            .into_iter()
            .flat_map(|h| h.join().expect("load client"))
            .collect()
    });
    let seconds = clock.elapsed_ns() as f64 / 1e9;
    let mut latencies: Vec<f64> = results.iter().filter(|r| r.0).map(|r| r.2).collect();
    latencies.sort_by(f64::total_cmp);
    let ok = latencies.len();
    LoadStep {
        arm: String::new(),
        batch: String::new(),
        query_id: String::new(),
        system: String::new(),
        concurrency,
        seconds,
        requests: results.len(),
        ok,
        not_plan: results.iter().filter(|r| r.1).count(),
        qps: ok as f64 / seconds,
        p50_ms: percentile(&latencies, 0.5),
        p99_ms: percentile(&latencies, 0.99),
    }
}

/// One query planned alone.
#[derive(Debug, Clone, Serialize)]
struct ProbeRow {
    arm: String,
    query_id: String,
    query: String,
    supported: bool,
    reason: String,
    plan_ms: f64,
    method: String,
    guarantee: String,
    accepted_without_guarantee: bool,
}

/// One query at one evaluation time.
#[derive(Debug, Clone, Serialize)]
struct QueryRow {
    arm: String,
    batch: String,
    query_id: String,
    eval_time_ms: i64,
    prom_status: u16,
    asap_status: u16,
    asap_execution: String,
    asap_error: String,
    prom_series: usize,
    asap_series: usize,
    /// Series both systems answered.
    matched: usize,
    /// Series only Prometheus answered, and only ASAP.
    missing: usize,
    extra: usize,
    rel_error_max: Option<f64>,
    rel_error_mean: Option<f64>,
    abs_error_max: Option<f64>,
    rank_error_max: Option<f64>,
    /// For a quantile: Prometheus's answer against the quantile of the raw
    /// samples it returned for the rank basis, the largest gap. Zero shows the
    /// rank error is measured on the samples Prometheus's answer came from.
    prom_vs_raw_quantile_max_abs: Option<f64>,
    /// ASAP's error in the query's own requirement metric (rank, relative or
    /// absolute, against Prometheus), the largest over the series; how many
    /// series had one; and how many exceeded `max_error`.
    requirement_error_max: Option<f64>,
    requirement_checked: usize,
    requirement_violations: usize,
    prom_latency_ms_first: f64,
    prom_latency_ms_median: f64,
    asap_latency_ms_first: f64,
    asap_latency_ms_median: f64,
    /// The watermark call at this evaluation time, shared by every query due
    /// then.
    watermark_ms: f64,
    watermark_barrier_ms: Option<i64>,
}

/// One output series of one query at one evaluation time.
#[derive(Debug, Clone, Serialize)]
struct AnswerRow {
    arm: String,
    query_id: String,
    eval_time_ms: i64,
    /// The series' labels ([`data::label_key`]).
    series: String,
    prom_value: Option<f64>,
    asap_value: Option<f64>,
    abs_error: Option<f64>,
    rel_error: Option<f64>,
    /// For a quantile: how many raw samples Prometheus returned for this
    /// series' multiset, their quantile computed the way Prometheus does, and
    /// ASAP's rank error in them.
    rank_basis_samples: Option<usize>,
    prom_raw_quantile: Option<f64>,
    asap_rank_error: Option<f64>,
    /// ASAP's error in the query's requirement metric.
    requirement_error: Option<f64>,
}

/// One query's results in one arm, over all its evaluations.
#[derive(Debug, Clone, Serialize)]
struct PerQueryRow {
    arm: String,
    query_id: String,
    query: String,
    kind: String,
    interval_s: i64,
    accuracy_metric: String,
    max_error: Option<f64>,
    /// Planned alone.
    supported: bool,
    unsupported_reason: String,
    alone_plan_ms: f64,
    alone_method: String,
    /// The batch it was planned in, and whether that joint plan was made.
    batch: String,
    joint_planned: bool,
    joint_method: String,
    joint_sketch: Option<bool>,
    joint_guarantee: String,
    accepted_without_guarantee: Option<bool>,
    evaluations: usize,
    answered_by_plan: usize,
    missing: usize,
    extra: usize,
    rel_error_max: Option<f64>,
    rel_error_mean: Option<f64>,
    abs_error_max: Option<f64>,
    rank_error_max: Option<f64>,
    prom_vs_raw_quantile_max_abs: Option<f64>,
    /// ASAP's largest error in the requirement metric, the values checked,
    /// and how many exceeded `max_error`.
    requirement_error_max: Option<f64>,
    requirement_checked: usize,
    requirement_violations: usize,
    /// Every evaluation answered by the plan with the same series as
    /// Prometheus, at least one value checked, and none over `max_error`.
    /// `None` when the query never ran or has no measurable requirement.
    requirement_met: Option<bool>,
    /// Median over its evaluations of each evaluation's median latency.
    prom_latency_ms: Option<f64>,
    asap_latency_ms: Option<f64>,
}

/// The two systems' run for one batch.
#[derive(Debug, Clone, Serialize)]
struct SystemsReport {
    prometheus_port: u16,
    data_plane_port: u16,
    prometheus_startup_ms: f64,
    data_plane_startup_ms: f64,
    ingest_prometheus: Ingest,
    ingest_data_plane: Ingest,
    prometheus_idle: ProcSample,
    data_plane_idle: ProcSample,
    prometheus_end: ProcSample,
    data_plane_end: ProcSample,
    /// The larger of the final `VmHWM` and every `VmRSS` reading (at idle,
    /// after each evaluation's queries, at the end). Neither alone is a
    /// reliable peak: `VmHWM` can read lower than an earlier `VmRSS` on a
    /// multi-threaded process (the kernel batches per-thread RSS counters),
    /// and the readings miss transient peaks between them.
    prometheus_rss_max_sampled_kb: Option<u64>,
    data_plane_rss_max_sampled_kb: Option<u64>,
    data_plane_status: Value,
    prometheus_tsdb_status: Value,
    queries_answered_by_plan: usize,
    queries: usize,
}

#[derive(Debug, Clone, Serialize)]
struct BatchReport {
    batch: String,
    query_ids: Vec<String>,
    planning: Planning,
    systems: Option<SystemsReport>,
}

#[derive(Debug, Clone, Serialize)]
struct ArmReport {
    name: String,
    capabilities: Option<Value>,
    ignore_accuracy_guarantees: bool,
    probes: Vec<ProbeRow>,
    batches: Vec<BatchReport>,
}

fn median(v: &mut [f64]) -> f64 {
    v.sort_by(f64::total_cmp);
    let n = v.len();
    if n % 2 == 1 {
        v[n / 2]
    } else {
        (v[n / 2 - 1] + v[n / 2]) / 2.0
    }
}

/// What every arm shares: the run's arguments, workload, samples and times.
struct Run<'a> {
    args: &'a Args,
    workload: &'a Value,
    data: &'a Dataset,
    tl: &'a Timeline,
}

/// Everything the run collects across arms and batches.
#[derive(Default)]
struct Rows {
    queries: Vec<QueryRow>,
    answers: Vec<AnswerRow>,
    load: Vec<LoadStep>,
}

/// Accuracy of one ASAP answer against Prometheus's, series by series.
struct Accuracy {
    matched: usize,
    missing: usize,
    extra: usize,
    rel_errors: Vec<f64>,
    abs_errors: Vec<f64>,
    rank_errors: Vec<f64>,
    prom_vs_raw_quantile: Vec<f64>,
    requirement_errors: Vec<f64>,
}

/// A quantile's raw samples from Prometheus, as one sorted multiset per
/// output series ([`queries::RankBasis`]).
fn rank_multisets(basis: &queries::RankBasis, raw: &promapi::Answer) -> BTreeMap<String, Vec<f64>> {
    let mut sets: BTreeMap<String, Vec<f64>> = BTreeMap::new();
    for (labels, values) in &raw.raw {
        sets.entry(basis.grouping.key(labels))
            .or_default()
            .extend(values.iter().copied().filter(|v| v.is_finite()));
    }
    for set in sets.values_mut() {
        set.sort_by(f64::total_cmp);
    }
    sets
}

/// Compares ASAP's answer with Prometheus's series by series. `rank` holds a
/// quantile's multisets, which give its rank error.
fn compare(
    arm: &str,
    q: &QuerySpec,
    eval: i64,
    prom: &promapi::Answer,
    asap: &promapi::Answer,
    rank: Option<&BTreeMap<String, Vec<f64>>>,
    answers: &mut Vec<AnswerRow>,
) -> Accuracy {
    let mut acc = Accuracy {
        matched: 0,
        missing: 0,
        extra: 0,
        rel_errors: Vec::new(),
        abs_errors: Vec::new(),
        rank_errors: Vec::new(),
        prom_vs_raw_quantile: Vec::new(),
        requirement_errors: Vec::new(),
    };
    let keys: BTreeSet<&String> = prom.values.keys().chain(asap.values.keys()).collect();
    for key in keys {
        let prom_value = prom.values.get(key).copied();
        let asap_value = asap.values.get(key).copied();
        match (prom_value, asap_value) {
            (Some(_), Some(_)) => acc.matched += 1,
            (Some(_), None) => acc.missing += 1,
            _ => acc.extra += 1,
        }
        let (abs_error, rel_error) = match (prom_value, asap_value) {
            (Some(p), Some(a)) if p.is_finite() && a.is_finite() => {
                let abs = (a - p).abs();
                let rel = if p != 0.0 {
                    Some(abs / p.abs())
                } else {
                    (abs == 0.0).then_some(0.0)
                };
                (Some(abs), rel)
            }
            _ => (None, None),
        };
        let (mut rank_basis_samples, mut prom_raw_quantile, mut asap_rank_error) =
            (None, None, None);
        if let (Some(basis), Some(set)) = (&q.rank_basis, rank.and_then(|r| r.get(key))) {
            rank_basis_samples = Some(set.len());
            prom_raw_quantile = prometheus_quantile(set, basis.quantile);
            asap_rank_error = asap_value.and_then(|a| rank_error(set, basis.quantile, a));
            if let (Some(p), Some(r)) = (prom_value, prom_raw_quantile) {
                acc.prom_vs_raw_quantile.push((p - r).abs());
            }
        }
        let requirement_error = match q.accuracy_metric.as_deref() {
            Some("rank") => asap_rank_error,
            Some("relative") => rel_error,
            Some("absolute") => abs_error,
            _ => None,
        };
        acc.abs_errors.extend(abs_error);
        acc.rel_errors.extend(rel_error);
        acc.rank_errors.extend(asap_rank_error);
        acc.requirement_errors.extend(requirement_error);
        answers.push(AnswerRow {
            arm: arm.into(),
            query_id: q.id.clone(),
            eval_time_ms: eval,
            series: key.clone(),
            prom_value,
            asap_value,
            abs_error,
            rel_error,
            rank_basis_samples,
            prom_raw_quantile,
            asap_rank_error,
            requirement_error,
        });
    }
    acc
}

fn max(v: &[f64]) -> Option<f64> {
    v.iter().copied().reduce(f64::max)
}

fn mean(v: &[f64]) -> Option<f64> {
    (!v.is_empty()).then(|| v.iter().sum::<f64>() / v.len() as f64)
}

/// Starts a fresh Prometheus and data plane on `plan_path`, replays the
/// samples, and queries the batch's queries at their evaluation times.
fn run_batch(
    run: &Run,
    arm: &str,
    batch: &str,
    dir: &Path,
    plan_path: &Path,
    batch_queries: &[&QuerySpec],
    rows: &mut Rows,
) -> anyhow::Result<SystemsReport> {
    let Run { args, data, tl, .. } = *run;
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(300))
        .build()?;

    // Prometheus: empty config, no scraping; Remote Write receiver on.
    let prom_port = procs::free_port()?;
    let prom_url = format!("http://127.0.0.1:{prom_port}");
    let tsdb = dir.join("prometheus-data");
    let _ = fs::remove_dir_all(&tsdb);
    let prom_config = dir.join("prometheus.yml");
    fs::write(&prom_config, "global:\n  scrape_interval: 15s\n")?;
    let mut command = Command::new(&args.prometheus_bin);
    command
        .arg(format!("--config.file={}", prom_config.display()))
        .arg(format!("--storage.tsdb.path={}", tsdb.display()))
        .arg(format!("--web.listen-address=127.0.0.1:{prom_port}"))
        .arg("--web.enable-remote-write-receiver")
        .arg("--storage.tsdb.retention.time=30d");
    let mut prom = Managed::spawn("prometheus", command, &dir.join("prometheus.log"))?;
    let prom_startup = prom.wait_ready(
        &client,
        &format!("{prom_url}/-/ready"),
        Duration::from_secs(60),
    )?;

    // Data plane: never forwards, so every answer it gives is the plan's.
    let dp_port = procs::free_port()?;
    let dp_url = format!("http://127.0.0.1:{dp_port}");
    let mut command = Command::new(&args.data_plane_bin);
    command
        .arg("--plan")
        .arg(plan_path)
        .arg("--prometheus-url")
        .arg(&prom_url)
        .arg("--listen")
        .arg(format!("127.0.0.1:{dp_port}"))
        .arg("--no-forward")
        // Every sample from the origin on is written, so the first pane is
        // complete and built.
        .arg("--stream-start-ms")
        .arg(tl.origin_ms.to_string())
        .env("RUST_LOG", "info")
        .env("NO_COLOR", "1");
    let mut dp = Managed::spawn("data_plane", command, &dir.join("data_plane.log"))?;
    let dp_startup = dp.wait_ready(
        &client,
        &format!("{dp_url}/-/ready"),
        Duration::from_secs(60),
    )?;
    let prometheus_idle = procs::sample(prom.pid());
    let data_plane_idle = procs::sample(dp.pid());
    let mut prom_rss_max = prometheus_idle.rss_kb;
    let mut dp_rss_max = data_plane_idle.rss_kb;

    let mut ingest_prom = Ingest::default();
    let mut ingest_dp = Ingest::default();
    let batch_ms = args.batch_seconds * 1000;
    let mut cursor = tl.origin_ms; // everything at or before it is written
    let mut answered = 0;
    let mut asked = 0;
    let times = tl.times_of(batch_queries.iter().copied());
    let last_eval = tl.last_sample_ms;
    for &eval in &times {
        // The segment up to `eval`, encoded once and sent to each system as
        // one block, so one system's timing never overlaps the other's work.
        let mut bodies = Vec::new();
        let mut segment_samples = 0u64;
        while cursor < eval {
            let hi = (cursor + batch_ms).min(eval);
            let request = data.write_request(cursor, hi);
            segment_samples += request
                .timeseries
                .iter()
                .map(|s| s.samples.len() as u64)
                .sum::<u64>();
            bodies.push(remote_write::encode(&request));
            cursor = hi;
        }
        let mut dp_send_ms = 0.0;
        for (url, stats, send_ms) in [
            (format!("{prom_url}/api/v1/write"), &mut ingest_prom, None),
            (
                format!("{dp_url}/api/v1/write"),
                &mut ingest_dp,
                Some(&mut dp_send_ms),
            ),
        ] {
            let clock = WallClock::start();
            for body in &bodies {
                remote_write::send(&client, &url, body)?;
                stats.requests += 1;
            }
            let ms = clock.elapsed_ns() as f64 / 1e6;
            stats.send_ms += ms;
            stats.samples += segment_samples;
            match send_ms {
                Some(out) => *out = ms,
                None => stats.absorb_ms += ms,
            }
        }
        prom.check_alive()?;
        dp.check_alive()?;

        // Every sample at or before `eval` is written: close its panes.
        let clock = WallClock::start();
        let response = client
            .post(format!(
                "{dp_url}/api/v1/precompute/watermark?time_ms={eval}"
            ))
            .send()?;
        let watermark_ms = clock.elapsed_ns() as f64 / 1e6;
        ingest_dp.absorb_ms += dp_send_ms + watermark_ms;
        let status = response.status();
        let body: Value = response.json().unwrap_or(Value::Null);
        ensure!(status.is_success(), "watermark at {eval}: {status}: {body}");
        let barrier = body["barrier_ms"].as_i64();

        for q in batch_queries
            .iter()
            .filter(|q| tl.evaluations_ms[&q.id].binary_search(&eval).is_ok())
        {
            let mut prom_answers = Vec::new();
            let mut asap_answers = Vec::new();
            for _ in 0..args.repeats.max(1) {
                prom_answers.push(promapi::instant_query(&client, &prom_url, &q.text, eval)?);
            }
            for _ in 0..args.repeats.max(1) {
                asap_answers.push(promapi::instant_query(&client, &dp_url, &q.text, eval)?);
            }
            // A quantile's raw samples, which its rank error is measured on:
            // asked after the timed requests, so they do not count in latency.
            let rank = match &q.rank_basis {
                Some(basis) => {
                    let raw = promapi::instant_query(&client, &prom_url, &basis.query, eval)?;
                    ensure!(
                        raw.status == 200,
                        "{} rank basis {:?} at {eval}: {} {:?}",
                        q.id,
                        basis.query,
                        raw.status,
                        raw.error
                    );
                    Some(rank_multisets(basis, &raw))
                }
                None => None,
            };
            let prom_first = &prom_answers[0];
            let asap_first = &asap_answers[0];
            asked += 1;
            if asap_first.execution.as_deref() == Some("plan") {
                answered += 1;
            }
            let acc = compare(
                arm,
                q,
                eval,
                prom_first,
                asap_first,
                rank.as_ref(),
                &mut rows.answers,
            );
            let limit = q.max_error.unwrap_or(f64::INFINITY);
            let mut prom_lat: Vec<f64> = prom_answers.iter().map(|a| a.latency_ms).collect();
            let mut asap_lat: Vec<f64> = asap_answers.iter().map(|a| a.latency_ms).collect();
            let row = QueryRow {
                arm: arm.into(),
                batch: batch.into(),
                query_id: q.id.clone(),
                eval_time_ms: eval,
                prom_status: prom_first.status,
                asap_status: asap_first.status,
                asap_execution: asap_first.execution.clone().unwrap_or_default(),
                asap_error: asap_first.error.clone().unwrap_or_default(),
                prom_series: prom_first.values.len(),
                asap_series: asap_first.values.len(),
                matched: acc.matched,
                missing: acc.missing,
                extra: acc.extra,
                rel_error_max: max(&acc.rel_errors),
                rel_error_mean: mean(&acc.rel_errors),
                abs_error_max: max(&acc.abs_errors),
                rank_error_max: max(&acc.rank_errors),
                prom_vs_raw_quantile_max_abs: max(&acc.prom_vs_raw_quantile),
                requirement_error_max: max(&acc.requirement_errors),
                requirement_checked: acc.requirement_errors.len(),
                requirement_violations: acc
                    .requirement_errors
                    .iter()
                    .filter(|&&e| e > limit)
                    .count(),
                prom_latency_ms_first: prom_lat[0],
                prom_latency_ms_median: median(&mut prom_lat),
                asap_latency_ms_first: asap_lat[0],
                asap_latency_ms_median: median(&mut asap_lat),
                watermark_ms,
                watermark_barrier_ms: barrier,
            };
            if eval == last_eval {
                eprintln!(
                    "[{arm}/{batch}] {} t=end asap={} {} series P/A={}/{} rel_err_max={:?} prom={:.2}ms asap={:.2}ms",
                    q.id,
                    row.asap_status,
                    row.asap_execution,
                    row.prom_series,
                    row.asap_series,
                    row.rel_error_max,
                    row.prom_latency_ms_median,
                    row.asap_latency_ms_median
                );
            }
            rows.queries.push(row);
        }
        prom_rss_max = prom_rss_max.max(procs::sample(prom.pid()).rss_kb);
        dp_rss_max = dp_rss_max.max(procs::sample(dp.pid()).rss_kb);

        if eval == last_eval {
            let duration = Duration::from_secs(args.load_seconds);
            for q in batch_queries {
                for &concurrency in &args.load_concurrency {
                    for (system, base) in [("prometheus", &prom_url), ("asap", &dp_url)] {
                        let mut step = load_step(base, &q.text, eval, concurrency, duration);
                        step.arm = arm.into();
                        step.batch = batch.into();
                        step.query_id = q.id.clone();
                        step.system = system.into();
                        eprintln!(
                            "[{arm}/{batch}] {} load {system} x{concurrency}: {} ok of {}, {:.1} qps, p50 {:?} ms",
                            q.id, step.ok, step.requests, step.qps, step.p50_ms
                        );
                        rows.load.push(step);
                    }
                    prom_rss_max = prom_rss_max.max(procs::sample(prom.pid()).rss_kb);
                    dp_rss_max = dp_rss_max.max(procs::sample(dp.pid()).rss_kb);
                }
            }
        }
    }
    for stats in [&mut ingest_prom, &mut ingest_dp] {
        if stats.absorb_ms > 0.0 {
            stats.samples_per_second = stats.samples as f64 / (stats.absorb_ms / 1e3);
        }
    }

    let prometheus_end = procs::sample(prom.pid());
    let data_plane_end = procs::sample(dp.pid());
    prom_rss_max = prom_rss_max
        .max(prometheus_end.rss_kb)
        .max(prometheus_end.rss_peak_kb);
    dp_rss_max = dp_rss_max
        .max(data_plane_end.rss_kb)
        .max(data_plane_end.rss_peak_kb);
    let data_plane_status = promapi::get_json(&client, &format!("{dp_url}/api/v1/status/asap"));
    let prometheus_tsdb_status =
        promapi::get_json(&client, &format!("{prom_url}/api/v1/status/tsdb"));
    drop(dp);
    drop(prom);
    if !args.keep_tsdb {
        let _ = fs::remove_dir_all(&tsdb);
    }

    Ok(SystemsReport {
        prometheus_port: prom_port,
        data_plane_port: dp_port,
        prometheus_startup_ms: prom_startup.as_secs_f64() * 1e3,
        data_plane_startup_ms: dp_startup.as_secs_f64() * 1e3,
        ingest_prometheus: ingest_prom,
        ingest_data_plane: ingest_dp,
        prometheus_idle,
        data_plane_idle,
        prometheus_end,
        data_plane_end,
        prometheus_rss_max_sampled_kb: prom_rss_max,
        data_plane_rss_max_sampled_kb: dp_rss_max,
        data_plane_status,
        prometheus_tsdb_status,
        queries_answered_by_plan: answered,
        queries: asked,
    })
}

/// Probes every query alone, then plans and runs the supported ones in
/// joint batches.
fn run_arm(
    run: &Run,
    arm: &ArmSpec,
    all: &[QuerySpec],
    rows: &mut Rows,
) -> anyhow::Result<ArmReport> {
    let args = run.args;
    let name = arm.name.as_str();
    let dir = args.out_dir.join(name);
    let probe_dir = dir.join("probe");
    fs::create_dir_all(&probe_dir)?;

    let mut probes = Vec::new();
    for q in all {
        let workload = probe_dir.join(format!("{}.workload.json", q.id));
        fs::write(
            &workload,
            serde_json::to_string_pretty(&queries::workload_with(run.workload, &[q]))? + "\n",
        )?;
        let planning = plan(
            args,
            &workload,
            arm,
            &probe_dir.join(format!("{}.plan.json", q.id)),
        )?;
        let method = planning.methods.get(&q.text).cloned().unwrap_or_default();
        eprintln!(
            "[{name}] probe {} {:?}: {} ({:.0} ms) {}",
            q.id,
            q.text,
            if planning.ok {
                "supported"
            } else {
                "unsupported"
            },
            planning.wall_ms,
            planning.error.as_deref().unwrap_or(&method.method)
        );
        probes.push(ProbeRow {
            arm: name.into(),
            query_id: q.id.clone(),
            query: q.text.clone(),
            supported: planning.ok,
            reason: planning.error.clone().unwrap_or_default(),
            plan_ms: planning.wall_ms,
            method: method.method,
            guarantee: method.guarantee,
            accepted_without_guarantee: method.accepted_without_guarantee,
        });
    }

    let supported: Vec<&QuerySpec> = all
        .iter()
        .zip(&probes)
        .filter(|(_, p)| p.supported)
        .map(|(q, _)| q)
        .collect();
    let mut batches = Vec::new();
    for (k, batch_queries) in supported.chunks(args.joint_size.max(1)).enumerate() {
        let batch = format!("b{}", k + 1);
        let batch_dir = dir.join(&batch);
        fs::create_dir_all(&batch_dir)?;
        let workload = batch_dir.join("workload.json");
        fs::write(
            &workload,
            serde_json::to_string_pretty(&queries::workload_with(run.workload, batch_queries))?
                + "\n",
        )?;
        let ids: Vec<String> = batch_queries.iter().map(|q| q.id.clone()).collect();
        eprintln!("[{name}/{batch}] planning {} jointly", ids.join(","));
        let plan_path = batch_dir.join("plan.json");
        let planning = plan(args, &workload, arm, &plan_path)?;
        eprintln!(
            "[{name}/{batch}] {} in {:.1} s",
            planning.error.as_deref().unwrap_or("planned"),
            planning.wall_ms / 1e3
        );
        let systems = if planning.ok {
            Some(run_batch(
                run,
                name,
                &batch,
                &batch_dir,
                &plan_path,
                batch_queries,
                rows,
            )?)
        } else {
            None
        };
        batches.push(BatchReport {
            batch,
            query_ids: ids,
            planning,
            systems,
        });
    }

    Ok(ArmReport {
        name: name.into(),
        capabilities: arm
            .caps
            .as_deref()
            .map(|p| fs::read_to_string(p).map(|s| serde_json::from_str(&s)))
            .transpose()?
            .transpose()?,
        ignore_accuracy_guarantees: arm.ignore_accuracy_guarantees,
        probes,
        batches,
    })
}

/// One row per arm and query: the probe, the joint plan, and the accuracy
/// and latency over its evaluations.
fn per_query_rows(all: &[QuerySpec], arms: &[ArmReport], rows: &Rows) -> Vec<PerQueryRow> {
    let mut out = Vec::new();
    for arm in arms {
        for (q, probe) in all.iter().zip(&arm.probes) {
            let batch = arm.batches.iter().find(|b| b.query_ids.contains(&q.id));
            let joint = batch.and_then(|b| b.planning.methods.get(&q.text));
            let evals: Vec<&QueryRow> = rows
                .queries
                .iter()
                .filter(|r| r.arm == arm.name && r.query_id == q.id)
                .collect();
            let collect = |f: fn(&QueryRow) -> Option<f64>| -> Vec<f64> {
                evals.iter().filter_map(|r| f(r)).collect()
            };
            let rel_sum: f64 = evals
                .iter()
                .filter_map(|r| Some(r.rel_error_mean? * r.matched as f64))
                .sum();
            let rel_n: usize = evals
                .iter()
                .filter(|r| r.rel_error_mean.is_some())
                .map(|r| r.matched)
                .sum();
            let mut prom_lat: Vec<f64> = evals.iter().map(|r| r.prom_latency_ms_median).collect();
            let mut asap_lat: Vec<f64> = evals.iter().map(|r| r.asap_latency_ms_median).collect();
            let answered_by_plan = evals.iter().filter(|r| r.asap_execution == "plan").count();
            let missing: usize = evals.iter().map(|r| r.missing).sum();
            let extra: usize = evals.iter().map(|r| r.extra).sum();
            let requirement_checked: usize = evals.iter().map(|r| r.requirement_checked).sum();
            let requirement_violations: usize =
                evals.iter().map(|r| r.requirement_violations).sum();
            let requirement_met = (!evals.is_empty() && requirement_checked > 0).then_some(
                answered_by_plan == evals.len()
                    && missing == 0
                    && extra == 0
                    && requirement_violations == 0,
            );
            out.push(PerQueryRow {
                arm: arm.name.clone(),
                query_id: q.id.clone(),
                query: q.text.clone(),
                kind: q.kind.clone(),
                interval_s: q.interval_ms / 1000,
                accuracy_metric: q.accuracy_metric.clone().unwrap_or_default(),
                max_error: q.max_error,
                supported: probe.supported,
                unsupported_reason: probe.reason.clone(),
                alone_plan_ms: probe.plan_ms,
                alone_method: probe.method.clone(),
                batch: batch.map(|b| b.batch.clone()).unwrap_or_default(),
                joint_planned: batch.is_some_and(|b| b.planning.ok),
                joint_method: joint.map(|m| m.method.clone()).unwrap_or_default(),
                joint_sketch: joint.map(|m| m.sketch),
                joint_guarantee: joint.map(|m| m.guarantee.clone()).unwrap_or_default(),
                accepted_without_guarantee: joint.map(|m| m.accepted_without_guarantee),
                evaluations: evals.len(),
                answered_by_plan,
                missing,
                extra,
                rel_error_max: max(&collect(|r| r.rel_error_max)),
                rel_error_mean: (rel_n > 0).then(|| rel_sum / rel_n as f64),
                abs_error_max: max(&collect(|r| r.abs_error_max)),
                rank_error_max: max(&collect(|r| r.rank_error_max)),
                prom_vs_raw_quantile_max_abs: max(&collect(|r| r.prom_vs_raw_quantile_max_abs)),
                requirement_error_max: max(&collect(|r| r.requirement_error_max)),
                requirement_checked,
                requirement_violations,
                requirement_met,
                prom_latency_ms: (!prom_lat.is_empty()).then(|| median(&mut prom_lat)),
                asap_latency_ms: (!asap_lat.is_empty()).then(|| median(&mut asap_lat)),
            });
        }
    }
    out
}

fn read_pins(path: &Path) -> anyhow::Result<Value> {
    let mut pins = serde_json::Map::new();
    for line in fs::read_to_string(path)?.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some((k, v)) = line.split_once('=') {
            pins.insert(k.trim().into(), Value::String(v.trim().into()));
        }
    }
    Ok(Value::Object(pins))
}

fn fmt_opt(v: Option<f64>, digits: usize) -> String {
    v.map_or("-".into(), |v| format!("{v:.digits$}"))
}

fn write_csv<T: Serialize>(path: &Path, rows: impl IntoIterator<Item = T>) -> anyhow::Result<()> {
    let mut w = csv::Writer::from_path(path)?;
    for row in rows {
        w.serialize(row)?;
    }
    w.flush()?;
    Ok(())
}

fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    fs::create_dir_all(&args.out_dir)?;
    let mut workload: Value = serde_json::from_str(
        &fs::read_to_string(&args.workload)
            .with_context(|| format!("reading {}", args.workload.display()))?,
    )?;
    let mut all = queries::read_queries(&workload)?;
    if !args.only.is_empty() {
        for id in &args.only {
            ensure!(
                all.iter().any(|q| &q.id == id),
                "no query {id:?} in the workload"
            );
        }
        all.retain(|q| args.only.contains(&q.id));
    }

    // The series' label hierarchy, and the workload made to match the data
    // the run generates: the gauge source's series count, the counter as a
    // second source, and the arrival rate of both. The planner sees this
    // workload (written to the output directory), or a part of its queries.
    let gauge_source = workload
        .pointer(&format!("/data_workload/sources/{}", args.gauge_metric))
        .with_context(|| format!("the workload has no source {:?}", args.gauge_metric))?;
    let sample_interval_ms = gauge_source["sample_interval"]
        .as_i64()
        .context("sample_interval")?;
    let workload_series = gauge_source["series_count"]
        .as_u64()
        .context("series_count")? as usize;
    let mut hierarchy = match &args.labels {
        Some(spec) => data::parse_hierarchy(spec)?,
        None => data::parse_hierarchy(&format!("instance:{workload_series}"))?,
    };
    if let Some(n) = args.series {
        let (leaf, above) = hierarchy.split_last_mut().expect("a hierarchy has a level");
        let parents: usize = above.iter().map(|l| l.fan_out).product();
        ensure!(
            n > 0 && n % parents == 0,
            "--series {n} is not a multiple of the {parents} nodes above level {:?}",
            leaf.label
        );
        leaf.fan_out = n / parents;
    }
    let series: usize = hierarchy.iter().map(|l| l.fan_out).product();
    let counter_metric = (!args.counter_metric.is_empty()).then_some(args.counter_metric.as_str());
    ensure!(
        counter_metric != Some(args.gauge_metric.as_str()),
        "the counter metric must differ from the gauge metric"
    );
    {
        let sources = workload
            .pointer_mut("/data_workload/sources")
            .and_then(Value::as_object_mut)
            .context("workload sources")?;
        sources
            .get_mut(&args.gauge_metric)
            .context("workload source")?["series_count"] = json!(series);
        if let Some(counter) = counter_metric {
            sources.insert(
                counter.to_owned(),
                json!({
                    "series_count": series,
                    "metric_type": "counter",
                    "sample_interval": sample_interval_ms,
                }),
            );
        }
    }
    let metrics = 1 + usize::from(counter_metric.is_some());
    let rows_per_second = (series * metrics) as f64 * 1000.0 / sample_interval_ms as f64;
    if let Some(rate) = workload.pointer_mut("/data_workload/arrival/ingesting/rows_per_second") {
        *rate = json!(rows_per_second);
    }
    let workload = queries::workload_with(&workload, &all.iter().collect::<Vec<_>>());
    let workload_path = args.out_dir.join("workload.json");
    fs::write(
        &workload_path,
        serde_json::to_string_pretty(&workload)? + "\n",
    )?;

    let tl = timeline(&all, args.evaluations, sample_interval_ms);
    let points = ((tl.last_sample_ms - tl.first_sample_ms) / sample_interval_ms + 1) as usize;
    let scale = args.scale.as_deref().map(data::parse_scale).transpose()?;
    let data = Dataset::generate(
        &args.gauge_metric,
        counter_metric,
        hierarchy,
        scale
            .as_ref()
            .map(|(label, range)| (label.as_str(), *range)),
        sample_interval_ms,
        tl.first_sample_ms,
        points,
        args.seed,
    )?;
    eprintln!(
        "{} queries, {} series x {} points per metric ({}), origin {}, end T0+{}m",
        all.len(),
        series,
        points,
        [Some(args.gauge_metric.as_str()), counter_metric]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join(", "),
        tl.origin_ms,
        (tl.last_sample_ms - tl.origin_ms) / 60_000
    );
    for q in &all {
        eprintln!(
            "  {} every {}s, {} evaluations: {}",
            q.id,
            q.interval_ms / 1000,
            tl.evaluations_ms[&q.id].len(),
            q.text
        );
    }

    let run = Run {
        args: &args,
        workload: &workload,
        data: &data,
        tl: &tl,
    };
    let mut rows = Rows::default();
    let mut arms = Vec::new();
    let specs = args
        .arms
        .iter()
        .map(|a| a.parse::<ArmSpec>())
        .collect::<anyhow::Result<Vec<_>>>()?;
    for arm in &specs {
        arms.push(run_arm(&run, arm, &all, &mut rows)?);
    }
    let per_query = per_query_rows(&all, &arms, &rows);

    write_csv(&args.out_dir.join("queries.csv"), &rows.queries)?;
    write_csv(&args.out_dir.join("answers.csv"), &rows.answers)?;
    write_csv(&args.out_dir.join("load.csv"), &rows.load)?;
    write_csv(
        &args.out_dir.join("probes.csv"),
        arms.iter().flat_map(|a| &a.probes),
    )?;
    write_csv(&args.out_dir.join("per_query.csv"), &per_query)?;
    let summary = json!({
        "pins": args.pins.as_deref().map(read_pins).transpose()?,
        "binaries": {
            "prometheus": args.prometheus_bin,
            "control_plane": args.control_plane_bin,
            "data_plane": args.data_plane_bin,
        },
        "workload": workload_path,
        "queries": all,
        "joint_size": args.joint_size,
        "timeline": tl,
        "dataset": data.spec,
        "repeats": args.repeats,
        "batch_seconds": args.batch_seconds,
        "arms": arms,
        "per_query": per_query,
    });
    fs::write(
        args.out_dir.join("summary.json"),
        serde_json::to_string_pretty(&summary)? + "\n",
    )?;

    println!(
        "| arm | query | alone | batch | method | guarantee | evals | by plan | miss/extra | max rel err | mean rel err | max rank err | required | max err | over / checked | met | Prom ms | ASAP ms |"
    );
    println!("|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|");
    for r in &per_query {
        println!(
            "| {} | {} `{}` | {} | {} | {} | {} | {} | {} | {}/{} | {} | {} | {} | {} <= {} | {} | {} / {} | {} | {} | {} |",
            r.arm,
            r.query_id,
            r.query,
            if r.supported { "yes" } else { "no" },
            if r.joint_planned {
                r.batch.clone()
            } else if r.batch.is_empty() {
                "-".into()
            } else {
                format!("{} (no plan)", r.batch)
            },
            if r.supported {
                &r.joint_method
            } else {
                &r.unsupported_reason
            },
            r.joint_guarantee,
            r.evaluations,
            r.answered_by_plan,
            r.missing,
            r.extra,
            fmt_opt(r.rel_error_max, 5),
            fmt_opt(r.rel_error_mean, 5),
            fmt_opt(r.rank_error_max, 5),
            r.accuracy_metric,
            fmt_opt(r.max_error, 3),
            fmt_opt(r.requirement_error_max, 5),
            r.requirement_violations,
            r.requirement_checked,
            match r.requirement_met {
                Some(true) => "yes",
                Some(false) => "no",
                None => "-",
            },
            fmt_opt(r.prom_latency_ms, 2),
            fmt_opt(r.asap_latency_ms, 2),
        );
    }
    println!();
    println!(
        "| arm | batch | queries | plan | plan s | CP peak RSS MB | Prom RSS peak MB | DP RSS peak MB | Prom samples/s | DP samples/s |"
    );
    println!("|---|---|---|---|---|---|---|---|---|---|");
    let mb = |kb: Option<u64>| fmt_opt(kb.map(|k| k as f64 / 1024.0), 1);
    for a in &arms {
        for b in &a.batches {
            let s = b.systems.as_ref();
            println!(
                "| {} | {} | {} | {} | {:.1} | {} | {} | {} | {} | {} |",
                a.name,
                b.batch,
                b.query_ids.join(","),
                b.planning.selected_candidate["selected_candidate_id"]
                    .as_str()
                    .or(b.planning.error.as_deref())
                    .unwrap_or("?"),
                b.planning.wall_ms / 1e3,
                mb(b.planning.peak_rss_kb),
                mb(s.and_then(|s| s.prometheus_rss_max_sampled_kb)),
                mb(s.and_then(|s| s.data_plane_rss_max_sampled_kb)),
                fmt_opt(s.map(|s| s.ingest_prometheus.samples_per_second), 0),
                fmt_opt(s.map(|s| s.ingest_data_plane.samples_per_second), 0),
            );
        }
    }
    println!();
    println!("| arm | query | system | clients | ok / requests | QPS | p50 ms | p99 ms |");
    println!("|---|---|---|---|---|---|---|---|");
    for step in &rows.load {
        println!(
            "| {} | {} | {} | {} | {} / {} | {:.1} | {} | {} |",
            step.arm,
            step.query_id,
            step.system,
            step.concurrency,
            step.ok,
            step.requests,
            step.qps,
            fmt_opt(step.p50_ms, 2),
            fmt_opt(step.p99_ms, 2)
        );
    }
    eprintln!("results in {}", args.out_dir.display());
    let all_asked = rows.queries.len();
    let by_plan = rows
        .queries
        .iter()
        .filter(|r| r.asap_execution == "plan")
        .count();
    ensure!(
        by_plan == all_asked,
        "{} of {all_asked} ASAP queries were not answered by the plan",
        all_asked - by_plan
    );
    let not_plan: usize = rows.load.iter().map(|s| s.not_plan).sum();
    ensure!(
        not_plan == 0,
        "{not_plan} load-test answers were not the plan's"
    );
    Ok(())
}
