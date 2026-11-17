//! V1–V12: translating planner-written plans into VictoriaMetrics stream
//! aggregation and one MetricsQL query per query root. The goldens under
//! `tests/golden/victoriametrics/` were written by hand before the
//! translator.

mod common;

use aqpbm_engines_external::plan_file::PlanFile;
use aqpbm_engines_external::victoriametrics::{
    translate, MetricsQlQuery, StreamAggregation, StreamOutput, VictoriaMetricsDeployment,
};
use aqpbm_engines_external::{SharingLost, TranslationError};
use common::{fixture_value_public, payload_body, public_fixture};
use serde_json::{json, Value};

const EQ: &str = "victoriametrics.equivalent-only";
const WS: &str = "victoriametrics.with-substitutes";

fn translated(workload: &str, variant: &str) -> VictoriaMetricsDeployment {
    translate(&public_fixture(workload, variant)).expect("translates")
}

fn from_value(value: Value) -> Result<VictoriaMetricsDeployment, TranslationError> {
    translate(&PlanFile::from_value(value).expect("decodes"))
}

fn shape(node: usize, reason: &str) -> TranslationError {
    TranslationError::UnsupportedShape {
        node,
        reason: reason.into(),
    }
}

fn golden(name: &str) -> String {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/golden/victoriametrics")
        .join(name);
    std::fs::read_to_string(path).expect("golden")
}

/// The golden's queries: one per non-comment line.
fn golden_queries(name: &str) -> Vec<String> {
    golden(name)
        .lines()
        .filter(|line| !line.trim().is_empty() && !line.starts_with('#'))
        .map(str::to_string)
        .collect()
}

fn exprs(deployment: &VictoriaMetricsDeployment) -> Vec<String> {
    deployment.queries.iter().map(|q| q.expr.clone()).collect()
}

/// `sot__1h__1hz__60s` (equivalent-only) with the chain's reduction set to `reduction`.
fn sot_with_reduction(reduction: Value) -> Value {
    let mut value = fixture_value_public("sot__1h__1hz__60s", EQ);
    payload_body(&mut value, 1)["reduction"] = reduction;
    value
}

/// `sot__1h__1hz__60s` with the chain's family set to the exact `kind`.
fn sot_with_family(value: &mut Value, kind: &str) {
    payload_body(value, 1)["summary_type"] = json!({"ExactAggregate": [kind, kind]});
}

#[test]
fn stored_sum_per_series_has_no_by() {
    let deployment = translated("sot__1h__1hz__60s", EQ);
    assert_eq!(
        deployment.stream_aggregation_config(),
        golden("sot_1h.yaml")
    );
    assert!(!deployment.stream_aggregation_config().contains("by:"));
    assert_eq!(
        deployment.stream_aggregations,
        vec![StreamAggregation {
            metric: "latency".into(),
            selector: "latency".into(),
            interval_ms: 60_000,
            by: None,
            outputs: vec![StreamOutput::SumSamples, StreamOutput::CountSamples],
        }]
    );
    assert_eq!(
        deployment.queries,
        vec![MetricsQlQuery {
            expr: golden_queries("sot_1h.metricsql")[0].clone(),
            time_step_ms: 60_000,
            per_series_of: None,
        }]
    );
    assert!(deployment.substitutions.is_empty());
    assert!(deployment.sharing_lost.is_empty());
}

