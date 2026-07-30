//! The catalog: one table naming every `(algorithm, impl)` this crate exposes,
//! and the dispatch resolving one to a concrete sketch type. It lives here,
//! not in the CLI, so a future `aqp-bench` can ship its own. A row is a
//! *type*, not a pair of strings — algorithm, impl name and `scores_accuracy`
//! are projected off it, so the list and the code cannot drift apart.

use anyhow::Result;
use aqpbm_core::accumulator::Accumulator;

use aqpbm_core::accuracy::cardinality::CardinalityGT;
use aqpbm_core::accuracy::frequency::FrequencyGT;
use aqpbm_core::accuracy::quantile::{RankErrorGT, RelativeErrorGT};
use aqpbm_core::accuracy::subpopulation::{
    SubpopCardinalityGT, SubpopFrequencyGT, SubpopRankErrorGT,
};
use aqpbm_core::accuracy::topk::TopkGT;
use aqpbm_core::accuracy::GroundTruth;
use aqpbm_core::cell::{self, AccuracyCfg, BenchItem, ParallelInit, RunError, WorkloadSpec};
use aqpbm_core::init::{BenchImpl, InitSketch};
use aqpbm_core::memory_footprint::MemoryFootprint;
use aqpbm_core::runner::{BenchConfig, BenchReport};

use crate::params::{ParamSet, TopkParams};
use crate::wrappers::{
    cms, countsketch, dd, elastic, hll, hydra, kll, nitro, parallel, polars, topk, univmon,
};

// ---------- what a row is ----------

/// The one item-type choice a user still makes: an `ordered` row (KLL, DDSketch)
/// builds at either width, while every other row's item type is fixed by its Rust
/// type — so the catalog can refuse before generating anything.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Numeric {
    #[default]
    I64,
    F64,
}

/// The executable half of a row: everything the frontend can hand a cell.
type RunFn = fn(
    &BenchConfig,
    &WorkloadSpec,
    &ParamSet,
    &AccuracyCfg,
    Numeric,
) -> Result<Vec<BenchReport>, RunError>;

/// One catalog entry. Built only by the four constructors below, so `algorithm`,
/// `impl_name` and `scores_accuracy` are always projections of the row's type
/// and its runner — never hand-written strings that could drift from it.
pub struct Row {
    pub algorithm: &'static str,
    pub impl_name: &'static str,
    /// The one field that is genuinely new data, and so is written in [`ROWS`].
    pub description: &'static str,
    /// Does `--accuracy` score this row? Derived: true iff it was built with a
    /// constructor that takes a ground-truth calculator.
    pub scores_accuracy: bool,
    /// Can this row run at [`Numeric::F64`]? Derived: only `ordered` rows can.
    pub picks_width: bool,
    /// Does this row ingest labelled records, and so need a `--spec` column
    /// list instead of a single-column spec? Derived off the row's item type.
    pub takes_columns: bool,
    run: RunFn,
}

// ---------- how a row builds its ground truth ----------

/// A [`GroundTruth`] that constructs itself from the run's accuracy knobs and the
/// row's params. A trait, not a `fn` argument, so the calculator is named as a
/// *type* in [`ROWS`] and the row stays `const`.
trait GroundTruthCalculator<S: Accumulator>: GroundTruth<S> {
    fn build(acc: &AccuracyCfg, params: &ParamSet) -> Self;
}

impl<S: Accumulator> GroundTruthCalculator<S> for CardinalityGT
where
    Self: GroundTruth<S>,
{
    fn build(acc: &AccuracyCfg, _params: &ParamSet) -> Self {
        CardinalityGT {
            record_calls: acc.record_query_calls,
        }
    }
}

impl<S: Accumulator> GroundTruthCalculator<S> for FrequencyGT
where
    Self: GroundTruth<S>,
{
    fn build(acc: &AccuracyCfg, _params: &ParamSet) -> Self {
        FrequencyGT {
            max_probes: acc.max_probes,
        }
    }
}

impl<S: Accumulator> GroundTruthCalculator<S> for SubpopFrequencyGT
where
    Self: GroundTruth<S>,
{
    /// Scores column 0. A grouped sketch stores every column subset, but each
    /// one is its own population with its own error, so a comparator names the
    /// one it scores instead of pooling them.
    fn build(acc: &AccuracyCfg, _params: &ParamSet) -> Self {
        SubpopFrequencyGT {
            label_column: 0,
            max_probes: acc.max_probes,
        }
    }
}

