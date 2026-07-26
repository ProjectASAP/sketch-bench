//! The catalog: the set of `(family, impl)` this crate exposes, and the
//! `match` that resolves one to a concrete sketch type + its oracle.
//!
//! This is the sketch domain's registry, so it lives here, not in the CLI: the
//! frontend does not know which sketches exist, how to build them, or which
//! oracle scores them — it asks this module. A future parallel bench crate
//! (`aqp-bench`) ships its own catalog, and the CLI multiplexes between them.
//!
//! There is no registry *table*: running a cell is a plain `cell::run_cell::<T>`
//! (timed) plus, when `--accuracy` is on, `cell::score_cell::<T, Oracle>`
//! (untimed). This module is where the runtime strings `("hll", "oxide")` bind
//! to those monomorphised calls. [`IMPLS`] is the human-facing list (for
//! `list-impls` and validation); [`run`] is the executable match. A test pins
//! that the two agree.

use anyhow::Result;
use aqpbm_core::sketch::Sketch;
use aqpbm_datagen::DType;

use crate::accuracy::cardinality::CardinalityGT;
use crate::accuracy::frequency::FrequencyGT;
use crate::accuracy::quantile::{RankErrorGT, RelativeErrorGT};
use crate::accuracy::topk::TopkGT;
use crate::accuracy::GroundTruth;
use crate::cell::{self, AccuracyCfg, DtypeMismatch, FromItems, Items, RunError};
use crate::init::{BenchImpl, InitSketch};
use crate::params::{ParamSet, TopkParams};
use crate::wrappers::{
    cms, countsketch, dd, elastic, hll, kll, nitro, parallel, polars, topk, univmon,
};
use crate::{BenchConfig, BenchReport};

/// One catalog entry: `(family, impl, description, scores_accuracy)`. Metadata
/// only — the executable binding is in [`run`]. `scores_accuracy` is `false`
/// for throughput-only rows (Elastic/Nitro/UnivMon stubs, parallel-insert).
pub const IMPLS: &[(&str, &str, &str, bool)] = &[
    // -------- HLL (cardinality) --------
    ("hll", "oxide", "sketch_oxide::cardinality::HyperLogLog", true),
    ("hll", "datasketches", "datasketches::hll::HllSketch (Hll8)", true),
    ("hll", "lib", "asap_sketchlib::HyperLogLog<Classic> (P14): O(m) estimate", true),
    ("hll", "lib-hip", "asap_sketchlib::HyperLogLogHIP (P14): O(1) estimate", true),
    ("hll", "polars", "polars exact: DataFrame.n_unique()", true),
    ("hll", "lib-fastpath-parallel", "asap HLL ErtlMLE, FastPath, parallel insert", false),
    // -------- KLL (quantile, rank error) --------
    ("kll", "oxide", "sketch_oxide::quantiles::KllSketch", true),
    ("kll", "lib", "asap_sketchlib::KLL", true),
    ("kll", "polars", "polars exact: 101-point quantile grid", true),
    // -------- CMS (frequency) --------
    ("cms", "oxide", "sketch_oxide::frequency::CountMinSketch", true),
    ("cms", "datasketches", "datasketches::countmin::CountMinSketch", true),
    ("cms", "lib-fixedmatrix-custom-fast", "asap CMS, custom FixedMatrix (5x65538), FastPath", true),
    ("cms", "lib-fixedmatrix-fast", "asap CMS, FixedMatrix (5x2048), FastPath", true),
    ("cms", "lib-fixedmatrix-fast-32k", "asap CMS, FixedMatrix (5x32768), FastPath", true),
    ("cms", "lib-vector2d-fast", "asap CMS, Vector2D, FastPath", true),
    ("cms", "lib-vector2d-regular", "asap CMS, Vector2D, RegularPath", true),
    ("cms", "polars", "polars exact: group_by(v).agg(len)", true),
    ("cms", "lib-fastpath-parallel", "asap CMS, FastPath, parallel insert on M5x32K", false),
    // -------- CountSketch (frequency) --------
    ("countsketch", "oxide", "sketch_oxide::frequency::CountSketch", true),
    ("countsketch", "lib-fixedmatrix-fast", "asap Count, FixedMatrix (5x2048), FastPath", true),
    ("countsketch", "lib-fixedmatrix-fast-32k", "asap Count, FixedMatrix (5x32768), FastPath", true),
    ("countsketch", "lib-vector2d-fast", "asap Count, Vector2D, FastPath", true),
    ("countsketch", "lib-vector2d-regular", "asap Count, Vector2D, RegularPath", true),
    ("countsketch", "polars", "polars exact: group_by(v).agg(len)", true),
    ("countsketch", "lib-fastpath-parallel", "asap Count, FastPath, parallel insert on M5x32K", false),
    // -------- DDSketch (quantile, relative error) --------
    ("dd", "lib", "asap_sketchlib::DDSketch (relative-error quantile)", true),
    ("dd", "polars", "polars exact: 101-point quantile grid", true),
    // -------- Top-k (counter array + size-k candidate tracker) --------
    ("topk", "cms-heap", "sketch_oxide CMS + size-k heap (top-k on the insert path)", true),
    ("topk", "cs-heap", "sketch_oxide CountSketch + size-k heap", true),
    ("topk", "polars", "polars exact: group_by(v).agg(len) sorted, top k", true),
    // -------- Elastic (heavy-hitter; no query capability, throughput-only) --------
    ("elastic", "lib", "asap_sketchlib::Elastic<DefaultXxHasher>", false),
    ("elastic", "oxide", "sketch_oxide::frequency::ElasticSketch", false),
    // -------- Nitro / UnivMon (no query capability; throughput-only) --------
    ("nitro", "lib", "asap_sketchlib::NitroBatch<Vector2D<u32>>", false),
    ("nitro", "oxide", "sketch_oxide::frequency::NitroSketch<CountMinSketch>", false),
    ("univmon", "lib", "asap_sketchlib::UnivMon", false),
    ("univmon", "oxide", "sketch_oxide::universal::UnivMon", false),
];

