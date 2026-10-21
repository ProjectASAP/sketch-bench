//! `aqpbm-e2e`: one PromQL query, one dataset, ASAPQuery-backend against
//! Prometheus, end to end. See `e2e/README.md`.
//!
//! For each arm (a set of `DeploymentCapabilities` the control plane plans
//! with) the harness plans the workload, starts a fresh Prometheus and a fresh
//! data plane, replays the same samples into both over Remote Write in
//! event-time order, closes panes with the data plane's watermark endpoint at
//! each evaluation time, and queries both systems there through
//! `/api/v1/query`. Every arm uses the same samples and evaluation times.

mod data;
mod procs;
mod promapi;
mod remote_write;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{bail, ensure, Context};
use aqpbm_core::metrics::WallClock;
use clap::Parser;
use serde::Serialize;
use serde_json::{json, Value};

use data::{prometheus_quantile, rank_error, Dataset};
use procs::{Managed, ProcSample};

#[derive(Parser)]
#[command(about = "Compare ASAPQuery-backend against Prometheus on one PromQL query")]
struct Args {
    #[arg(long)]
    prometheus_bin: PathBuf,
    #[arg(long)]
    control_plane_bin: PathBuf,
    #[arg(long)]
    data_plane_bin: PathBuf,
    /// Planner `PlanningWorkload` JSON with exactly one repeating
    /// `quantile_over_time(q, metric[W])` query over one source.
    #[arg(long)]
    workload: PathBuf,
    /// `NAME` or `NAME=CAPABILITIES.json`; repeat for several arms. Without
    /// a file the control plane uses its default capabilities.
    #[arg(long = "arm", required = true)]
    arms: Vec<String>,
    /// `KEY=VALUE` pins file copied into the summary.
    #[arg(long)]
    pins: Option<PathBuf>,
    #[arg(long)]
    out_dir: PathBuf,
    /// Seed for the value column.
    #[arg(long, default_value_t = 42)]
    seed: u64,
    /// Evaluation times queried, one query interval apart.
    #[arg(long, default_value_t = 6)]
    evaluations: usize,
    /// Times each query is sent to each system at each evaluation time.
    #[arg(long, default_value_t = 5)]
    repeats: usize,
    /// Event-time span of samples per Remote Write request.
    #[arg(long, default_value_t = 60)]
    batch_seconds: i64,
    /// Keep each arm's Prometheus TSDB directory instead of deleting it.
    #[arg(long)]
    keep_tsdb: bool,
    /// Override the source's `series_count` (and the arrival rate) in the
    /// workload; the patched workload is written to the output directory.
    #[arg(long)]
    series: Option<usize>,
    /// Seconds each closed-loop query-load step runs.
    #[arg(long, default_value_t = 5)]
    load_seconds: u64,
    /// Concurrent clients of one load step; repeat for several steps. The
    /// steps run at the last evaluation time, after its normal queries.
    #[arg(long = "load-concurrency", default_values_t = vec![1, 4, 16])]
    load_concurrency: Vec<usize>,
}

/// The one query the run supports, read from the workload.
#[derive(Debug, Clone, Serialize)]
struct QuerySpec {
    text: String,
    quantile: f64,
    metric: String,
    window_ms: i64,
    interval_ms: i64,
    series: usize,
    sample_interval_ms: i64,
}

fn parse_duration_ms(s: &str) -> anyhow::Result<i64> {
    let (num, unit) = s.split_at(s.find(|c: char| !c.is_ascii_digit()).unwrap_or(s.len()));
    let n: i64 = num.parse().with_context(|| format!("duration {s:?}"))?;
    Ok(n * match unit {
        "ms" => 1,
        "s" => 1_000,
        "m" => 60_000,
        "h" => 3_600_000,
        _ => bail!("duration {s:?}: unsupported unit"),
    })
}

