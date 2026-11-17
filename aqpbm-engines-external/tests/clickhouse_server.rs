//! S1–S10: the ClickHouse engine adapter against an in-test HTTP stub that
//! records every request and answers by rules. No server is needed.

mod common;

use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use aqpbm_engines_external::clickhouse_server::{
    series_identity, ClickHouseEngine, ClickHouseServer,
};
use aqpbm_core::harness::cgroup::CgroupSource;
use aqpbm_core::harness::data::{DataRecord, Sample};
use aqpbm_core::harness::engine::{
    ArmSpec, CpuAccounting, EngineUnderTest, QueryLanguage, QueryText, SampleChunk, SeriesLabels,
    WorkloadQuery, WorkloadSpec,
};
use common::{fixture_path, fixture_value, payload_body};
use serde_json::json;

/// One request the stub received.
#[derive(Debug, Clone)]
struct Request {
    params: BTreeMap<String, String>,
    body: String,
}

impl Request {
    /// The statement: the `query` parameter of an insert, else the body.
    fn statement(&self) -> &str {
        self.params
            .get("query")
            .map(String::as_str)
            .unwrap_or(&self.body)
    }
}

/// An answer: status, extra headers, body.
type Reply = (u16, Vec<(String, String)>, String);
type Rule = dyn Fn(&Request) -> Option<Reply> + Send + Sync;

/// The stub: answers the version and leftover-database questions, then the
/// first rule that matches, else `200 ""`.
struct Stub {
    url: String,
    requests: Arc<Mutex<Vec<Request>>>,
}

fn start_stub(rules: Vec<Box<Rule>>) -> Stub {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let requests = Arc::new(Mutex::new(Vec::new()));
    let seen = Arc::clone(&requests);
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { return };
            let Some(request) = read_request(&mut stream) else {
                continue;
            };
            seen.lock().unwrap().push(request.clone());
            let (status, headers, body) = rules
                .iter()
                .find_map(|rule| rule(&request))
                .or_else(|| default_reply(&request))
                .unwrap_or((200, vec![], String::new()));
            let mut head = format!(
                "HTTP/1.1 {status} X\r\nContent-Length: {}\r\nConnection: close\r\n",
                body.len()
            );
            for (name, value) in headers {
                head.push_str(&format!("{name}: {value}\r\n"));
            }
            let _ = stream.write_all(format!("{head}\r\n{body}").as_bytes());
        }
    });
    Stub { url, requests }
}

fn default_reply(request: &Request) -> Option<Reply> {
    let statement = request.statement();
    if statement.contains("SELECT version()") {
        return Some((200, vec![], "26.8.39.13\td5bbb64d\n".into()));
    }
    if statement.contains("system.databases") {
        return Some((200, vec![], String::new()));
    }
    None
}

fn read_request(stream: &mut std::net::TcpStream) -> Option<Request> {
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    reader.read_line(&mut line).ok()?;
    let target = line.split_whitespace().nth(1)?.to_string();
    let mut length = 0;
    loop {
        let mut header = String::new();
        reader.read_line(&mut header).ok()?;
        if header == "\r\n" || header.is_empty() {
            break;
        }
        if let Some((name, value)) = header.split_once(':') {
            if name.eq_ignore_ascii_case("content-length") {
                length = value.trim().parse().ok()?;
            }
        }
    }
    let mut body = vec![0; length];
    reader.read_exact(&mut body).ok()?;
    let params = target
        .split_once('?')
        .map(|(_, query)| query)
        .unwrap_or("")
        .split('&')
        .filter(|pair| !pair.is_empty())
        .map(|pair| {
            let (name, value) = pair.split_once('=').unwrap_or((pair, ""));
            (decode(name), decode(value))
        })
        .collect();
    Some(Request {
        params,
        body: String::from_utf8(body).ok()?,
    })
}

/// URL form decoding: `+` is a space, `%XX` a byte.
fn decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'+' => out.push(b' '),
            b'%' if i + 2 < bytes.len() => {
                out.push(u8::from_str_radix(&text[i + 1..i + 3], 16).unwrap());
                i += 2;
            }
            b => out.push(b),
        }
        i += 1;
    }
    String::from_utf8(out).unwrap()
}

fn engine(stub: &Stub) -> ClickHouseEngine {
    ClickHouseEngine::new(ClickHouseServer {
        url: stub.url.clone(),
        cgroup_agent_url: "http://10.0.0.2:9101".into(),
        max_threads: 1,
        statement_timeout: Duration::from_secs(5),
    })
    .expect("engine")
}

