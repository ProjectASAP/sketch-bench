//! The frontend catalog: the set of `(family, impl)` this CLI exposes, and the
//! `match` that resolves one to a concrete sketch type + its oracle.
//!
//! There is no registry table in the library — running a cell is a plain
//! `cell::run_cell::<T>` (timed) plus, when `--accuracy` is on,
//! `cell::score_cell::<T, Oracle>` (untimed). This module is where the runtime
//! strings `("hll", "oxide")` bind to those monomorphised calls. [`IMPLS`] is
//! the human-facing list (for `list-impls` and validation); [`run`] is the
//! executable match. A test pins that the two agree.

use anyhow::Result;
use aqpbm_core::sketch::Sketch;
use aqpbm_datagen::DType;

use sketch_bench::accuracy::cardinality::CardinalityGT;
use sketch_bench::accuracy::frequency::FrequencyGT;
use sketch_bench::accuracy::quantile::{RankErrorGT, RelativeErrorGT};
use sketch_bench::accuracy::GroundTruth;
use sketch_bench::cell::{self, AccuracyCfg, DtypeMismatch, FromItems, Items, RunError};
use sketch_bench::init::InitSketch;
use sketch_bench::params::ParamSet;
use sketch_bench::wrappers::{cms, countsketch, dd, elastic, hll, kll, nitro, parallel, polars, univmon};
use sketch_bench::{BenchConfig, BenchReport};

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
    // -------- Elastic (heavy-hitter; throughput-only pending top-k) --------
    ("elastic", "lib", "asap_sketchlib::Elastic<DefaultXxHasher>", false),
    ("elastic", "oxide", "sketch_oxide::frequency::ElasticSketch", false),
    // -------- Nitro / UnivMon (stub query; throughput-only) --------
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
    family: &'static str,
    impl_name: &'static str,
    cfg: &BenchConfig,
    items: &Items,
    params: &ParamSet,
    acc: &AccuracyCfg,
    make_gt: impl FnOnce(&AccuracyCfg) -> G,
) -> Result<Vec<BenchReport>, RunError>
where
    S: Sketch + InitSketch,
    S::Item: FromItems,
    G: GroundTruth<S>,
{
    let mut reports = cell::run_cell::<S>(family, impl_name, cfg, items, params)?;
    if acc.enabled {
        let gt = make_gt(acc);
        reports.extend(cell::score_cell::<S, G>(family, impl_name, cfg, items, params, &gt)?);
    }
    Ok(reports)
}

/// An ordered quantile family (KLL, DDSketch): the concrete type follows the
/// run-time dtype, so the arm names both and this picks between them.
fn ordered<Si, Sf, G>(
    family: &'static str,
    impl_name: &'static str,
    cfg: &BenchConfig,
    items: &Items,
    params: &ParamSet,
    acc: &AccuracyCfg,
    make_gt: impl FnOnce(&AccuracyCfg) -> G,
) -> Result<Vec<BenchReport>, RunError>
where
    Si: Sketch<Item = i64> + InitSketch,
    Sf: Sketch<Item = f64> + InitSketch,
    G: GroundTruth<Si> + GroundTruth<Sf>,
{
    match items {
        Items::I64(_) => scored::<Si, G>(family, impl_name, cfg, items, params, acc, make_gt),
        Items::F64(_) => scored::<Sf, G>(family, impl_name, cfg, items, params, acc, make_gt),
        other => Err(DtypeMismatch {
            wanted: &[DType::I64, DType::F64],
            got: other.dtype(),
        }
        .into()),
    }
}

