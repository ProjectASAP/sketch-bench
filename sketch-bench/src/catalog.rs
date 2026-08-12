//! This bundle's table: every `(algorithm, impl)` `sketch-bench` exposes, and
//! the two dispatch shapes whose row type is chosen by a construction
//! parameter rather than sized by one.
//!
//! What a row *is* lives in [`aqpbm_core::catalog`], not here, and that split
//! is the point: a second bundle depends on the core and publishes its own
//! [`ROWS`], without depending on this crate or on the libraries it wraps. The
//! lookups below are this bundle's table bound into the core's, so a frontend
//! addressing one bundle keeps the call it already had.

use anyhow::Result;
use aqpbm_core::accumulator::Accumulator;
use aqpbm_core::accuracy::cardinality::CardinalityGT;
use aqpbm_core::accuracy::frequency::FrequencyGT;
use aqpbm_core::accuracy::quantile::{RankErrorGT, RelativeErrorGT};
use aqpbm_core::accuracy::subpopulation::{
    SubpopCardinalityGT, SubpopFrequencyGT, SubpopRankErrorGT,
};
use aqpbm_core::accuracy::topk::TopkGT;
use aqpbm_core::accuracy::FrequencyOps;
use aqpbm_core::catalog::{
    ordered, parallel_row, plain, run_scored, scored, GroundTruthCalculator, Row,
};
use aqpbm_core::cell::{BenchItem, RunError, WorkloadSpec};
use aqpbm_core::init::{BenchImpl, BuildError, InitSketch};
use aqpbm_core::memory_footprint::MemoryFootprint;
use aqpbm_core::runner::{BenchConfig, BenchReport};

use asap_sketchlib::{
    DefaultXxHasher, FastPathHasher, HllBucketListP12, HllBucketListP14, HllBucketListP16,
    MatrixStorage,
};

use crate::params::{HllParams, ParamSet};
use crate::wrappers::{
    cms, countsketch, dd, elastic, fixed_matrix, hll, hydra, kll, nitro, parallel, polars, topk,
    univmon,
};

/// Re-exported so a caller addressing this bundle names one path. The type is
/// the core's: a width is a property of a row, not of a bundle.
pub use aqpbm_core::catalog::Numeric;

/// A row whose `(rows, cols)` selects a *type* rather than sizing a field.
///
/// `impl_fixed_matrix!` bakes the dimensions into the storage type, which is
/// what the row exists to price, so the shape is a monomorphisation and the
/// dispatch is what turns two runtime integers back into one. Same shape as
/// [`run_lib_hll`], and the same reason.
/// Both fixed-matrix rows are frequency rows, so the comparator is named here
/// instead of being a type parameter. That is not a shortcut: a comparator
/// parameter would have to hold for *every* storage type the dispatch can
/// select, which is a bound over all `M` and not something a caller can state.
/// Requiring the row to be `FrequencyOps` at every shape says the same thing in
/// a form the compiler accepts, and [`FrequencyGT`] follows from it.
fn run_fixed_matrix<W: FixedMatrixRow>(
    cfg: &BenchConfig,
    spec: &WorkloadSpec,
    params: &ParamSet,
    width: Numeric,
) -> Result<Vec<BenchReport>, RunError> {
    let (rows, cols) = W::shape(params)?;
    let visitor = RunFixedMatrix::<W> {
        cfg,
        spec,
        params,
        width,
        _row: std::marker::PhantomData,
    };
    fixed_matrix::with_fixed_matrix(rows, cols, visitor).unwrap_or_else(|| {
        Err(RunError::Build(BuildError(fixed_matrix::unsupported_shape(
            W::ALGORITHM,
            rows,
            cols,
        ))))
    })
}

/// The half of a fixed-matrix row that does not depend on the storage type:
/// which algorithm it is, and how to read its shape out of a `ParamSet`.
/// Implemented once per sketch type, in the wrapper that owns it.
pub trait FixedMatrixRow {
    const ALGORITHM: &'static str;
    /// The row's concrete type at storage `M`, which is what actually runs.
    type At<
        M: MatrixStorage<Counter = i32>
            + FastPathHasher<DefaultXxHasher>
            + Default
            + Clone
            + 'static,
    >: Accumulator<Item = i64>
        + InitSketch
        + BenchImpl
        + MemoryFootprint
        + FrequencyOps<Key = i64>;
    fn shape(params: &ParamSet) -> Result<(usize, usize), RunError>;
}

