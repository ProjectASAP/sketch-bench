use std::collections::BTreeSet;
use std::path::Path;

use aqpbm_planeval::accuracy::{Labels, QueryTruth};
use aqpbm_planeval::data::{generate, Series};
use aqpbm_planeval::plan::{plan, workload, Arm};
use aqpbm_planeval::run::Driver;
use aqpbm_planeval::spec::Spec;

fn small_spec() -> Spec {
    let spec: Spec = serde_json::from_value(serde_json::json!({
        "use_case": "test",
        "start_ms": 1767225600000i64,
        "duration_ms": 60000,
        "seed": 1,
        "metrics": [{
            "name": "src_ip",
            "sample_interval_ms": 10,
            "max_distinct_per_evaluation": 1000,
            "floor": true,
            "series": [{
                "labels": {"vm": "instance1"},
                "baseline": {"kind": "zipf", "skewness": 1.1, "population_size": 200, "seed": 2},
                "phases": [{"start_offset_ms": 20000, "end_offset_ms": 40000, "ramp_ms": 2000,
                    "change": {"mix": {"share": 0.5, "distribution":
                        {"kind": "uniform", "lower_bound": 1000000, "upper_bound": 4294967296u64, "seed": 3}}}}]
            }]
        }],
        "queries": [
            {"id": "distinct_10s", "promql": "distinct_over_time(src_ip{vm=\"instance1\"}[10s])",
             "window_ms": 10000, "interval_ms": 5000, "threshold": 300, "unit": "addresses"}
        ]
    }))
    .unwrap();
    spec.check().unwrap();
    spec
}

#[test]
fn evaluation_times_start_one_window_in_and_stay_on_the_grid() {
    let spec = small_spec();
    let times = spec.evaluation_times(&spec.queries[0]);
    assert_eq!(times.first(), Some(&(spec.start_ms + 10000)));
    assert_eq!(times.last(), Some(&spec.end_ms()));
    assert!(times.windows(2).all(|w| w[1] - w[0] == 5000));
}

#[tokio::test(flavor = "current_thread")]
async fn exact_distinct_count_equals_the_distinct_values_in_each_window() {
    let spec = small_spec();
    let series = generate(&spec, std::path::Path::new(".")).unwrap();
    let arm = Arm {
        max_error: None,
        failure_probability: 0.01,
        raw_data_retained: true,
        ignore_guarantees: false,
    };
    let planned = plan(&spec, &spec.queries[0], &arm).await.unwrap();
    let mut driver = Driver::new(planned.plan, false).unwrap();
    for t in spec.evaluation_times(&spec.queries[0]) {
        let answers = driver.evaluate(&series, t).unwrap().answers;
        assert_eq!(answers.len(), 1);
        let distinct: BTreeSet<u64> = series[0]
            .between(t - 10000, t)
            .iter()
            .map(|&(_, v)| v as u64)
            .collect();
        assert_eq!(answers[0].1, distinct.len() as f64, "at {t}");
    }
}

#[tokio::test(flavor = "current_thread")]
async fn panes_built_at_ingestion_answer_as_the_query_time_reference() {
    let spec = small_spec();
    let series = generate(&spec, std::path::Path::new(".")).unwrap();
    let arm = Arm {
        max_error: Some(0.05),
        failure_probability: 0.01,
        raw_data_retained: false,
        ignore_guarantees: false,
    };
    let planned = plan(&spec, &spec.queries[0], &arm).await.unwrap();
    assert!(!planned.plan.materialized_views().unwrap().is_empty());
    let mut driver = Driver::new(planned.plan, true).unwrap();
    let mut refreshed = 0;
    for t in spec.evaluation_times(&spec.queries[0]) {
        let evaluation = driver.evaluate(&series, t).unwrap();
        refreshed += evaluation.views_refreshed;
        assert_eq!(evaluation.cost.samples_read_at_query, 0, "at {t}");
        assert!(evaluation.cost.state_bytes > 0, "at {t}");
        assert_eq!(
            evaluation.cost.samples_ingested > 0,
            evaluation.views_refreshed > 0,
            "at {t}"
        );
        assert_eq!(
            Some(&evaluation.answers),
            evaluation.reference_answers.as_ref(),
            "at {t}"
        );
    }
    assert!(refreshed > 0);
}

