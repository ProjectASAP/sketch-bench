//! `--group-columns`: a Hydra row asks and scores the label subset it names,
//! any non-empty one, not only column 0. On a grid large enough that no two of
//! the few subkeys share a column in most rows, the sketch answers each group
//! as the exact baseline does, so a group asked at the wrong columns shows up
//! as error.

use std::process::Command;

/// Two overlapping label columns (both render ranks over one alphabet, so
/// `b` is a value of each) over few values, and a value column at `dtype`.
fn spec(dtype: &str) -> std::path::PathBuf {
    let path = std::env::temp_dir().join(format!(
        "hydra_grouping_{}_{dtype}.yaml",
        std::process::id()
    ));
    let yaml = format!(
        "column_num: 3
column_label: [key1, key2, value]
row_num: 5000
column_spec:
  - data_type: string
    distribution: {{kind: uniform, lower_bound: 0.0, upper_bound: 4.0, seed: 1}}
  - data_type: string
    distribution: {{kind: zipf, skewness: 1.1, population_size: 3, seed: 2}}
  - data_type: {dtype}
    distribution: {{kind: zipf, skewness: 1.2, population_size: 40, seed: 3}}
"
    );
    std::fs::write(&path, yaml).expect("the temp dir is writable");
    path
}

/// The query accuracy record of one run, plus the per-group CSV it wrote.
fn scored(
    variant: &str,
    library: &str,
    config: &str,
    dtype: &str,
    group_columns: &str,
) -> (serde_json::Value, String) {
    let spec = spec(dtype);
    let csv = spec.with_extension(format!("{variant}.{library}.{group_columns}.csv"));
    let out = Command::new(env!("CARGO_BIN_EXE_approxbench"))
        .args(["sketchbench", "--variant", variant, "--library", library])
        .args(["--config", config, "--dtype", dtype])
        .args(["--spec", spec.to_str().expect("utf-8 path")])
        .args(["--group-columns", group_columns])
        .args(["--per-group-out", csv.to_str().expect("utf-8 path")])
        .args(["--operations", "query", "--metrics", "accuracy"])
        .args(["--runs", "1", "--warmup-runs", "0"])
        .env("BENCH_WARMUP_SECS", "0")
        .output()
        .expect("approxbench runs");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8(out.stdout).expect("utf-8");
    let record: serde_json::Value =
        serde_json::from_str(stdout.lines().last().expect("one record")).expect("JSON");
    let csv = std::fs::read_to_string(&csv).expect("--per-group-out was written");
    (record["bench"]["accuracy"].clone(), csv)
}

/// Every Hydra family, lib at a grid far larger than its 19 subkeys and the
/// polars baseline, grouped by column 1 alone and by both columns: both score
/// (about) zero, and the per-group file has one row per scored group, keyed by
/// the columns asked.
#[test]
fn any_label_subset_is_asked_where_it_was_inserted() {
    let rows = [
        ("hydra-cms", "rows=3 cols=1024 cell_rows=3 cell_cols=256", "i64"),
        ("hydra-cs", "rows=3 cols=1024 cell_rows=3 cell_cols=256", "i64"),
        ("hydra-hll", "rows=3 cols=1024", "i64"),
        ("hydra-kll", "rows=3 cols=1024 cell_k=4096", "f64"),
        (
            "hydra-univmon-l1-norm",
            "rows=3 cols=1024 cell_heap_size=64 cell_sketch_row=3 cell_sketch_col=256 cell_layer_size=4",
            "i64",
        ),
    ];
    for (variant, config, dtype) in rows {
        for group_columns in ["1", "0,1"] {
            for (library, tolerance) in [("polars", 0.0), ("lib", 0.01)] {
                let (acc, csv) = scored(variant, library, config, dtype, group_columns);
                let at = format!("{variant}/{library} --group-columns {group_columns}: {acc}");
                assert!(acc["err_max"].as_f64().expect(&at) <= tolerance, "{at}");
                assert_eq!(acc["schema_width"], 2.0, "{at}");
                assert_eq!(acc["records"], 5000.0, "{at}");
                assert_eq!(acc["fanned_mass"], 15000.0, "{at}");

                let mut lines = csv.lines();
                assert_eq!(lines.next(), Some("group_key,n_q,error"), "{at}");
                let rows: Vec<&str> = lines.collect();
                assert_eq!(rows.len() as f64, acc["groups_scored"].as_f64().expect(&at));
                let n_q: u64 = rows
                    .iter()
                    .map(|r| r.split(',').nth(1).and_then(|n| n.parse::<u64>().ok()))
                    .map(|n| n.expect("n_q is a count"))
                    .sum();
                assert_eq!(n_q, 5000, "every record is in exactly one group: {at}");
                let key = rows[0].split(',').next().expect("a key");
                match group_columns {
                    "1" => assert!(key.starts_with("label1:") && !key.contains(';'), "{key}"),
                    _ => assert!(
                        key.starts_with("label0:") && key.contains(";label1:"),
                        "{key}"
                    ),
                }
            }
        }
    }
}

/// A grouping column has to be a label column: the value column is not a key
/// of the grid.
#[test]
fn a_group_column_past_the_labels_is_refused() {
    let spec = spec("i64");
    let out = Command::new(env!("CARGO_BIN_EXE_approxbench"))
        .args(["sketchbench", "--variant", "hydra-cms", "--library", "lib"])
        .args([
            "--spec",
            spec.to_str().expect("utf-8 path"),
            "--dtype",
            "i64",
        ])
        .args(["--group-columns", "2"])
        .args(["--operations", "query", "--metrics", "accuracy"])
        .args(["--runs", "1", "--warmup-runs", "0"])
        .output()
        .expect("approxbench runs");
    assert!(!out.status.success());
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("--group-columns 2"), "{err}");
}
