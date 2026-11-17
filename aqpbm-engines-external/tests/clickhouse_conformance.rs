//! Conformance of the ClickHouse adapter against a real server, one case
//! per row of `capabilities/README.md` (`#[ignore]`: set
//! `AQPBM_CLICKHOUSE_URL` to the server's HTTP interface and
//! `AQPBM_CGROUP_AGENT_URL` to its cgroup agent, e.g. a server started with
//! `scripts/extbench-up.sh`). Each case installs a planner-written plan
//! through the engine, ingests hand-made chunks and compares the answers
//! with hand-computed ones: (a) rows to 1e-9 relative, (b) rows by the
//! stated difference. Run with `--test-threads 1`: the cases share the
//! server.

mod common;

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use aqpbm_engines_external::clickhouse::translate;
use aqpbm_engines_external::clickhouse_server::{ClickHouseEngine, ClickHouseServer};
use aqpbm_engines_external::plan_file::PlanFile;
use aqpbm_core::harness::data::{workload_chunks, DataRecord, Sample};
use aqpbm_core::harness::engine::{
    Answers, ArmSpec, CpuAccounting, EngineDescription, EngineError, EngineUnderTest, InstalledArm,
    QueryLanguage, QueryText, SampleChunk, SeriesLabels, WorkloadQuery, WorkloadSpec,
};
use common::{fixture_path, fixture_value, fixture_value_public, lcm, payload_body};
use serde_json::{json, Value};

const IGNORE: &str =
    "needs a ClickHouse 26.8.39.13 server (AQPBM_CLICKHOUSE_URL, AQPBM_CGROUP_AGENT_URL)";

fn server(max_threads: u32) -> ClickHouseServer {
    ClickHouseServer {
        url: std::env::var("AQPBM_CLICKHOUSE_URL").expect("AQPBM_CLICKHOUSE_URL"),
        cgroup_agent_url: std::env::var("AQPBM_CGROUP_AGENT_URL").expect("AQPBM_CGROUP_AGENT_URL"),
        max_threads,
        statement_timeout: Duration::from_secs(600),
    }
}

fn engine(max_threads: u32) -> ClickHouseEngine {
    ClickHouseEngine::new(server(max_threads)).expect("the server answers")
}

/// `plan` written to a temporary file, for an arm's `plan_file`.
fn plan_file(name: &str, plan: &Value) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("ch-conformance-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(format!("{name}.plan.json"));
    std::fs::write(&path, plan.to_string()).unwrap();
    path
}

fn arm(path: &std::path::Path) -> ArmSpec {
    ArmSpec {
        name: "conformance".into(),
        raw_data_retained: true,
        engine_options: json!({ "plan_file": path }),
    }
}

/// A workload of `plan`'s query texts (ids `q0`, `q1`, …), evaluated every
/// `interval_ms` (the lcm of the plan's stored steps unless given), with
/// data from 0 ms.
fn workload(plan: &Value, interval_ms: Option<u64>) -> WorkloadSpec {
    let texts: Vec<String> = serde_json::from_value(plan["query_texts"].clone()).unwrap();
    let steps = translate(&PlanFile::from_value(plan.clone()).unwrap())
        .map(|d| d.query_time_steps_ms)
        .unwrap_or_default();
    let interval_ms = interval_ms.unwrap_or_else(|| steps.iter().fold(10_000, |a, &b| lcm(a, b)));
    WorkloadSpec {
        name: "conformance".into(),
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
        data: vec![],
    }
}

fn install(engine: &ClickHouseEngine, name: &str, plan: &Value) -> Box<dyn InstalledArm> {
    engine
        .install(&workload(plan, None), &arm(&plan_file(name, plan)))
        .unwrap_or_else(|e| panic!("{name}: {e}"))
}

fn series(pairs: &[(&str, &str)]) -> Arc<SeriesLabels> {
    Arc::new(
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect(),
    )
}

