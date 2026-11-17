//! U1–U7: the VictoriaMetrics adapter's pieces without a real server: the
//! wall offset, the paced slices, the import body, the server metrics and
//! answers it reads, and the install path against a small fake server.

mod common;

use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use aqpbm_engines_external::victoriametrics::{StreamAggregation, StreamOutput};
use aqpbm_engines_external::victoriametrics_server::{
    answer_metric_name, import_body, lead_ms, parse_answer, previous_arm_gap_ms, slices,
    wall_offset_ms, ServerMetrics, VictoriaMetricsEngine, VictoriaMetricsServer, SLICE_MS,
};
use aqpbm_core::harness::cgroup::serve_agent;
use aqpbm_core::harness::data::Sample;
use aqpbm_core::harness::engine::{
    ArmSpec, EngineUnderTest, QueryLanguage, QueryText, SampleChunk, SeriesLabels, WorkloadQuery,
    WorkloadSpec,
};
use common::{fixture_value_public, payload_body};
use serde_json::json;

fn aggregation(interval_ms: u64) -> StreamAggregation {
    StreamAggregation {
        metric: "m".into(),
        selector: "m".into(),
        interval_ms,
        by: None,
        outputs: vec![StreamOutput::Max],
    }
}

fn labels(pairs: &[(&str, &str)]) -> Arc<SeriesLabels> {
    Arc::new(
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect(),
    )
}

#[test]
fn wall_offset_puts_the_data_start_on_a_later_tick() {
    // Aligned data start: the next multiple of the step at least 2 s ahead.
    assert_eq!(wall_offset_ms(1_000_000_500, 0, 10_000), 1_000_010_000);
    assert_eq!(wall_offset_ms(1_000_008_000, 0, 10_000), 1_000_010_000);
    assert_eq!(wall_offset_ms(1_000_008_001, 0, 10_000), 1_000_020_000);
    // A data start that is not a window end: its step boundary (0) goes on
    // the tick, so the first samples are after the first flushed window's start.
    assert_eq!(wall_offset_ms(1_000_000_500, 10_000, 60_000), 1_000_020_000);
    let offset = wall_offset_ms(1_000_000_500, 10_000, 60_000);
    assert_eq!(offset % 60_000, 0);
    assert!(offset >= 1_000_002_500);
}

#[test]
fn lead_is_at_least_a_second_and_below_every_width() {
    assert_eq!(lead_ms(&[]), 1_000);
    assert_eq!(lead_ms(&[aggregation(60_000), aggregation(10_000)]), 4_500);
    assert_eq!(lead_ms(&[aggregation(2_000)]), 1_000);
    for width in [2_000, 3_000, 10_000, 60_000, 3_600_000] {
        assert!(lead_ms(&[aggregation(width)]) < width as i64);
    }
}

#[test]
fn slices_are_one_second_and_sent_a_lead_early() {
    let series = labels(&[("__name__", "m"), ("job", "a")]);
    let sample = |ts_ms| Sample {
        labels: Arc::clone(&series),
        ts_ms,
        value: 1.0,
    };
    // 1 Hz and 10 Hz samples in (0, 3 s].
    let mut samples = vec![sample(1_000), sample(2_000), sample(3_000)];
    samples.extend((1..=10).map(|i| sample(2_000 + i * 100)));
    let chunk = SampleChunk {
        end_ms: 3_000,
        samples,
    };
    let cut = slices(&chunk, 10_000, 1_000);
    let ends: Vec<i64> = cut.iter().map(|s| s.end_ms).collect();
    assert_eq!(ends, vec![1_000, 2_000, 3_000]);
    let sends: Vec<i64> = cut.iter().map(|s| s.send_at_wall_ms).collect();
    assert_eq!(sends, vec![10_000, 11_000, 12_000]);
    assert_eq!(cut[2].samples.len(), 11);
    // On time, no sample is older than the wall clock when sent: a slice
    // ends `lead` after its send time, and holds samples of the last second.
    for slice in &cut {
        for sample in &slice.samples {
            assert!(sample.ts_ms + 10_000 > slice.send_at_wall_ms);
            assert!(sample.ts_ms > slice.end_ms - SLICE_MS);
        }
    }
}

#[test]
fn import_body_is_one_line_per_series() {
    let a = labels(&[("__name__", "m"), ("job", "a")]);
    let b = labels(&[("__name__", "m"), ("job", "b")]);
    let samples = [
        Sample {
            labels: Arc::clone(&a),
            ts_ms: 1_000,
            value: 1.5,
        },
        Sample {
            labels: Arc::clone(&b),
            ts_ms: 1_000,
            value: f64::INFINITY,
        },
        Sample {
            labels: Arc::clone(&a),
            ts_ms: 2_000,
            value: f64::NAN,
        },
    ];
    let refs: Vec<&Sample> = samples.iter().collect();
    assert_eq!(
        import_body(&refs, 10_000),
        "{\"metric\":{\"__name__\":\"m\",\"job\":\"a\"},\"values\":[1.5,NaN],\"timestamps\":[11000,12000]}\n\
         {\"metric\":{\"__name__\":\"m\",\"job\":\"b\"},\"values\":[Inf],\"timestamps\":[11000]}\n"
    );
}

