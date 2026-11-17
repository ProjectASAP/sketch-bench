//! P1: an equivalent-only plan must translate (the capability README's
//! rule). Every planner-written plan of a public synthetic workload under
//! `fixtures/plans/public/` translates, and the equivalent-only ones with no
//! substitution. A query-time node several readers share is recomputed per
//! reader (`SharingLost`); that changes cost, not answers, so it is no
//! substitution.
//!
//! The committed plans are the RUN-1 showcase shapes plus, per engine, a
//! cover of every node shape that planning all 206 public PromQL workloads
//! with that engine's two files produces (see `fixtures/plans/REV`).

mod common;

use aqpbm_engines_external::{clickhouse, victoriametrics, Substitution};

/// `(workload, variant)` of every committed public plan, in name order.
fn public_plans() -> Vec<(String, String)> {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/plans/public");
    let mut plans: Vec<(String, String)> = std::fs::read_dir(dir)
        .expect("public plans")
        .map(|entry| {
            let name = entry.unwrap().file_name().into_string().unwrap();
            let stem = name.strip_suffix(".plan.json").expect("a plan file");
            let (workload, variant) = ["clickhouse", "victoriametrics"]
                .iter()
                .find_map(|engine| {
                    let (workload, variant) = stem.split_once(&format!(".{engine}."))?;
                    Some((workload.to_string(), format!("{engine}.{variant}")))
                })
                .expect("<workload>.<engine>.<variant>");
            (workload, variant)
        })
        .collect();
    plans.sort();
    plans
}

#[test]
fn public_plans_translate() {
    let plans = public_plans();
    // ClickHouse: 18 equivalent-only plans and 11 with substitutes;
    // VictoriaMetrics: 8 equivalent-only and 17 with substitutes.
    let count = |engine: &str| plans.iter().filter(|(_, v)| v.starts_with(engine)).count();
    assert_eq!(count("clickhouse."), 29, "{plans:?}");
    assert_eq!(count("victoriametrics."), 25, "{plans:?}");
    // VictoriaMetrics stores a stream output at its flush time in whole
    // seconds, so 1 s windows cannot be told apart; the capability file
    // cannot say so (the planner has no window-width restriction), so these
    // plans are refused, as the README states.
    let expected_refusals = [
        "mixed-08 victoriametrics.with-substitutes: node 34: no translation for window width 1000 ms is below 2 s",
        "mixed-09 victoriametrics.with-substitutes: node 34: no translation for window width 1000 ms is below 2 s",
    ];
    let mut failures = Vec::new();
    for (workload, variant) in &plans {
        let plan = common::public_fixture(workload, variant);
        let translated: Result<(Vec<Substitution>, usize), _> =
            if variant.starts_with("clickhouse.") {
                clickhouse::translate(&plan).map(|d| (d.substitutions, d.queries.len()))
            } else {
                victoriametrics::translate(&plan).map(|d| (d.substitutions, d.queries.len()))
            };
        match translated {
            Err(error) => failures.push(format!("{workload} {variant}: {error}")),
            Ok((substitutions, queries)) => {
                if variant.ends_with(".equivalent-only") && !substitutions.is_empty() {
                    failures.push(format!(
                        "{workload} {variant}: substitutions {substitutions:?}"
                    ));
                }
                assert_eq!(queries, plan.roots.len());
            }
        }
    }
    assert_eq!(failures, expected_refusals, "{failures:#?}");
}