/// `samples` cut into time-ordered chunks of `chunk_ms`, ending at
/// multiples of it, time-major as the harness's chunks are.
fn chunks(mut samples: Vec<Sample>, chunk_ms: i64) -> Vec<SampleChunk> {
    samples.sort_by_key(|s| s.ts_ms);
    let mut out: Vec<SampleChunk> = Vec::new();
    for sample in samples {
        let end_ms = (sample.ts_ms + chunk_ms - 1).div_euclid(chunk_ms) * chunk_ms;
        match out.last_mut() {
            Some(chunk) if chunk.end_ms == end_ms => chunk.samples.push(sample),
            _ => out.push(SampleChunk {
                end_ms,
                samples: vec![sample],
            }),
        }
    }
    out
}

fn ingest_all(arm: &mut dyn InstalledArm, chunks: &[SampleChunk]) {
    for chunk in chunks {
        arm.ingest(chunk).expect("ingest");
    }
}

/// The answers of query `q<index>` as (labels without `__name__` joined by
/// `,`, value), for comparing with hand-written rows.
fn rows(answers: &Answers, index: usize) -> BTreeMap<String, f64> {
    answers
        .0
        .get(&format!("q{index}"))
        .into_iter()
        .flatten()
        .map(|(labels, value)| {
            let key: Vec<String> = labels
                .iter()
                .filter(|(name, _)| name.as_str() != "__name__")
                .map(|(name, value)| format!("{name}={value}"))
                .collect();
            (key.join(","), *value)
        })
        .collect()
}

fn assert_rows(got: &BTreeMap<String, f64>, expected: &[(&str, f64)]) {
    assert_eq!(got.len(), expected.len(), "{got:?}");
    for (key, want) in expected {
        let value = got
            .get(*key)
            .unwrap_or_else(|| panic!("no {key} in {got:?}"));
        assert!(
            (value - want).abs() <= 1e-9 * want.abs().max(1.0),
            "{key}: {value} vs {want}"
        );
    }
}

#[test]
#[ignore = "needs a ClickHouse server"]
fn the_server_is_the_pinned_binary() {
    let _ = IGNORE;
    let description: EngineDescription = engine(1).description();
    assert_eq!(
        description.version,
        "26.8.39.13 (d5bbb64d525197f84f9dd21489f5f6d6fdb966e5)"
    );
    assert_eq!(description.build_profile, "max_threads=1");
}

/// Row "stored windows, exact Sum / Count / Min / Max" on F1's shape
/// `sum by (job) (<f>_over_time(latency[1h]))` at t = 1 h: series a/1, a/2
/// and b/1 with base 1.0, 0.5 and 2.0, value base · (1 + s mod 2) each
/// second. Per series over 3600 s: sum 5400 · base, count 3600, min base,
/// max 2 · base. So sum a 8100, b 10800; count a 7200, b 3600; min a 1.5,
/// b 2; max a 3, b 4.
#[test]
#[ignore = "needs a ClickHouse server"]
fn stored_sum_count_min_max() {
    let engine = engine(1);
    let samples: Vec<Sample> = (1..=3600)
        .flat_map(|s: i64| {
            [("a", "1", 1.0), ("a", "2", 0.5), ("b", "1", 2.0)]
                .into_iter()
                .map(move |(job, i, base)| Sample {
                    labels: series(&[("__name__", "latency"), ("job", job), ("i", i)]),
                    ts_ms: s * 1000,
                    value: base * (1 + s % 2) as f64,
                })
        })
        .collect();
    let chunks = chunks(samples, 60_000);
    for (kind, expected) in [
        ("Sum", [("job=a", 8100.0), ("job=b", 10800.0)]),
        ("Count", [("job=a", 7200.0), ("job=b", 3600.0)]),
        ("Min", [("job=a", 1.5), ("job=b", 2.0)]),
        ("Max", [("job=a", 3.0), ("job=b", 4.0)]),
    ] {
        let mut plan = fixture_value("sot_1h_stored_sum");
        payload_body(&mut plan, 1)["summary_type"] = json!({"ExactAggregate": [kind, kind]});
        let mut arm = install(&engine, &format!("stored_{kind}"), &plan);
        ingest_all(arm.as_mut(), &chunks);
        let (answers, _) = arm.evaluate(3_600_000).unwrap();
        assert_rows(&rows(&answers, 0), &expected);
        let footprint = arm.footprint().unwrap();
        println!("stored {kind}: {:?} {footprint:?}", rows(&answers, 0));
    }
}