#[test]
fn server_metrics_are_read_from_the_exposition() {
    let text = "\
# HELP ignored\n\
vm_app_version{version=\"victoria-metrics-20251019-v1.154.1\", short_version=\"v1.154.1\"} 1\n\
vminsert_streamagg_config_reloads_total 3\n\
vminsert_streamagg_config_last_reload_successful 1\n\
vminsert_streamagg_config_reloads_errors_total 1\n\
vm_streamaggr_flush_duration_seconds_count{outputs=\"max\",name=\"none\",path=\"/w/a.yaml\",url=\"global\",position=\"1\"} 7\n\
vm_streamaggr_flush_duration_seconds_count{outputs=\"sum_samples,count_samples\",name=\"none\",path=\"/w/a.yaml\",url=\"global\",position=\"2\"} 4\n\
vm_streamaggr_ignored_samples_total{reason=\"nan\",outputs=\"max\",name=\"none\",position=\"1\"} 9\n\
vm_streamaggr_ignored_samples_total{reason=\"too_old\",outputs=\"max\",name=\"none\",position=\"1\"} 2\n\
vm_streamaggr_ignored_samples_total{reason=\"too_old\",outputs=\"max\",name=\"none\",position=\"2\"} 1\n\
vm_data_size_bytes{type=\"storage/small\"} 100\n\
vm_data_size_bytes{type=\"indexdb/file\"} 20\n";
    assert_eq!(
        ServerMetrics::parse(text, "none"),
        ServerMetrics {
            app_version: Some("victoria-metrics-20251019-v1.154.1".into()),
            config_reloads: 3,
            config_reload_errors: 1,
            running_positions: [1, 2].into(),
            flushes_by_position: BTreeMap::from([(1, 7), (2, 4)]),
            too_old_samples: 3,
            data_size_bytes: 120,
        }
    );
    // Another install's aggregations are not counted.
    let other = ServerMetrics::parse(text, "install-1-0");
    assert!(other.flushes_by_position.is_empty() && other.running_positions.is_empty());
    assert_eq!(other.too_old_samples, 0);
}

#[test]
fn answers_are_keyed_like_the_harness() {
    let body = json!({"status": "success", "data": {"resultType": "vector", "result": [
        {"metric": {"__name__": "latency:60s_max", "job": "a"}, "value": [1, "3.5"]},
        {"metric": {"job": "b"}, "value": [1, "NaN"]},
        {"metric": {"job": "c"}, "value": [1, "+Inf"]}
    ]}})
    .to_string();
    let per_series = parse_answer(&body, Some("latency")).unwrap();
    let key = |job: &str| -> SeriesLabels {
        [("__name__", "latency"), ("job", job)]
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    };
    assert_eq!(per_series[&key("a")], 3.5);
    assert!(per_series[&key("b")].is_nan());
    assert_eq!(per_series[&key("c")], f64::INFINITY);
    let grouped = parse_answer(&body, None).unwrap();
    let job_a: SeriesLabels = [("job".to_string(), "a".to_string())].into();
    assert_eq!(grouped[&job_a], 3.5);
    // Two results with one key once `__name__` is dropped.
    let twice = json!({"status": "success", "data": {"resultType": "vector", "result": [
        {"metric": {"__name__": "x", "job": "a"}, "value": [1, "1"]},
        {"metric": {"__name__": "y", "job": "a"}, "value": [1, "2"]}
    ]}})
    .to_string();
    assert!(parse_answer(&twice, None)
        .unwrap_err()
        .contains("two series"));
}

