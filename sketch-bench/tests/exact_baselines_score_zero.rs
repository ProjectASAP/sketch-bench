//! The three shipped grouped registrations, each against its exact baseline,
//! which must score zero. Weaker than the name: a depth-1 key holds no
//! separator, so changing `subset_key`'s `;` passes here and fails
//! `bundle_ground_truth`; and these baselines store every subset, so a ground
//! truth scoring the wrong column also reads zero. It would not catch `28ef649`.

use std::collections::BTreeMap;

use aqpbm_core::metrics::{MetricsMask, OperationMask};
use aqpbm_core::registry::Numeric;
use aqpbm_core::runner::BenchConfig;
use aqpbm_core::{GenSpec, WorkloadSpec};

use sketch_bench::registry;

/// Small, but with the two label columns drawing from **disjoint alphabets**.
/// A depth-1 subset key is the bare label string, carrying nothing that names
/// its column, so a shared alphabet puts two populations in one group and every
/// row here reads non-zero — which is the defect, not a property of the test.
/// `keys` renders a rank through a fixed positional encoding, so at these
/// cardinalities a shared alphabet guarantees the collision rather than risking
/// it.
fn spec() -> WorkloadSpec {
    let column = |cardinality: u64, seed: u64, alphabet: &str| -> GenSpec {
        serde_json::from_value(serde_json::json!({
            "shape": "keys",
            "cardinality": cardinality,
            "dist": { "kind": "uniform" },
            "size": 4000,
            "seed": seed,
            "string": { "alphabet": alphabet },
        }))
        .expect("column spec parses")
    };
    WorkloadSpec::Columns(vec![
        column(24, 1, "abcdefghijklmnopqr"),
        column(10, 2, "stuvwxyz0123456789"),
        // The value column's alphabet is irrelevant: it is rendered as the
        // numeric value, not as a label.
        column(64, 3, "abcdefghijklmnopqrstuvwxyz0123456789"),
    ])
}

fn cfg() -> BenchConfig {
    BenchConfig {
        runs: 1,
        warmup_runs: 0,
        metrics: MetricsMask::ACCURACY,
        operations: OperationMask::QUERY,
        ..Default::default()
    }
}

/// Run one shipped registration's `polars` half through the real `REGISTRY`, so
/// this exercises what ships and not a table assembled here.
fn score(algorithm: &str, config: &str) -> BTreeMap<String, f64> {
    let params = registry::config_point(algorithm, config).expect("config parses");
    let reports = registry::run(
        algorithm,
        "polars",
        &cfg(),
        &spec(),
        &params,
        Numeric::I64,
        None,
    )
    .unwrap_or_else(|e| panic!("{algorithm}/polars cannot run: {e}"));
    reports[0].per_run[0]
        .accuracy
        .clone()
        .unwrap_or_else(|| panic!("{algorithm}/polars scored no accuracy"))
}

fn assert_zero(algorithm: &str, metrics: &BTreeMap<String, f64>, keys: &[&str]) {
    for key in keys {
        let v = metrics
            .get(*key)
            .unwrap_or_else(|| panic!("{algorithm}/polars reported no {key}"));
        assert_eq!(
            *v, 0.0,
            "{algorithm}/polars is exact and must score zero on {key}, got {v}. \
             A non-zero reading is the ground truth and the wrapper disagreeing \
             about the subset key space."
        );
    }
    // Guard the guard: a run that probed nothing would pass every assertion
    // above without having checked anything.
    assert!(
        metrics["subpopulations"] > 1.0,
        "{algorithm}/polars scored {} subpopulations, so the run proves nothing",
        metrics["subpopulations"]
    );
}

#[test]
fn subpop_frequency_baseline_scores_zero() {
    let m = score("hydra-cms", "rows=3 cols=256 cell_rows=3 cell_cols=256");
    assert_zero("hydra-cms", &m, &["are_all", "aae_all", "l1_err", "l2_err"]);
}

#[test]
fn subpop_cardinality_baseline_scores_zero() {
    let m = score("hydra-hll", "rows=3 cols=256");
    assert_zero("hydra-hll", &m, &["are_all", "aae_all", "l1_err", "l2_err"]);
}

#[test]
fn subpop_rank_error_baseline_scores_zero() {
    let m = score("hydra-kll", "rows=3 cols=256 cell_k=200");
    // Rank error, not ARE: this one is read on the ruler its statistic is
    // defined against.
    assert_zero("hydra-kll", &m, &["mean_rank_err", "max_rank_err"]);
}
