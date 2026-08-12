//! The three shipped grouped registrations, each run against its exact
//! baseline, which must score zero.
//!
//! # What this does and does not catch
//!
//! It pins that all three run end to end through the shipped `REGISTRY` and
//! that the depth-1 key space agrees: the ground truth groups on a bare label
//! and the baseline files one, so a truth that transformed the label — trimmed,
//! cased, prefixed — reads non-zero here.
//!
//! It does **not** catch two things it might look like it does.
//!
//! The separator is invisible at depth 1, because a one-column subset key emits
//! none. Changing `subset_key`'s `;` passes every assertion below;
//! `bundle_ground_truth.rs` is what fails, because its population is depth 2.
//!
//! Nor would it have caught `28ef649`. That was label columns aliasing through
//! a shared alphabet, which needs the shipped 200/50 cardinalities to bite — at
//! the sizes below the rank encoding gives the two columns different key
//! lengths, so a shared alphabet does not collide. That defect lives in the
//! workload, and only a test over the shipped spec would see it.
//!
//! One more thing no comparison against these baselines can see: they store
//! *every* label subset, so they answer correctly whichever one the ground
//! truth picks. A ground truth scoring the wrong column still reads zero here.

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