impl<S: Accumulator> GroundTruthCalculator<S> for SubpopCardinalityGT
where
    Self: GroundTruth<S>,
{
    /// Column 0, for the same reason as [`SubpopFrequencyGT`].
    fn build(acc: &AccuracyCfg, _params: &ParamSet) -> Self {
        SubpopCardinalityGT {
            label_column: 0,
            max_probes: acc.max_probes,
        }
    }
}

impl<S: Accumulator> GroundTruthCalculator<S> for SubpopRankErrorGT
where
    Self: GroundTruth<S>,
{
    /// Column 0, and `max_probes` caps groups instead of keys: this comparator
    /// issues 101 estimate calls per group, so the cap bites much sooner here
    /// than it does for the two counting comparators.
    fn build(acc: &AccuracyCfg, _params: &ParamSet) -> Self {
        SubpopRankErrorGT {
            label_column: 0,
            max_probes: acc.max_probes,
        }
    }
}

impl<S: Accumulator> GroundTruthCalculator<S> for RankErrorGT
where
    Self: GroundTruth<S>,
{
    fn build(acc: &AccuracyCfg, _params: &ParamSet) -> Self {
        RankErrorGT {
            record_calls: acc.record_query_calls,
        }
    }
}

impl<S: Accumulator> GroundTruthCalculator<S> for RelativeErrorGT
where
    Self: GroundTruth<S>,
{
    fn build(acc: &AccuracyCfg, _params: &ParamSet) -> Self {
        RelativeErrorGT {
            record_calls: acc.record_query_calls,
        }
    }
}

impl<S: Accumulator> GroundTruthCalculator<S> for TopkGT
where
    Self: GroundTruth<S>,
{
    /// Scores against the same `k` the sketch was built with — a different prefix
    /// would measure the mismatch, not the sketch. Infallible because the timed
    /// half runs first, so an unreadable `k` has already failed the build.
    fn build(_acc: &AccuracyCfg, params: &ParamSet) -> Self {
        TopkGT {
            k: params
                .parse::<TopkParams>()
                .expect("the row built, so its params parse")
                .k,
        }
    }
}

// ---------- the four ways a row runs ----------

/// Timed measurement, plus accuracy scored against `G` when `--accuracy` is on.
fn run_scored<S, G>(
    cfg: &BenchConfig,
    spec: &WorkloadSpec,
    params: &ParamSet,
    acc: &AccuracyCfg,
    _width: Numeric,
) -> Result<Vec<BenchReport>, RunError>
where
    S: Accumulator + InitSketch + BenchImpl + MemoryFootprint,
    S::Item: BenchItem,
    G: GroundTruthCalculator<S>,
{
    let mut reports = cell::run_cell::<S>(cfg, spec, params)?;
    if acc.enabled {
        let gt = G::build(acc, params);
        reports.extend(cell::score_cell::<S, G>(cfg, spec, params, &gt)?);
    }
    Ok(reports)
}

/// An ordered quantile algorithm (KLL, DDSketch): the row names both widths and
/// the caller's [`Numeric`] picks one. The only place a runtime value still
/// selects an item type.
fn run_ordered<Si, Sf, G>(
    cfg: &BenchConfig,
    spec: &WorkloadSpec,
    params: &ParamSet,
    acc: &AccuracyCfg,
    width: Numeric,
) -> Result<Vec<BenchReport>, RunError>
where
    Si: Accumulator<Item = i64> + InitSketch + BenchImpl + MemoryFootprint,
    Sf: Accumulator<Item = f64> + InitSketch + BenchImpl + MemoryFootprint,
    G: GroundTruthCalculator<Si> + GroundTruthCalculator<Sf>,
{
    match width {
        Numeric::I64 => run_scored::<Si, G>(cfg, spec, params, acc, width),
        Numeric::F64 => run_scored::<Sf, G>(cfg, spec, params, acc, width),
    }
}

/// A row with no query capability: timed only, no ground truth, nothing to score.
fn run_plain<S>(
    cfg: &BenchConfig,
    spec: &WorkloadSpec,
    params: &ParamSet,
    _acc: &AccuracyCfg,
    _width: Numeric,
) -> Result<Vec<BenchReport>, RunError>
where
    S: Accumulator + InitSketch + BenchImpl + MemoryFootprint,
    S::Item: BenchItem,
{
    cell::run_cell::<S>(cfg, spec, params)
}