fn read_query(workload: &Value) -> anyhow::Result<QuerySpec> {
    let queries = workload
        .pointer("/query_workload/repeating_queries")
        .and_then(Value::as_array)
        .context("workload has no query_workload.repeating_queries")?;
    ensure!(queries.len() == 1, "the harness runs exactly one query");
    let text = queries[0]["query"]
        .as_str()
        .context("query text")?
        .to_owned();
    let interval_ms = queries[0]
        .pointer("/demand/fixed_interval")
        .and_then(Value::as_i64)
        .context("only demand.fixed_interval is supported")?;
    // quantile_over_time(<q>, <metric>[<window>])
    let inner = text
        .strip_prefix("quantile_over_time(")
        .and_then(|s| s.strip_suffix("])"))
        .with_context(|| format!("unsupported query {text:?}"))?;
    let (q, selector) = inner.split_once(',').context("quantile argument")?;
    let (metric, window) = selector.trim().split_once('[').context("range selector")?;
    let sources = workload
        .pointer("/data_workload/sources")
        .and_then(Value::as_object)
        .context("data_workload.sources")?;
    let source = sources
        .get(metric)
        .with_context(|| format!("no source for metric {metric:?}"))?;
    Ok(QuerySpec {
        quantile: q.trim().parse()?,
        metric: metric.to_owned(),
        window_ms: parse_duration_ms(window)?,
        interval_ms,
        series: source["series_count"].as_u64().context("series_count")? as usize,
        sample_interval_ms: source["sample_interval"]
            .as_i64()
            .context("sample_interval")?,
        text,
    })
}

#[derive(Debug, Clone, Serialize)]
struct Timeline {
    /// Pane-grid origin; the first sample is one sample interval after it.
    origin_ms: i64,
    first_sample_ms: i64,
    last_sample_ms: i64,
    evaluations_ms: Vec<i64>,
}

/// Picks evaluation times on the query's interval grid, all in the past:
/// the data plane closes panes no later than the wall clock and drops samples
/// too far ahead of it. The first pane a data plane sees is never built, so
/// the first evaluation comes one interval after a full window of data.
fn timeline(q: &QuerySpec, evaluations: usize) -> Timeline {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64;
    let margin = 5 * 60_000;
    let span = q.window_ms + q.interval_ms * evaluations as i64;
    let origin_ms = (now - margin - span).div_euclid(q.interval_ms) * q.interval_ms;
    let first_eval = origin_ms + q.window_ms + q.interval_ms;
    let evaluations_ms: Vec<i64> = (0..evaluations as i64)
        .map(|k| first_eval + k * q.interval_ms)
        .collect();
    Timeline {
        origin_ms,
        first_sample_ms: origin_ms + q.sample_interval_ms,
        last_sample_ms: *evaluations_ms.last().unwrap(),
        evaluations_ms,
    }
}

#[derive(Debug, Clone, Serialize)]
struct Planning {
    wall_ms: f64,
    peak_rss_kb: Option<u64>,
    stdout: String,
    selected_candidate: Value,
    stored_outputs: usize,
    query_time_raw_windows: usize,
}