#[test]
fn the_use_case_1_spec_draws_the_samples_it_drew_before_specs_had_many_series() {
    let spec = Spec::read(Path::new("workloads/uc1_syn_flood.json")).unwrap();
    let series = generate(&spec, std::path::Path::new(".")).unwrap();
    assert_eq!(series.len(), 1);
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for &(t, v) in &series[0].points {
        for byte in t.to_le_bytes().into_iter().chain(v.to_bits().to_le_bytes()) {
            hash ^= byte as u64;
            hash = hash.wrapping_mul(0x100_0000_01b3);
        }
    }
    assert_eq!(series[0].points.len(), 11_340_033);
    assert_eq!(hash, 0x0ab5_805d_18d8_d702);
}

#[test]
fn a_quantile_answer_is_scored_by_rank_within_its_window_and_series() {
    let labels = |k: &str| -> Labels {
        [("__name__", "m"), ("k", k)]
            .iter()
            .map(|(a, b)| (a.to_string(), b.to_string()))
            .collect()
    };
    let series = vec![
        Series {
            labels: labels("a"),
            points: vec![(1, 5.0), (2, 1.0), (3, 3.0), (4, 3.0), (5, 2.0)],
        },
        Series {
            labels: labels("b"),
            points: vec![(2, 10.0), (4, 20.0)],
        },
    ];
    let mut spec = small_spec();
    spec.metrics[0].name = "m".into();
    spec.queries[0].promql = "quantile_over_time(0.5, m[4ms])".into();
    let arm = Arm {
        max_error: None,
        failure_probability: 0.01,
        raw_data_retained: true,
        ignore_guarantees: false,
    };
    let truth = QueryTruth::of(&workload(&spec, &spec.queries[0], &arm).unwrap())
        .unwrap()
        .expect("a quantile over a plain selector has a computed truth");
    let cells = truth
        .score(&series, &[5], &[(5, labels("a"), 1.0)])
        .unwrap();
    assert_eq!(cells.len(), 2);
    let (_, a, scores) = &cells[0];
    assert_eq!(a.get("k").map(String::as_str), Some("a"));
    assert_eq!(scores.as_ref().unwrap()["rank_err_q0.5"], 0.25);
    assert!(cells[1].2.is_none());
}

#[test]
fn a_rate_phase_writes_more_samples_while_it_holds_and_none_change_elsewhere() {
    let mut spec = small_spec();
    let baseline = generate(&spec, std::path::Path::new(".")).unwrap()[0].points.clone();
    spec.metrics[0].series[0].phases = serde_json::from_value(serde_json::json!([
        {"start_offset_ms": 20000, "end_offset_ms": 30000, "ramp_ms": 0, "change": {"rate": {"factor": 4}}}
    ]))
    .unwrap();
    spec.check().unwrap();
    let points = generate(&spec, std::path::Path::new(".")).unwrap()[0].points.clone();
    let count = |points: &[(i64, f64)], from: i64, to: i64| {
        points
            .iter()
            .filter(|(t, _)| *t > spec.start_ms + from && *t <= spec.start_ms + to)
            .count()
    };
    assert_eq!(count(&baseline, 0, 20000), 2000);
    assert_eq!(count(&points, 0, 20000), 2000);
    assert_eq!(count(&points, 20000, 30000), 4000);
    assert_eq!(count(&points, 30010, 60000), 2999);
    assert!(points.windows(2).all(|w| w[0].0 <= w[1].0));
}

#[test]
fn a_rate_phase_needs_a_positive_factor() {
    let mut spec = small_spec();
    spec.metrics[0].series[0].phases = serde_json::from_value(serde_json::json!([
        {"start_offset_ms": 20000, "end_offset_ms": 30000, "ramp_ms": 0, "change": {"rate": {"factor": 0}}}
    ]))
    .unwrap();
    assert!(spec.check().is_err());
}