/// A parallel-insert row: built with the worker count, so not an `InitSketch`.
fn run_parallel<S>(
    cfg: &BenchConfig,
    spec: &WorkloadSpec,
    params: &ParamSet,
    _acc: &AccuracyCfg,
    _width: Numeric,
) -> Result<Vec<BenchReport>, RunError>
where
    S: ParallelInit + BenchImpl + MemoryFootprint,
    S::Item: BenchItem,
{
    cell::run_cell_parallel::<S>(cfg, spec, params)
}

// ---------- the four row constructors ----------
// Each reads `S::ALGORITHM` / `S::IMPL` off the type and fixes `scores_accuracy`.
// `const fn`, so `ROWS` stays `const` and a bad row fails at compile time.

const fn scored<S, G>(description: &'static str) -> Row
where
    S: Accumulator + InitSketch + BenchImpl + MemoryFootprint,
    S::Item: BenchItem,
    G: GroundTruthCalculator<S>,
{
    Row {
        algorithm: S::ALGORITHM,
        impl_name: S::IMPL,
        description,
        scores_accuracy: true,
        picks_width: false,
        takes_columns: <S::Item as BenchItem>::TAKES_COLUMNS,
        run: run_scored::<S, G>,
    }
}

const fn ordered<Si, Sf, G>(description: &'static str) -> Row
where
    Si: Accumulator<Item = i64> + InitSketch + BenchImpl + MemoryFootprint,
    Sf: Accumulator<Item = f64> + InitSketch + BenchImpl + MemoryFootprint,
    G: GroundTruthCalculator<Si> + GroundTruthCalculator<Sf>,
{
    Row {
        // Both halves are the same row; the i64 one names it.
        algorithm: Si::ALGORITHM,
        impl_name: Si::IMPL,
        description,
        scores_accuracy: true,
        picks_width: true,
        takes_columns: <Si::Item as BenchItem>::TAKES_COLUMNS,
        run: run_ordered::<Si, Sf, G>,
    }
}

const fn plain<S>(description: &'static str) -> Row
where
    S: Accumulator + InitSketch + BenchImpl + MemoryFootprint,
    S::Item: BenchItem,
{
    Row {
        algorithm: S::ALGORITHM,
        impl_name: S::IMPL,
        description,
        scores_accuracy: false,
        picks_width: false,
        takes_columns: <S::Item as BenchItem>::TAKES_COLUMNS,
        run: run_plain::<S>,
    }
}

const fn parallel_row<S>(description: &'static str) -> Row
where
    S: ParallelInit + BenchImpl + MemoryFootprint,
    S::Item: BenchItem,
{
    Row {
        algorithm: S::ALGORITHM,
        impl_name: S::IMPL,
        description,
        scores_accuracy: false,
        picks_width: false,
        takes_columns: <S::Item as BenchItem>::TAKES_COLUMNS,
        run: run_parallel::<S>,
    }
}

// ---------- the catalog ----------

/// Every `(algorithm, impl)` this crate exposes. Adding one is one line here plus
/// the wrapper it names; nothing else in this file changes.
pub const ROWS: &[Row] = &[
    // -------- HLL (cardinality) --------
    scored::<hll::HllOxide, CardinalityGT>("sketch_oxide::cardinality::HyperLogLog"),
    scored::<hll::HllDatasketches, CardinalityGT>("datasketches::hll::HllSketch (Hll8)"),
    scored::<hll::HllLib, CardinalityGT>(
        "asap_sketchlib::HyperLogLog<Classic> (P14): O(m) estimate",
    ),
    scored::<hll::HllLibHip, CardinalityGT>("asap_sketchlib::HyperLogLogHIP (P14): O(1) estimate"),
    scored::<polars::PolarsCardinality, CardinalityGT>("polars exact: DataFrame.n_unique()"),
    parallel_row::<parallel::ParallelHllFastPath>("asap HLL ErtlMLE, FastPath, parallel insert"),
    // -------- KLL (quantile, rank error) --------
    // Two libraries × two query paths. Both libraries offer both paths, so the
    // path belongs on the impl axis: one row per library would have compared
    // libraries and query strategies in the same column.
    ordered::<kll::KllOxidePerCall<i64>, kll::KllOxidePerCall<f64>, RankErrorGT>(
        "sketch_oxide KllSketch: quantile() per call",
    ),
    ordered::<kll::KllOxideCdf<i64>, kll::KllOxideCdf<f64>, RankErrorGT>(
        "sketch_oxide KllSketch: cdf() built in prepare",
    ),
    ordered::<kll::KllLibPerCall<i64>, kll::KllLibPerCall<f64>, RankErrorGT>(
        "asap_sketchlib::KLL: quantile() per call",
    ),
    ordered::<kll::KllLibCdf<i64>, kll::KllLibCdf<f64>, RankErrorGT>(
        "asap_sketchlib::KLL: cdf() built in prepare",
    ),
    scored::<polars::PolarsQuantileKll, RankErrorGT>("polars exact: 101-point quantile grid"),
    // -------- CMS (frequency) --------
    scored::<cms::CmsOxide, FrequencyGT>("sketch_oxide::frequency::CountMinSketch"),
    scored::<cms::CmsDatasketches, FrequencyGT>("datasketches::countmin::CountMinSketch"),
    scored::<cms::CmsLibFixedmatrixCustomFast, FrequencyGT>(
        "asap CMS, custom FixedMatrix (5x65538), FastPath",
    ),
    scored::<cms::CmsLibFixedmatrixFast, FrequencyGT>("asap CMS, FixedMatrix (5x2048), FastPath"),
    scored::<cms::CmsLibFixedmatrixFast32k, FrequencyGT>(
        "asap CMS, FixedMatrix (5x32768), FastPath",
    ),
    scored::<cms::CmsLibVector2dFast, FrequencyGT>("asap CMS, Vector2D, FastPath"),
    scored::<cms::CmsLibVector2dRegular, FrequencyGT>("asap CMS, Vector2D, RegularPath"),
    scored::<polars::PolarsFrequencyCms, FrequencyGT>("polars exact: group_by(v).agg(len)"),
    parallel_row::<parallel::ParallelCmsFastPath>("asap CMS, FastPath, parallel insert on M5x32K"),
    // -------- CountSketch (frequency) --------
    scored::<countsketch::CsOxide, FrequencyGT>("sketch_oxide::frequency::CountSketch"),
    scored::<countsketch::CsLibFixedmatrixFast, FrequencyGT>(
        "asap Count, FixedMatrix (5x2048), FastPath",
    ),
    scored::<countsketch::CsLibFixedmatrixFast32k, FrequencyGT>(
        "asap Count, FixedMatrix (5x32768), FastPath",
    ),
    scored::<countsketch::CsLibVector2dFast, FrequencyGT>("asap Count, Vector2D, FastPath"),
    scored::<countsketch::CsLibVector2dRegular, FrequencyGT>("asap Count, Vector2D, RegularPath"),
    scored::<polars::PolarsFrequencyCs, FrequencyGT>("polars exact: group_by(v).agg(len)"),
    parallel_row::<parallel::ParallelCsFastPath>("asap Count, FastPath, parallel insert on M5x32K"),
    // -------- DDSketch (quantile, relative error) --------
    ordered::<dd::DdLib<i64>, dd::DdLib<f64>, RelativeErrorGT>(
        "asap_sketchlib::DDSketch (relative-error quantile)",
    ),
    scored::<polars::PolarsQuantileDd, RelativeErrorGT>("polars exact: 101-point quantile grid"),
    // -------- Top-k (counter array + size-k candidate tracker) --------
    scored::<topk::TopKHeap<cms::CmsOxide>, TopkGT>(
        "sketch_oxide CMS + size-k heap (top-k on the insert path)",
    ),
    scored::<topk::TopKHeap<countsketch::CsOxide>, TopkGT>(
        "sketch_oxide CountSketch + size-k heap",
    ),
    scored::<polars::PolarsTopK, TopkGT>("polars exact: group_by(v).agg(len) sorted, top k"),
    // -------- Elastic (heavy-hitter; no query capability, throughput-only) --------
    plain::<elastic::ElasticLib>("asap_sketchlib::Elastic<DefaultXxHasher>"),
    plain::<elastic::ElasticOxide>("sketch_oxide::frequency::ElasticSketch"),
    // -------- Hydra (per-subpopulation statistics over labelled records) --------
    // One algorithm per cell type, because the cell decides which statistic the
    // grid answers and each is scored by a different comparator. See the module
    // header in `wrappers/hydra.rs`.
    scored::<hydra::HydraCms, SubpopFrequencyGT>(
        "asap_sketchlib::Hydra over Count-Min cells (subpopulation frequency)",
    ),
    scored::<polars::PolarsSubpopFrequency, SubpopFrequencyGT>(
        "polars exact: group_by(subset, v).agg(len) over every label subset",
    ),
    scored::<hydra::HydraHll, SubpopCardinalityGT>(
        "asap_sketchlib::Hydra over HyperLogLog cells (subpopulation cardinality)",
    ),
    scored::<polars::PolarsSubpopCardinality, SubpopCardinalityGT>(
        "polars exact: group_by(subset).agg(v.n_unique()) over every label subset",
    ),
    scored::<hydra::HydraKll, SubpopRankErrorGT>(
        "asap_sketchlib::Hydra over KLL cells (subpopulation quantile)",
    ),
    scored::<polars::PolarsSubpopQuantile, SubpopRankErrorGT>(
        "polars exact: sorted values per label subset, quantile by rank",
    ),
    // -------- Nitro / UnivMon (no query capability; throughput-only) --------
    plain::<nitro::NitroLib>("asap_sketchlib::NitroBatch<Vector2D<u32>>"),
    plain::<nitro::NitroOxide>("sketch_oxide::frequency::NitroSketch<CountMinSketch>"),
    plain::<univmon::UnivMonLib>("asap_sketchlib::UnivMon"),
    plain::<univmon::UnivMonOxide>("sketch_oxide::universal::UnivMon"),
];

// ---------- what the frontend asks ----------

fn find(algorithm: &str, impl_name: &str) -> Option<&'static Row> {
    ROWS.iter()
        .find(|r| r.algorithm == algorithm && r.impl_name == impl_name)
}

pub fn list() -> Vec<String> {
    ROWS.iter()
        .map(|r| format!("{:12} {:28} {}", r.algorithm, r.impl_name, r.description))
        .collect()
}

pub fn algorithm_exists(algorithm: &str) -> bool {
    ROWS.iter().any(|r| r.algorithm == algorithm)
}

/// Does `--accuracy` score this row? `None` if the row is unknown.
pub fn scores_accuracy(algorithm: &str, impl_name: &str) -> Option<bool> {
    find(algorithm, impl_name).map(|r| r.scores_accuracy)
}

/// Parse the single `--config` point for an algorithm, checking the algorithm exists.
pub fn config_point(algorithm: &str, spec: &str) -> Result<ParamSet> {
    if !algorithm_exists(algorithm) {
        anyhow::bail!("unknown sketch algorithm: {algorithm}");
    }
    ParamSet::single(algorithm, spec).map_err(Into::into)
}

/// Resolve `(algorithm, impl)` to a concrete measurement and run it. The timed
/// half is always run; the accuracy half only when `acc.enabled`.
pub fn run(
    algorithm: &str,
    impl_name: &str,
    cfg: &BenchConfig,
    spec: &WorkloadSpec,
    params: &ParamSet,
    acc: &AccuracyCfg,
    width: Numeric,
) -> Result<Vec<BenchReport>> {
    let row = find(algorithm, impl_name)
        .ok_or_else(|| anyhow::anyhow!("no impl '{impl_name}' for algorithm '{algorithm}'"))?;
    // Asked for a width this row's type cannot be built at — answerable from
    // the catalog, before a single item is generated.
    if width == Numeric::F64 && !row.picks_width {
        anyhow::bail!("{algorithm}/{impl_name} runs over i64 only; drop --dtype f64");
    }
    Ok((row.run)(cfg, spec, params, acc, width)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::params::{
        CmsParams, CountSketchParams, DdParams, ElasticParams, HllParams, HydraCmsParams,
        HydraHllParams, HydraKllParams, KllParams, NitroParams, SketchParams, TopkParams,
        UnivMonParams,
    };
    use std::collections::BTreeSet;

    /// One buildable config per algorithm, from each params type's own
    /// `canonical()`. The `panic!` arm is what makes a newly added algorithm
    /// show up here rather than silently skipping the tests below.
    fn canonical_params(algorithm: &str) -> ParamSet {
        match algorithm {
            "hll" => ParamSet::of(&HllParams::canonical()),
            "kll" => ParamSet::of(&KllParams::canonical()),
            "cms" => ParamSet::of(&CmsParams::canonical()),
            "countsketch" => ParamSet::of(&CountSketchParams::canonical()),
            "dd" => ParamSet::of(&DdParams::canonical()),
            "elastic" => ParamSet::of(&ElasticParams::canonical()),
            "nitro" => ParamSet::of(&NitroParams::canonical()),
            "hydra-cms" => ParamSet::of(&HydraCmsParams::canonical()),
            "hydra-hll" => ParamSet::of(&HydraHllParams::canonical()),
            "hydra-kll" => ParamSet::of(&HydraKllParams::canonical()),
            "topk" => ParamSet::of(&TopkParams::canonical()),
            "univmon" => ParamSet::of(&UnivMonParams::canonical()),
            other => panic!("no canonical params known for algorithm '{other}'"),
        }
    }

    /// The one way two rows can still collide: distinct types declaring the same
    /// `IMPL` under the same algorithm. [`find`] takes the first, so the second is
    /// dead. Not a compile error — the strings come from two different types.
    #[test]
    fn algorithm_impl_pairs_are_unique() {
        let mut seen = BTreeSet::new();
        for r in ROWS {
            assert!(
                seen.insert((r.algorithm, r.impl_name)),
                "duplicate row {}/{}",
                r.algorithm,
                r.impl_name
            );
        }
    }

    /// A small workload spec, enough for any row to build and ingest. The item
    /// type is not named here — each row materialises its own
    /// `Accumulator::Item`.
    fn smoke_spec() -> WorkloadSpec {
        WorkloadSpec::Generated(column(64, 256, 1))
    }

    fn column(cardinality: u64, size: usize, seed: u64) -> aqpbm_core::GenSpec {
        aqpbm_core::GenSpec {
            shape: aqpbm_core::Shape::Keys {
                cardinality,
                dist: aqpbm_core::Distribution::Uniform,
            },
            size,
            seed,
            string: None,
        }
    }

    /// The same, for the rows whose item is a record: two label columns and a
    /// value column. A row states which of the two it wants through
    /// `Row::takes_columns`, so neither is guessed here.
    fn smoke_columns_spec() -> WorkloadSpec {
        WorkloadSpec::Columns(vec![column(8, 256, 1), column(4, 256, 2), column(32, 256, 3)])
    }

    /// The spec shape `row` can actually ingest.
    fn spec_for(row: &Row) -> WorkloadSpec {
        if row.takes_columns {
            smoke_columns_spec()
        } else {
            smoke_spec()
        }
    }

    fn smoke_cfg() -> (BenchConfig, AccuracyCfg) {
        (
            BenchConfig {
                runs: 1,
                warmup_runs: 0,
                ..Default::default()
            },
            AccuracyCfg {
                enabled: false,
                max_probes: 0,
                record_query_calls: false,
            },
        )
    }

    /// Every row actually builds and ingests — the part the types cannot state.
    /// A row *is* its runner, so there is no `_` bail to fall into and no strings
    /// for the list and the dispatch to disagree about.
    #[test]
    fn every_catalog_entry_runs() {
        let (cfg, acc) = smoke_cfg();
        for r in ROWS {
            // Canonical, not `empty`: every algorithm's params have required
            // fields, so `empty` built only the 5 polars rows that ignore
            // their config — the other 31 were "checked" without ever running.
            let params = canonical_params(r.algorithm);
            let got = run(
                r.algorithm,
                r.impl_name,
                &cfg,
                &spec_for(r),
                &params,
                &acc,
                Numeric::I64,
            );
            // Fixed-matrix rows refuse an off-shape config (a `RunError::Build`
            // surfaced as an error); every other row runs.
            if let Ok(reports) = &got {
                for report in reports {
                    assert_eq!(
                        (report.sketch.as_str(), report.impl_name.as_str()),
                        (r.algorithm, r.impl_name),
                        "row {}/{} emits records labelled {}/{}",
                        r.algorithm,
                        r.impl_name,
                        report.sketch,
                        report.impl_name,
                    );
                }
            }
        }
    }

    /// An algorithm's rows are only comparable if asked the same question, so they
    /// must agree on which configs are answerable — one row silently accepting a
    /// config its peers reject scores a different experiment.
    #[test]
    fn topk_rows_accept_and_reject_the_same_configs() {
        let spec = smoke_spec();
        let (cfg, acc) = smoke_cfg();
        for (cfg_spec, buildable) in [
            ("rows=5 cols=2048 k=5", true),
            ("rows=5 cols=2048 kk=5", false), // misspelled `k`
            ("rows=5 cols=2048 k=0", false),  // a top-k of nothing
            ("rows=5 cols=2048", false),      // no `k` at all
        ] {
            let params = config_point("topk", cfg_spec).unwrap();
            for r in ROWS.iter().filter(|r| r.algorithm == "topk") {
                let got = run(
                    r.algorithm,
                    r.impl_name,
                    &cfg,
                    &spec,
                    &params,
                    &acc,
                    Numeric::I64,
                );
                assert_eq!(
                    got.is_ok(),
                    buildable,
                    "topk/{} disagrees with the algorithm on `{cfg_spec}`: {got:?}",
                    r.impl_name
                );
            }
        }
    }
}