fn card_gt(acc: &AccuracyCfg) -> CardinalityGT {
    CardinalityGT {
        record_calls: acc.record_query_calls,
    }
}
fn freq_gt(acc: &AccuracyCfg) -> FrequencyGT {
    FrequencyGT {
        max_probes: acc.max_probes,
    }
}
fn rank_gt(acc: &AccuracyCfg) -> RankErrorGT {
    RankErrorGT {
        record_calls: acc.record_query_calls,
    }
}
fn rel_gt(acc: &AccuracyCfg) -> RelativeErrorGT {
    RelativeErrorGT {
        record_calls: acc.record_query_calls,
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
        ("hll", "oxide") => scored::<hll::HllOxide, _>("hll", "oxide", cfg, items, params, acc, card_gt),
        ("hll", "datasketches") => scored::<hll::HllDatasketches, _>("hll", "datasketches", cfg, items, params, acc, card_gt),
        ("hll", "lib") => scored::<hll::HllLib, _>("hll", "lib", cfg, items, params, acc, card_gt),
        ("hll", "lib-hip") => scored::<hll::HllLibHip, _>("hll", "lib-hip", cfg, items, params, acc, card_gt),
        ("hll", "polars") => scored::<polars::PolarsCardinality, _>("hll", "polars", cfg, items, params, acc, card_gt),
        ("hll", "lib-fastpath-parallel") => cell::run_cell_parallel::<parallel::ParallelHllFastPath>("hll", "lib-fastpath-parallel", cfg, items, params),
        // -------- KLL --------
        ("kll", "oxide") => ordered::<kll::KllOxide<i64>, kll::KllOxide<f64>, _>("kll", "oxide", cfg, items, params, acc, rank_gt),
        ("kll", "lib") => ordered::<kll::KllLib<i64>, kll::KllLib<f64>, _>("kll", "lib", cfg, items, params, acc, rank_gt),
        ("kll", "polars") => scored::<polars::PolarsQuantileKll, _>("kll", "polars", cfg, items, params, acc, rank_gt),
        // -------- CMS --------
        ("cms", "oxide") => scored::<cms::CmsOxide, _>("cms", "oxide", cfg, items, params, acc, freq_gt),
        ("cms", "datasketches") => scored::<cms::CmsDatasketches, _>("cms", "datasketches", cfg, items, params, acc, freq_gt),
        ("cms", "lib-fixedmatrix-custom-fast") => scored::<cms::CmsLibFixedmatrixCustomFast, _>("cms", "lib-fixedmatrix-custom-fast", cfg, items, params, acc, freq_gt),
        ("cms", "lib-fixedmatrix-fast") => scored::<cms::CmsLibFixedmatrixFast, _>("cms", "lib-fixedmatrix-fast", cfg, items, params, acc, freq_gt),
        ("cms", "lib-fixedmatrix-fast-32k") => scored::<cms::CmsLibFixedmatrixFast32k, _>("cms", "lib-fixedmatrix-fast-32k", cfg, items, params, acc, freq_gt),
        ("cms", "lib-vector2d-fast") => scored::<cms::CmsLibVector2dFast, _>("cms", "lib-vector2d-fast", cfg, items, params, acc, freq_gt),
        ("cms", "lib-vector2d-regular") => scored::<cms::CmsLibVector2dRegular, _>("cms", "lib-vector2d-regular", cfg, items, params, acc, freq_gt),
        ("cms", "polars") => scored::<polars::PolarsFrequencyCms, _>("cms", "polars", cfg, items, params, acc, freq_gt),
        ("cms", "lib-fastpath-parallel") => cell::run_cell_parallel::<parallel::ParallelCmsFastPath>("cms", "lib-fastpath-parallel", cfg, items, params),
        // -------- CountSketch --------
        ("countsketch", "oxide") => scored::<countsketch::CsOxide, _>("countsketch", "oxide", cfg, items, params, acc, freq_gt),
        ("countsketch", "lib-fixedmatrix-fast") => scored::<countsketch::CsLibFixedmatrixFast, _>("countsketch", "lib-fixedmatrix-fast", cfg, items, params, acc, freq_gt),
        ("countsketch", "lib-fixedmatrix-fast-32k") => scored::<countsketch::CsLibFixedmatrixFast32k, _>("countsketch", "lib-fixedmatrix-fast-32k", cfg, items, params, acc, freq_gt),
        ("countsketch", "lib-vector2d-fast") => scored::<countsketch::CsLibVector2dFast, _>("countsketch", "lib-vector2d-fast", cfg, items, params, acc, freq_gt),
        ("countsketch", "lib-vector2d-regular") => scored::<countsketch::CsLibVector2dRegular, _>("countsketch", "lib-vector2d-regular", cfg, items, params, acc, freq_gt),
        ("countsketch", "polars") => scored::<polars::PolarsFrequencyCs, _>("countsketch", "polars", cfg, items, params, acc, freq_gt),
        ("countsketch", "lib-fastpath-parallel") => cell::run_cell_parallel::<parallel::ParallelCsFastPath>("countsketch", "lib-fastpath-parallel", cfg, items, params),
        // -------- DDSketch --------
        ("dd", "lib") => ordered::<dd::DdLib<i64>, dd::DdLib<f64>, _>("dd", "lib", cfg, items, params, acc, rel_gt),
        ("dd", "polars") => scored::<polars::PolarsQuantileDd, _>("dd", "polars", cfg, items, params, acc, rel_gt),
        // -------- Elastic / Nitro / UnivMon (throughput-only) --------
        ("elastic", "lib") => cell::run_cell::<elastic::ElasticLib>("elastic", "lib", cfg, items, params),
        ("elastic", "oxide") => cell::run_cell::<elastic::ElasticOxide>("elastic", "oxide", cfg, items, params),
        ("nitro", "lib") => cell::run_cell::<nitro::NitroLib>("nitro", "lib", cfg, items, params),
        ("nitro", "oxide") => cell::run_cell::<nitro::NitroOxide>("nitro", "oxide", cfg, items, params),
        ("univmon", "lib") => cell::run_cell::<univmon::UnivMonLib>("univmon", "lib", cfg, items, params),
        ("univmon", "oxide") => cell::run_cell::<univmon::UnivMonOxide>("univmon", "oxide", cfg, items, params),
        _ => anyhow::bail!("no impl '{impl_name}' for family '{family}'"),
    };
    Ok(out?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    /// The catalog's size, pinned — a silent row loss is the failure this
    /// guards, the same as the old registry's `EXPECTED_ROWS`.
    #[test]
    fn catalog_size_is_pinned() {
        assert_eq!(IMPLS.len(), 33, "catalog changed size; update the count if deliberate");
    }

    #[test]
    fn family_impl_pairs_are_unique() {
        let mut seen = BTreeSet::new();
        for (f, i, _, _) in IMPLS {
            assert!(seen.insert((*f, *i)), "duplicate row {f}/{i}");
        }
    }

    /// Every catalog entry is runnable — `run` has a matching arm, never the
    /// `_` bail. Pins that `IMPLS` (the list) and the `run` match agree.
    #[test]
    fn every_catalog_entry_runs() {
        let items = {
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
        };
        let cfg = BenchConfig {
            runs: 1,
            warmup_runs: 0,
            ..Default::default()
        };
        let acc = AccuracyCfg {
            enabled: false,
            max_probes: 0,
            record_query_calls: false,
        };
        for (family, impl_name, _, _) in IMPLS {
            let params = ParamSet::empty(family);
            let got = run(family, impl_name, &cfg, &items, &params, &acc);
            // Fixed-matrix rows refuse an off-shape config (a `RunError::Build`
            // surfaced as an error); every other row runs. Neither is the
            // "no impl" bail, which is what this test forbids.
            if let Err(e) = &got {
                assert!(
                    !e.to_string().contains("no impl"),
                    "{family}/{impl_name} has no run arm"
                );
            }
        }
    }
}