/// The counter data of the translator tests (x, y, z, w), each series in its
/// own job, over 600 s.
fn counter_samples() -> Vec<Sample> {
    let mut samples = Vec::new();
    for s in 1..=600i64 {
        let x = if s <= 450 { s } else { s - 450 } as f64;
        let y = if s <= 360 { 3 * s } else { s - 360 } as f64;
        let z = if s < 400 {
            s as f64
        } else if s < 500 {
            (s - 5) as f64
        } else {
            (s - 500) as f64 / 2.0
        };
        for (job, value) in [("x", x), ("y", y), ("z", z)] {
            samples.push(Sample {
                labels: series(&[("__name__", "requests_total"), ("job", job)]),
                ts_ms: s * 1000,
                value,
            });
        }
        if s % 10 == 5 {
            samples.push(Sample {
                labels: series(&[("__name__", "requests_total"), ("job", "w")]),
                ts_ms: s * 1000,
                value: s as f64,
            });
        }
    }
    samples
}

/// Row "stored windows, exact Rate / Increase" (b). Three installs take the
/// 10 s chunks in turn, as `run_arm` feeds its installs, at max_threads 1
/// and 16. Each answers PromQL's rate minus the first value after each
/// reset (rates are increase / 299 here; see the translator's
/// `stored_rate_differs_from_promql_by_the_first_value_after_each_reset`):
/// x 298/299, y 416/299, z 247/299, w 1.
#[test]
#[ignore = "needs a ClickHouse server"]
fn stored_rate_is_promql_minus_the_first_value_after_each_reset() {
    let chunks = chunks(counter_samples(), 10_000);
    let expected = [
        ("job=x", 298.0 / 299.0),
        ("job=y", 416.0 / 299.0),
        ("job=z", 247.0 / 299.0),
        ("job=w", 1.0),
    ];
    for threads in [1, 16] {
        let engine = engine(threads);
        let plan = fixture_value("rate_5m_stored");
        let mut installs: Vec<_> = (0..3)
            .map(|i| install(&engine, &format!("rate_stored_{threads}_{i}"), &plan))
            .collect();
        for chunk in &chunks {
            for arm in installs.iter_mut() {
                arm.ingest(chunk).unwrap();
            }
        }
        for arm in installs.iter_mut() {
            let (answers, _) = arm.evaluate(600_000).unwrap();
            assert_rows(&rows(&answers, 0), &expected);
        }
        assert_eq!(
            installs[0].plan_summary().substitutions.len(),
            1,
            "the substitute is recorded"
        );
    }
}

/// Row "query-time Rate / Increase" (a): the same data, PromQL's rates
/// x 1, y 417/299, z 642/299, w 1.
#[test]
#[ignore = "needs a ClickHouse server"]
fn query_time_rate_is_promql() {
    let engine = engine(1);
    let mut arm = install(
        &engine,
        "rate_query_time",
        &fixture_value("rate_5m_query_time"),
    );
    ingest_all(arm.as_mut(), &chunks(counter_samples(), 10_000));
    let (answers, _) = arm.evaluate(600_000).unwrap();
    assert_rows(
        &rows(&answers, 0),
        &[
            ("job=x", 1.0),
            ("job=y", 417.0 / 299.0),
            ("job=z", 642.0 / 299.0),
            ("job=w", 1.0),
        ],
    );
    // Row "raw samples retained": the source table keeps rows, no view.
    let footprint = arm.footprint().unwrap();
    assert!(
        footprint.raw_retained_bytes > 0 && footprint.view_bytes == 0,
        "{footprint:?}"
    );
    println!("query-time rate footprint {footprint:?}");
}