fn arm(plan: &str) -> ArmSpec {
    ArmSpec {
        name: "equivalent-only".into(),
        raw_data_retained: true,
        engine_options: json!({ "plan_file": fixture_path(plan) }),
    }
}

/// A workload whose queries are the plan's texts, with ids `q0`, `q1`, …,
/// evaluated every `interval_ms`, with data from `start_ms`.
fn workload_of(plan: &str, interval_ms: u64, start_ms: i64) -> WorkloadSpec {
    let value = fixture_value(plan);
    let texts: Vec<String> = serde_json::from_value(value["query_texts"].clone()).unwrap();
    WorkloadSpec {
        name: plan.into(),
        queries: texts
            .iter()
            .enumerate()
            .map(|(i, text)| WorkloadQuery {
                id: format!("q{i}"),
                texts: vec![QueryText {
                    language: QueryLanguage::PromQl,
                    text: text.clone(),
                }],
                window_ms: 60_000,
                evaluation_interval_ms: interval_ms,
            })
            .collect(),
        data: vec![data_record(start_ms)],
    }
}

fn data_record(start_ms: i64) -> DataRecord {
    serde_json::from_value(json!({
        "metric": "latency",
        "description": {
            "labels": [], "series_num": 1, "start_ms": start_ms, "sample_interval_ms": 1000,
            "duration_ms": 60000,
            "value_distribution": {"kind": "uniform", "lower_bound": 0.0, "upper_bound": 1.0, "seed": 0},
            "counter": false, "seed": 1
        }
    }))
    .unwrap()
}

fn labels(pairs: &[(&str, &str)]) -> Arc<SeriesLabels> {
    Arc::new(
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect(),
    )
}

fn statements(stub: &Stub) -> Vec<String> {
    stub.requests
        .lock()
        .unwrap()
        .iter()
        .map(|r| r.statement().to_string())
        .collect()
}

#[test]
fn install_creates_a_database_and_the_translated_tables() {
    // S1
    let stub = start_stub(vec![]);
    let engine = engine(&stub);
    let installed = engine
        .install(
            &workload_of("rate_5m_stored", 60_000, 0),
            &arm("rate_5m_stored"),
        )
        .expect("installs");
    let requests = stub.requests.lock().unwrap().clone();
    // version, leftover check, CREATE DATABASE, src table, view table, view.
    let created: Vec<&Request> = requests
        .iter()
        .filter(|r| r.statement().starts_with("CREATE"))
        .collect();
    assert_eq!(created.len(), 4, "{requests:#?}");
    let database = created[0]
        .statement()
        .strip_prefix("CREATE DATABASE ")
        .unwrap()
        .to_string();
    assert!(database.starts_with("bench_"));
    for request in &created[1..] {
        assert_eq!(request.params.get("database"), Some(&database));
        assert_eq!(
            request.params.get("wait_end_of_query").map(String::as_str),
            Some("1")
        );
    }
    assert!(created[1]
        .statement()
        .starts_with("CREATE TABLE src_requests_total"));
    assert!(created[2].statement().starts_with("CREATE TABLE v1"));
    assert!(created[3]
        .statement()
        .starts_with("CREATE MATERIALIZED VIEW v1_mv"));
    let summary = installed.plan_summary();
    assert_eq!(
        summary.substitutions,
        vec![
            "clickhouse.stored_rate.delta_sum_timestamp at node 1: a counter reset adds 0 \
              for the step across it, where PromQL adds the first value after the reset"
        ]
    );
    assert_eq!(summary.stored_outputs, vec!["v1"]);
    assert_eq!(summary.node_kinds[1], "SummaryTumblingWindowChain");
    // S8: dropping the arm drops its database, synchronously.
    drop(installed);
    assert!(statements(&stub).contains(&format!("DROP DATABASE IF EXISTS {database} SYNC")));

    let joint = engine
        .install(&workload_of("joint_5m", 60_000, 0), &arm("joint_5m"))
        .expect("installs");
    assert_eq!(
        joint.plan_summary().sharing_lost,
        vec!["node 1 for nodes 2, 3, 4"]
    );
}

