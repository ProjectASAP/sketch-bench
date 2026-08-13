//! This bundle's [`REGISTRY`]: every measurement `sketch-bench` publishes, and
//! nothing else. What a registration *is* lives in [`aqpbm_core::registry`], so
//! a second bundle publishes its own table without depending on this crate.

use anyhow::Result;
use aqpbm_core::accuracy::cardinality::CardinalityGT;
use aqpbm_core::accuracy::frequency::FrequencyGT;
use aqpbm_core::accuracy::quantile::{RankErrorGT, RelativeErrorGT};
use aqpbm_core::accuracy::subpopulation::{
    SubpopCardinalityGT, SubpopFrequencyGT, SubpopRankErrorGT,
};
use aqpbm_core::accuracy::topk::TopkGT;
use aqpbm_core::cell::WorkloadSpec;
use aqpbm_core::registry::{ordered, parallel_row, plain, scored, Registration};
use aqpbm_core::runner::{BenchConfig, BenchReport};

use asap_sketchlib::{HllBucketListP12, HllBucketListP14, HllBucketListP16};

use crate::params::ParamSet;
use crate::wrappers::fixed_matrix::fixed_matrix_row;
use crate::wrappers::hll::lib_hll;
use crate::wrappers::{
    cms, countsketch, dd, elastic, hll, hydra, kll, nitro, parallel, polars, topk, univmon,
};

/// Re-exported so a caller names one path. The type is the core's.
pub use aqpbm_core::registry::Numeric;