/// A fake VictoriaMetrics: `/metrics` with a version, a reload counter that
/// grows on `POST /-/reload`, an ignored-samples counter per entry of the
/// stream-aggregation config at `config` (named as the config names it),
/// `/api/v1/query` with no series, and `/internal/force_flush`.
fn fake_server(config: PathBuf) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let reloads = Arc::new(AtomicU64::new(0));
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let mut stream = stream.unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            let mut length = 0;
            let mut header = String::new();
            while reader.read_line(&mut header).unwrap() > 0 && header.trim() != "" {
                if let Some(v) = header.to_ascii_lowercase().strip_prefix("content-length:") {
                    length = v.trim().parse().unwrap();
                }
                header.clear();
            }
            let mut body = vec![0; length];
            reader.read_exact(&mut body).unwrap();
            let path = line.split_whitespace().nth(1).unwrap_or("");
            let answer = if path.starts_with("/-/reload") {
                reloads.fetch_add(1, Ordering::SeqCst);
                String::new()
            } else if path.starts_with("/api/v1/query") {
                "{\"status\":\"success\",\"data\":{\"resultType\":\"vector\",\"result\":[]}}"
                    .to_string()
            } else if path.starts_with("/metrics") {
                let entries: String = std::fs::read_to_string(&config)
                    .unwrap_or_default()
                    .lines()
                    .filter_map(|line| line.strip_prefix("  name: "))
                    .enumerate()
                    .map(|(i, name)| {
                        format!(
                            "vm_streamaggr_ignored_samples_total{{reason=\"too_old\",name=\"{}\",position=\"{}\"}} 0\n",
                            name.trim_matches('\''),
                            i + 1
                        )
                    })
                    .collect();
                format!(
                    "vm_app_version{{version=\"victoria-metrics-fake-v1.154.1\"}} 1\n\
                     vminsert_streamagg_config_reloads_total {}\n\
                     vminsert_streamagg_config_reloads_errors_total 0\n\
                     vm_data_size_bytes{{type=\"storage/small\"}} 0\n{entries}",
                    reloads.load(Ordering::SeqCst)
                )
            } else {
                String::new()
            };
            let _ = write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{answer}",
                answer.len()
            );
        }
    });
    url
}

fn fake_agent(dir: &std::path::Path) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let config = dir.join("stream-aggregation.yaml");
    std::fs::write(&config, "[]\n").unwrap();
    let dir = dir.to_path_buf();
    let proc_stat = dir.join("proc-stat");
    std::thread::spawn(move || serve_agent(listener, &dir, &proc_stat, Some(&config)));
    url
}