/// Carries the run's arguments into the monomorphisation the shape selected.
struct RunFixedMatrix<'a, W> {
    cfg: &'a BenchConfig,
    spec: &'a WorkloadSpec,
    params: &'a ParamSet,
    width: Numeric,
    _row: std::marker::PhantomData<W>,
}

impl<W: FixedMatrixRow> fixed_matrix::FixedMatrixVisitor for RunFixedMatrix<'_, W> {
    type Out = Result<Vec<BenchReport>, RunError>;

    fn visit<M>(self) -> Self::Out
    where
        M: MatrixStorage<Counter = i32>
            + FastPathHasher<DefaultXxHasher>
            + Default
            + Clone
            + 'static,
    {
        run_scored::<W::At<M>, FrequencyGT>(self.cfg, self.spec, self.params, self.width)
    }
}

/// A row whose construction parameter selects a *type* rather than a field.
/// `asap_sketchlib` puts the HLL register count in the storage type, so `lg_k`
/// picks a monomorphisation and this dispatch is what turns a runtime value
/// back into one. Same shape as [`run_ordered`], and for the same reason: an
/// enum inside the wrapper would put a branch in `update`, on rows whose whole
/// purpose is to price that insert.
fn run_lib_hll<S12, S14, S16, G>(
    cfg: &BenchConfig,
    spec: &WorkloadSpec,
    params: &ParamSet,
    width: Numeric,
) -> Result<Vec<BenchReport>, RunError>
where
    S12: Accumulator<Item = i64> + InitSketch + BenchImpl + MemoryFootprint,
    S14: Accumulator<Item = i64> + InitSketch + BenchImpl + MemoryFootprint,
    S16: Accumulator<Item = i64> + InitSketch + BenchImpl + MemoryFootprint,
    G: GroundTruthCalculator<S12> + GroundTruthCalculator<S14> + GroundTruthCalculator<S16>,
{
    let p: HllParams = params.parse().map_err(BuildError::from)?;
    match p.lg_k {
        12 => run_scored::<S12, G>(cfg, spec, params, width),
        14 => run_scored::<S14, G>(cfg, spec, params, width),
        16 => run_scored::<S16, G>(cfg, spec, params, width),
        other => Err(RunError::Build(hll::unsupported_precision(other))),
    }
}