#[test]
fn install_maps_queries_by_index_and_text() {
    // S2
    let stub = start_stub(vec![]);
    let engine = engine(&stub);
    let mut workload = workload_of("sot_1h_stored_sum", 60_000, 0);
    // Another language's text that equals the plan's is accepted.
    workload.queries[0].texts.insert(
        0,
        QueryText {
            language: QueryLanguage::Sql,
            text: "SELECT 1".into(),
        },
    );
    assert!(engine.install(&workload, &arm("sot_1h_stored_sum")).is_ok());
    workload.queries[0].texts.remove(1);
    let error = engine
        .install(&workload, &arm("sot_1h_stored_sum"))
        .err()
        .expect("refused");
    assert!(
        error
            .0
            .contains("sum by (job) (sum_over_time(latency[1h]))")
            && error.0.contains("SELECT 1"),
        "{error}"
    );
    let mut two = workload_of("sot_1h_stored_sum", 60_000, 0);
    two.queries.push(two.queries[0].clone());
    assert!(engine
        .install(&two, &arm("sot_1h_stored_sum"))
        .err()
        .expect("refused")
        .0
        .contains("1 queries"));
    // A stored step (60 s) that does not divide the evaluation interval.
    let error = engine
        .install(
            &workload_of("sot_1h_stored_sum", 90_000, 0),
            &arm("sot_1h_stored_sum"),
        )
        .err()
        .expect("refused");
    assert!(error.0.contains("60000 ms"), "{error}");
    // ... or the data start.
    assert!(engine
        .install(
            &workload_of("sot_1h_stored_sum", 60_000, 30_000),
            &arm("sot_1h_stored_sum")
        )
        .is_err());
}

