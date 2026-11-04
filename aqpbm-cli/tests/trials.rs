use std::process::{Command, Output};

const ZIPF_I64: &[&str] = &[
    "--dataset",
    "zipf",
    "--zipf-s",
    "1.1",
    "--cardinality",
    "1000",
    "--size",
    "5000",
    "--dtype",
    "i64",
];

const PARETO_F64: &[&str] = &[
    "--dataset",
    "pareto",
    "--pareto-alpha",
    "2",
    "--cardinality",
    "1",
    "--size",
    "5000",
    "--dtype",
    "f64",
];

fn approxbench(variant: &str, config: &str, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_approxbench"))
        .args(["sketchbench", "--variant", variant, "--library", "lib"])
        .args(["--config", config])
        .args(["--operations", "query", "--metrics", "accuracy"])
        .args(["--runs", "1", "--warmup-runs", "0"])
        .args(args)
        .output()
        .expect("approxbench runs")
}

fn records(variant: &str, config: &str, args: &[&str]) -> Vec<serde_json::Value> {
    let out = approxbench(variant, config, args);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout)
        .expect("utf-8")
        .lines()
        .map(|line| serde_json::from_str(line).expect("a JSON record"))
        .collect()
}

fn identity(record: &serde_json::Value) -> serde_json::Value {
    serde_json::json!({
        "trial": record["bench"]["trial"],
        "trial_mode": record["bench"]["trial_mode"],
        "accuracy": record["bench"]["accuracy"],
        "sketch_config": record["sketch_config"],
        "workload": record["workload"],
    })
}

#[test]
fn a_later_start_reproduces_the_same_trials() {
    let cms = "rows=3 cols=256";
    let all = records(
        "cms-fastpath-vector2d",
        cms,
        &[ZIPF_I64, &["--trials", "64"]].concat(),
    );
    let tail = records(
        "cms-fastpath-vector2d",
        cms,
        &[ZIPF_I64, &["--trial-start", "32", "--trials", "32"]].concat(),
    );
    assert_eq!(all.len(), 64);
    let expected: Vec<_> = all[32..].iter().map(identity).collect();
    let actual: Vec<_> = tail.iter().map(identity).collect();
    assert_eq!(expected, actual);
}

#[test]
fn relabel_keeps_the_stream_and_moves_the_collisions() {
    let trials = records(
        "cms-fastpath-vector2d",
        "rows=3 cols=64",
        &[ZIPF_I64, &["--trials", "8", "--trial-mode", "relabel"]].concat(),
    );
    let accuracy: Vec<_> = trials.iter().map(|r| &r["bench"]["accuracy"]).collect();
    for field in ["stream_items", "stream_distinct", "stream_l2"] {
        assert!(
            accuracy.iter().all(|a| a[field] == accuracy[0][field]),
            "{field}"
        );
    }
    assert!(
        accuracy
            .iter()
            .any(|a| a["aae_top1"] != accuracy[0]["aae_top1"]),
        "{accuracy:?}"
    );
}

#[test]
fn reseed_varies_the_sketch_and_reruns_the_same() {
    let run = || {
        records(
            "kll-percall",
            "k=50",
            &[
                PARETO_F64,
                &[
                    "--quantiles",
                    "0.5,0.9",
                    "--trials",
                    "4",
                    "--trial-mode",
                    "reseed",
                ],
            ]
            .concat(),
        )
    };
    let first = run();
    let errors: Vec<_> = first
        .iter()
        .map(|r| r["bench"]["accuracy"]["rank_err_q0.5"].clone())
        .collect();
    assert!(errors.iter().any(|e| *e != errors[0]), "{errors:?}");
    assert!(first.iter().all(|r| r["workload"] == first[0]["workload"]));
    let again: Vec<_> = run().iter().map(identity).collect();
    assert_eq!(first.iter().map(identity).collect::<Vec<_>>(), again);
}

#[test]
fn reseed_on_a_row_without_a_seed_names_the_rows_with_one() {
    let out = approxbench(
        "cms-fastpath-vector2d",
        "rows=3 cols=256",
        &[ZIPF_I64, &["--trials", "2", "--trial-mode", "reseed"]].concat(),
    );
    assert!(!out.status.success());
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(
        err.contains("kll-percall/lib") && err.contains("coco/lib"),
        "{err}"
    );
}

#[test]
fn a_fresh_trial_reruns_from_its_own_descriptor() {
    let dir = std::env::temp_dir().join(format!("approxbench-trials-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    for (variant, config, dtype) in [
        ("cms-fastpath-vector2d", "rows=3 cols=256", "i64"),
        ("bloom", "rows=3 cols=2048", "string"),
    ] {
        let dataset = [&ZIPF_I64[..8], &["--dtype", dtype]].concat();
        let trials = records(
            variant,
            config,
            &[&dataset[..], &["--trials", "3"]].concat(),
        );
        let last = &trials[2];
        let description = &last["workload"]["synthetic"]["description"];
        assert!(!description["column_spec"][0]["key_offset"].is_null());
        let spec = dir.join(format!("{variant}.json"));
        std::fs::write(&spec, description.to_string()).unwrap();
        let rerun = records(
            variant,
            config,
            &["--spec", spec.to_str().unwrap(), "--dtype", dtype],
        );
        assert_eq!(rerun.len(), 1);
        assert_eq!(rerun[0]["bench"]["accuracy"], last["bench"]["accuracy"]);
        assert_eq!(rerun[0]["workload"], last["workload"]);
        assert!(rerun[0]["bench"].get("trial").is_none());
    }
    std::fs::remove_dir_all(&dir).unwrap();
}