/// Rows "query-time Sum … grouped" and "Aggregate": q4c at 1 m over job a
/// (1.0 and 0.5 each second) and b (2.0): at 120 s, a 90, b 120. q1 (0.99
/// quantile per series over 1 m) of values 1..60: 1 + 0.99 · 59 = 59.41.
/// q3 (avg by job of the 0.9 quantiles) of two series 1..60 and 2·(1..60):
/// (54.1 + 108.2) / 2 = 81.15. F4's max of j3 = s over (300 s, 600 s]: 600.
#[test]
#[ignore = "needs a ClickHouse server"]
fn grouped_sums_quantiles_and_aggregates() {
    let engine = engine(1);
    let latency =
        |rows: &[(&str, &str, f64)], seconds: std::ops::RangeInclusive<i64>, step: bool| {
            seconds
                .flat_map(|s| {
                    rows.iter().map(move |(job, id, base)| Sample {
                        labels: series(&[("__name__", "latency"), ("job", job), ("id", id)]),
                        ts_ms: s * 1000,
                        value: if step { base * (s - 60) as f64 } else { *base },
                    })
                })
                .collect::<Vec<_>>()
        };
    let mut q4c = install(
        &engine,
        "q4c",
        &fixture_value_public(
            "q4c_sum_over_time__1m_1hz_10s",
            "clickhouse.equivalent-only",
        ),
    );
    ingest_all(
        q4c.as_mut(),
        &chunks(
            latency(
                &[("a", "1", 1.0), ("a", "2", 0.5), ("b", "1", 2.0)],
                1..=120,
                false,
            ),
            10_000,
        ),
    );
    assert_rows(
        &rows(&q4c.evaluate(120_000).unwrap().0, 0),
        &[("job=a", 90.0), ("job=b", 120.0)],
    );

    // Values 1..60 over (60 s, 120 s].
    let ramp = chunks(
        latency(&[("a", "1", 1.0), ("a", "2", 2.0)], 61..=120, true),
        10_000,
    );
    let mut q1 = install(
        &engine,
        "q1",
        &fixture_value_public("q1_quantile__1m_1hz_10s", "clickhouse.equivalent-only"),
    );
    ingest_all(q1.as_mut(), &ramp);
    let answers = q1.evaluate(120_000).unwrap().0;
    assert_rows(
        &rows(&answers, 0),
        &[("id=1,job=a", 59.41), ("id=2,job=a", 2.0 * 59.41)],
    );
    let mut q3 = install(
        &engine,
        "q3",
        &fixture_value_public("q3_avg_quantile__1m_1hz_10s", "clickhouse.equivalent-only"),
    );
    ingest_all(q3.as_mut(), &ramp);
    assert_rows(
        &rows(&q3.evaluate(120_000).unwrap().0, 0),
        &[("job=a", 81.15)],
    );

    let mut f4 = install(&engine, "f4", &fixture_value("max_5m_label_filter"));
    let samples: Vec<Sample> = (1..=600)
        .flat_map(|s: i64| {
            [("j3", 1.0), ("j4", 10.0)]
                .into_iter()
                .map(move |(job, m)| Sample {
                    labels: series(&[("__name__", "latency"), ("job", job)]),
                    ts_ms: s * 1000,
                    value: m * s as f64,
                })
        })
        .collect();
    ingest_all(f4.as_mut(), &chunks(samples, 300_000));
    assert_rows(
        &rows(&f4.evaluate(600_000).unwrap().0, 0),
        &[("job=j3", 600.0)],
    );
}

/// Row "instant selector": rangeheavy's `sum by (job) (data)` with the
/// plan's 5 ms lookback at t = 120 s: s1 (a) at t − 10 ms (100) and t − 3 ms
/// (1); s2 (a) only at t − 5 ms, outside (t − 5 ms, t]; s3 (a) at t (2); s4
/// (b) at t − 1 ms (4): a 3, b 4.
#[test]
#[ignore = "needs a ClickHouse server"]
fn instant_selector() {
    let engine = engine(1);
    let plan = fixture_value_public("rangeheavy", "clickhouse.equivalent-only");
    let mut arm = install(&engine, "rangeheavy", &plan);
    let t = 120_000;
    let samples = [
        (10, 100.0, "a", "s1"),
        (5, 50.0, "a", "s2"),
        (3, 1.0, "a", "s1"),
        (1, 4.0, "b", "s4"),
        (0, 2.0, "a", "s3"),
    ]
    .into_iter()
    .map(|(before, value, job, id)| Sample {
        labels: series(&[("__name__", "data"), ("job", job), ("id", id)]),
        ts_ms: t - before,
        value,
    })
    .collect();
    arm.ingest(&SampleChunk { end_ms: t, samples }).unwrap();
    let answers = arm.evaluate(t).unwrap().0;
    assert_rows(&rows(&answers, 0), &[("job=a", 3.0), ("job=b", 4.0)]);
}

