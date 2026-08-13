//! Can a bundle wire in its own grouped ground truth without touching
//! `aqpbm-core`? This scores the depth-2 population none of the three shipped
//! ground truths reach, declared entirely here. An integration test sees only
//! the public API, which is what a bundle author has — so if it compiles, the
//! swap is available to them.

use std::collections::BTreeMap;
use std::collections::HashMap;
use std::collections::HashSet;

use aqpbm_core::accumulator::Accumulator;
use aqpbm_core::accuracy::subpopulation::{
    rank_and_shuffle, score_counting_population, subset_key,
};
use aqpbm_core::accuracy::{GroundTruth, GroundTruthCalculator, GroundTruthName, SubpopFrequencyOps};
use aqpbm_core::config::ParamSet;
use aqpbm_core::metrics::{MetricsMask, OperationMask};
use aqpbm_core::registry::{run, scored, Numeric, Registration};
use aqpbm_core::runner::BenchConfig;
use aqpbm_core::workload::Labeled;
use aqpbm_core::{Distribution, GenSpec, Shape, WorkloadSpec};

use sketch_bench::params::HydraCmsParams;
use sketch_bench::wrappers::hydra::HydraCms;
use sketch_bench::wrappers::polars::PolarsSubpopFrequency;

/// The population the shipped ground truths cannot reach: both label columns,
/// not one. `Truth` is declared here because core's is not constructible from
/// outside — but everything numeric below is core's.
struct DepthTwoFreqGT;

struct Truth {
    exact: HashMap<(String, i64), u64>,
    ranked: Vec<(String, i64)>,
    all: Vec<(String, i64)>,
}

impl GroundTruthName for DepthTwoFreqGT {
    const NAME: &'static str = "subpop-frequency-depth2";
}

impl<S> GroundTruthCalculator<S> for DepthTwoFreqGT
where
    S: Accumulator<Item = Labeled<i64>> + SubpopFrequencyOps<Value = i64>,
{
    fn build(_params: &ParamSet) -> Self {
        DepthTwoFreqGT
    }
}

impl<S> GroundTruth<S> for DepthTwoFreqGT
where
    S: Accumulator<Item = Labeled<i64>> + SubpopFrequencyOps<Value = i64>,
{
    type Truth = Truth;
    type Probe = (String, i64);
    type Answer = f64;

    fn truth(&self, items: &[Labeled<i64>]) -> Truth {
        let mut exact: HashMap<(String, i64), u64> = HashMap::new();
        for it in items {
            // core's key space, so this cannot drift from what the sketch files.
            let Some(group) = subset_key(it, &[0, 1]) else {
                continue;
            };
            *exact.entry((group, it.value)).or_insert(0) += 1;
        }
        let (ranked, all) = rank_and_shuffle(exact.iter().map(|(k, c)| (k.clone(), *c)));
        Truth { exact, ranked, all }
    }

    fn probes(&self, truth: &Truth) -> Vec<(String, i64)> {
        truth.all.clone()
    }

    fn ask(&self, sketch: &S, probe: &(String, i64)) -> f64 {
        let labels: Vec<&str> = probe.0.split(';').collect();
        sketch.estimate_subpop_frequency(&labels, &probe.1)
    }

    fn answer_as_f64(&self, answer: &f64) -> f64 {
        *answer
    }

    fn score(
        &self,
        truth: &Truth,
        probes: &[(String, i64)],
        answers: &[f64],
    ) -> BTreeMap<String, f64> {
        let est: HashMap<&(String, i64), f64> =
            probes.iter().zip(answers).map(|(p, a)| (p, *a)).collect();
        let groups: HashSet<&str> = truth.exact.keys().map(|(g, _)| g.as_str()).collect();
        // core's roll-up: ARE, AAE, the norms, the top-k prefixes, the percentile.
        score_counting_population(&truth.ranked, &truth.all, groups.len(), |pair| {
            (
                *est.get(pair).unwrap_or(&0.0),
                *truth.exact.get(pair).unwrap_or(&0) as f64,
            )
        })
    }
}

/// The bundle's own table: `hydra-cms` scored at depth 2, sketch and control.
static DEPTH_TWO: &[Registration] = &[
    scored::<HydraCms, DepthTwoFreqGT>("asap Hydra over Count-Min cells, scored at depth 2"),
    scored::<PolarsSubpopFrequency, DepthTwoFreqGT>("polars exact, scored at depth 2"),
];

fn spec() -> WorkloadSpec {
    let col = |cardinality, seed| GenSpec {
        shape: Shape::Keys {
            cardinality,
            dist: Distribution::Uniform,
        },
        size: 4096,
        seed,
        string: None,
        depends_on: None,
    };
    // Two label columns then the value column.
    WorkloadSpec::Columns(vec![col(16, 1), col(8, 2), col(64, 3)])
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

fn score(impl_name: &str) -> BTreeMap<String, f64> {
    let params = ParamSet::of(&HydraCmsParams {
        rows: 5,
        cols: 4096,
        cell_rows: 5,
        cell_cols: 4096,
    });
    let reports = run(
        DEPTH_TWO,
        "hydra-cms",
        impl_name,
        &cfg(),
        &spec(),
        &params,
        Numeric::I64,
        Some("subpop-frequency-depth2"),
    )
    .expect("the depth-2 registration runs");
    reports[0].per_run[0]
        .accuracy
        .clone()
        .expect("the accuracy square was scored")
}

/// That this file compiles is most of the point: a bundle declares the struct,
/// the three impls and the registration, and reuses core's key space and
/// roll-up. Nothing in `aqpbm-core` was edited to reach this population.
#[test]
fn a_bundle_can_register_its_own_grouped_ground_truth() {
    let metrics = score("lib");
    assert!(metrics.contains_key("are_all"));
    // Depth 2 over 16x8 labels is up to 128 groups, against 16 at depth 1.
    assert!(
        metrics["subpopulations"] > 16.0,
        "depth 2 must reach more groups than one column does: {}",
        metrics["subpopulations"]
    );
}

/// The check the shipped rows do not have: an exact answer must score zero. A
/// non-zero reading here is the harness scoring a key space the sketch does not
/// have, which is the defect `28ef649` found by hand.
#[test]
fn the_exact_baseline_scores_zero_at_depth_two() {
    let metrics = score("polars");
    for key in ["are_all", "aae_all", "l1_err"] {
        assert_eq!(
            metrics[key], 0.0,
            "the exact baseline must score zero on {key}, got {}",
            metrics[key]
        );
    }
}