/// The three precisions are one row: they are one algorithm at one impl, and
/// `lg_k` is the knob that moves between them. `S14` names the row, the way the
/// `i64` half names an [`ordered`] one.
const fn lib_hll<S12, S14, S16, G>(description: &'static str) -> Row
where
    S12: Accumulator<Item = i64> + InitSketch + BenchImpl + MemoryFootprint,
    S14: Accumulator<Item = i64> + InitSketch + BenchImpl + MemoryFootprint,
    S16: Accumulator<Item = i64> + InitSketch + BenchImpl + MemoryFootprint,
    G: GroundTruthCalculator<S12> + GroundTruthCalculator<S14> + GroundTruthCalculator<S16>,
{
    Row {
        family: S14::FAMILY,
        algorithm: S14::ALGORITHM,
        impl_name: S14::IMPL,
        description,
        scores_accuracy: true,
        picks_width: false,
        takes_columns: <S14::Item as BenchItem>::TAKES_COLUMNS,
        run: run_lib_hll::<S12, S14, S16, G>,
        comparators: &[(
            <G as GroundTruthCalculator<S12>>::NAME,
            run_lib_hll::<S12, S14, S16, G>,
        )],
    }
}

/// The shape-dispatching counterpart of [`scored`]: identity comes off the
/// row's `FixedMatrixRow` impl and its params type, because no one storage type
/// names a row that exists at every shape.
const fn fixed_matrix_row<W: FixedMatrixRow, P: crate::params::SketchParams>(
    description: &'static str,
) -> Row {
    Row {
        family: P::FAMILY,
        algorithm: W::ALGORITHM,
        impl_name: "lib",
        description,
        scores_accuracy: true,
        picks_width: false,
        takes_columns: false,
        run: run_fixed_matrix::<W>,
        // The fixed-matrix rows carry their comparator inside the generated
        // dispatch, so there is no calculator type here to name.
        comparators: &[],
    }
}
// ---------- the catalog ----------

/// Every `(algorithm, impl)` this crate exposes. Adding one is one line here plus
/// the wrapper it names; nothing else in this file changes.
pub const ROWS: &[Row] = &[
    // -------- HLL (cardinality) --------
    scored::<hll::HllOxide, CardinalityGT>("sketch_oxide::cardinality::HyperLogLog (lg_k 4..=18)"),
    scored::<hll::HllDatasketches, CardinalityGT>("datasketches::hll::HllSketch (Hll8)"),
    lib_hll::<
        hll::HllLib<HllBucketListP12>,
        hll::HllLib<HllBucketListP14>,
        hll::HllLib<HllBucketListP16>,
        CardinalityGT,
    >("asap_sketchlib::HyperLogLog<Classic>: O(m) estimate, lg_k in {12,14,16}"),
    scored::<polars::PolarsCardinality, CardinalityGT>("polars exact: DataFrame.n_unique()"),
    // -------- HLL, HIP estimator --------
    // Its own algorithm: the estimate is maintained on the insert path instead
    // of scanned at query time, so it is different arithmetic and a different
    // number, not a different implementation of one number.
    lib_hll::<
        hll::HllLibHip<HllBucketListP12>,
        hll::HllLibHip<HllBucketListP14>,
        hll::HllLibHip<HllBucketListP16>,
        CardinalityGT,
    >("asap_sketchlib::HyperLogLogHIP: O(1) estimate, lg_k in {12,14,16}"),
    // -------- HLL, parallel insert --------
    parallel_row::<parallel::ParallelHllFastPath>("asap HLL ErtlMLE, FastPath, parallel insert"),
    // -------- KLL (quantile, rank error) --------
    // Two query paths × two libraries. The paths answer differently, so the path
    // names the algorithm and each algorithm holds the two libraries against
    // each other. One row per library would have compared libraries and query
    // strategies in the same column.
    ordered::<kll::KllOxidePerCall<i64>, kll::KllOxidePerCall<f64>, RankErrorGT>(
        "sketch_oxide KllSketch: quantile() per call",
    ),
    ordered::<kll::KllLibPerCall<i64>, kll::KllLibPerCall<f64>, RankErrorGT>(
        "asap_sketchlib::KLL: quantile() per call, k in [8, 26602]",
    ),
    ordered::<kll::KllOxideCdf<i64>, kll::KllOxideCdf<f64>, RankErrorGT>(
        "sketch_oxide KllSketch: cdf() built in prepare",
    ),
    ordered::<kll::KllLibCdf<i64>, kll::KllLibCdf<f64>, RankErrorGT>(
        "asap_sketchlib::KLL: cdf() built in prepare, k in [8, 26602]",
    ),
    scored::<polars::PolarsQuantileKll, RankErrorGT>("polars exact: 101-point quantile grid"),
    // -------- CMS (frequency) --------
    scored::<cms::CmsOxide, FrequencyGT>("sketch_oxide::frequency::CountMinSketch"),
    scored::<cms::CmsDatasketches, FrequencyGT>("datasketches::countmin::CountMinSketch"),
    scored::<polars::PolarsFrequencyCms, FrequencyGT>("polars exact: group_by(v).agg(len)"),
    fixed_matrix_row::<cms::CmsFixedMatrixRow, crate::params::CmsParams>(
        "asap CMS, FixedMatrix (shape baked at compile time), FastPath",
    ),
    scored::<cms::CmsLibVector2dFast, FrequencyGT>("asap CMS, Vector2D, FastPath"),
    scored::<cms::CmsLibVector2dRegular, FrequencyGT>("asap CMS, Vector2D, RegularPath"),
    parallel_row::<parallel::ParallelCmsFastPath>("asap CMS, FastPath, parallel insert on M5x32K"),
    // -------- CountSketch (frequency) --------
    scored::<countsketch::CsOxide, FrequencyGT>("sketch_oxide::frequency::CountSketch"),
    scored::<polars::PolarsFrequencyCs, FrequencyGT>("polars exact: group_by(v).agg(len)"),
    fixed_matrix_row::<countsketch::CsFixedMatrixRow, crate::params::CountSketchParams>(
        "asap Count, FixedMatrix (shape baked at compile time), FastPath",
    ),
    scored::<countsketch::CsLibVector2dFast, FrequencyGT>("asap Count, Vector2D, FastPath"),
    scored::<countsketch::CsLibVector2dRegular, FrequencyGT>("asap Count, Vector2D, RegularPath"),
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
//
// This bundle's table bound into the core's lookups. One line each: the logic
// is the core's and belongs to every bundle, and the only thing that is this
// bundle's is which table to look in.

/// One line per `(algorithm, impl)` in this bundle, grouped by family.
pub fn list() -> Vec<String> {
    aqpbm_core::catalog::list(ROWS)
}

pub fn algorithm_exists(algorithm: &str) -> bool {
    aqpbm_core::catalog::algorithm_exists(ROWS, algorithm)
}

/// The family an algorithm belongs to, for the record's `family` field.
pub fn family_of(algorithm: &str) -> Option<&'static str> {
    aqpbm_core::catalog::family_of(ROWS, algorithm)
}

/// Can a comparator score this row? `None` if the row is unknown.
pub fn scores_accuracy(algorithm: &str, impl_name: &str) -> Option<bool> {
    aqpbm_core::catalog::scores_accuracy(ROWS, algorithm, impl_name)
}

/// Parse the single `--config` point for an algorithm, checking it exists.
pub fn config_point(algorithm: &str, spec: &str) -> Result<ParamSet> {
    aqpbm_core::catalog::config_point(ROWS, algorithm, spec)
}

/// Resolve `(algorithm, impl)` to a concrete measurement and run it.
pub fn run(
    algorithm: &str,
    impl_name: &str,
    cfg: &BenchConfig,
    spec: &WorkloadSpec,
    params: &ParamSet,
    width: Numeric,
    comparator: Option<&str>,
) -> Result<Vec<BenchReport>> {
    aqpbm_core::catalog::run(ROWS, algorithm, impl_name, cfg, spec, params, width, comparator)
}

/// Which comparators a row admits. `None` for an unknown row, so a frontend can
/// tell "no such row" from "that row is scored by nothing".
pub fn comparators(algorithm: &str, impl_name: &str) -> Option<Vec<&'static str>> {
    aqpbm_core::catalog::comparators(ROWS, algorithm, impl_name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use aqpbm_core::metrics::{MetricsMask, OperationMask};
    use crate::params::{
        CmsParams, CountSketchParams, DdParams, ElasticParams, HllParams, HydraCmsParams,
        HydraHllParams, HydraKllParams, KllParams, NitroParams, SketchParams, TopkParams,
        UnivMonParams,
    };
    use std::collections::BTreeSet;

    /// One buildable config per family, from each params type's own
    /// `canonical()`, tagged with the row's own algorithm. The `panic!` arm is
    /// what makes a newly added family show up here rather than silently
    /// skipping the tests below.
    fn canonical_params(row: &Row) -> ParamSet {
        let a = row.algorithm;
        match row.family {
            "hll" => ParamSet::of_algorithm(a, &HllParams::canonical()),
            "kll" => ParamSet::of_algorithm(a, &KllParams::canonical()),
            "cms" => ParamSet::of_algorithm(a, &CmsParams::canonical()),
            "countsketch" => ParamSet::of_algorithm(a, &CountSketchParams::canonical()),
            "dd" => ParamSet::of_algorithm(a, &DdParams::canonical()),
            "elastic" => ParamSet::of_algorithm(a, &ElasticParams::canonical()),
            "nitro" => ParamSet::of_algorithm(a, &NitroParams::canonical()),
            "hydra-cms" => ParamSet::of_algorithm(a, &HydraCmsParams::canonical()),
            "hydra-hll" => ParamSet::of_algorithm(a, &HydraHllParams::canonical()),
            "hydra-kll" => ParamSet::of_algorithm(a, &HydraKllParams::canonical()),
            "topk" => ParamSet::of_algorithm(a, &TopkParams::canonical()),
            "univmon" => ParamSet::of_algorithm(a, &UnivMonParams::canonical()),
            other => panic!("no canonical params known for family '{other}'"),
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

    /// A row's algorithm must sit in the family whose vocabulary it parses, or
    /// `--config` would be checked against knobs the row does not take. The
    /// `ALGORITHM` override is a hand-written string, so this is the one thing
    /// about a row's identity the type system does not already guarantee.
    #[test]
    fn every_algorithm_belongs_to_its_family() {
        for r in ROWS {
            assert!(
                crate::params::in_family(r.algorithm, r.family),
                "row {}/{} declares family '{}', which its algorithm is not in",
                r.algorithm,
                r.impl_name,
                r.family
            );
        }
    }

    /// The impl axis carries the library and nothing else. A storage backend, a
    /// code path or a query strategy in this column is the defect this naming
    /// exists to prevent: it makes the column mean two things at once, so
    /// "which library is faster" stops being answerable by grouping on it.
    #[test]
    fn impl_names_are_library_names() {
        const LIBRARIES: [&str; 4] = ["oxide", "datasketches", "lib", "polars"];
        for r in ROWS {
            assert!(
                LIBRARIES.contains(&r.impl_name),
                "row {}/{}: '{}' is not a library name; a structural variant \
                 belongs in the algorithm",
                r.algorithm,
                r.impl_name,
                r.impl_name
            );
        }
    }

    /// Every family has at least one row a `--config` sweep can walk, and every
    /// family's rows are reachable by name. A family whose every row were fixed
    /// shape would be a panel with no x-axis.
    #[test]
    fn every_family_is_reachable_by_name() {
        for r in ROWS {
            assert_eq!(
                family_of(r.algorithm),
                Some(r.family),
                "{} does not resolve to its own family",
                r.algorithm
            );
        }
        assert_eq!(family_of("no-such-algorithm"), None);
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

    fn smoke_cfg() -> BenchConfig {
        BenchConfig {
            runs: 1,
            warmup_runs: 0,
            // Only squares something measures. The default request reaches
            // `(insert, accuracy)` and `(query, latency)`, which nothing
            // does, and reaching an empty square is an error.
            metrics: MetricsMask::THROUGHPUT,
            operations: OperationMask::INSERT,
            ..Default::default()
        }
    }

    /// Every row actually builds and ingests — the part the types cannot state.
    /// A row *is* its runner, so there is no `_` bail to fall into and no strings
    /// for the list and the dispatch to disagree about.
    #[test]
    fn every_catalog_entry_runs() {
        let cfg = smoke_cfg();
        for r in ROWS {
            // Canonical, not `empty`: every family's params have required
            // fields, so `empty` builds nothing at all now that the exact
            // baselines parse their config too.
            let params = canonical_params(r);
            let got = run(
                r.algorithm,
                r.impl_name,
                &cfg,
                &spec_for(r),
                &params,
                Numeric::I64,
                None,
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
        let cfg = smoke_cfg();
        for (cfg_spec, buildable) in [
            ("rows=5 cols=2048 k=5", true),
            ("rows=5 cols=2048 kk=5", false), // misspelled `k`
            ("rows=5 cols=2048 k=0", false),  // a top-k of nothing
            ("rows=5 cols=2048", false),      // no `k` at all
        ] {
            for r in ROWS.iter().filter(|r| r.family == "topk") {
                let params = config_point(r.algorithm, cfg_spec).unwrap();
                let got = run(
                    r.algorithm,
                    r.impl_name,
                    &cfg,
                    &spec,
                    &params,
                    Numeric::I64,
                    None,
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