/// Every measurement this crate exposes. Adding one is one line here plus the
/// wrapper it names; scoring a sketch a second way is one more line.
pub const REGISTRY: &[Registration] = &[
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
    fixed_matrix_row::<cms::CmsFixedMatrix, crate::params::CmsParams>(
        "asap CMS, FixedMatrix (shape baked at compile time), FastPath",
    ),
    scored::<cms::CmsLibVector2dFast, FrequencyGT>("asap CMS, Vector2D, FastPath"),
    scored::<cms::CmsLibVector2dRegular, FrequencyGT>("asap CMS, Vector2D, RegularPath"),
    parallel_row::<parallel::ParallelCmsFastPath>("asap CMS, FastPath, parallel insert on M5x32K"),
    // -------- CountSketch (frequency) --------
    scored::<countsketch::CsOxide, FrequencyGT>("sketch_oxide::frequency::CountSketch"),
    scored::<polars::PolarsFrequencyCs, FrequencyGT>("polars exact: group_by(v).agg(len)"),
    fixed_matrix_row::<countsketch::CsFixedMatrix, crate::params::CountSketchParams>(
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
    // One algorithm per cell type: the cell decides which statistic the grid
    // answers. See the module header in `wrappers/hydra.rs`.
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
// This bundle's table bound into the core's lookups: the logic belongs to every
// bundle, and only which table to look in is this one's.

/// One line per registration in this bundle, grouped by family.
pub fn list() -> Vec<String> {
    aqpbm_core::registry::list(REGISTRY)
}

pub fn algorithm_exists(algorithm: &str) -> bool {
    aqpbm_core::registry::algorithm_exists(REGISTRY, algorithm)
}

/// The family an algorithm belongs to, for the record's `family` field.
pub fn family_of(algorithm: &str) -> Option<&'static str> {
    aqpbm_core::registry::family_of(REGISTRY, algorithm)
}

/// Can a ground truth score this `(algorithm, impl)`? `None` if it is unknown.
pub fn scores_accuracy(algorithm: &str, impl_name: &str) -> Option<bool> {
    aqpbm_core::registry::scores_accuracy(REGISTRY, algorithm, impl_name)
}

/// Parse the single `--config` point for an algorithm, checking it exists.
pub fn config_point(algorithm: &str, spec: &str) -> Result<ParamSet> {
    aqpbm_core::registry::config_point(REGISTRY, algorithm, spec)
}

/// Resolve `(algorithm, impl)` to a concrete measurement and run it.
pub fn run(
    algorithm: &str,
    impl_name: &str,
    cfg: &BenchConfig,
    spec: &WorkloadSpec,
    params: &ParamSet,
    width: Numeric,
    ground_truth: Option<&str>,
) -> Result<Vec<BenchReport>> {
    aqpbm_core::registry::run(REGISTRY, algorithm, impl_name, cfg, spec, params, width, ground_truth)
}

/// Every ground truth an `(algorithm, impl)` is registered against.
pub fn ground_truths(algorithm: &str, impl_name: &str) -> Vec<&'static str> {
    aqpbm_core::registry::ground_truths(REGISTRY, algorithm, impl_name)
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

    /// One buildable config per family. The `panic!` arm is what makes a newly
    /// added family show up here rather than silently skipping the tests below.
    fn canonical_params(row: &Registration) -> ParamSet {
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

    /// Two registrations may share an `(algorithm, impl)` only by scoring
    /// differently: a full collision makes the second dead, since the lookup
    /// takes the first.
    #[test]
    fn registrations_are_unique() {
        let mut seen = BTreeSet::new();
        for r in REGISTRY {
            assert!(
                seen.insert((r.algorithm, r.impl_name, r.ground_truth)),
                "duplicate registration {}/{}/{:?}",
                r.algorithm,
                r.impl_name,
                r.ground_truth
            );
        }
    }

    /// An algorithm must sit in the family whose vocabulary it parses, or
    /// `--config` is checked against knobs the row does not take. `ALGORITHM` is
    /// hand-written, so this is the one identity the types do not guarantee.
    #[test]
    fn every_algorithm_belongs_to_its_family() {
        for r in REGISTRY {
            assert!(
                crate::params::in_family(r.algorithm, r.family),
                "{}/{} declares family '{}', which its algorithm is not in",
                r.algorithm,
                r.impl_name,
                r.family
            );
        }
    }

    /// The impl axis carries the library and nothing else. A storage backend or
    /// a code path here makes the column mean two things, so "which library is
    /// faster" stops being answerable by grouping on it.
    #[test]
    fn impl_names_are_library_names() {
        const LIBRARIES: [&str; 4] = ["oxide", "datasketches", "lib", "polars"];
        for r in REGISTRY {
            assert!(
                LIBRARIES.contains(&r.impl_name),
                "{}/{}: '{}' is not a library name; a structural variant \
                 belongs in the algorithm",
                r.algorithm,
                r.impl_name,
                r.impl_name
            );
        }
    }

    /// Every family is reachable by name.
    #[test]
    fn every_family_is_reachable_by_name() {
        for r in REGISTRY {
            assert_eq!(
                family_of(r.algorithm),
                Some(r.family),
                "{} does not resolve to its own family",
                r.algorithm
            );
        }
        assert_eq!(family_of("no-such-algorithm"), None);
    }

    /// Enough for any row to build and ingest. The item type is not named here —
    /// each row materialises its own `Accumulator::Item`.
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
            depends_on: None,
        }
    }

    /// The same for record items: two label columns and a value column.
    fn smoke_columns_spec() -> WorkloadSpec {
        WorkloadSpec::Columns(vec![column(8, 256, 1), column(4, 256, 2), column(32, 256, 3)])
    }

    /// The spec shape this can actually ingest.
    fn spec_for(row: &Registration) -> WorkloadSpec {
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

    /// Every registration actually builds and ingests — the part types cannot
    /// state.
    #[test]
    fn every_registration_runs() {
        let cfg = smoke_cfg();
        for r in REGISTRY {
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
                        "{}/{} emits records labelled {}/{}",
                        r.algorithm,
                        r.impl_name,
                        report.sketch,
                        report.impl_name,
                    );
                }
            }
        }
    }

    /// An algorithm's registrations must agree on which configs are answerable:
    /// one accepting what its peers reject scores a different experiment.
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
            for r in REGISTRY.iter().filter(|r| r.family == "topk") {
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
