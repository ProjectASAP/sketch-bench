//! Conformance of the VictoriaMetrics adapter against a real server, one
//! case per row of `capabilities/README.md` § VictoriaMetrics (`#[ignore]`:
//! set `AQPBM_VICTORIAMETRICS_URL` to the server's HTTP interface and
//! `AQPBM_CGROUP_AGENT_URL` to its agent, e.g. a server started with
//! `scripts/extbench-up-victoriametrics.sh`). Each case installs a
//! planner-written plan (changed in test code where a shape needs it)
//! through the engine, feeds hand-made samples in real time and compares the
//! answers with hand-computed ones: (a) rows to 1e-9 relative, (b) rows by
//! the stated difference. The cases run in real time (about a minute each)
//! and share the server: run with `--test-threads 1`.

mod common;

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use aqpbm_engines_external::victoriametrics_server::{
    VictoriaMetricsEngine, VictoriaMetricsServer,
};
use aqpbm_core::harness::data::Sample;
use aqpbm_core::harness::engine::{
    Answers, ArmSpec, EngineUnderTest, InstalledArm, QueryLanguage, QueryText, SampleChunk,
    SeriesLabels, WorkloadQuery, WorkloadSpec,
};
use common::{fixture_value_public, payload_body};
use serde_json::{json, Value};

const EQ: &str = "victoriametrics.equivalent-only";
const WS: &str = "victoriametrics.with-substitutes";

fn engine() -> VictoriaMetricsEngine {
    VictoriaMetricsEngine::new(VictoriaMetricsServer {
        url: std::env::var("AQPBM_VICTORIAMETRICS_URL").expect("AQPBM_VICTORIAMETRICS_URL"),
        cgroup_agent_url: std::env::var("AQPBM_CGROUP_AGENT_URL").expect("AQPBM_CGROUP_AGENT_URL"),
        request_timeout: Duration::from_secs(60),
    })
    .expect("the server answers")
}

