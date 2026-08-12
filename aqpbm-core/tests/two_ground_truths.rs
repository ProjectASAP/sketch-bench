//! One sketch, two ground truths, two rows.
//!
//! A row is `(algorithm, impl, ground truth)`, so scoring a sketch a second way
//! is a second `scored::<S, G>` line and not an edit to the first. This is that,
//! end to end: registration, selection by name, and what the record carries.
//!
//! Everything here is declared in the test, so `aqpbm-core` still knows no
//! concrete sketch — that it can be exercised this way *is* the property under
//! test.

use serde::{Deserialize, Serialize};

use aqpbm_core::accumulator::Accumulator;
use aqpbm_core::accuracy::cardinality::CardinalityGT;
use aqpbm_core::accuracy::frequency::FrequencyGT;
use aqpbm_core::accuracy::{CardinalityOps, FrequencyOps};
use aqpbm_core::catalog::{ground_truths, list, run, scored, Numeric, Row};
use aqpbm_core::config::{ParamSet, SketchParams};
use aqpbm_core::init::{BenchImpl, BuildError, InitSketch};
use aqpbm_core::memory_footprint::MemoryFootprint;
use aqpbm_core::metrics::{MetricsMask, OperationMask};
use aqpbm_core::runner::BenchConfig;
use aqpbm_core::{Distribution, GenSpec, Shape, WorkloadSpec};

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct TallyParams {}

impl SketchParams for TallyParams {
    const FAMILY: &'static str = "tally";
    fn canonical() -> Self {
        TallyParams {}
    }
}

/// Exact, and answers **two** statistics — which is what makes it registerable
/// twice. A sketch answering one is the ordinary case and stays one row.
#[derive(Default)]
struct Tally {
    counts: std::collections::HashMap<i64, u64>,
}

impl Accumulator for Tally {
    type Item = i64;
    fn update(&mut self, v: &i64) {
        *self.counts.entry(*v).or_insert(0) += 1;
    }
}

impl InitSketch for Tally {
    fn init(config: &ParamSet) -> Result<Self, BuildError> {
        let _p: TallyParams = config.parse()?;
        Ok(Self::default())
    }
}

impl MemoryFootprint for Tally {
    fn memory_bytes(&self) -> usize {
        self.counts.capacity() * std::mem::size_of::<(i64, u64)>()
    }
}

impl BenchImpl for Tally {
    type Params = TallyParams;
    const IMPL: &'static str = "lib";
}

impl CardinalityOps for Tally {
    fn estimate_distinct(&self) -> f64 {
        self.counts.len() as f64
    }
}

impl FrequencyOps for Tally {
    type Key = i64;
    fn estimate_frequency(&self, key: &i64) -> u64 {
        self.counts.get(key).copied().unwrap_or(0)
    }
}

/// The registration under test: one sketch, two lines, no hand-written `Row`.
static ROWS: &[Row] = &[
    scored::<Tally, CardinalityGT>("exact tally, scored as a cardinality sketch"),
    scored::<Tally, FrequencyGT>("exact tally, scored as a frequency sketch"),
];

fn cfg() -> BenchConfig {
    BenchConfig {
        runs: 1,
        warmup_runs: 0,
        metrics: MetricsMask::ACCURACY,
        operations: OperationMask::QUERY,
        ..Default::default()
    }
}

fn spec() -> WorkloadSpec {
    WorkloadSpec::Generated(GenSpec {
        shape: Shape::Keys {
            cardinality: 32,
            dist: Distribution::Uniform,
        },
        size: 256,
        seed: 1,
        string: None,
    })
}

fn params() -> ParamSet {
    ParamSet::of(&TallyParams::canonical())
}

fn go(ground_truth: Option<&str>) -> anyhow::Result<Vec<aqpbm_core::runner::BenchReport>> {
    run(
        ROWS,
        "tally",
        "lib",
        &cfg(),
        &spec(),
        &params(),
        Numeric::I64,
        ground_truth,
    )
}

/// Both rows exist under one `(algorithm, impl)`, which is the whole point.
#[test]
fn one_sketch_is_registered_against_two_ground_truths() {
    assert_eq!(
        ground_truths(ROWS, "tally", "lib"),
        vec!["cardinality", "frequency"]
    );
    // Every field still comes off the types: neither row was hand-written.
    for r in ROWS {
        assert_eq!((r.family, r.algorithm, r.impl_name), ("tally", "tally", "lib"));
        assert!(r.scores_accuracy());
    }
}

/// Naming the ground truth picks the row, and the two really do score
/// differently: a cardinality row reports `relative_error`, a frequency row
/// reports the `are_*` curve. Neither key appears in the other.
#[test]
fn the_name_selects_which_row_runs() {
    let card = go(Some("cardinality")).unwrap();
    let freq = go(Some("frequency")).unwrap();

    let card_metrics = card[0].per_run[0]
        .accuracy
        .as_ref()
        .expect("the cardinality row scored the accuracy square");
    let freq_metrics = freq[0].per_run[0]
        .accuracy
        .as_ref()
        .expect("the frequency row scored the accuracy square");

    assert!(card_metrics.contains_key("relative_error"));
    assert!(!card_metrics.contains_key("are_all"));

    assert!(freq_metrics.contains_key("are_all"));
    assert!(!freq_metrics.contains_key("relative_error"));
}

/// The record says which one ran, so two rows of one `(sketch, impl)` are not
/// indistinguishable downstream. Stamped from the type that ran, not from the
/// request.
#[test]
fn the_record_carries_the_ground_truth_that_scored_it() {
    for name in ["cardinality", "frequency"] {
        let reports = go(Some(name)).unwrap();
        assert_eq!(reports[0].ground_truth.as_deref(), Some(name));
        assert_eq!(
            reports[0].to_record().ground_truth.as_deref(),
            Some(name),
            "the JSONL record must carry it too"
        );
    }
}

/// Omitting the name takes the first row registered for the pair, so a caller
/// that never heard of the axis keeps the behaviour it had.
#[test]
fn omitting_the_name_takes_the_first_row() {
    let reports = go(None).unwrap();
    assert_eq!(reports[0].ground_truth.as_deref(), Some("cardinality"));
}

/// An unregistered name is refused from the catalog, before anything is
/// generated, and the error names what is on offer.
#[test]
fn an_unknown_ground_truth_is_refused_by_name() {
    let err = go(Some("rank-error")).unwrap_err().to_string();
    assert!(err.contains("rank-error"), "{err}");
    assert!(err.contains("cardinality, frequency"), "{err}");
}

/// `--list-impls` shows the axis, or a second row would look like a duplicate.
#[test]
fn the_listing_shows_the_ground_truth_column() {
    let lines = list(ROWS);
    assert!(lines[0].contains("ground-truth"), "{:?}", lines[0]);
    let body: Vec<&String> = lines.iter().filter(|l| l.contains("tally")).collect();
    assert_eq!(body.len(), 2);
    assert!(body[0].contains("cardinality"));
    assert!(body[1].contains("frequency"));
}