#[test]
fn install_refuses_missing_unreadable_and_untranslatable_plans() {
    // S3
    let stub = start_stub(vec![]);
    let engine = engine(&stub);
    let workload = workload_of("sot_1h_stored_sum", 60_000, 0);
    let mut no_plan = arm("sot_1h_stored_sum");
    no_plan.engine_options = json!({});
    assert!(engine
        .install(&workload, &no_plan)
        .err()
        .unwrap()
        .0
        .contains("plan_file"));
    let mut missing = arm("sot_1h_stored_sum");
    missing.engine_options = json!({"plan_file": "/nonexistent/plan.json"});
    assert!(engine
        .install(&workload, &missing)
        .err()
        .unwrap()
        .0
        .contains("/nonexistent"));
    let mut value = fixture_value("sot_1h_stored_sum");
    payload_body(&mut value, 1)["summary_type"] = json!({"ExactAggregate": ["IRate", "IRate"]});
    let dir = std::env::temp_dir().join(format!("ch-server-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("irate.plan.json");
    std::fs::write(&path, value.to_string()).unwrap();
    let mut irate = arm("sot_1h_stored_sum");
    irate.engine_options = json!({ "plan_file": path });
    let error = engine.install(&workload, &irate).err().expect("refused");
    assert!(
        error.0.contains("no translation for exact IRate"),
        "{error}"
    );
    // Nothing was created for a refused install.
    assert!(!statements(&stub).iter().any(|s| s.starts_with("CREATE")));
}

#[test]
fn ingest_inserts_each_metric_in_chunk_order() {
    // S4
    let stub = start_stub(vec![]);
    let engine = engine(&stub);
    let mut installed = engine
        .install(
            &workload_of("max_5m_label_filter", 300_000, 0),
            &arm("max_5m_label_filter"),
        )
        .unwrap();
    let odd = "a\tb\nc\\d\"é";
    let a = labels(&[("__name__", "latency"), ("job", "j3")]);
    let b = labels(&[("__name__", "latency"), ("instance", odd)]);
    let other = labels(&[("__name__", "unread")]);
    let chunk = SampleChunk {
        end_ms: 2000,
        samples: vec![
            Sample {
                labels: Arc::clone(&a),
                ts_ms: 1000,
                value: 1.5,
            },
            Sample {
                labels: Arc::clone(&b),
                ts_ms: 1000,
                value: f64::NAN,
            },
            Sample {
                labels: other,
                ts_ms: 1000,
                value: 9.0,
            },
            Sample {
                labels: Arc::clone(&a),
                ts_ms: 2000,
                value: 2.0,
            },
        ],
    };
    let report = installed.ingest(&chunk).unwrap();
    assert_eq!(report.engine_reported_cpu_ns, None);
    let requests = stub.requests.lock().unwrap().clone();
    let inserts: Vec<&Request> = requests
        .iter()
        .filter(|r| r.statement().starts_with("INSERT"))
        .collect();
    assert_eq!(
        inserts.len(),
        1,
        "one insert for the one metric the plan reads"
    );
    let insert = inserts[0];
    assert!(
        insert.statement().starts_with(
            "INSERT INTO src_latency (ts, value, `job`, `$promql_series_identity`) \
         SETTINGS input_format_null_as_default = 0, insert_null_as_default = 0 SELECT"
        ),
        "{}",
        insert.statement()
    );
    let escaped_identity = |l: &SeriesLabels| {
        series_identity(l)
            .replace('\\', "\\\\")
            .replace('\t', "\\t")
            .replace('\n', "\\n")
    };
    assert_eq!(
        insert.body,
        format!(
            "1000\t1.5\tj3\t{}\n1000\tnan\t\t{}\n2000\t2\tj3\t{}\n",
            escaped_identity(&a),
            escaped_identity(&b),
            escaped_identity(&a)
        )
    );
    assert_eq!(series_identity(&a), r#"{"__name__":"latency","job":"j3"}"#);
    // A sample at 0 ms is refused (the window key needs ts > 0).
    let zero = SampleChunk {
        end_ms: 0,
        samples: vec![Sample {
            labels: a,
            ts_ms: 0,
            value: 1.0,
        }],
    };
    assert!(installed.ingest(&zero).is_err());
}

#[test]
fn evaluate_parses_series_and_values() {
    // S5, S6, S6b
    let identity = r#"{"__name__":"latency","instance":"a\tb","job":"j3"}"#;
    let escaped = identity.replace('\\', "\\\\").replace('\t', "\\t");
    let answer = format!("job\t$promql_series_identity\tvalue\nj3\t{escaped}\tnan\n");
    let stub = start_stub(vec![Box::new(move |request: &Request| {
        request
            .statement()
            .contains("FORMAT TabSeparatedWithNames")
            .then(|| {
                (
                    200,
                    vec![(
                        "X-ClickHouse-Summary".to_string(),
                        r#"{"read_rows":"3","read_bytes":"48"}"#.to_string(),
                    )],
                    answer.clone(),
                )
            })
    })]);
    let engine = engine(&stub);
    let mut installed = engine
        .install(
            &workload_of("max_5m_label_filter", 300_000, 0),
            &arm("max_5m_label_filter"),
        )
        .unwrap();
    let (answers, report) = installed.evaluate(600_000).unwrap();
    let series: SeriesLabels = serde_json::from_str(identity).unwrap();
    assert!(answers.get("q0", &series).unwrap().is_nan());
    assert_eq!(report.scanned_bytes, Some(48));
    assert_eq!(report.engine_reported_cpu_ns, None);
    let query = stub
        .requests
        .lock()
        .unwrap()
        .iter()
        .rev()
        .find(|r| r.statement().contains("FORMAT TabSeparatedWithNames"))
        .cloned()
        .unwrap();
    assert_eq!(
        query.params.get("param_t").map(String::as_str),
        Some("600000")
    );
    assert_eq!(
        query.params.get("max_threads").map(String::as_str),
        Some("1")
    );
    assert!(query.params.get("database").unwrap().starts_with("bench_"));

    // Without an identity column: the label columns, empty ones left out;
    // two rows of one series are an error.
    for (body, expected) in [
        (
            "job\tvalue\nj3\t2\n\t5\n",
            Ok(vec![(vec![("job", "j3")], 2.0), (vec![], 5.0)]),
        ),
        ("job\tvalue\nj3\t2\nj3\t5\n", Err("two rows")),
    ] {
        let body = body.to_string();
        let stub = start_stub(vec![Box::new(move |request: &Request| {
            request
                .statement()
                .contains("FORMAT TabSeparatedWithNames")
                .then(|| (200, vec![], body.clone()))
        })]);
        let mut installed = engine_for(&stub);
        match (installed.evaluate(600_000), expected) {
            (Ok((answers, _)), Ok(rows)) => {
                for (pairs, value) in rows {
                    let key: SeriesLabels = pairs
                        .iter()
                        .map(|(k, v)| (k.to_string(), v.to_string()))
                        .collect();
                    assert_eq!(answers.get("q0", &key), Some(value));
                }
            }
            (Err(error), Err(text)) => assert!(error.0.contains(text), "{error}"),
            (got, want) => panic!("{got:?} vs {want:?}"),
        }
    }

    // S6: an error status; S6b: status 200 with an exception header.
    for reply in [
        (
            500,
            vec![],
            "Code: 395. DB::Exception: not a window end".to_string(),
        ),
        (
            200,
            vec![("X-ClickHouse-Exception-Code".to_string(), "395".to_string())],
            "Code: 395. DB::Exception: not a window end".to_string(),
        ),
    ] {
        let stub = start_stub(vec![Box::new(move |request: &Request| {
            request
                .statement()
                .contains("FORMAT TabSeparatedWithNames")
                .then(|| reply.clone())
        })]);
        let error = engine_for(&stub).evaluate(600_000).expect_err("an error");
        assert!(error.0.contains("not a window end"), "{error}");
    }
}

fn engine_for(stub: &Stub) -> Box<dyn aqpbm_core::harness::engine::InstalledArm> {
    engine(stub)
        .install(
            &workload_of("max_5m_label_filter", 300_000, 0),
            &arm("max_5m_label_filter"),
        )
        .unwrap()
}

#[test]
fn a_server_that_never_answers_times_out() {
    // S6c: a listener that accepts and never answers.
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    std::thread::spawn(move || {
        let _held: Vec<_> = listener.incoming().take(4).collect();
        std::thread::sleep(Duration::from_secs(60));
    });
    let started = std::time::Instant::now();
    let result = ClickHouseEngine::new(ClickHouseServer {
        url,
        cgroup_agent_url: "http://10.0.0.2:9101".into(),
        max_threads: 1,
        statement_timeout: Duration::from_secs(1),
    });
    assert!(result.is_err());
    assert!(started.elapsed() < Duration::from_secs(20));
}

#[test]
fn footprint_splits_view_and_raw_bytes() {
    // S7
    let stub = start_stub(vec![Box::new(|request: &Request| {
        request
            .statement()
            .contains("system.parts")
            .then(|| (200, vec![], "src_latency\t100\nv1\t40\n".to_string()))
    })]);
    let engine = engine(&stub);
    let installed = engine
        .install(
            &workload_of("sot_1h_stored_sum", 60_000, 0),
            &arm("sot_1h_stored_sum"),
        )
        .unwrap();
    let footprint = installed.footprint().unwrap();
    assert_eq!(footprint.view_bytes, 40);
    assert_eq!(footprint.raw_retained_bytes, 100);
}

#[test]
fn description_and_cpu_accounting() {
    // S9
    let stub = start_stub(vec![]);
    let engine = engine(&stub);
    let description = engine.description();
    assert_eq!(description.id, "clickhouse");
    assert_eq!(description.version, "26.8.39.13 (d5bbb64d)");
    assert_eq!(description.build_profile, "max_threads=1");
    assert_eq!(
        engine.cpu_accounting(),
        CpuAccounting::Cgroup(CgroupSource::Agent {
            url: "http://10.0.0.2:9101".into()
        })
    );
}

#[test]
fn leftover_databases_are_refused() {
    // S10
    let stub = start_stub(vec![Box::new(|request: &Request| {
        request
            .statement()
            .contains("system.databases")
            .then(|| (200, vec![], "bench_1_0\nbench_1_1\n".to_string()))
    })]);
    let error = ClickHouseEngine::new(ClickHouseServer {
        url: stub.url.clone(),
        cgroup_agent_url: "http://10.0.0.2:9101".into(),
        max_threads: 1,
        statement_timeout: Duration::from_secs(5),
    })
    .err()
    .expect("refused");
    assert!(error.0.contains("bench_1_0, bench_1_1"), "{error}");
}

#[test]
fn a_failed_install_drops_its_database() {
    // The database exists once CREATE DATABASE ran; a table that fails
    // drops it again.
    let stub = start_stub(vec![Box::new(|request: &Request| {
        request.statement().starts_with("CREATE TABLE").then(|| {
            (
                500,
                vec![],
                "Code: 57. DB::Exception: table exists".to_string(),
            )
        })
    })]);
    let engine = engine(&stub);
    let error = engine
        .install(
            &workload_of("sot_1h_stored_sum", 60_000, 0),
            &arm("sot_1h_stored_sum"),
        )
        .err()
        .expect("refused");
    assert!(error.0.contains("table exists"), "{error}");
    let statements = statements(&stub);
    let database = statements
        .iter()
        .find_map(|s| s.strip_prefix("CREATE DATABASE "))
        .expect("a database was created")
        .to_string();
    assert!(statements.contains(&format!("DROP DATABASE IF EXISTS {database} SYNC")));
}