fn temp_dir(test: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("vm-server-{}-{test}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn workload(texts: &[&str], interval_ms: u64) -> WorkloadSpec {
    WorkloadSpec {
        name: "u".into(),
        queries: texts
            .iter()
            .enumerate()
            .map(|(i, text)| WorkloadQuery {
                id: format!("q{i}"),
                texts: vec![QueryText {
                    language: QueryLanguage::PromQl,
                    text: text.to_string(),
                }],
                window_ms: 60_000,
                evaluation_interval_ms: interval_ms,
            })
            .collect(),
        data: vec![],
    }
}

fn arm(dir: &std::path::Path, plan: &serde_json::Value) -> ArmSpec {
    let path = dir.join("p.plan.json");
    std::fs::write(&path, plan.to_string()).unwrap();
    ArmSpec {
        name: "u".into(),
        raw_data_retained: true,
        engine_options: json!({ "plan_file": path }),
    }
}

#[test]
fn one_arm_at_a_time_and_its_config_reaches_the_agent() {
    let dir = temp_dir("one-arm");
    let engine = VictoriaMetricsEngine::new(VictoriaMetricsServer {
        url: fake_server(dir.join("stream-aggregation.yaml")),
        cgroup_agent_url: fake_agent(&dir),
        request_timeout: Duration::from_secs(10),
    })
    .unwrap();
    assert_eq!(engine.description().id, "victoriametrics");
    assert_eq!(
        engine.description().version,
        "victoria-metrics-fake-v1.154.1"
    );
    // A stored plan with 10 s windows: the install waits at most the
    // margin plus one step of 10 s.
    let mut plan = fixture_value_public("sot__1h__1hz__60s", "victoriametrics.equivalent-only");
    payload_body(&mut plan, 1)["tumbling_window_width_ms"] = json!(10_000);
    let texts = ["sum by (job) (sum_over_time(latency[1h]))"];
    let first = engine
        .install(&workload(&texts, 10_000), &arm(&dir, &plan))
        .unwrap();
    let config = std::fs::read_to_string(dir.join("stream-aggregation.yaml")).unwrap();
    assert!(config.contains("  name: 'install-"), "{config}");
    assert!(config.contains("  interval: 10s\n"), "{config}");
    let error = engine
        .install(&workload(&texts, 10_000), &arm(&dir, &plan))
        .err()
        .expect("a second arm is refused");
    assert!(error.0.contains("--warmup 0 --runs 1"), "{error}");
    drop(first);
    // Dropping the arm unloads its config and frees the engine; the next
    // install's config has another name.
    assert_eq!(
        std::fs::read_to_string(dir.join("stream-aggregation.yaml")).unwrap(),
        "[]\n"
    );
    let second = engine
        .install(&workload(&texts, 10_000), &arm(&dir, &plan))
        .unwrap();
    let again = std::fs::read_to_string(dir.join("stream-aggregation.yaml")).unwrap();
    assert_ne!(again, config);
    drop(second);
}

#[test]
fn plans_are_checked_against_the_workload_before_anything_loads() {
    let dir = temp_dir("checks");
    let engine = VictoriaMetricsEngine::new(VictoriaMetricsServer {
        url: fake_server(dir.join("stream-aggregation.yaml")),
        cgroup_agent_url: fake_agent(&dir),
        request_timeout: Duration::from_secs(10),
    })
    .unwrap();
    let plan = fixture_value_public("sot__1h__1hz__60s", "victoriametrics.equivalent-only");
    // Another query text than the plan's.
    let error = engine
        .install(&workload(&["other"], 60_000), &arm(&dir, &plan))
        .err()
        .unwrap();
    assert!(error.0.contains("the plan answers"), "{error}");
    // Stored windows of 60 s cannot be read every 10 s.
    let error = engine
        .install(
            &workload(&["sum by (job) (sum_over_time(latency[1h]))"], 10_000),
            &arm(&dir, &plan),
        )
        .err()
        .unwrap();
    assert!(
        error.0.contains("do not divide its evaluation interval"),
        "{error}"
    );
    // Nothing was loaded.
    assert_eq!(
        std::fs::read_to_string(dir.join("stream-aggregation.yaml")).unwrap(),
        "[]\n"
    );
}

#[test]
fn a_query_time_arm_ingests_then_evaluates() {
    // No stream aggregation: the ingest of the chunk ending at t still
    // returns only once wall time reached t, so the evaluation at t follows.
    let dir = temp_dir("query-time");
    let engine = VictoriaMetricsEngine::new(VictoriaMetricsServer {
        url: fake_server(dir.join("stream-aggregation.yaml")),
        cgroup_agent_url: fake_agent(&dir),
        request_timeout: Duration::from_secs(10),
    })
    .unwrap();
    let plan = fixture_value_public(
        "q4c_sum_over_time__1m_1hz_10s",
        "victoriametrics.equivalent-only",
    );
    let texts = ["sum by (job) (sum_over_time(latency[1m]))"];
    let mut installed = engine
        .install(&workload(&texts, 10_000), &arm(&dir, &plan))
        .unwrap();
    let series = labels(&[("__name__", "latency"), ("job", "a")]);
    let chunk = SampleChunk {
        end_ms: 10_000,
        samples: (1..=10)
            .map(|s| Sample {
                labels: Arc::clone(&series),
                ts_ms: s * 1000,
                value: s as f64,
            })
            .collect(),
    };
    installed.ingest(&chunk).unwrap();
    let (answers, _) = installed.evaluate(10_000).unwrap();
    assert!(answers.0["q0"].is_empty());
}

#[test]
fn only_counter_plans_wait_for_the_previous_arms_samples_to_age_out() {
    use aqpbm_engines_external::plan_file::PlanFile;
    let plan = |workload: &str, variant: &str| {
        PlanFile::from_value(fixture_value_public(workload, variant)).unwrap()
    };
    // Rollups over a window read only the window: no wait.
    assert_eq!(
        previous_arm_gap_ms(&plan(
            "q4c_sum_over_time__1m_1hz_10s",
            "victoriametrics.equivalent-only"
        )),
        0
    );
    assert_eq!(
        previous_arm_gap_ms(&plan(
            "sot__1h__1hz__60s",
            "victoriametrics.equivalent-only"
        )),
        0
    );
    // `rate` looks back for the sample before its range: 10 min plus the
    // longest range (60 s).
    assert_eq!(
        previous_arm_gap_ms(&plan(
            "q2_sum_rate__1m_1hz_10s",
            "victoriametrics.with-substitutes"
        )),
        660_000
    );
}

#[test]
fn only_a_bare_selector_answer_keeps_the_metric_name() {
    use aqpbm_engines_external::plan_file::PlanFile;
    use aqpbm_engines_external::victoriametrics::translate;
    // PromQL drops `__name__` in every function and aggregation, e.g.
    // `max_over_time(latency[1m])` answers `{job="a", instance="i"}`.
    let value = fixture_value_public("q4b_max__1m_1hz_10s", "victoriametrics.equivalent-only");
    let plan = PlanFile::from_value(value.clone()).unwrap();
    let deployment = translate(&plan).unwrap();
    assert_eq!(
        answer_metric_name(&plan, plan.roots[0], &deployment.queries[0]),
        None
    );
    // A root that is the instant selector `latency` keeps it.
    let mut instant = value;
    payload_body(&mut instant, 1)["kind"] = json!("instant");
    payload_body(&mut instant, 1)["range"] = json!({"secs": 0, "nanos": 5_000_000});
    instant["dag"]["roots"] = json!([1]);
    let plan = PlanFile::from_value(instant).unwrap();
    let deployment = translate(&plan).unwrap();
    assert_eq!(
        answer_metric_name(&plan, plan.roots[0], &deployment.queries[0]).as_deref(),
        Some("latency")
    );
}