/// `plan` written to a temporary file, as an arm's `plan_file`.
fn arm(name: &str, plan: &Value) -> ArmSpec {
    let dir = std::env::temp_dir().join(format!("vm-conformance-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path: PathBuf = dir.join(format!("{name}.plan.json"));
    std::fs::write(&path, plan.to_string()).unwrap();
    ArmSpec {
        name: "conformance".into(),
        raw_data_retained: true,
        engine_options: json!({ "plan_file": path }),
    }
}

/// A workload of `plan`'s query texts (ids `q0`, `q1`, …), evaluated every
/// `interval_ms`, with data from 0 ms supplied by the test.
fn workload(plan: &Value, interval_ms: u64) -> WorkloadSpec {
    let texts: Vec<String> = serde_json::from_value(plan["query_texts"].clone()).unwrap();
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

/// A series with `pairs` and a `conformance_run` label unique to this
/// process, so a rerun never reads an earlier run's samples of the series
/// (MetricsQL looks up to 10 minutes back for the sample before a range).
fn series(pairs: &[(&str, &str)]) -> Arc<SeriesLabels> {
    let mut labels: SeriesLabels = pairs
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
    labels.insert("conformance_run".into(), RUN.with(|run| run.clone()));
    Arc::new(labels)
}

thread_local! {
    static RUN: String = format!(
        "{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs()
    );
}

/// `samples` cut into chunks ending at multiples of `chunk_ms` from
/// `chunk_ms` to `end_ms` (empty chunks included), time-major.
fn chunks(mut samples: Vec<Sample>, chunk_ms: i64, end_ms: i64) -> Vec<SampleChunk> {
    samples.sort_by_key(|s| s.ts_ms);
    (1..=end_ms / chunk_ms)
        .map(|k| SampleChunk {
            end_ms: k * chunk_ms,
            samples: samples
                .iter()
                .filter(|s| s.ts_ms > (k - 1) * chunk_ms && s.ts_ms <= k * chunk_ms)
                .cloned()
                .collect(),
        })
        .collect()
}

/// Feeds every chunk in real time and evaluates after each chunk ending at
/// a multiple of `interval_ms` from `first_eval_ms`; the answers per time.
fn run(
    arm: &mut dyn InstalledArm,
    chunks: &[SampleChunk],
    first_eval_ms: i64,
    interval_ms: i64,
) -> Vec<(i64, Answers)> {
    let mut out = Vec::new();
    for chunk in chunks {
        arm.ingest(chunk)
            .unwrap_or_else(|e| panic!("ingest {}: {e}", chunk.end_ms));
        let t = chunk.end_ms;
        if t >= first_eval_ms && (t - first_eval_ms) % interval_ms == 0 {
            let (answers, _) = arm
                .evaluate(t)
                .unwrap_or_else(|e| panic!("evaluate {t}: {e}"));
            out.push((t, answers));
        }
    }
    out
}

/// Asserts `answers` of query `q<index>` equal `expected` to 1e-9 relative,
/// series by series; NaN equals NaN.
/// Answers are keyed as PromQL keys them: only a bare selector (query
/// `q{INSTANT_QUERY}` of the query-time case) keeps `__name__`.
fn assert_answers(t: i64, answers: &Answers, index: usize, expected: &BTreeMap<SeriesLabels, f64>) {
    let id = format!("q{index}");
    let got = answers.0.get(&id).cloned().unwrap_or_default();
    let keeps_name = got.keys().any(|labels| labels.contains_key("__name__"));
    let expected: BTreeMap<SeriesLabels, f64> = expected
        .iter()
        .map(|(labels, &v)| {
            let mut labels = labels.clone();
            if !keeps_name {
                labels.remove("__name__");
            }
            (labels, v)
        })
        .collect();
    let expected = &expected;
    let keys = |m: &BTreeMap<SeriesLabels, f64>| m.keys().cloned().collect::<Vec<_>>();
    assert_eq!(keys(&got), keys(expected), "{id} at {t}: series");
    for (labels, &want) in expected {
        let have = got[labels];
        let close = have == want
            || (have.is_nan() && want.is_nan())
            || (have - want).abs() <= 1e-9 * want.abs().max(1.0);
        assert!(close, "{id} at {t}, {labels:?}: {have} != {want}");
    }
}

/// One series' points: (data time in ms, value).
type Points = Vec<(i64, f64)>;

/// The samples of each series.
struct Data(Vec<(Arc<SeriesLabels>, Points)>);

impl Data {
    fn samples(&self) -> Vec<Sample> {
        self.0
            .iter()
            .flat_map(|(labels, points)| {
                points.iter().map(|&(ts_ms, value)| Sample {
                    labels: Arc::clone(labels),
                    ts_ms,
                    value,
                })
            })
            .collect()
    }

    /// Per series, the values in `(t − range, t]`.
    fn windows(&self, t: i64, range_ms: i64) -> Vec<(&Arc<SeriesLabels>, Vec<f64>)> {
        self.0
            .iter()
            .map(|(labels, points)| {
                let values = points
                    .iter()
                    .filter(|(ts, _)| *ts > t - range_ms && *ts <= t)
                    .map(|&(_, v)| v)
                    .collect();
                (labels, values)
            })
            .filter(|(_, values): &(_, Vec<f64>)| !values.is_empty())
            .collect()
    }
}

fn group_key(labels: &SeriesLabels, by: &[&str]) -> SeriesLabels {
    by.iter()
        .filter_map(|l| Some((l.to_string(), labels.get(*l)?.clone())))
        .collect()
}

/// PromQL `quantile` over `values`: linear interpolation at rank q·(n − 1).
fn quantile(q: f64, values: &[f64]) -> f64 {
    let mut sorted = values.to_vec();
    sorted.sort_by(f64::total_cmp);
    let rank = q * (sorted.len() - 1) as f64;
    let (lower, upper) = (rank.floor() as usize, rank.ceil() as usize);
    sorted[lower] + (sorted[upper] - sorted[lower]) * (rank - lower as f64)
}

/// Appends a stored-chain read to `plan` (a copy of the chain, range, merge
/// and finalize nodes 1–4 of `sot__1h__1hz__60s`) with `kind`, `width_ms`,
/// `range_ms` and `reduction`; returns the read's root node.
fn add_chain_read(
    plan: &mut Value,
    kind: &str,
    width_ms: u64,
    range_ms: u64,
    reduction: Value,
) -> u64 {
    let template = fixture_value_public("sot__1h__1hz__60s", EQ);
    let base = plan["dag"]["nodes"].as_array().unwrap().len() as u64;
    for (offset, id) in (1..=4).enumerate() {
        let mut node = template["dag"]["nodes"][id].clone();
        node["id"] = json!(base + offset as u64);
        plan["dag"]["nodes"].as_array_mut().unwrap().push(node);
    }
    let (chain, range, merge, finalize) = (
        base as usize,
        base as usize + 1,
        base as usize + 2,
        base as usize + 3,
    );
    payload_body(plan, chain)["summary_type"] = json!({"ExactAggregate": [kind, kind]});
    payload_body(plan, chain)["tumbling_window_width_ms"] = json!(width_ms);
    payload_body(plan, chain)["reduction"] = reduction;
    payload_body(plan, range)["child"] = json!(chain);
    payload_body(plan, range)["range"] = json!({"secs": range_ms / 1000, "nanos": 0});
    payload_body(plan, merge)["children"] = json!([range]);
    payload_body(plan, finalize)["child"] = json!(merge);
    let edge = template["dag"]["edges"][0].clone();
    for (producer, consumer) in [
        (0, chain),
        (chain, range),
        (range, merge),
        (merge, finalize),
    ] {
        let mut e = edge.clone();
        e["producer"] = json!(producer);
        e["consumer"] = json!(consumer);
        plan["dag"]["edges"].as_array_mut().unwrap().push(e);
    }
    finalize as u64
}

/// `plans` merged into one plan file: node ids shifted, roots and query
/// texts concatenated.
fn merged(plans: &[Value]) -> Value {
    let mut out = plans[0].clone();
    for plan in &plans[1..] {
        let shift = out["dag"]["nodes"].as_array().unwrap().len() as u64;
        let shifted = |v: &Value| json!(v.as_u64().unwrap() + shift);
        for node in plan["dag"]["nodes"].as_array().unwrap() {
            let mut node = node.clone();
            node["id"] = shifted(&node["id"]);
            let body = node["payload"]
                .as_object_mut()
                .unwrap()
                .values_mut()
                .next()
                .unwrap()
                .as_object_mut()
                .unwrap()
                .values_mut()
                .next()
                .unwrap();
            for field in ["child", "lhs", "rhs", "summary_input"] {
                if let Some(v) = body.get(field).cloned() {
                    body[field] = shifted(&v);
                }
            }
            if let Some(children) = body.get("children").cloned() {
                body["children"] = children.as_array().unwrap().iter().map(shifted).collect();
            }
            out["dag"]["nodes"].as_array_mut().unwrap().push(node);
        }
        for edge in plan["dag"]["edges"].as_array().unwrap() {
            let mut edge = edge.clone();
            edge["producer"] = shifted(&edge["producer"]);
            edge["consumer"] = shifted(&edge["consumer"]);
            out["dag"]["edges"].as_array_mut().unwrap().push(edge);
        }
        for root in plan["dag"]["roots"].as_array().unwrap() {
            out["dag"]["roots"]
                .as_array_mut()
                .unwrap()
                .push(shifted(root));
        }
        for text in plan["query_texts"].as_array().unwrap() {
            out["query_texts"]
                .as_array_mut()
                .unwrap()
                .push(text.clone());
        }
    }
    out
}

/// Three latency series at 1 Hz over (0, 80 s]; `i3` stops at 40 s, a window
/// end, so its later windows are empty.
fn latency_data() -> Data {
    let points = |base: f64, last_s: i64| -> Vec<(i64, f64)> {
        (1..=last_s)
            .map(|s| (s * 1000, base * ((s * 7) % 11) as f64 - 3.0))
            .collect()
    };
    Data(vec![
        (
            series(&[("__name__", "latency"), ("job", "a"), ("instance", "i1")]),
            points(1.0, 80),
        ),
        (
            series(&[("__name__", "latency"), ("job", "a"), ("instance", "i2")]),
            points(2.5, 80),
        ),
        (
            series(&[("__name__", "latency"), ("job", "b"), ("instance", "i3")]),
            points(-1.0, 40),
        ),
    ])
}

/// R1–R5: stored Sum / Count / Min / Max per series (no `by`), a grouped
/// chain (`by: [job]`), chains grouped by no label (per-series form plus the
/// outer aggregate), two stream intervals in one install, windows that end
/// exactly on samples, and a series whose later windows are empty.
#[test]
#[ignore = "needs a VictoriaMetrics v1.154.1 server (AQPBM_VICTORIAMETRICS_URL, AQPBM_CGROUP_AGENT_URL)"]
fn stored_windows_equal_the_exact_windows() {
    let mut plan = fixture_value_public("sot__1h__1hz__60s", EQ);
    plan["dag"]["roots"] = json!([]);
    plan["query_texts"] = json!([]);
    let reads = [
        ("Sum", 10_000, 30_000, json!("PerEntity")),
        ("Count", 10_000, 30_000, json!("PerEntity")),
        ("Min", 10_000, 30_000, json!("PerEntity")),
        ("Max", 20_000, 40_000, json!("PerEntity")),
        ("Sum", 10_000, 30_000, json!({"Reduce": [2]})),
        ("Sum", 10_000, 30_000, json!({"Reduce": []})),
        ("Max", 20_000, 40_000, json!({"Reduce": []})),
    ];
    for (i, (kind, width, range, reduction)) in reads.iter().enumerate() {
        let root = add_chain_read(&mut plan, kind, *width, *range, reduction.clone());
        plan["dag"]["roots"]
            .as_array_mut()
            .unwrap()
            .push(json!(root));
        plan["query_texts"]
            .as_array_mut()
            .unwrap()
            .push(json!(format!("read {i}")));
    }
    let engine = engine();
    let mut installed = engine
        .install(&workload(&plan, 20_000), &arm("stored", &plan))
        .unwrap();
    let text = installed.plan_summary().text;
    assert!(
        !text.contains("by:") || text.contains("by: ['job']"),
        "{text}"
    );
    let data = latency_data();
    let answers = run(
        installed.as_mut(),
        &chunks(data.samples(), 20_000, 80_000),
        40_000,
        20_000,
    );
    assert_eq!(answers.len(), 3);
    for (_, got) in &answers {
        for series in got.0.values() {
            assert!(series.keys().all(|l| !l.contains_key("__name__")));
        }
    }
    for (t, got) in &answers {
        for (i, (kind, _, range, reduction)) in reads.iter().enumerate() {
            let fold = |values: &[f64]| -> f64 {
                match *kind {
                    "Sum" => values.iter().sum(),
                    "Count" => values.len() as f64,
                    "Min" => values.iter().copied().fold(f64::INFINITY, f64::min),
                    _ => values.iter().copied().fold(f64::NEG_INFINITY, f64::max),
                }
            };
            let windows = data.windows(*t, *range as i64);
            let mut expected = BTreeMap::new();
            match reduction {
                Value::String(_) => {
                    for (labels, values) in &windows {
                        expected.insert((***labels).clone(), fold(values));
                    }
                }
                _ if reduction["Reduce"] == json!([2]) => {
                    let mut groups: BTreeMap<SeriesLabels, Vec<f64>> = BTreeMap::new();
                    for (labels, values) in &windows {
                        groups
                            .entry(group_key(labels, &["job"]))
                            .or_default()
                            .extend(values);
                    }
                    for (key, values) in groups {
                        expected.insert(key, fold(&values));
                    }
                }
                _ => {
                    let all: Vec<f64> = windows.iter().flat_map(|(_, v)| v.clone()).collect();
                    if !all.is_empty() {
                        expected.insert(SeriesLabels::new(), fold(&all));
                    }
                }
            }
            assert_answers(*t, got, i, &expected);
        }
    }
    // At 80 s with 30 s ranges, `i3` (stopped at 40 s) has no answer.
    let (_, last) = answers.last().unwrap();
    assert!(last.0["q0"].keys().all(|l| l["instance"] != "i3"));
    println!(
        "R1-R5 stored windows: {} evaluations x {} queries equal",
        answers.len(),
        reads.len()
    );
}

/// Builds a one-node-more plan from `base`: `node` (a copy of node `like`
/// with `payload`) read by a new root.
fn with_root(mut plan: Value, like: usize, payload: Value, inputs: &[usize]) -> Value {
    let id = plan["dag"]["nodes"].as_array().unwrap().len();
    let mut node = plan["dag"]["nodes"][like].clone();
    node["id"] = json!(id);
    node["payload"] = payload;
    plan["dag"]["nodes"].as_array_mut().unwrap().push(node);
    let edge = plan["dag"]["edges"][0].clone();
    for &input in inputs {
        let mut e = edge.clone();
        e["producer"] = json!(input);
        e["consumer"] = json!(id);
        plan["dag"]["edges"].as_array_mut().unwrap().push(e);
    }
    plan["dag"]["roots"] = json!([id]);
    plan
}

/// Requests counters at 1 Hz over (0, 80 s]: `r1` has a full reset to 0 at
/// 25 s and a partial drop smaller than prev/8 at 52 s; `r2` grows steadily.
fn counter_data() -> Data {
    let r1: Vec<(i64, f64)> = (1..=80)
        .map(|s| {
            let v = match s {
                ..=24 => 100.0 + 3.0 * s as f64,
                25..=51 => 2.0 * (s - 25) as f64,
                52 => 2.0 * 26.0 - 4.0, // 48 after 52 at 51: a drop of 4 < 52/8
                _ => 48.0 + 5.0 * (s - 52) as f64,
            };
            (s * 1000, v)
        })
        .collect();
    let r2: Vec<(i64, f64)> = (1..=80).map(|s| (s * 1000, 10.0 * s as f64)).collect();
    Data(vec![
        (
            series(&[
                ("__name__", "requests_total"),
                ("job", "a"),
                ("instance", "r1"),
            ]),
            r1,
        ),
        (
            series(&[
                ("__name__", "requests_total"),
                ("job", "a"),
                ("instance", "r2"),
            ]),
            r2,
        ),
    ])
}

/// VictoriaMetrics `increase` and `rate` over `(t − range, t]` of one series
/// (`rollupDelta`, `rollupDerivFast`, `removeCounterResets` at
/// v1.154.1): values corrected for resets (a drop smaller than prev/8 adds
/// prev − v, a larger one adds prev); with a sample at or before
/// `t − range`, both run from it to the last sample in the range. With
/// none, `rate` runs from the first sample in the range (none from one
/// sample), and `increase` from 0 when the first value is under 10 × (the
/// next step + 1), else from the first value.
fn vm_increase_rate(points: &[(i64, f64)], t: i64, range_ms: i64) -> Option<(f64, f64)> {
    let mut corrected = Vec::new();
    let (mut correction, mut prev) = (0.0, points.first()?.1);
    for &(ts, v) in points.iter().filter(|(ts, _)| *ts <= t) {
        let d = v - prev;
        if d < 0.0 {
            correction += if -d * 8.0 < prev { prev - v } else { prev };
        }
        prev = v;
        corrected.push((ts, v + correction));
    }
    let previous = corrected
        .iter()
        .copied()
        .rfind(|(ts, _)| *ts <= t - range_ms);
    let inside: Vec<(i64, f64)> = corrected
        .iter()
        .copied()
        .filter(|(ts, _)| *ts > t - range_ms)
        .collect();
    let &(last_ts, last) = inside.last()?;
    match previous {
        Some((prev_ts, prev_value)) => Some((
            last - prev_value,
            (last - prev_value) / ((last_ts - prev_ts) as f64 / 1000.0),
        )),
        None => {
            let (first_ts, first) = inside[0];
            if inside.len() < 2 {
                return None;
            }
            let step = inside[1].1 - first;
            let increase = if first.abs() < 10.0 * (step.abs() + 1.0) {
                last
            } else {
                last - first
            };
            Some((
                increase,
                (last - first) / ((last_ts - first_ts) as f64 / 1000.0),
            ))
        }
    }
}

/// R6–R7 and the query-time rows: per-series and grouped rollups, quantiles,
/// avg, instant selector, topk, a division whose 0/0 series is dropped, and
/// rate/increase by VictoriaMetrics' formula (with-substitutes plans).
#[test]
#[ignore = "needs a VictoriaMetrics v1.154.1 server (AQPBM_VICTORIAMETRICS_URL, AQPBM_CGROUP_AGENT_URL)"]
fn query_time_rows_equal_the_hand_computed_answers() {
    let q4b = fixture_value_public("q4b_max__1m_1hz_10s", EQ);
    let max_body = q4b["dag"]["nodes"][2]["payload"].clone();
    let measure = |kind: Value| {
        let mut p = max_body.clone();
        p["NonASAP"]["Aggregate"]["measures"] = json!([kind]);
        p
    };
    let avg = with_root(
        q4b.clone(),
        2,
        measure(json!({"kind": "avg", "col": null})),
        &[1],
    );
    let mut instant = q4b.clone();
    payload_body(&mut instant, 1)["kind"] = json!("instant");
    payload_body(&mut instant, 1)["range"] = json!({"secs": 0, "nanos": 5_000_000});
    instant["dag"]["roots"] = json!([1]);
    let topk = with_root(
        q4b.clone(),
        2,
        json!({"NonASAP": {"Aggregate": {"child": 2, "measures": [{"kind": "top_k", "k": 2}],
            "reduction": {"Reduce": []}, "filters": [], "having": null}}}),
        &[2],
    );
    let q1 = fixture_value_public("q1_quantile__1m_1hz_10s", EQ);
    let q50 = with_root(
        q1.clone(),
        2,
        {
            let mut p = q1["dag"]["nodes"][2]["payload"].clone();
            p["NonASAP"]["Aggregate"]["measures"] =
                json!([{"kind": "quantile", "col": null, "q": 0.5}]);
            p
        },
        &[1],
    );
    let ratio = with_root(
        q50,
        2,
        json!({"NonASAP": {"BinaryOp": {"lhs": 2, "rhs": 3, "return_bool": false,
            "operator": {"checked_relative_division": false, "checked_finite_division": false,
                         "kind": {"Arithmetic": "Div"}, "vector_match": null}}}}),
        &[2, 3],
    );
    let parts = [
        fixture_value_public("q4c_sum_over_time__1m_1hz_10s", EQ),
        q1,
        fixture_value_public("q3_avg_quantile__1m_1hz_10s", EQ),
        fixture_value_public("q4_count__1m_1hz_10s", EQ),
        q4b,
        avg,
        instant,
        topk,
        ratio,
        fixture_value_public("q2_sum_rate__1m_1hz_10s", WS),
    ];
    let mut plan = merged(&parts);
    // A rate per series and an increase per series of the same counters.
    let q2 = fixture_value_public("q2_sum_rate__1m_1hz_10s", WS);
    let mut increase = q2.clone();
    payload_body(&mut increase, 2)["summary_type"] =
        json!({"ExactAggregate": ["Increase", "Increase"]});
    increase["dag"]["roots"] = json!([3]);
    let mut rate = q2;
    rate["dag"]["roots"] = json!([3]);
    plan = merged(&[plan, rate, increase]);
    let texts: Vec<String> = (0..plan["dag"]["roots"].as_array().unwrap().len())
        .map(|i| format!("query {i}"))
        .collect();
    plan["query_texts"] = json!(texts);

    let engine = engine();
    let mut installed = engine
        .install(&workload(&plan, 10_000), &arm("query-time", &plan))
        .unwrap();
    let mut data = latency_data();
    // A series of zeros: its 0.9/0.5 quantile ratio is 0/0.
    data.0.push((
        series(&[("__name__", "latency"), ("job", "b"), ("instance", "z0")]),
        (1..=80).map(|s| (s * 1000, 0.0)).collect(),
    ));
    let counters = counter_data();
    let mut samples = data.samples();
    samples.extend(counters.samples());
    let answers = run(
        installed.as_mut(),
        &chunks(samples, 10_000, 80_000),
        60_000,
        10_000,
    );
    assert_eq!(answers.len(), 3);
    let per_series = |t: i64, f: &dyn Fn(&[f64]) -> Option<f64>| -> BTreeMap<SeriesLabels, f64> {
        data.windows(t, 60_000)
            .into_iter()
            .filter_map(|(labels, values)| Some(((**labels).clone(), f(&values)?)))
            .collect()
    };
    let grouped = |t: i64, f: &dyn Fn(&[f64]) -> f64, outer: &dyn Fn(&[f64]) -> f64| {
        let mut groups: BTreeMap<SeriesLabels, Vec<f64>> = BTreeMap::new();
        for (labels, values) in data.windows(t, 60_000) {
            groups
                .entry(group_key(labels, &["job"]))
                .or_default()
                .push(f(&values));
        }
        groups
            .into_iter()
            .map(|(k, v)| (k, outer(&v)))
            .collect::<BTreeMap<_, _>>()
    };
    let sum = |v: &[f64]| v.iter().sum::<f64>();
    let max = |v: &[f64]| v.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    for (t, got) in &answers {
        let t = *t;
        assert_answers(t, got, 0, &grouped(t, &sum, &sum));
        assert_answers(t, got, 1, &per_series(t, &|v| Some(quantile(0.99, v))));
        assert_answers(
            t,
            got,
            2,
            &grouped(t, &|v| quantile(0.9, v), &|v| sum(v) / v.len() as f64),
        );
        assert_answers(t, got, 3, &grouped(t, &|v| v.len() as f64, &sum));
        assert_answers(t, got, 4, &per_series(t, &|v| Some(max(v))));
        assert_answers(
            t,
            got,
            5,
            &per_series(t, &|v| Some(sum(v) / v.len() as f64)),
        );
        // The instant selector: each series' sample at exactly t.
        let at_t: BTreeMap<SeriesLabels, f64> = data
            .0
            .iter()
            .filter_map(|(labels, points)| {
                let &(_, v) = points.iter().find(|(ts, _)| *ts == t)?;
                Some(((**labels).clone(), v))
            })
            .collect();
        assert_answers(t, got, 6, &at_t);
        // Only the bare selector keeps `__name__`.
        assert!(got.0["q6"].keys().all(|l| l.contains_key("__name__")));
        for other in ["q1", "q4", "q5", "q7", "q8", "q10", "q11"] {
            assert!(
                got.0[other].keys().all(|l| !l.contains_key("__name__")),
                "{other}"
            );
        }
        let mut maxes: Vec<(SeriesLabels, f64)> =
            per_series(t, &|v| Some(max(v))).into_iter().collect();
        maxes.sort_by(|a, b| b.1.total_cmp(&a.1));
        assert_answers(t, got, 7, &maxes.into_iter().take(2).collect());
        // 0/0 is NaN: VictoriaMetrics drops that series (the recorded substitution).
        assert_answers(
            t,
            got,
            8,
            &per_series(t, &|v| {
                let r = quantile(0.99, v) / quantile(0.5, v);
                (!r.is_nan()).then_some(r)
            }),
        );
        let counter = |f: &dyn Fn((f64, f64)) -> f64| -> BTreeMap<SeriesLabels, f64> {
            counters
                .0
                .iter()
                .filter_map(|(labels, points)| {
                    Some(((**labels).clone(), f(vm_increase_rate(points, t, 60_000)?)))
                })
                .collect()
        };
        let rates = counter(&|(_, rate)| rate);
        let mut by_job: BTreeMap<SeriesLabels, f64> = BTreeMap::new();
        for (labels, rate) in &rates {
            *by_job.entry(group_key(labels, &["job"])).or_default() += rate;
        }
        assert_answers(t, got, 9, &by_job);
        assert_answers(t, got, 10, &rates);
        assert_answers(t, got, 11, &counter(&|(increase, _)| increase));
    }
    println!(
        "R6-R7 query-time rows: {} evaluations x 12 queries equal",
        answers.len()
    );
}

/// R8: with `-search.latencyOffset=0`, a point written at wall time t is
/// visible to `/api/v1/query?time=t` at once.
#[test]
#[ignore = "needs a VictoriaMetrics v1.154.1 server (AQPBM_VICTORIAMETRICS_URL, AQPBM_CGROUP_AGENT_URL)"]
fn a_point_is_visible_at_its_own_time() {
    let url = std::env::var("AQPBM_VICTORIAMETRICS_URL").unwrap();
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64;
    let line = format!(
        "{{\"metric\":{{\"__name__\":\"latency_probe\",\"run\":\"{now_ms}\"}},\"values\":[42],\"timestamps\":[{now_ms}]}}\n"
    );
    ureq::post(&format!("{url}/api/v1/import"))
        .send_string(&line)
        .unwrap();
    ureq::get(&format!("{url}/internal/force_flush"))
        .call()
        .unwrap();
    let time = format!("{}.{:03}", now_ms / 1000, now_ms % 1000);
    let body = ureq::get(&format!("{url}/api/v1/query"))
        .query(
            "query",
            &format!("last_over_time(latency_probe{{run=\"{now_ms}\"}}[1s])"),
        )
        .query("time", &time)
        .query("nocache", "1")
        .call()
        .unwrap()
        .into_string()
        .unwrap();
    assert!(body.contains("\"42\""), "{body}");
}

/// R9: a chunk sent after its window flushed fails the step (pacing).
#[test]
#[ignore = "needs a VictoriaMetrics v1.154.1 server (AQPBM_VICTORIAMETRICS_URL, AQPBM_CGROUP_AGENT_URL)"]
fn a_late_chunk_fails_the_step() {
    let mut plan = fixture_value_public("sot__1h__1hz__60s", EQ);
    plan["dag"]["roots"] = json!([]);
    plan["query_texts"] = json!(["late"]);
    let root = add_chain_read(&mut plan, "Sum", 10_000, 10_000, json!("PerEntity"));
    plan["dag"]["roots"] = json!([root]);
    let engine = engine();
    let mut installed = engine
        .install(&workload(&plan, 10_000), &arm("late", &plan))
        .unwrap();
    let data = latency_data();
    let all = chunks(data.samples(), 10_000, 20_000);
    // Wait past the first window's end (its slices are due before it).
    std::thread::sleep(Duration::from_secs(16));
    let error = installed.ingest(&all[0]).expect_err("a late chunk fails");
    assert!(error.0.contains("pacing"), "{error}");
}