pub fn list() -> Vec<String> {
    IMPLS
        .iter()
        .map(|(f, i, d, _)| format!("{f:12} {i:28} {d}"))
        .collect()
}

pub fn family_exists(family: &str) -> bool {
    IMPLS.iter().any(|(f, _, _, _)| *f == family)
}

/// Does `--accuracy` score this row? `None` if the row is unknown.
pub fn scores_accuracy(family: &str, impl_name: &str) -> Option<bool> {
    IMPLS
        .iter()
        .find(|(f, i, _, _)| *f == family && *i == impl_name)
        .map(|(_, _, _, a)| *a)
}

/// Parse the single `--config` point for a family, checking the family exists.
pub fn config_point(family: &str, spec: &str) -> Result<ParamSet> {
    if !family_exists(family) {
        anyhow::bail!("unknown sketch family: {family}");
    }
    ParamSet::single(family, spec).map_err(Into::into)
}

// ---------- the two generic frontend helpers ----------

/// Timed measurement, plus accuracy scored against `make_gt(acc)` when
/// `--accuracy` is on. One generic function; the match arm supplies `S` and the
/// oracle closure.
fn scored<S, G>(
    cfg: &BenchConfig,
    items: &Items,
    params: &ParamSet,
    acc: &AccuracyCfg,
    make_gt: impl FnOnce(&AccuracyCfg, &ParamSet) -> G,
) -> Result<Vec<BenchReport>, RunError>
where
    S: Sketch + InitSketch + BenchImpl,
    S::Item: FromItems,
    G: GroundTruth<S>,
{
    let mut reports = cell::run_cell::<S>(cfg, items, params)?;
    if acc.enabled {
        let gt = make_gt(acc, params);
        reports.extend(cell::score_cell::<S, G>(cfg, items, params, &gt)?);
    }
    Ok(reports)
}

/// An ordered quantile family (KLL, DDSketch): the concrete type follows the
/// run-time dtype, so the arm names both and this picks between them.
fn ordered<Si, Sf, G>(
    cfg: &BenchConfig,
    items: &Items,
    params: &ParamSet,
    acc: &AccuracyCfg,
    make_gt: impl FnOnce(&AccuracyCfg, &ParamSet) -> G,
) -> Result<Vec<BenchReport>, RunError>
where
    Si: Sketch<Item = i64> + InitSketch + BenchImpl,
    Sf: Sketch<Item = f64> + InitSketch + BenchImpl,
    G: GroundTruth<Si> + GroundTruth<Sf>,
{
    match items {
        Items::I64(_) => scored::<Si, G>(cfg, items, params, acc, make_gt),
        Items::F64(_) => scored::<Sf, G>(cfg, items, params, acc, make_gt),
        other => Err(DtypeMismatch {
            wanted: &[DType::I64, DType::F64],
            got: other.dtype(),
        }
        .into()),
    }
}

