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
            ArmSchedule::for_workload(&spec).unwrap_or_else(|error| panic!("{name}: {error}"));
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
            for (query, planned) in spec.queries.iter().zip(
                planner["query_workload"]["repeating_queries"].as_array().unwrap(),
            ) {
                assert_eq!(
                    query.evaluation_interval_ms,
                    planned["demand"]["fixed_interval_at"]["interval"].as_u64().unwrap(),
                    "{name} {}",
                    query.id
                );
            }
            checked += 1;
        }
    }
    assert!(checked >= 8, "only {checked} suite workloads");
}