/// Row "Aggregate top_k": rates 1..5 for i1..i5, NaN for i6 and 5 for i7.
/// k = 2 keeps i5 and i7 (equal values: the label order breaks the tie);
/// k = 6 keeps every series but the NaN one.
#[test]
#[ignore = "needs a ClickHouse server"]
fn top_k_ties_and_nan() {
    let engine = engine(1);
    let samples: Vec<Sample> = (1..=600)
        .flat_map(|s: i64| {
            [1.0, 2.0, 3.0, 4.0, 5.0, f64::NAN, 5.0]
                .into_iter()
                .enumerate()
                .map(move |(i, rate)| Sample {
                    labels: series(&[
                        ("__name__", "requests_total"),
                        ("instance", &format!("i{}", i + 1)),
                    ]),
                    ts_ms: s * 1000,
                    value: rate * s as f64,
                })
        })
        .collect();
    let chunks = chunks(samples, 60_000);
    for (k, expected) in [
        (2, vec![("instance=i5", 5.0), ("instance=i7", 5.0)]),
        (
            6,
            vec![
                ("instance=i1", 1.0),
                ("instance=i2", 2.0),
                ("instance=i3", 3.0),
                ("instance=i4", 4.0),
                ("instance=i5", 5.0),
                ("instance=i7", 5.0),
            ],
        ),
    ] {
        let mut plan = fixture_value_public("topk__5m__1hz__60s", "clickhouse.equivalent-only");
        payload_body(&mut plan, 5)["measures"][0]["k"] = json!(k);
        let mut arm = install(&engine, &format!("topk_{k}"), &plan);
        ingest_all(arm.as_mut(), &chunks);
        assert_rows(&rows(&arm.evaluate(600_000).unwrap().0, 0), &expected);
    }
}

/// Rows "query-time … regrouping merges" and "Project / BinaryOp":
/// mixed-03 root 11, `avg by (region, service) (quantile_over_time(0.9,
/// data[100s]))` as sum / count, at 120 s over constant series (j1, r1) 1,
/// (j2, r1) 3, (j1, r2) 5: r1/s1 2, r2/s1 5. Two p90s of 1.5e308 overflow
/// the sum: the checked division is an error.
#[test]
#[ignore = "needs a ClickHouse server"]
fn average_as_sum_over_count_and_checked_division() {
    let engine = engine(1);
    let plan = fixture_value_public("mixed-03", "clickhouse.equivalent-only");
    let constant = |rows: &[(&str, &str, f64)]| {
        (1..=120)
            .flat_map(|s: i64| {
                rows.iter().map(move |(job, region, value)| Sample {
                    labels: series(&[
                        ("__name__", "data"),
                        ("job", job),
                        ("region", region),
                        ("service", "s1"),
                    ]),
                    ts_ms: s * 1000,
                    value: *value,
                })
            })
            .collect::<Vec<_>>()
    };
    let mut arm = install(&engine, "mixed03", &plan);
    ingest_all(
        arm.as_mut(),
        &chunks(
            constant(&[("j1", "r1", 1.0), ("j2", "r1", 3.0), ("j1", "r2", 5.0)]),
            30_000,
        ),
    );
    let answers = arm.evaluate(120_000).unwrap().0;
    assert_rows(
        &rows(&answers, 3),
        &[("region=r1,service=s1", 2.0), ("region=r2,service=s1", 5.0)],
    );
    let mut overflow = install(&engine, "mixed03_overflow", &plan);
    ingest_all(
        overflow.as_mut(),
        &chunks(
            constant(&[("j1", "r1", 1.5e308), ("j2", "r1", 1.5e308)]),
            30_000,
        ),
    );
    let error = overflow.evaluate(120_000).err().expect("an error");
    assert!(error.0.contains("checked division"), "{error}");
}

