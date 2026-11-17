//! K1: the committed deployment-capabilities files (ASAPPlanner #1374
//! format, as merged into ASAPPlanner main at `bc725bda8`) are exactly the hand-computed values of the design. An edit to a
//! file must also edit this test and the README row it comes from.

use serde_json::{json, Value};

fn file(name: &str) -> Value {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("capabilities")
        .join(name);
    serde_json::from_str(&std::fs::read_to_string(path).expect("file")).expect("JSON")
}

fn irate_exclusions() -> Value {
    json!([
        {"summary_family": {"exact": "IRate"}},
        {"operator": "Aggregate", "aggregate_measure": "i_rate"}
    ])
}

#[test]
fn capability_files_match_the_design() {
    assert_eq!(
        file("clickhouse.equivalent-only.json"),
        json!({
            // No sketch until the translator builds sketch states.
            "supported_summaries": [],
            "raw_data_retained": true,
            "query_time_results_retained": false,
            "counter_semantics": {"reset_handling": true, "extrapolation": true},
            "state_not_mergeable_across_windows": [{"exact": "Rate"}, {"exact": "Increase"}],
            "excluded_candidates": irate_exclusions()
        })
    );
    assert_eq!(
        file("clickhouse.with-substitutes.json"),
        json!({
            "supported_summaries": [],
            "raw_data_retained": true,
            "query_time_results_retained": false,
            "counter_semantics": {"reset_handling": true, "extrapolation": true},
            "state_not_mergeable_across_windows": [],
            "excluded_candidates": irate_exclusions()
        })
    );
    assert_eq!(
        file("victoriametrics.equivalent-only.json"),
        json!({
            "supported_summaries": [],
            "raw_data_retained": true,
            "query_time_results_retained": false,
            "counter_semantics": {"reset_handling": false, "extrapolation": false},
            "state_not_mergeable_across_windows": [],
            "excluded_candidates": []
        })
    );
    assert_eq!(
        file("victoriametrics.with-substitutes.json"),
        json!({
            // No sketch until the translator builds sketch states, and no
            // stored counter chain until it has a rule for one.
            "supported_summaries": [],
            "raw_data_retained": true,
            "query_time_results_retained": false,
            "counter_semantics": {"reset_handling": true, "extrapolation": true},
            "state_not_mergeable_across_windows": [{"exact": "Rate"}, {"exact": "Increase"}],
            "excluded_candidates": irate_exclusions()
        })
    );
}

#[test]
fn capability_files_are_pretty_printed() {
    // L10: one format for all four files, as `serde_json::to_string_pretty`
    // writes it (which this test cannot call: it would sort the keys, and
    // the files keep the README's order): two-space indents, at most one
    // key per line, a final newline.
    for name in [
        "clickhouse.equivalent-only.json",
        "clickhouse.with-substitutes.json",
        "victoriametrics.equivalent-only.json",
        "victoriametrics.with-substitutes.json",
    ] {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("capabilities")
            .join(name);
        let text = std::fs::read_to_string(path).expect("file");
        assert!(text.ends_with("}\n"), "{name}");
        for line in text.lines() {
            let indent = line.len() - line.trim_start().len();
            assert!(indent % 2 == 0, "{name}: {line}");
            assert!(line.matches("\": ").count() <= 1, "{name}: {line}");
        }
    }
}
