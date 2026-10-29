use std::collections::BTreeSet;

use aqpbm_inprocess::data::generate;
use aqpbm_inprocess::plan::{plan, Arm};
use aqpbm_inprocess::run::Driver;
use aqpbm_inprocess::spec::Spec;

fn small_spec() -> Spec {
    let spec: Spec = serde_json::from_value(serde_json::json!({
        "use_case": "test",
        "metric": "src_ip",
        "labels": {"vm": "instance1"},
        "start_ms": 1767225600000i64,
        "duration_ms": 60000,
        "sample_interval_ms": 10,
        "max_distinct_per_evaluation": 1000,
        "seed": 1,
        "baseline": {"kind": "zipf", "skewness": 1.1, "population_size": 200, "seed": 2},
        "attack": {"kind": "uniform", "lower_bound": 1000000, "upper_bound": 4294967296u64, "seed": 3},
        "attacks": [{"start_offset_ms": 20000, "end_offset_ms": 40000, "ramp_ms": 2000, "attack_share": 0.5}],
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
    let series = generate(&spec).unwrap();
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
        let distinct: BTreeSet<u64> = series
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
    let series = generate(&spec).unwrap();
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
        assert_eq!(
            Some(&evaluation.answers),
            evaluation.reference_answers.as_ref(),
            "at {t}"
        );
    }
    assert!(refreshed > 0);
}