/// Rows of class (c): IRate and a query-time result kept across
/// evaluations are refused at install.
#[test]
#[ignore = "needs a ClickHouse server"]
fn class_c_rows_are_refused() {
    let engine = engine(1);
    let mut irate = fixture_value("rate_5m_query_time");
    payload_body(&mut irate, 2)["summary_type"] = json!({"ExactAggregate": ["IRate", "IRate"]});
    let error = engine
        .install(&workload(&irate, None), &arm(&plan_file("irate", &irate)))
        .err()
        .expect("refused");
    assert!(error.0.contains("exact IRate"), "{error}");
    let mut kept = fixture_value("joint_5m");
    kept["dag"]["nodes"][1]["lifetime"] = json!({"AcrossEvaluations": "Memory"});
    let error = engine
        .install(&workload(&kept, None), &arm(&plan_file("kept", &kept)))
        .err()
        .expect("refused");
    assert!(error.0.contains("kept across evaluations"), "{error}");
}

/// Rows "raw samples retained" (a stored-only metric keeps no rows) and
/// "retention" (the TTL statements apply on the server).
#[test]
#[ignore = "needs a ClickHouse server"]
fn storage_and_retention() {
    let engine = engine(1);
    let plan = fixture_value("sot_1h_stored_sum");
    let mut arm = install(&engine, "sot_storage", &plan);
    let samples: Vec<Sample> = (1..=120)
        .map(|s: i64| Sample {
            labels: series(&[("__name__", "latency"), ("job", "a")]),
            ts_ms: s * 1000,
            value: 1.0,
        })
        .collect();
    ingest_all(arm.as_mut(), &chunks(samples, 60_000));
    let footprint = arm.footprint().unwrap();
    assert!(
        footprint.raw_retained_bytes == 0 && footprint.view_bytes > 0,
        "{footprint:?}"
    );
    println!("stored-only footprint {footprint:?}");

    let deployment = translate(&PlanFile::from_value(plan).unwrap()).unwrap();
    let url = server(1).url;
    let http = |sql: &str| {
        ureq::post(&format!("{url}/"))
            .query("database", "conformance_ttl")
            .send_string(sql)
            .map(|r| r.into_string().unwrap())
    };
    ureq::post(&format!("{url}/"))
        .send_string("CREATE DATABASE IF NOT EXISTS conformance_ttl")
        .unwrap();
    for statement in deployment.tables.iter().chain(&deployment.ttl_statements()) {
        http(statement).unwrap_or_else(|e| panic!("{statement}: {e}"));
    }
    let created = http("SELECT create_table_query FROM system.tables WHERE database = 'conformance_ttl' AND name = 'v1'").unwrap();
    assert!(created.contains("TTL"), "{created}");
    ureq::post(&format!("{url}/"))
        .send_string("DROP DATABASE conformance_ttl SYNC")
        .unwrap();
}

/// The harness kit on an engine that names the planner-written probe plans
/// (`sum_over_time(probe[10s])`, `count_over_time(probe[10s])`): answers
/// are deterministic and windows are left-open.
#[test]
#[ignore = "needs a ClickHouse server"]
fn the_harness_conformance_kit_passes() {
    struct WithProbePlans(ClickHouseEngine);
    impl EngineUnderTest for WithProbePlans {
        fn description(&self) -> EngineDescription {
            self.0.description()
        }
        fn cpu_accounting(&self) -> CpuAccounting {
            self.0.cpu_accounting()
        }
        fn install(
            &self,
            workload: &WorkloadSpec,
            arm: &ArmSpec,
        ) -> Result<Box<dyn InstalledArm>, EngineError> {
            let plan = match workload.name.as_str() {
                "probe-sum" => "probe_sum_10s",
                "probe-count" => "probe_count_10s",
                other => return Err(EngineError(format!("no probe plan for {other}"))),
            };
            let mut arm = arm.clone();
            arm.engine_options = json!({ "plan_file": fixture_path(plan) });
            self.0.install(workload, &arm)
        }
    }
    let mut engine = WithProbePlans(engine(1));
    let clock = aqpbm_core::harness::protocol::OsCpuClock;
    let clocks = aqpbm_core::harness::protocol::MeasurementClocks::with_calling_thread(&clock);
    let report = aqpbm_core::harness::conformance::check(&mut engine, &clocks);
    println!("harness kit: {report:?}");
    let passed = report.expect("the kit passes").passed;
    assert!(passed.contains(&aqpbm_core::harness::conformance::ConformanceCheck::DeterministicAnswers));
    assert!(passed.contains(&aqpbm_core::harness::conformance::ConformanceCheck::LeftOpenWindow));
}