fn card_gt(acc: &AccuracyCfg, _params: &ParamSet) -> CardinalityGT {
    CardinalityGT {
        record_calls: acc.record_query_calls,
    }
}
fn freq_gt(acc: &AccuracyCfg, _params: &ParamSet) -> FrequencyGT {
    FrequencyGT {
        max_probes: acc.max_probes,
    }
}
fn rank_gt(acc: &AccuracyCfg, _params: &ParamSet) -> RankErrorGT {
    RankErrorGT {
        record_calls: acc.record_query_calls,
    }
}
fn rel_gt(acc: &AccuracyCfg, _params: &ParamSet) -> RelativeErrorGT {
    RelativeErrorGT {
        record_calls: acc.record_query_calls,
    }
}

/// Scores against the same `k` the sketch was built with, read from the
/// row's params — a comparator asked for a different prefix than the tracker
/// keeps would be measuring the mismatch, not the sketch.
///
/// Infallible because every `topk` row parses these same params in its own
/// `init`, and [`scored`] runs the timed half first: a `k` this cannot read
/// has already failed the build. Falling back to a default `k` here is what
/// let a typo'd config run and publish a score at a `k` nobody asked for.
fn topk_gt(_acc: &AccuracyCfg, params: &ParamSet) -> TopkGT {
    TopkGT {
        k: params
            .parse::<TopkParams>()
            .expect("the row built, so its params parse")
            .k,
    }
}

