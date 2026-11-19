//! Every committed bench-external suite: each workload parses as a harness
//! `WorkloadSpec`, has a schedule, and lists the same queries, in the same
//! order, as the ASAPPlanner planning workload beside it (the planner
//! numbers them q1..qN in that order, and lanes map plan roots to queries
//! by that index).

use std::path::Path;

use aqpbm_core::harness::arm_run::ArmSchedule;
use aqpbm_core::harness::engine::{QueryLanguage, WorkloadSpec};

#[test]
fn suite_workloads_match_their_planning_workloads() {
    let suites = Path::new(env!("CARGO_MANIFEST_DIR")).join("../suites");
    let mut checked = 0;
    for suite in std::fs::read_dir(&suites).unwrap() {
        let suite = suite.unwrap().path();
        if !suite.is_dir() {
            continue;
        }
        let mut names: Vec<_> = std::fs::read_dir(&suite)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .filter_map(|file| file.strip_suffix(".workload.json").map(str::to_string))
            .collect();
        names.sort();
        for name in names {
            let read = |suffix: &str| {
                std::fs::read_to_string(suite.join(format!("{name}.{suffix}.json")))
                    .unwrap_or_else(|error| panic!("{name}.{suffix}.json: {error}"))
            };
            let spec: WorkloadSpec = serde_json::from_str(&read("workload")).unwrap();
            assert_eq!(spec.name, name);
            let planner: serde_json::Value = serde_json::from_str(&read("planner")).unwrap();
            let planned: Vec<&str> = planner["query_workload"]["repeating_queries"]
                .as_array()
                .unwrap()
                .iter()
                .map(|query| query["query"].as_str().unwrap())
                .collect();
            let specified: Vec<&str> = spec
                .queries
                .iter()
                .map(|query| {
                    query
                        .texts
                        .iter()
                        .find(|text| text.language == QueryLanguage::PromQl)
                        .unwrap()
                        .text
                        .as_str()
                })
                .collect();
            assert_eq!(specified, planned, "{name}");
            let ids: Vec<String> = (1..=spec.queries.len()).map(|i| format!("q{i}")).collect();
            let spec_ids: Vec<String> = spec.queries.iter().map(|q| q.id.clone()).collect();
            assert_eq!(spec_ids, ids, "{name}");
            let schedule =
                ArmSchedule::for_workload(&spec).unwrap_or_else(|error| panic!("{name}: {error}"));
            for ((query, planned), text) in spec
                .queries
                .iter()
                .zip(
                    planner["query_workload"]["repeating_queries"]
                        .as_array()
                        .unwrap(),
                )
                .zip(&specified)
            {
                let at = &planned["demand"]["fixed_interval_at"];
                let interval = at["interval"].as_u64().unwrap();
                assert_eq!(
                    query.evaluation_interval_ms, interval,
                    "{name} {}",
                    query.id
                );
                // Evaluations land on the planner's phase.
                let phase = at["evaluation_phase"].as_i64().unwrap();
                assert_eq!(
                    schedule.evaluate_at_ms[0].rem_euclid(interval as i64),
                    phase.rem_euclid(interval as i64),
                    "{name} {}",
                    query.id
                );
                // Every range in the text is the query's window.
                let ranges = range_ms(text);
                assert!(!ranges.is_empty(), "{name}: no range in {text}");
                assert!(
                    ranges.iter().all(|&ms| ms == query.window_ms),
                    "{name}: {text} vs {}",
                    query.window_ms
                );
                // Grouping labels are labels the data has.
                for label in grouping_labels(text) {
                    assert!(
                        spec.data.iter().all(|record| record
                            .description
                            .labels
                            .iter()
                            .any(|spec| spec.name == label)),
                        "{name}: {text} groups by {label}, which the data lacks"
                    );
                }
                // The metrics it reads have data.
                for record_metric in metrics_read(text, &spec) {
                    assert!(
                        spec.data.iter().any(|r| r.metric == record_metric),
                        "{name}: {record_metric}"
                    );
                }
            }
            // The data is what the planner was told: per source, the
            // series count, sample interval, counter kind, distinct label
            // values and value sign; and no source without data.
            let sources = planner["data_workload"]["sources"].as_object().unwrap();
            let rows_per_second: f64 = spec
                .data
                .iter()
                .map(|r| {
                    r.description.series_num as f64 * 1_000.0
                        / r.description.sample_interval_ms as f64
                })
                .sum();
            assert_eq!(
                planner["data_workload"]["arrival"]["ingesting"]["rows_per_second"].as_f64(),
                Some(rows_per_second),
                "{name}: rows_per_second"
            );
            assert_eq!(
                sources.len(),
                spec.data.len(),
                "{name}: sources vs data records"
            );
            for record in &spec.data {
                let facts = &sources[&record.metric];
                let description = &record.description;
                assert_eq!(
                    description.series_num,
                    facts["series_count"].as_u64().unwrap(),
                    "{name}"
                );
                assert_eq!(
                    description.sample_interval_ms,
                    facts["sample_interval"].as_u64().unwrap(),
                    "{name}"
                );
                assert_eq!(
                    description.counter,
                    facts["metric_type"] == "counter",
                    "{name} {}",
                    record.metric
                );
                let product: u64 = description
                    .labels
                    .iter()
                    .map(|label| label.cardinality)
                    .product();
                assert_eq!(
                    product, description.series_num,
                    "{name}: label sets vs series"
                );
                for label in &description.labels {
                    assert_eq!(
                        facts["distinct_values_per_label"][&label.name].as_u64(),
                        Some(label.cardinality),
                        "{name} {} {}",
                        record.metric,
                        label.name
                    );
                }
                if facts["column_signs"]["value"] == "non_negative" {
                    let lower = serde_json::to_value(&description.value_distribution).unwrap()
                        ["lower_bound"]
                        .as_f64()
                        .unwrap();
                    assert!(lower >= 0.0, "{name} {}: negative values", record.metric);
                }
            }
            checked += 1;
        }
    }
    assert!(checked >= 8, "only {checked} suite workloads");
}

