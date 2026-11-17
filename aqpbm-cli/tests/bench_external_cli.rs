//! `approxbench bench-external run` and `report` on the fake lane.

use std::path::{Path, PathBuf};
use std::process::Command;

use aqpbm_core::harness::data::DataRecord;
use aqpbm_core::harness::engine::{QueryLanguage, QueryText, WorkloadQuery, WorkloadSpec};
use aqpbm_core::harness::rundir::RunDir;
use aqpbm_datagen::{DataDistribution, SeriesLabelSpec, TimeSeriesDescription, UniformParameter};

fn workload(name: &str) -> WorkloadSpec {
    WorkloadSpec {
        name: name.into(),
        queries: vec![WorkloadQuery {
            id: "sum".into(),
            texts: vec![QueryText {
                language: QueryLanguage::PromQl,
                text: "sum_over_time(m[10s])".into(),
            }],
            window_ms: 10_000,
            evaluation_interval_ms: 10_000,
        }],
        data: vec![DataRecord {
            metric: "m".into(),
            description: TimeSeriesDescription {
                labels: vec![SeriesLabelSpec {
                    name: "s".into(),
                    cardinality: 2,
                    skewness: 0.0,
                }],
                series_num: 2,
                start_ms: 0,
                sample_interval_ms: 1_000,
                duration_ms: 60_000,
                value_distribution: DataDistribution::Uniform(UniformParameter {
                    lower_bound: 0.0,
                    upper_bound: 10.0,
                    seed: 0,
                }),
                counter: false,
                seed: 1,
            },
        }],
    }
}

fn write(path: &Path, text: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

fn fixture(test: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!(
        "approxbench-bench-external-{}-{test}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&root);
    let run = RunDir::create_at(&root.join("run")).unwrap();
    for name in ["w1", "w2"] {
        write(
            &run.path().join(format!("workloads/{name}.json")),
            &serde_json::to_string(&workload(name)).unwrap(),
        );
    }
    let plans = run.path().join("plans");
    write(
        &plans.join("v1/variant.json"),
        r#"{"substitutions_allowed": false}"#,
    );
    write(&plans.join("v1/w1.plan.json"), "{}");
    write(
        &plans.join("v1/w2.no-plan.json"),
        r#"{"reason": "ExcludedByDeployment"}"#,
    );
    write(
        &plans.join("v2/variant.json"),
        r#"{"substitutions_allowed": true}"#,
    );
    write(&plans.join("v2/w1.plan.json"), "{}");
    write(&plans.join("v2/w2.plan.json"), "{}");
    run.path().to_path_buf()
}

fn approxbench(args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_approxbench"))
        .args(args)
        .output()
        .unwrap()
}

/// X1: a fake lane runs every cell, and the report has one row per cell.
#[test]
fn a_fake_lane_runs_and_reports() {
    let run = fixture("x1");
    let run_dir = run.to_str().unwrap();
    let out = approxbench(&[
        "bench-external",
        "run",
        "--run-dir",
        run_dir,
        "--lane",
        "fake",
        "--warmup",
        "0",
        "--runs",
        "1",
        "--max-core-busy",
        "1.0",
    ]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let out = approxbench(&[
        "bench-external",
        "report",
        "--run-dir",
        run_dir,
        "--truth",
        "fake/v2",
    ]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let summary = std::fs::read_to_string(run.join("summary.csv")).unwrap();
    assert_eq!(summary.lines().count(), 5, "{summary}");
    assert!(
        summary
            .lines()
            .any(|line| line.starts_with("w1,fake,v1,ok,thread,")),
        "{summary}"
    );
    assert!(
        summary
            .lines()
            .any(|line| line.starts_with("w2,fake,v1,no plan: ExcludedByDeployment")),
        "{summary}"
    );
    let _ = std::fs::remove_dir_all(run.parent().unwrap());
}

/// X2: an unknown lane names the known ones.
#[test]
fn an_unknown_lane_names_the_known_lanes() {
    let run = fixture("x2");
    let out = approxbench(&[
        "bench-external",
        "run",
        "--run-dir",
        run.to_str().unwrap(),
        "--lane",
        "victoriametrics",
    ]);
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("clickhouse") && stderr.contains("fake"),
        "{stderr}"
    );
    let _ = std::fs::remove_dir_all(run.parent().unwrap());
}

/// X3: the clickhouse lane needs the server and its cgroup agent.
#[test]
fn the_clickhouse_lane_needs_its_server() {
    let run = fixture("x3");
    let out = approxbench(&[
        "bench-external",
        "run",
        "--run-dir",
        run.to_str().unwrap(),
        "--lane",
        "clickhouse",
        "--clickhouse-url",
        "http://10.0.0.2:8123",
    ]);
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("--clickhouse-url") && stderr.contains("--cgroup-agent-url"),
        "{stderr}"
    );
    let _ = std::fs::remove_dir_all(run.parent().unwrap());
}