/// Resolve `(family, impl)` to a concrete measurement and run it. The timed
/// half is always run; the accuracy half only when `acc.enabled`.
pub fn run(
    family: &str,
    impl_name: &str,
    cfg: &BenchConfig,
    items: &Items,
    params: &ParamSet,
    acc: &AccuracyCfg,
) -> Result<Vec<BenchReport>> {
    let out: Result<Vec<BenchReport>, RunError> = match (family, impl_name) {
        // -------- HLL --------
        ("hll", "oxide") => scored::<hll::HllOxide, _>(cfg, items, params, acc, card_gt),
        ("hll", "datasketches") => scored::<hll::HllDatasketches, _>(cfg, items, params, acc, card_gt),
        ("hll", "lib") => scored::<hll::HllLib, _>(cfg, items, params, acc, card_gt),
        ("hll", "lib-hip") => scored::<hll::HllLibHip, _>(cfg, items, params, acc, card_gt),
        ("hll", "polars") => scored::<polars::PolarsCardinality, _>(cfg, items, params, acc, card_gt),
        ("hll", "lib-fastpath-parallel") => cell::run_cell_parallel::<parallel::ParallelHllFastPath>(cfg, items, params),
        // -------- KLL --------
        ("kll", "oxide") => ordered::<kll::KllOxide<i64>, kll::KllOxide<f64>, _>(cfg, items, params, acc, rank_gt),
        ("kll", "lib") => ordered::<kll::KllLib<i64>, kll::KllLib<f64>, _>(cfg, items, params, acc, rank_gt),
        ("kll", "polars") => scored::<polars::PolarsQuantileKll, _>(cfg, items, params, acc, rank_gt),
        // -------- CMS --------
        ("cms", "oxide") => scored::<cms::CmsOxide, _>(cfg, items, params, acc, freq_gt),
        ("cms", "datasketches") => scored::<cms::CmsDatasketches, _>(cfg, items, params, acc, freq_gt),
        ("cms", "lib-fixedmatrix-custom-fast") => scored::<cms::CmsLibFixedmatrixCustomFast, _>(cfg, items, params, acc, freq_gt),
        ("cms", "lib-fixedmatrix-fast") => scored::<cms::CmsLibFixedmatrixFast, _>(cfg, items, params, acc, freq_gt),
        ("cms", "lib-fixedmatrix-fast-32k") => scored::<cms::CmsLibFixedmatrixFast32k, _>(cfg, items, params, acc, freq_gt),
        ("cms", "lib-vector2d-fast") => scored::<cms::CmsLibVector2dFast, _>(cfg, items, params, acc, freq_gt),
        ("cms", "lib-vector2d-regular") => scored::<cms::CmsLibVector2dRegular, _>(cfg, items, params, acc, freq_gt),
        ("cms", "polars") => scored::<polars::PolarsFrequencyCms, _>(cfg, items, params, acc, freq_gt),
        ("cms", "lib-fastpath-parallel") => cell::run_cell_parallel::<parallel::ParallelCmsFastPath>(cfg, items, params),
        // -------- CountSketch --------
        ("countsketch", "oxide") => scored::<countsketch::CsOxide, _>(cfg, items, params, acc, freq_gt),
        ("countsketch", "lib-fixedmatrix-fast") => scored::<countsketch::CsLibFixedmatrixFast, _>(cfg, items, params, acc, freq_gt),
        ("countsketch", "lib-fixedmatrix-fast-32k") => scored::<countsketch::CsLibFixedmatrixFast32k, _>(cfg, items, params, acc, freq_gt),
        ("countsketch", "lib-vector2d-fast") => scored::<countsketch::CsLibVector2dFast, _>(cfg, items, params, acc, freq_gt),
        ("countsketch", "lib-vector2d-regular") => scored::<countsketch::CsLibVector2dRegular, _>(cfg, items, params, acc, freq_gt),
        ("countsketch", "polars") => scored::<polars::PolarsFrequencyCs, _>(cfg, items, params, acc, freq_gt),
        ("countsketch", "lib-fastpath-parallel") => cell::run_cell_parallel::<parallel::ParallelCsFastPath>(cfg, items, params),
        // -------- DDSketch --------
        ("dd", "lib") => ordered::<dd::DdLib<i64>, dd::DdLib<f64>, _>(cfg, items, params, acc, rel_gt),
        ("dd", "polars") => scored::<polars::PolarsQuantileDd, _>(cfg, items, params, acc, rel_gt),
        // -------- Top-k --------
        ("topk", "cms-heap") => scored::<topk::TopKHeap<cms::CmsOxide>, _>(cfg, items, params, acc, topk_gt),
        ("topk", "cs-heap") => scored::<topk::TopKHeap<countsketch::CsOxide>, _>(cfg, items, params, acc, topk_gt),
        ("topk", "polars") => scored::<polars::PolarsTopK, _>(cfg, items, params, acc, topk_gt),
        // -------- Elastic / Nitro / UnivMon (throughput-only) --------
        ("elastic", "lib") => cell::run_cell::<elastic::ElasticLib>(cfg, items, params),
        ("elastic", "oxide") => cell::run_cell::<elastic::ElasticOxide>(cfg, items, params),
        ("nitro", "lib") => cell::run_cell::<nitro::NitroLib>(cfg, items, params),
        ("nitro", "oxide") => cell::run_cell::<nitro::NitroOxide>(cfg, items, params),
        ("univmon", "lib") => cell::run_cell::<univmon::UnivMonLib>(cfg, items, params),
        ("univmon", "oxide") => cell::run_cell::<univmon::UnivMonOxide>(cfg, items, params),
        _ => anyhow::bail!("no impl '{impl_name}' for family '{family}'"),
    };
    Ok(out?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::params::{
        CmsParams, CountSketchParams, DdParams, ElasticParams, HllParams, KllParams, NitroParams,
        SketchParams, TopkParams, UnivMonParams,
    };
    use std::collections::BTreeSet;

    /// One buildable config per family, from each params type's own
    /// `canonical()`. The `panic!` arm is what makes a newly added family
    /// show up here rather than silently skipping the tests below.
    fn canonical_params(family: &str) -> ParamSet {
        match family {
            "hll" => ParamSet::of(&HllParams::canonical()),
            "kll" => ParamSet::of(&KllParams::canonical()),
            "cms" => ParamSet::of(&CmsParams::canonical()),
            "countsketch" => ParamSet::of(&CountSketchParams::canonical()),
            "dd" => ParamSet::of(&DdParams::canonical()),
            "elastic" => ParamSet::of(&ElasticParams::canonical()),
            "nitro" => ParamSet::of(&NitroParams::canonical()),
            "topk" => ParamSet::of(&TopkParams::canonical()),
            "univmon" => ParamSet::of(&UnivMonParams::canonical()),
            other => panic!("no canonical params known for family '{other}'"),
        }
    }

    /// The catalog's size, pinned — a silent row loss is the failure this
    /// guards, the same as the old registry's `EXPECTED_ROWS`.
    #[test]
    fn catalog_size_is_pinned() {
        assert_eq!(IMPLS.len(), 36, "catalog changed size; update the count if deliberate");
    }

    #[test]
    fn family_impl_pairs_are_unique() {
        let mut seen = BTreeSet::new();
        for (f, i, _, _) in IMPLS {
            assert!(seen.insert((*f, *i)), "duplicate row {f}/{i}");
        }
    }

    /// A small i64 workload, enough for any row to build and ingest.
    fn smoke_items() -> Items {
        let spec = aqpbm_datagen::GenSpec {
            shape: aqpbm_datagen::Shape::Keys {
                cardinality: 64,
                dist: aqpbm_datagen::Distribution::Uniform,
            },
            size: 256,
            seed: 1,
            dtype: DType::I64,
            string: None,
        };
        cell::WorkloadSpec::Generated(spec).build(DType::I64).unwrap()
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

    /// Every catalog entry is runnable — `run` has a matching arm, never the
    /// `_` bail. Pins that `IMPLS` (the list) and the `run` match agree.
    #[test]
    fn every_catalog_entry_runs() {
        let items = smoke_items();
        let (cfg, acc) = smoke_cfg();
        for (family, impl_name, _, _) in IMPLS {
            // Canonical, not `empty`: every family's params have required
            // fields, so `empty` built only the 5 polars rows that ignore
            // their config — the other 31 were "checked" without ever running.
            let params = canonical_params(family);
            let got = run(family, impl_name, &cfg, &items, &params, &acc);
            // Fixed-matrix rows refuse an off-shape config (a `RunError::Build`
            // surfaced as an error); every other row runs. Neither is the
            // "no impl" bail, which is what this test forbids.
            match &got {
                Err(e) => assert!(
                    !e.to_string().contains("no impl"),
                    "{family}/{impl_name} has no run arm"
                ),
                // The row's labels come from its type's `BenchImpl`, not from
                // the arm's arguments, so this pins the last place the two can
                // still disagree: the `IMPLS` strings against what the type
                // says it is called.
                Ok(reports) => {
                    for r in reports {
                        assert_eq!(
                            (r.sketch.as_str(), r.impl_name.as_str()),
                            (*family, *impl_name),
                            "IMPLS row {family}/{impl_name} is labelled \
                             {}/{} by its type",
                            r.sketch,
                            r.impl_name,
                        );
                    }
                }
            }
        }
    }

    /// A family's rows are only comparable if they were asked the same
    /// question, so they must agree on which configs are answerable. The
    /// `topk` panel did not: the exact row ignored its `ParamSet` entirely,
    /// so a typo'd or zero `k` built there and got scored at a default `k`
    /// while the tracker rows rejected the same config outright.
    #[test]
    fn topk_rows_accept_and_reject_the_same_configs() {
        let items = smoke_items();
        let (cfg, acc) = smoke_cfg();
        let topk_rows = || IMPLS.iter().filter(|(f, _, _, _)| *f == "topk");
        for (spec, buildable) in [
            ("rows=5 cols=2048 k=5", true),
            ("rows=5 cols=2048 kk=5", false), // misspelled `k`
            ("rows=5 cols=2048 k=0", false),  // a top-k of nothing
            ("rows=5 cols=2048", false),      // no `k` at all
        ] {
            let params = config_point("topk", spec).unwrap();
            for (family, impl_name, _, _) in topk_rows() {
                let got = run(family, impl_name, &cfg, &items, &params, &acc);
                assert_eq!(
                    got.is_ok(),
                    buildable,
                    "topk/{impl_name} disagrees with the family on `{spec}`: {got:?}"
                );
            }
        }
    }
}