/// Runs the control plane and reaps it with `wait4`, which gives its own
/// peak RSS rather than the maximum over every child reaped so far.
fn plan(args: &Args, workload: &Path, caps: Option<&Path>, out: &Path) -> anyhow::Result<Planning> {
    let mut command = Command::new(&args.control_plane_bin);
    command
        .arg("--workload")
        .arg(workload)
        .arg("--out")
        .arg(out);
    if let Some(caps) = caps {
        command.arg("--deployment-capabilities").arg(caps);
    }
    let log = out.with_extension("log");
    let log_file = fs::File::create(&log)?;
    command.stdout(log_file.try_clone()?).stderr(log_file);
    let clock = WallClock::start();
    let child = command.spawn().context("starting control_plane")?;
    let mut status = 0;
    let mut usage = std::mem::MaybeUninit::<libc::rusage>::zeroed();
    // SAFETY: waits for our own child; status and usage are written on success.
    let rc = unsafe {
        libc::wait4(
            child.id() as libc::pid_t,
            &mut status,
            0,
            usage.as_mut_ptr(),
        )
    };
    let wall_ms = clock.elapsed_ns() as f64 / 1e6;
    ensure!(rc > 0, "wait4 on control_plane failed");
    let stdout = fs::read_to_string(&log).unwrap_or_default();
    ensure!(
        libc::WIFEXITED(status) && libc::WEXITSTATUS(status) == 0,
        "control_plane failed (status {status}):\n{stdout}"
    );
    // SAFETY: rc > 0, so wait4 wrote the struct; ru_maxrss is kB on Linux.
    let peak_rss_kb = Some(unsafe { usage.assume_init() }.ru_maxrss as u64);
    let plan: Value = serde_json::from_str(&fs::read_to_string(out)?)?;
    Ok(Planning {
        wall_ms,
        peak_rss_kb,
        stdout: stdout.trim().to_owned(),
        selected_candidate: plan["selection"].clone(),
        stored_outputs: plan["stored_outputs"].as_array().map_or(0, Vec::len),
        query_time_raw_windows: plan["query_time_raw_windows"]
            .as_array()
            .map_or(0, Vec::len),
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

/// One closed-loop load step: `concurrency` clients, each sending the query
/// back to back for the step's duration.
#[derive(Debug, Clone, Serialize)]
struct LoadStep {
    arm: String,
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
    arm: &str,
    system: &str,
    base: &str,
    query: &str,
    time_ms: i64,
    concurrency: usize,
    duration: Duration,
) -> LoadStep {
    let clock = WallClock::start();
    let deadline = std::time::Instant::now() + duration;
    let results: Vec<(bool, bool, f64)> = std::thread::scope(|scope| {
        let handles: Vec<_> = (0..concurrency)
            .map(|_| {
                scope.spawn(move || {
                    let client = reqwest::blocking::Client::builder()
                        .timeout(Duration::from_secs(300))
                        .build()
                        .expect("http client");
                    let mut out = Vec::new();
                    while std::time::Instant::now() < deadline {
                        out.push(match promapi::instant_query(&client, base, query, time_ms) {
                            Ok(a) => (
                                a.status == 200,
                                a.execution.as_deref().is_some_and(|e| e != "plan"),
                                a.latency_ms,
                            ),
                            Err(_) => (false, false, f64::NAN),
                        });
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
        arm: arm.into(),
        system: system.into(),
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

#[derive(Debug, Clone, Serialize)]
struct QueryRow {
    arm: String,
    eval_time_ms: i64,
    prom_status: u16,
    asap_status: u16,
    asap_execution: String,
    asap_error: String,
    prom_series: usize,
    asap_series: usize,
    rel_error_max: Option<f64>,
    rel_error_mean: Option<f64>,
    rank_error_max: Option<f64>,
    prom_vs_truth_max_abs: Option<f64>,
    prom_latency_ms_first: f64,
    prom_latency_ms_median: f64,
    asap_latency_ms_first: f64,
    asap_latency_ms_median: f64,
    watermark_ms: f64,
    watermark_barrier_ms: Option<i64>,
}

#[derive(Debug, Clone, Serialize)]
struct AnswerRow {
    arm: String,
    eval_time_ms: i64,
    instance: String,
    window_samples: usize,
    truth: Option<f64>,
    prom_value: Option<f64>,
    asap_value: Option<f64>,
    rel_error: Option<f64>,
    asap_rank_error: Option<f64>,
}

#[derive(Debug, Clone, Serialize)]
struct ArmReport {
    name: String,
    capabilities: Option<Value>,
    planning: Planning,
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
    load: Vec<LoadStep>,
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

/// What every arm shares: the run's arguments, query, samples and times.
struct Run<'a> {
    args: &'a Args,
    workload: &'a Path,
    q: &'a QuerySpec,
    data: &'a Dataset,
    tl: &'a Timeline,
}

fn run_arm(
    run: &Run,
    name: &str,
    caps: Option<&Path>,
    queries: &mut Vec<QueryRow>,
    answers: &mut Vec<AnswerRow>,
) -> anyhow::Result<ArmReport> {
    let Run {
        args,
        workload,
        q,
        data,
        tl,
    } = *run;
    let dir = args.out_dir.join(name);
    fs::create_dir_all(&dir)?;
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(300))
        .build()?;

    eprintln!("[{name}] planning");
    let plan_path = dir.join("plan.json");
    let planning = plan(args, workload, caps, &plan_path)?;
    eprintln!("[{name}] {}", planning.stdout);

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
        .arg(&plan_path)
        .arg("--prometheus-url")
        .arg(&prom_url)
        .arg("--listen")
        .arg(format!("127.0.0.1:{dp_port}"))
        .arg("--no-forward")
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
    let last_eval = *tl.evaluations_ms.last().unwrap();
    let mut load = Vec::new();
    for &eval in &tl.evaluations_ms {
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

        let mut prom_answers = Vec::new();
        let mut asap_answers = Vec::new();
        for _ in 0..args.repeats.max(1) {
            prom_answers.push(promapi::instant_query(&client, &prom_url, &q.text, eval)?);
        }
        for _ in 0..args.repeats.max(1) {
            asap_answers.push(promapi::instant_query(&client, &dp_url, &q.text, eval)?);
        }
        prom_rss_max = prom_rss_max.max(procs::sample(prom.pid()).rss_kb);
        dp_rss_max = dp_rss_max.max(procs::sample(dp.pid()).rss_kb);
        let prom_first = &prom_answers[0];
        let asap_first = &asap_answers[0];
        if asap_first.execution.as_deref() == Some("plan") {
            answered += 1;
        }

        let mut rel_errors = Vec::new();
        let mut rank_errors = Vec::new();
        let mut prom_vs_truth = Vec::new();
        for s in 0..data.spec.series {
            let instance = Dataset::instance(s);
            let window = data.window(s, eval, q.window_ms);
            let truth = prometheus_quantile(&window, q.quantile);
            let prom_value = prom_first.values.get(&instance).copied();
            let asap_value = asap_first.values.get(&instance).copied();
            let rel_error = match (prom_value, asap_value) {
                (Some(p), Some(a)) if p != 0.0 => Some((a - p).abs() / p.abs()),
                _ => None,
            };
            let asap_rank_error = asap_value.and_then(|a| rank_error(&window, q.quantile, a));
            if let (Some(p), Some(t)) = (prom_value, truth) {
                prom_vs_truth.push((p - t).abs());
            }
            rel_errors.extend(rel_error);
            rank_errors.extend(asap_rank_error);
            answers.push(AnswerRow {
                arm: name.into(),
                eval_time_ms: eval,
                instance,
                window_samples: window.len(),
                truth,
                prom_value,
                asap_value,
                rel_error,
                asap_rank_error,
            });
        }
        let max = |v: &[f64]| v.iter().copied().reduce(f64::max);
        let mut prom_lat: Vec<f64> = prom_answers.iter().map(|a| a.latency_ms).collect();
        let mut asap_lat: Vec<f64> = asap_answers.iter().map(|a| a.latency_ms).collect();
        let row = QueryRow {
            arm: name.into(),
            eval_time_ms: eval,
            prom_status: prom_first.status,
            asap_status: asap_first.status,
            asap_execution: asap_first.execution.clone().unwrap_or_default(),
            asap_error: asap_first.error.clone().unwrap_or_default(),
            prom_series: prom_first.values.len(),
            asap_series: asap_first.values.len(),
            rel_error_max: max(&rel_errors),
            rel_error_mean: (!rel_errors.is_empty())
                .then(|| rel_errors.iter().sum::<f64>() / rel_errors.len() as f64),
            rank_error_max: max(&rank_errors),
            prom_vs_truth_max_abs: max(&prom_vs_truth),
            prom_latency_ms_first: prom_lat[0],
            prom_latency_ms_median: median(&mut prom_lat),
            asap_latency_ms_first: asap_lat[0],
            asap_latency_ms_median: median(&mut asap_lat),
            watermark_ms,
            watermark_barrier_ms: barrier,
        };
        eprintln!(
            "[{name}] t={eval} asap={} {} series={} rel_err_max={:?} prom={:.2}ms asap={:.2}ms",
            row.asap_status,
            row.asap_execution,
            row.asap_series,
            row.rel_error_max,
            row.prom_latency_ms_median,
            row.asap_latency_ms_median
        );
        queries.push(row);

        if eval == last_eval {
            let duration = Duration::from_secs(args.load_seconds);
            for &concurrency in &args.load_concurrency {
                for (system, base) in [("prometheus", &prom_url), ("asap", &dp_url)] {
                    let step = load_step(name, system, base, &q.text, eval, concurrency, duration);
                    eprintln!(
                        "[{name}] load {system} x{concurrency}: {} ok of {} in {:.1}s, {:.1} qps, p50 {:?} ms, p99 {:?} ms",
                        step.ok, step.requests, step.seconds, step.qps, step.p50_ms, step.p99_ms
                    );
                    load.push(step);
                }
                prom_rss_max = prom_rss_max.max(procs::sample(prom.pid()).rss_kb);
                dp_rss_max = dp_rss_max.max(procs::sample(dp.pid()).rss_kb);
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

    Ok(ArmReport {
        name: name.into(),
        capabilities: caps
            .map(|p| fs::read_to_string(p).map(|s| serde_json::from_str(&s)))
            .transpose()?
            .transpose()?,
        planning,
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
        queries: tl.evaluations_ms.len(),
        load,
    })
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

fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    fs::create_dir_all(&args.out_dir)?;
    let workload: Value = serde_json::from_str(
        &fs::read_to_string(&args.workload)
            .with_context(|| format!("reading {}", args.workload.display()))?,
    )?;
    let mut workload = workload;
    let mut workload_path = args.workload.clone();
    if let Some(series) = args.series {
        let base = read_query(&workload)?;
        let source = workload
            .pointer_mut(&format!("/data_workload/sources/{}", base.metric))
            .context("workload source")?;
        source["series_count"] = json!(series);
        let rows_per_second = series as f64 * 1000.0 / base.sample_interval_ms as f64;
        if let Some(rate) = workload.pointer_mut("/data_workload/arrival/ingesting/rows_per_second") {
            *rate = json!(rows_per_second);
        }
        workload_path = args.out_dir.join("workload.json");
        fs::write(&workload_path, serde_json::to_string_pretty(&workload)? + "\n")?;
    }
    let q = read_query(&workload)?;
    let tl = timeline(&q, args.evaluations);
    let points = ((tl.last_sample_ms - tl.first_sample_ms) / q.sample_interval_ms + 1) as usize;
    let data = Dataset::generate(
        &q.metric,
        q.series,
        q.sample_interval_ms,
        tl.first_sample_ms,
        points,
        args.seed,
    )?;
    eprintln!(
        "query {:?}: {} series x {} points, evaluations {:?}",
        q.text, q.series, points, tl.evaluations_ms
    );

    let run = Run {
        args: &args,
        workload: &workload_path,
        q: &q,
        data: &data,
        tl: &tl,
    };
    let mut queries = Vec::new();
    let mut answers = Vec::new();
    let mut arms = Vec::new();
    for arm in &args.arms {
        let (name, caps) = match arm.split_once('=') {
            Some((n, c)) => (n, Some(PathBuf::from(c))),
            None => (arm.as_str(), None),
        };
        arms.push(run_arm(
            &run,
            name,
            caps.as_deref(),
            &mut queries,
            &mut answers,
        )?);
    }

    let mut w = csv::Writer::from_path(args.out_dir.join("queries.csv"))?;
    for row in &queries {
        w.serialize(row)?;
    }
    w.flush()?;
    let mut w = csv::Writer::from_path(args.out_dir.join("answers.csv"))?;
    for row in &answers {
        w.serialize(row)?;
    }
    w.flush()?;
    let mut w = csv::Writer::from_path(args.out_dir.join("load.csv"))?;
    for step in arms.iter().flat_map(|a| &a.load) {
        w.serialize(step)?;
    }
    w.flush()?;
    let summary = json!({
        "pins": args.pins.as_deref().map(read_pins).transpose()?,
        "binaries": {
            "prometheus": args.prometheus_bin,
            "control_plane": args.control_plane_bin,
            "data_plane": args.data_plane_bin,
        },
        "workload": workload_path,
        "query": q,
        "timeline": tl,
        "dataset": data.spec,
        "repeats": args.repeats,
        "batch_seconds": args.batch_seconds,
        "arms": arms,
        "queries": queries,
    });
    fs::write(
        args.out_dir.join("summary.json"),
        serde_json::to_string_pretty(&summary)? + "\n",
    )?;

    println!(
        "| arm | eval time | ASAP exec | series P/A | max rel err | mean rel err | max rank err | Prom p50 ms | ASAP p50 ms |"
    );
    println!("|---|---|---|---|---|---|---|---|---|");
    for r in &queries {
        println!(
            "| {} | T0+{}m | {} {} | {}/{} | {} | {} | {} | {:.2} | {:.2} |",
            r.arm,
            (r.eval_time_ms - tl.origin_ms) / 60_000,
            r.asap_status,
            r.asap_execution,
            r.prom_series,
            r.asap_series,
            fmt_opt(r.rel_error_max, 5),
            fmt_opt(r.rel_error_mean, 5),
            fmt_opt(r.rank_error_max, 5),
            r.prom_latency_ms_median,
            r.asap_latency_ms_median
        );
    }
    println!();
    println!(
        "| arm | plan | plan ms | CP peak RSS MB | Prom RSS idle/peak MB | DP RSS idle/peak MB | Prom CPU s | DP CPU s |"
    );
    println!("|---|---|---|---|---|---|---|---|");
    let mb = |kb: Option<u64>| fmt_opt(kb.map(|k| k as f64 / 1024.0), 1);
    for a in &arms {
        println!(
            "| {} | {} | {:.1} | {} | {} / {} | {} / {} | {} | {} |",
            a.name,
            a.planning.selected_candidate["selected_candidate_id"]
                .as_str()
                .unwrap_or("?"),
            a.planning.wall_ms,
            mb(a.planning.peak_rss_kb),
            mb(a.prometheus_idle.rss_kb),
            mb(a.prometheus_rss_max_sampled_kb),
            mb(a.data_plane_idle.rss_kb),
            mb(a.data_plane_rss_max_sampled_kb),
            fmt_opt(a.prometheus_end.cpu_seconds, 2),
            fmt_opt(a.data_plane_end.cpu_seconds, 2),
        );
    }
    println!();
    println!("| arm | samples | Prom absorb ms | Prom samples/s | DP absorb ms | DP samples/s |");
    println!("|---|---|---|---|---|---|");
    for a in &arms {
        println!(
            "| {} | {} | {:.1} | {:.0} | {:.1} | {:.0} |",
            a.name,
            a.ingest_prometheus.samples,
            a.ingest_prometheus.absorb_ms,
            a.ingest_prometheus.samples_per_second,
            a.ingest_data_plane.absorb_ms,
            a.ingest_data_plane.samples_per_second
        );
    }
    println!();
    println!("| arm | system | clients | ok / requests | QPS | p50 ms | p99 ms |");
    println!("|---|---|---|---|---|---|---|");
    for step in arms.iter().flat_map(|a| &a.load) {
        println!(
            "| {} | {} | {} | {} / {} | {:.1} | {} | {} |",
            step.arm,
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
    let all = queries.len();
    let by_plan = queries
        .iter()
        .filter(|r| r.asap_execution == "plan")
        .count();
    ensure!(
        by_plan == all,
        "{} of {all} ASAP queries were not answered by the plan",
        all - by_plan
    );
    let not_plan: usize = arms.iter().flat_map(|a| &a.load).map(|s| s.not_plan).sum();
    ensure!(not_plan == 0, "{not_plan} load-test answers were not the plan's");
    Ok(())
}