/// Every `[<n>m]` / `[<n>s]` range of a PromQL text, in ms.
fn range_ms(text: &str) -> Vec<u64> {
    text.split('[')
        .skip(1)
        .filter_map(|rest| rest.split_once(']'))
        .map(|(range, _)| {
            let (number, unit) = range.split_at(range.len() - 1);
            let number: u64 = number.parse().unwrap_or_else(|_| panic!("range {range}"));
            match unit {
                "s" => number * 1_000,
                "m" => number * 60_000,
                "h" => number * 3_600_000,
                _ => panic!("range {range}"),
            }
        })
        .collect()
}

/// The labels of every `by (...)` / `without (...)` clause.
fn grouping_labels(text: &str) -> Vec<String> {
    let mut labels = Vec::new();
    for keyword in ["by (", "without ("] {
        for rest in text.split(keyword).skip(1) {
            let (list, _) = rest.split_once(')').unwrap();
            labels.extend(list.split(',').map(|label| label.trim().to_string()));
        }
    }
    labels
}

/// The data metrics whose names appear in the text.
fn metrics_read<'a>(text: &str, spec: &'a WorkloadSpec) -> Vec<&'a str> {
    let read: Vec<&str> = spec
        .data
        .iter()
        .map(|record| record.metric.as_str())
        .filter(|metric| {
            text.contains(&format!("{metric}[")) || text.contains(&format!("{metric}{{"))
        })
        .collect();
    assert!(!read.is_empty(), "{text} reads no metric with data");
    read
}

/// `"MetricsQl"` is a query language of a workload spec.
#[test]
fn metricsql_is_a_workload_language() {
    let language: aqpbm_core::harness::engine::QueryLanguage =
        serde_json::from_str("\"MetricsQl\"").unwrap();
    assert_eq!(language, QueryLanguage::MetricsQl);
}