#[test]
fn stored_max_with_label_filter() {
    let deployment = translated("s1000__maxf__5m__1hz__60s", WS);
    assert_eq!(
        deployment.stream_aggregation_config(),
        golden("maxf_5m.yaml")
    );
    assert_eq!(
        deployment.queries,
        vec![MetricsQlQuery {
            expr: golden_queries("maxf_5m.metricsql")[0].clone(),
            time_step_ms: 60_000,
            per_series_of: Some("latency".into()),
        }]
    );
    assert!(deployment.substitutions.is_empty());

    // A label value with a quote, a backslash and a single quote is escaped
    // in the selector, and the YAML single-quoted string doubles the quote.
    let mut value = fixture_value_public("s1000__maxf__5m__1hz__60s", WS);
    payload_body(&mut value, 0)["predicates"][0]["Compare"]["right"] =
        json!({"Literal": {"Utf8": "j\"3\\'"}});
    let deployment = from_value(value).unwrap();
    assert_eq!(
        deployment.stream_aggregations[0].selector,
        r#"latency{job="j\"3\\'"}"#
    );
    assert!(
        deployment
            .stream_aggregation_config()
            .contains(r#"- match: 'latency{job="j\"3\\''"}'"#),
        "{}",
        deployment.stream_aggregation_config()
    );
}

#[test]
fn nested_counts_over_raw_samples() {
    let deployment = translated("cdist__5m__1hz__300s", EQ);
    assert!(deployment.stream_aggregations.is_empty());
    assert_eq!(exprs(&deployment), golden_queries("cdist_5m.metricsql"));
    assert_eq!(deployment.queries[0].time_step_ms, 1);
    assert_eq!(deployment.queries[0].per_series_of, None);
}

#[test]
fn grouped_chain_uses_by() {
    let deployment = from_value(sot_with_reduction(json!({"Reduce": [2]}))).unwrap();
    let config = deployment.stream_aggregation_config();
    assert!(config.contains("  by: [job]\n"), "{config}");
    assert_eq!(
        exprs(&deployment),
        vec![
            "sum by (job) (sum_over_time(latency:60s_by_job_sum_samples[3600s] offset -29500ms) \
             and count_over_time(latency:60s_by_job_count_samples[3600s] offset -29500ms))"
        ]
    );
    assert_eq!(
        deployment.stream_aggregations[0].by,
        Some(vec!["job".into()])
    );

    // Grouping by the series identity is per series, which `by` cannot say.
    assert_eq!(
        from_value(sot_with_reduction(json!({"Reduce": [3]}))).unwrap_err(),
        shape(1, "group by the series identity")
    );
    // `without` is refused on the node that carries it.
    assert_eq!(
        from_value(sot_with_reduction(json!({"Reduce": {"without": [2]}}))).unwrap_err(),
        shape(1, "without")
    );
}

#[test]
fn empty_reduce_chain_is_per_series_plus_outer_aggregate() {
    for (kind, output, expr) in [
        (
            "Sum",
            "sum_samples, count_samples",
            "sum(sum_over_time(latency:60s_sum_samples[3600s] offset -29500ms) \
             and count_over_time(latency:60s_count_samples[3600s] offset -29500ms))",
        ),
        (
            "Count",
            "count_samples",
            "sum(sum_over_time(latency:60s_count_samples[3600s] offset -29500ms))",
        ),
        (
            "Min",
            "min",
            "min(min_over_time(latency:60s_min[3600s] offset -29500ms))",
        ),
        (
            "Max",
            "max",
            "max(max_over_time(latency:60s_max[3600s] offset -29500ms))",
        ),
    ] {
        let mut value = sot_with_reduction(json!({"Reduce": []}));
        sot_with_family(&mut value, kind);
        // The root reads the chain's single group directly: drop the outer
        // `sum by (job)` so the query is the chain read itself.
        value["dag"]["roots"] = json!([4]);
        let deployment = from_value(value).unwrap();
        let config = deployment.stream_aggregation_config();
        assert!(!config.contains("by:"), "{kind}: {config}");
        assert!(config.contains(&format!("outputs: [{output}]")), "{config}");
        assert_eq!(exprs(&deployment), vec![expr.to_string()], "{kind}");
        assert_eq!(deployment.queries[0].per_series_of, None);
    }
}

#[test]
fn query_time_shapes() {
    let expected = golden_queries("showcase_eq.metricsql");
    let per_series = [None, Some("latency"), None, None, Some("latency")];
    for (i, workload) in [
        "q4c_sum_over_time__1m_1hz_10s",
        "q1_quantile__1m_1hz_10s",
        "q3_avg_quantile__1m_1hz_10s",
        "q4_count__1m_1hz_10s",
        "q4b_max__1m_1hz_10s",
    ]
    .into_iter()
    .enumerate()
    {
        let deployment = translated(workload, EQ);
        assert!(deployment.stream_aggregations.is_empty(), "{workload}");
        assert!(deployment.substitutions.is_empty(), "{workload}");
        assert_eq!(
            deployment.queries,
            vec![MetricsQlQuery {
                expr: expected[i].clone(),
                time_step_ms: 1,
                per_series_of: per_series[i].map(str::to_string),
            }],
            "{workload}"
        );
        assert_eq!(deployment.stream_aggregation_config(), "[]\n");
    }
}

#[test]
fn rate_is_a_recorded_substitute() {
    let deployment = translated("q2_sum_rate__1m_1hz_10s", WS);
    assert_eq!(
        exprs(&deployment),
        vec!["sum by (job) (rate(requests_total[60s]))"]
    );
    assert_eq!(deployment.substitutions.len(), 1);
    let substitution = &deployment.substitutions[0];
    assert_eq!(substitution.node, 2);
    assert_eq!(
        substitution.rule,
        "victoriametrics.rate_increase.no_extrapolation"
    );
    for phrase in [
        "no extrapolation",
        "last sample before the range",
        "from 0",
        "prev/8",
        "one sample",
    ] {
        assert!(
            substitution.guarantee_difference.contains(phrase),
            "{phrase}: {}",
            substitution.guarantee_difference
        );
    }

    // Increase is the same rule.
    let mut value = fixture_value_public("q2_sum_rate__1m_1hz_10s", WS);
    payload_body(&mut value, 2)["summary_type"] =
        json!({"ExactAggregate": ["Increase", "Increase"]});
    assert_eq!(
        exprs(&from_value(value).unwrap()),
        vec!["sum by (job) (increase(requests_total[60s]))"]
    );
}

#[test]
fn shared_query_time_node_is_reported() {
    let plan = public_fixture("q5_joint__1m_1hz_10s", WS);
    let deployment = translate(&plan).unwrap();
    assert_eq!(
        exprs(&deployment),
        vec![
            "quantile_over_time(0.99, latency[60s])",
            "quantile_over_time(0.5, latency[60s])",
            "avg by (job) (quantile_over_time(0.9, latency[60s]))",
            "max_over_time(latency[60s])",
            "sum by (job) (rate(requests_total[60s]))",
        ]
    );
    assert_eq!(
        deployment.sharing_lost,
        vec![SharingLost {
            node: 1,
            readers: vec![2, 3, 4]
        }]
    );
    assert_eq!(deployment.substitutions.len(), 1);
}

#[test]
fn refusals() {
    let maxf = || fixture_value_public("s1000__maxf__5m__1hz__60s", WS);
    let q4c = || fixture_value_public("q4c_sum_over_time__1m_1hz_10s", EQ);

    let mut v = q4c();
    payload_body(&mut v, 0)["source"] = json!({"Table": {"table_ref": "t"}});
    assert_eq!(from_value(v).unwrap_err(), shape(0, "table source"));

    let mut v = maxf();
    payload_body(&mut v, 0)["predicates"][0]["Compare"]["semantics"] = json!("sql");
    assert_eq!(from_value(v).unwrap_err(), shape(0, "predicate"));

    let mut v = maxf();
    payload_body(&mut v, 1)["filter"] = json!({"Literal": {"Bool": true}});
    assert_eq!(from_value(v).unwrap_err(), shape(1, "filter"));

    let mut v = maxf();
    payload_body(&mut v, 1)["input"] = json!({"item": {"Column": 2}, "weight": null});
    assert_eq!(from_value(v).unwrap_err(), shape(1, "summary input"));

    let mut v = maxf();
    payload_body(&mut v, 1)["summary_type"] = json!({"Sketch": "Kll"});
    assert_eq!(from_value(v).unwrap_err(), shape(1, "sketch Kll"));

    let mut v = maxf();
    payload_body(&mut v, 1)["summary_type"] = json!({"ExactAggregate": ["Rate", "Rate"]});
    assert_eq!(
        from_value(v).unwrap_err(),
        shape(1, "stored counter window chain")
    );

    let mut v = maxf();
    payload_body(&mut v, 2)["range"] = json!({"secs": 90, "nanos": 0});
    assert_eq!(
        from_value(v).unwrap_err(),
        shape(2, "a range that is not a whole number of windows")
    );

    let mut v = q4c();
    v["dag"]["nodes"][2]["payload"] = json!({"ASAP": {"FancyNewOp": {}}});
    assert_eq!(
        from_value(v).unwrap_err(),
        shape(2, "operator ASAP.FancyNewOp")
    );

    let mut v = q4c();
    payload_body(&mut v, 2)["reduction"] = json!({"Reduce": {"without": [2]}});
    assert_eq!(from_value(v).unwrap_err(), shape(2, "without"));

    // A label name MetricsQL cannot write.
    let mut v = q4c();
    v["dag"]["nodes"][0]["output_schema"]["fields"][2]["name"] = json!("a-b");
    v["dag"]["nodes"][1]["output_schema"]["fields"][2]["name"] = json!("a-b");
    v["dag"]["nodes"][2]["output_schema"]["fields"][0]["name"] = json!("a-b");
    assert_eq!(from_value(v).unwrap_err(), shape(2, "label name a-b"));
}

#[test]
fn aggregate_and_binary_refusals() {
    let q4 = || fixture_value_public("q4_count__1m_1hz_10s", EQ);
    let mut v = q4();
    payload_body(&mut v, 3)["having"] = json!({"Literal": {"Bool": true}});
    assert_eq!(from_value(v).unwrap_err(), shape(3, "having"));

    let mut v = q4();
    payload_body(&mut v, 3)["filters"] = json!([{"Literal": {"Bool": true}}]);
    assert_eq!(from_value(v).unwrap_err(), shape(3, "filter"));

    let mut v = q4();
    payload_body(&mut v, 3)["measures"] = json!([{"kind": "stddev", "col": null}]);
    assert_eq!(from_value(v).unwrap_err(), shape(3, "measure stddev"));

    // A grouped aggregate straight over raw samples has no MetricsQL form.
    let mut v = q4();
    payload_body(&mut v, 2)["reduction"] = json!({"Reduce": [2]});
    assert_eq!(
        from_value(v).unwrap_err(),
        shape(2, "grouped aggregate over raw samples")
    );

    let rangeheavy = || fixture_value_public("rangeheavy", WS);
    let binary = rangeheavy()["dag"]["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .position(|n| n["payload"]["NonASAP"].get("BinaryOp").is_some())
        .expect("rangeheavy has a BinaryOp");
    let mut v = rangeheavy();
    payload_body(&mut v, binary)["operator"]["vector_match"] = json!({"card": "OneToOne"});
    assert_eq!(from_value(v).unwrap_err(), shape(binary, "vector matching"));
    let mut v = rangeheavy();
    payload_body(&mut v, binary)["return_bool"] = json!(true);
    assert_eq!(from_value(v).unwrap_err(), shape(binary, "bool comparison"));
    let mut v = rangeheavy();
    payload_body(&mut v, binary)["operator"]["checked_relative_division"] = json!(true);
    assert_eq!(
        from_value(v).unwrap_err(),
        shape(binary, "checked relative division")
    );
}

#[test]
fn instant_selector_needs_a_lookback() {
    let value = fixture_value_public("rangeheavy", WS);
    let plan = PlanFile::from_value(value.clone()).unwrap();
    let instant = plan
        .nodes
        .iter()
        .position(|n| {
            matches!(
                n.operator,
                aqpbm_engines_external::plan_file::PlanOperator::TimeRange {
                    kind: aqpbm_engines_external::plan_file::RangeKind::Instant,
                    ..
                }
            )
        })
        .expect("rangeheavy has an instant selector");
    let mut v = value.clone();
    payload_body(&mut v, instant)["range"] = json!({"secs": 0, "nanos": 0});
    assert_eq!(
        from_value(v).unwrap_err(),
        shape(instant, "instant selector without a lookback")
    );
    let mut v = value;
    payload_body(&mut v, instant)["range"] = json!({"secs": 5, "nanos": 0});
    let deployment = from_value(v).unwrap();
    assert!(
        exprs(&deployment)
            .iter()
            .any(|e| e.contains("last_over_time(data[5s])")),
        "{:?}",
        exprs(&deployment)
    );
}

#[test]
fn interval_must_divide_a_day() {
    for (width_ms, ok) in [
        (60_000u64, true),
        (300_000, true),
        (3_600_000, true),
        (2_000, true),
        (7_000, false),
        (1_500, false),
        (1_000, false),
    ] {
        let mut value = fixture_value_public("s1000__maxf__5m__1hz__60s", WS);
        payload_body(&mut value, 1)["tumbling_window_width_ms"] = json!(width_ms);
        // Keep the range a whole number of windows.
        let range_ms = width_ms * 5;
        payload_body(&mut value, 2)["range"] =
            json!({"secs": range_ms / 1000, "nanos": (range_ms % 1000) * 1_000_000});
        let result = from_value(value);
        if ok {
            assert!(result.is_ok(), "{width_ms}: {result:?}");
        } else {
            let reason = match width_ms {
                7_000 => "window width 7000 ms does not divide 86,400 s",
                1_500 => "window width 1500 ms is not whole seconds",
                _ => "window width 1000 ms is below 2 s",
            };
            assert_eq!(result.unwrap_err(), shape(1, reason), "{width_ms}");
        }
    }
}

#[test]
fn recording_rules_mirror_stream_outputs() {
    let deployment = translated("sot__1h__1hz__60s", EQ);
    assert_eq!(
        deployment.recording_rules(),
        "groups:\n\
         \x20 - name: stream-aggregation-60s\n\
         \x20   interval: 60s\n\
         \x20   eval_alignment: true\n\
         \x20   eval_delay: 0s\n\
         \x20   rules:\n\
         \x20     - record: latency:60s_sum_samples\n\
         \x20       expr: 'sum_over_time(latency[60s])'\n\
         \x20     - record: latency:60s_count_samples\n\
         \x20       expr: 'count_over_time(latency[60s])'\n"
    );
    let deployment = from_value(sot_with_reduction(json!({"Reduce": [2]}))).unwrap();
    assert!(deployment
        .recording_rules()
        .contains("- record: latency:60s_by_job_sum_samples\n        expr: 'sum by (job) (sum_over_time(latency[60s]))'\n"));
    let mut value = sot_with_reduction(json!({"Reduce": [2]}));
    sot_with_family(&mut value, "Count");
    assert!(from_value(value)
        .unwrap()
        .recording_rules()
        .contains("expr: 'sum by (job) (count_over_time(latency[60s]))'"));
}

#[test]
fn outputs_merge_per_aggregation() {
    // A second chain, Max, over the same Scan and width, read by a second root.
    let mut value = fixture_value_public("sot__1h__1hz__60s", EQ);
    let nodes = value["dag"]["nodes"].as_array_mut().unwrap();
    let copies: Vec<Value> = (1..=4).map(|id| nodes[id].clone()).collect();
    for (offset, mut node) in copies.into_iter().enumerate() {
        node["id"] = json!(6 + offset);
        nodes.push(node);
    }
    payload_body(&mut value, 6)["summary_type"] = json!({"ExactAggregate": ["Max", "Max"]});
    payload_body(&mut value, 7)["child"] = json!(6);
    payload_body(&mut value, 8)["children"] = json!([7]);
    payload_body(&mut value, 9)["child"] = json!(8);
    let edges = value["dag"]["edges"].as_array_mut().unwrap();
    let template = edges[0].clone();
    for (producer, consumer) in [(0, 6), (6, 7), (7, 8), (8, 9)] {
        let mut edge = template.clone();
        edge["producer"] = json!(producer);
        edge["consumer"] = json!(consumer);
        edges.push(edge);
    }
    value["dag"]["roots"] = json!([5, 9]);
    value["query_texts"] = json!(["q0", "q1"]);
    let deployment = from_value(value.clone()).unwrap();
    assert_eq!(deployment.stream_aggregations.len(), 1);
    assert_eq!(
        deployment.stream_aggregations[0].outputs,
        vec![
            StreamOutput::SumSamples,
            StreamOutput::CountSamples,
            StreamOutput::Max
        ]
    );
    assert!(deployment
        .stream_aggregation_config()
        .contains("outputs: [sum_samples, count_samples, max]"));
    assert_eq!(
        exprs(&deployment)[1],
        "max_over_time(latency:60s_max[3600s] offset -29500ms)"
    );

    // The same output twice (two roots reading one chain) is one output.
    payload_body(&mut value, 6)["summary_type"] = json!({"ExactAggregate": ["Sum", "Sum"]});
    let deployment = from_value(value).unwrap();
    assert_eq!(
        deployment.stream_aggregations[0].outputs,
        vec![StreamOutput::SumSamples, StreamOutput::CountSamples]
    );
}

#[test]
fn stored_count_reads_count_samples() {
    let mut value = fixture_value_public("sot__1h__1hz__60s", EQ);
    sot_with_family(&mut value, "Count");
    let deployment = from_value(value).unwrap();
    assert!(deployment
        .stream_aggregation_config()
        .contains("outputs: [count_samples]\n"));
    assert_eq!(exprs(&deployment), golden_queries("sot_1h_count.metricsql"));
}

#[test]
fn chain_reads_other_than_merge_and_finalize_are_refused() {
    // A regrouping merge of stored windows.
    let mut value = fixture_value_public("sot__1h__1hz__60s", EQ);
    payload_body(&mut value, 3)["group_by"] = json!([2]);
    assert_eq!(
        from_value(value).unwrap_err(),
        shape(3, "regrouping merge of stored windows")
    );
    // An instant selector over a chain.
    let mut value = fixture_value_public("sot__1h__1hz__60s", EQ);
    payload_body(&mut value, 2)["kind"] = json!("instant");
    value["dag"]["roots"] = json!([2]);
    assert_eq!(
        from_value(value).unwrap_err(),
        shape(2, "time range over this operator")
    );
}

#[test]
fn grouped_shapes_of_a_mixed_workload() {
    let deployment = translated("mixed-04", WS);
    let exprs = exprs(&deployment);
    // Root 4: top 4 per (region, service) of the per-series increase.
    assert_eq!(
        exprs[7],
        "topk by (region, service) (4, increase(data_total[600s]))"
    );
    assert_eq!(
        deployment.queries[7].per_series_of.as_deref(),
        Some("data_total")
    );
    // Root 16: avg as sum over count of the regrouped per-series quantiles,
    // a checked division.
    assert_eq!(
        exprs[2],
        "(sum by (region) (sum by (region, service) (quantile_over_time(0.75, data[100s])))) \
         / (sum by (region) (count by (region, service) (quantile_over_time(0.75, data[100s]))))"
    );
    let division = deployment
        .substitutions
        .iter()
        .find(|s| s.node == 15)
        .expect("the checked division is recorded");
    assert_eq!(division.rule, "victoriametrics.checked_division.plain");
    // Root 33: a 10 s per-series Sum chain regrouped at query time.
    assert_eq!(
        exprs[0],
        "sum by (job, region, service) (sum_over_time(data:10s_sum_samples[600s] offset -4500ms) \
         and count_over_time(data:10s_count_samples[600s] offset -4500ms))"
    );
}

#[test]
fn project_keeps_labels_under_their_names() {
    let value = fixture_value_public("mixed-04", WS);
    // Node 16 aliases the value column to `avg`: allowed.
    assert!(from_value(value.clone()).is_ok());
    // Renaming a label is refused.
    let mut v = value;
    v["dag"]["nodes"][16]["output_schema"]["fields"][0]["name"] = json!("area");
    assert_eq!(
        from_value(v).unwrap_err(),
        shape(16, "projected label rename")
    );
}