/// Suite size: 1,000 series in 10 jobs, 1 s samples for 10 min from
/// 1,700,000,000,000 ms, q4c at 1 m evaluated every 10 s. Every chunk is one
/// insert of 10,000 rows; at the last evaluation each job's answer equals
/// the sum, computed here, of its samples in the last minute. The server
/// cgroup's CPU grows over the ingest and is never throttled.
#[test]
#[ignore = "needs a ClickHouse server"]
fn suite_sized_workload() {
    let engine = engine(1);
    let start_ms: i64 = 1_700_000_000_000;
    let record: DataRecord = serde_json::from_value(json!({
        "metric": "latency",
        "description": {
            "labels": [{"name": "job", "cardinality": 10}, {"name": "instance", "cardinality": 100}],
            "series_num": 1000, "start_ms": start_ms, "sample_interval_ms": 1000,
            "duration_ms": 600000,
            "value_distribution": {"kind": "uniform", "lower_bound": 0.0, "upper_bound": 10.0, "seed": 0},
            "counter": false, "seed": 7
        }
    }))
    .unwrap();
    let plan = fixture_value_public(
        "q4c_sum_over_time__1m_1hz_10s",
        "clickhouse.equivalent-only",
    );
    let mut spec = workload(&plan, Some(10_000));
    spec.data = vec![record.clone()];
    let mut arm = engine
        .install(&spec, &arm(&plan_file("suite", &plan)))
        .expect("installs");
    let CpuAccounting::Cgroup(source) = engine.cpu_accounting() else {
        panic!("cgroup accounting");
    };
    let cpu_before = source.cpu().expect("the agent answers");
    let mut last_minute: BTreeMap<String, f64> = BTreeMap::new();
    let end_ms = start_ms + 600_000;
    let ingest_started = std::time::Instant::now();
    let mut rows_per_chunk = Vec::new();
    for chunk in workload_chunks(&[record], 10_000).unwrap() {
        let chunk = chunk.unwrap();
        rows_per_chunk.push(chunk.samples.len());
        for sample in &chunk.samples {
            if sample.ts_ms > end_ms - 60_000 {
                *last_minute.entry(sample.labels["job"].clone()).or_default() += sample.value;
            }
        }
        arm.ingest(&chunk).unwrap();
        if chunk.end_ms >= start_ms + 60_000 {
            arm.evaluate(chunk.end_ms).unwrap();
        }
    }
    let ingest_wall = ingest_started.elapsed();
    let cpu_after = source.cpu().unwrap();
    let answers = arm.evaluate(end_ms).unwrap().0;
    let got = rows(&answers, 0);
    let expected: Vec<(String, f64)> = last_minute
        .iter()
        .map(|(job, sum)| (format!("job={job}"), *sum))
        .collect();
    let expected: Vec<(&str, f64)> = expected.iter().map(|(k, v)| (k.as_str(), *v)).collect();
    assert_rows(&got, &expected);
    assert!(
        rows_per_chunk.iter().all(|&n| n == 10_000),
        "{rows_per_chunk:?}"
    );
    assert!(cpu_after.usage_ns > cpu_before.usage_ns);
    assert_eq!(cpu_after.nr_throttled, cpu_before.nr_throttled);
    println!(
        "suite size: 60 chunks of 10,000 rows, 55 evaluations; wall {:.1} s; server cgroup CPU {:.3} s",
        ingest_wall.as_secs_f64(),
        (cpu_after.usage_ns - cpu_before.usage_ns) as f64 / 1e9
    );
    // The idle server's CPU over 10 s.
    let idle_before = source.cpu().unwrap();
    std::thread::sleep(Duration::from_secs(10));
    let idle_after = source.cpu().unwrap();
    println!(
        "idle server cgroup CPU over 10 s: {:.1} ms",
        (idle_after.usage_ns - idle_before.usage_ns) as f64 / 1e6
    );
}

