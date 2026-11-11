use std::collections::BTreeSet;

use aqpbm_planeval::plan::{plan, Arm};
use aqpbm_planeval::run::Driver;
use aqpbm_usecase::data::generate;
use aqpbm_usecase::spec::Spec;

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
