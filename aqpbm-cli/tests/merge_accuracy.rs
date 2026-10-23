//! `--operations merge --metrics accuracy`: the sketch folded from shards is
//! scored like the query, against the whole stream.

use std::process::Command;

/// One `--flat` record measuring both the single sketch's and the merged
/// sketch's accuracy over the same small fixed-seed stream.
fn query_and_merge_accuracy(variant: &str, config: &str, dataset: &[&str]) -> serde_json::Value {
    run_merge_accuracy(variant, "lib", &["--config", config], dataset)
}

fn run_merge_accuracy(
    variant: &str,
    library: &str,
    config: &[&str],
    dataset: &[&str],
) -> serde_json::Value {
    let out = Command::new(env!("CARGO_BIN_EXE_approxbench"))
        .args(["sketchbench", "--variant", variant, "--library", library])
        .args(config)
        .args(["--operations", "query,merge", "--metrics", "accuracy"])
        .args(["--merge-shards", "4", "--runs", "1", "--warmup-runs", "0"])
        .args(["--size", "20000", "--seed", "7", "--flat"])
        .args(dataset)
        .env("BENCH_WARMUP_SECS", "0")
        .output()
        .expect("approxbench runs");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8(out.stdout).expect("utf-8");
    let last = stdout.lines().last().expect("one record");
    serde_json::from_str(last).expect("a JSON record")
}

/// CMS counters add, so four shards folded are the sketch fed the whole
/// stream: every score must match, not just come close.
#[test]
fn merged_cms_scores_exactly_like_one_sketch() {
    let r = query_and_merge_accuracy(
        "cms-fastpath-vector2d",
        "rows=3 cols=1024",
        &[
            "--dataset",
            "zipf",
            "--zipf-s",
            "1.0",
            "--cardinality",
            "1000",
            "--dtype",
            "i64",
        ],
    );
    assert!(r["merge_accuracy"].is_object(), "{r}");
    assert_eq!(r["merge_accuracy"], r["query_accuracy"]);
}

/// KLL merges are lossy, so only a finite score is promised.
#[test]
fn merged_kll_scores_finite() {
    let r = query_and_merge_accuracy(
        "kll-percall",
        "k=200",
        &[
            "--dataset",
            "pareto",
            "--pareto-alpha",
            "2",
            "--cardinality",
            "1",
            "--dtype",
            "f64",
        ],
    );
    let err = r["merge_accuracy"]["mean_rank_err"]
        .as_f64()
        .unwrap_or_else(|| panic!("no merged mean_rank_err in {r}"));
    assert!(err.is_finite(), "{err}");
}

/// Exact accumulators merge without loss (the optimizer relies on it and no
/// longer measures merged accuracy per row, #174): every merged score is the
/// single accumulator's, and the worst group's error is 0. Increase reads
/// counters, so its shard boundaries split counter runs.
#[test]
fn merged_exact_accumulators_score_exactly_like_one() {
    let spec = |name: &str| format!("{}/../configs/datagen/{name}", env!("CARGO_MANIFEST_DIR"));
    for (variant, comparator, columns) in [
        ("exact-sum", "sum-or-count", "hydra_columns.yaml"),
        ("exact-min", "min", "hydra_columns.yaml"),
        ("exact-max", "max", "hydra_columns.yaml"),
        ("exact-increase", "rate-or-increase", "counter_columns.yaml"),
        ("exact-delta-set", "key-set", "hydra_columns.yaml"),
    ] {
        let path = spec(columns);
        let r = run_merge_accuracy(
            variant,
            "exact",
            &[],
            &[
                "--comparator",
                comparator,
                "--spec",
                &path,
                "--dtype",
                "i64",
            ],
        );
        assert_eq!(r["merge_accuracy"], r["query_accuracy"], "{variant}: {r}");
        assert_eq!(r["merge_accuracy"]["relative_error"], 0.0, "{variant}: {r}");
    }
}