/// The lane end to end: the probe workloads on lane `fake` and lane
/// `clickhouse` over one run directory, reported with the fake as truth:
/// every point is there, and none differs.
#[test]
#[ignore = "needs a ClickHouse server"]
fn the_clickhouse_lane_matches_the_fake_lane() {
    use aqpbm_core::harness::bench_external::{report, run_lane, LaneSetup};
    use aqpbm_core::harness::fake::FakeEngine;
    use aqpbm_core::harness::protocol::{
        load_probe_for, MeasurementClocks, OsCpuClock, ProcessCpuFiles, Protocol,
    };
    use aqpbm_core::harness::rundir::RunDir;

    let root = std::env::temp_dir().join(format!("ch-lane-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let run_dir = RunDir::create_at(&root).unwrap();
    let write = |path: PathBuf, text: String| {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    };
    for workload in aqpbm_core::harness::conformance::probe_workloads() {
        let plan = match workload.name.as_str() {
            "probe-sum" => "probe_sum_10s",
            _ => "probe_count_10s",
        };
        write(
            run_dir
                .path()
                .join(format!("workloads/{}.json", workload.name)),
            serde_json::to_string(&workload).unwrap(),
        );
        write(
            run_dir
                .path()
                .join(format!("plans/equivalent-only/{}.plan.json", workload.name)),
            std::fs::read_to_string(fixture_path(plan)).unwrap(),
        );
    }
    write(
        run_dir.path().join("plans/equivalent-only/variant.json"),
        r#"{"substitutions_allowed": false}"#.into(),
    );
    let protocol = Protocol {
        warmup_runs: 1,
        measured_runs: 3,
        max_core_busy_fraction: 1.0,
    };
    let clock = OsCpuClock;
    let clocks = MeasurementClocks::with_calling_thread(&clock);
    let fake = FakeEngine::default();
    let clickhouse = engine(1);
    for lane in [&fake as &dyn EngineUnderTest, &clickhouse] {
        let mut probe = load_probe_for(&lane.cpu_accounting(), &ProcessCpuFiles::os()).unwrap();
        let cells = run_lane(
            &run_dir,
            lane,
            None,
            LaneSetup {
                protocol: &protocol,
                core: 0,
                probe: probe.as_mut(),
                clocks: &clocks,
                args: vec![],
            },
        )
        .unwrap();
        println!("{} cells: {cells:?}", lane.description().id);
    }
    let report = report(&run_dir, "fake", "equivalent-only").unwrap();
    println!("{}", report.summary_markdown());
    let csv = report.summary_csv();
    let clickhouse_rows: Vec<&str> = csv.lines().filter(|l| l.contains(",clickhouse,")).collect();
    assert_eq!(clickhouse_rows.len(), 2, "{csv}");
    let accuracy: Vec<_> = report
        .accuracy
        .iter()
        .filter(|a| a.lane == "clickhouse")
        .collect();
    assert_eq!(accuracy.len(), 2, "{:?}", report.accuracy);
    for query in accuracy {
        let errors = &query.errors;
        assert!(errors.points > 0, "{query:?}");
        assert_eq!(errors.missing_points, 0, "{query:?}");
        assert_eq!(errors.extra_points, 0, "{query:?}");
        assert!(errors.max_rel_error.unwrap_or(0.0) <= 1e-9, "{query:?}");
    }
    let _ = std::fs::remove_dir_all(&root);
}
