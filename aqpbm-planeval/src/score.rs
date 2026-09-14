//! Scoring a readout against the exact answer, and checking the planner's
//! [`ResultGuarantee`] against what actually happened (PLAN.md §1.8).
//!
//! # Why this module reports rates, not booleans
//!
//! The bound the planner attaches to a KLL readout comes from an **empirical
//! 99th-percentile curve fit** — `2.296 / k^0.9723`
//! (`asap-aware-mapping/src/accuracy.rs`, `kll_rank_error_99`). The δ≈0.01 is
//! baked into the curve itself. Checking such a bound with one seed and one
//! probe is a single Bernoulli draw from an event designed to be true ~99% of
//! the time: `true` proves nothing and `false` disproves nothing.
//!
//! So [`check_guarantee`] takes *many* observations — one per (seed, probe) —
//! and reports [`GuaranteeCheck::observed_violation_rate`]. The proposition
//! under test is `observed_violation_rate ≈ failure_probability`, which is a
//! one-sided binomial tail question, not an assertion.
//!
//! # Why the `contract` string is not a version anchor
//!
//! `GuaranteeSource::SketchReadout::contract` is copied into the check
//! verbatim, but it is a **hardcoded string literal** — a hand-copied upstream
//! commit SHA prefix. Nothing computes it from the coefficients it names, so
//! changing `KLL_RANK_ERROR_COEFFICIENT_99` or `_EXPONENT_99` upstream leaves
//! the same id in place. [`ImpliedKllFit`] therefore back-solves the
//! coefficients from the observed `(k, bound)` pair and records them beside the
//! literal, so a future run can detect the literal lying.


use asap_types::post_asap::guarantee::{ErrorMetric, GuaranteeSource, ResultGuarantee};
use asap_types::post_asap::SketchQuery;
use asap_types::pre_asap::ColumnRef;
use serde::{Deserialize, Serialize};

use crate::types::EvalError;

/// The coefficient `asap-aware-mapping` used for the KLL empirical 99th
/// percentile fit when this module was written. Copied, not imported: the
/// upstream constant is `pub(crate)`, and the whole point of
/// [`ImpliedKllFit`] is to notice when the two stop agreeing.
pub const REFERENCE_KLL_COEFFICIENT_99: f64 = 2.296;

/// The exponent of the same fit, copied for the same reason.
pub const REFERENCE_KLL_EXPONENT_99: f64 = 0.9723;

/// Relative slack allowed when deciding whether a back-solved coefficient
/// still agrees with [`REFERENCE_KLL_COEFFICIENT_99`]. Wide enough for the
/// `powf` round trip, far too narrow to hide a real coefficient change.
const FIT_AGREEMENT_TOLERANCE: f64 = 1e-9;

/// One guarantee, checked against a population of observations rather than a
/// single draw.
///
/// `claimed_bound` and `failure_probability` are `None` — never `0.0` — when
/// the corresponding expression has a reachable `Unknown` leaf. "Unknown" and
/// "zero" are different answers, and collapsing them would turn an
/// unevaluatable bound into the tightest bound in the record.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GuaranteeCheck {
    /// The quantile this readout asked for. `f64::NAN` for readouts that are
    /// not quantile queries.
    #[serde(with = "nan_null")]
    pub q: f64,
    /// [`ResultGuarantee::metric`] in its serialized spelling (`"rank"`,
    /// `"relative_value"`, …). A string because `ErrorMetric` is
    /// `#[non_exhaustive]`: a deployment may carry a metric this crate has
    /// never heard of, and dropping it would be worse than not typing it.
    pub metric: String,
    /// `None` when `BoundExpr::evaluate()` returns `None`.
    pub claimed_bound: Option<f64>,
    /// `None` when `ProbabilityExpr::evaluate()` returns `None`.
    pub failure_probability: Option<f64>,
    /// How many observations back the three rates below. Named `seeds`
    /// because the sweep varies the sketch seed; one observation per
    /// (seed, probe) pair.
    pub seeds: usize,
    /// Fraction of observations whose error exceeded `claimed_bound`.
    /// `NaN` — serialized as `null` — when there is no bound to exceed or no
    /// observation to test, because `0.0` there would read as "nothing was
    /// violated" when in fact nothing was checked.
    #[serde(with = "nan_null")]
    pub observed_violation_rate: f64,
    #[serde(with = "nan_null")]
    pub mean_rank_err: f64,
    #[serde(with = "nan_null")]
    pub max_rank_err: f64,
    /// [`GuaranteeSource::SketchReadout::contract`], verbatim.
    pub contract: Option<String>,
    /// The coefficients implied by this guarantee's own `(k, bound)` pair.
    /// Present only for KLL readouts, whose bound is the curve fit this
    /// back-solve inverts.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub implied_fit: Option<ImpliedKllFit>,
}

/// The KLL fit coefficients back-solved from one `(k, bound)` pair.
///
/// One pair cannot pin both the coefficient and the exponent, so both are
/// reported *conditionally*: each assumes the other is still at its reference
/// value. A change to either one moves both numbers, which is all a drift
/// detector needs.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImpliedKllFit {
    pub k: u32,
    pub claimed_bound: f64,
    /// `bound · k^reference_exponent` — the coefficient the planner must be
    /// using if the exponent is unchanged.
    pub coefficient_at_reference_exponent: f64,
    /// `ln(reference_coefficient / bound) / ln(k)` — the exponent the planner
    /// must be using if the coefficient is unchanged.
    pub exponent_at_reference_coefficient: f64,
    pub reference_coefficient: f64,
    pub reference_exponent: f64,
    /// `false` means the `contract` literal is lying: the curve moved and the
    /// hardcoded id did not.
    pub agrees_with_reference: bool,
}

impl ImpliedKllFit {
    /// Back-solve from `k` and the bound the planner claimed for it.
    pub fn back_solve(k: u32, claimed_bound: f64) -> Option<Self> {
        if k == 0 || !claimed_bound.is_finite() || claimed_bound <= 0.0 {
            return None;
        }
        let kf = f64::from(k);
        let coefficient_at_reference_exponent = claimed_bound * kf.powf(REFERENCE_KLL_EXPONENT_99);
        let exponent_at_reference_coefficient =
            (REFERENCE_KLL_COEFFICIENT_99 / claimed_bound).ln() / kf.ln();
        let agrees_with_reference = (coefficient_at_reference_exponent
            - REFERENCE_KLL_COEFFICIENT_99)
            .abs()
            <= FIT_AGREEMENT_TOLERANCE * REFERENCE_KLL_COEFFICIENT_99;
        Some(Self {
            k,
            claimed_bound,
            coefficient_at_reference_exponent,
            exponent_at_reference_coefficient,
            reference_coefficient: REFERENCE_KLL_COEFFICIENT_99,
            reference_exponent: REFERENCE_KLL_EXPONENT_99,
            agrees_with_reference,
        })
    }
}

// ── rank error ──────────────────────────────────────────────────────────────

/// Exact rank error of `estimate` against the true distribution of `values` at
/// quantile `q`, in units of `n`.
///
/// Numerically identical to `aqpbm_core::accuracy::quantile`'s `rank_err`,
/// which `RankErrorGT` scores every existing `MergedRecord` with. That
/// function is `pub(crate)` and its probe grid is a hard-coded 101 points, so
/// it cannot be asked for one specific `q` from outside the crate — hence the
/// reimplementation, and hence `rank_error_agrees_with_rank_error_gt`, which
/// keeps the two in lockstep.
///
/// Range-based over tie intervals: the answer occupies the rank interval
/// `[lower, upper]` that a run of equal values spans, so a `q·n` inside that
/// interval is not an error, and outside it the distance to the near edge.
pub fn rank_error(values: &[f64], estimate: f64, q: f64) -> f64 {
    let mut sorted = values.to_vec();
    sorted.sort_by(f64::total_cmp);
    rank_error_sorted(&sorted, estimate, q)
}

/// [`rank_error`] over an already-sorted slice — the form to use when scoring a
/// whole probe grid, so the sort is paid once instead of once per probe.
pub fn rank_error_sorted(sorted: &[f64], estimate: f64, q: f64) -> f64 {
    let nf = sorted.len() as f64;
    if nf == 0.0 {
        return 0.0;
    }
    let target = q * nf;
    let lower = lower_bound(sorted, estimate) as f64;
    let upper = upper_bound(sorted, estimate) as f64;
    let raw = if target < lower {
        lower - target
    } else if target > upper {
        target - upper
    } else {
        0.0
    };
    raw / nf
}

/// Count of elements strictly less than `x` in a sorted slice. The loop is
/// `aqpbm_core::accuracy::quantile::lower_bound`'s, verbatim rather than
/// `slice::partition_point`, because a `total_cmp`-sorted column may hold
/// `NaN` — and there the two disagree about which midpoints they probe.
fn lower_bound(sorted: &[f64], x: f64) -> usize {
    let mut lo = 0usize;
    let mut hi = sorted.len();
    while lo < hi {
        let mid = (lo + hi) / 2;
        if sorted[mid] < x {
            lo = mid + 1;
        } else {
            hi = mid;
        }
    }
    lo
}

/// Count of elements `<= x` in a sorted slice — the upstream loop, for the
/// reason [`lower_bound`] gives.
fn upper_bound(sorted: &[f64], x: f64) -> usize {
    let mut lo = 0usize;
    let mut hi = sorted.len();
    while lo < hi {
        let mid = (lo + hi) / 2;
        if sorted[mid] <= x {
            lo = mid + 1;
        } else {
            hi = mid;
        }
    }
    lo
}

// ── exact answers ───────────────────────────────────────────────────────────

/// Exact answer for a readout, computed from the same rows the sketch
/// consumed — the `exact` arm of PLAN.md §1.10.3.
///
/// `values` is the column the node's `SummaryUpdate.weight` resolved to, in
/// stream order. The frequency-flavoured queries are taken over the
/// *value-frequency distribution* of that column (a value's frequency is how
/// many times it occurred), matching `SketchQuery`'s own definitions, not over
/// the numeric values themselves.
pub fn exact_answer(values: &[f64], query: &SketchQuery) -> Result<f64, EvalError> {
    match query {
        SketchQuery::Quantile { q } => {
            if !(0.0..=1.0).contains(q) {
                return Err(EvalError::Validation(format!(
                    "quantile q = {q} is outside [0, 1]"
                )));
            }
            if values.is_empty() {
                return Err(EvalError::Validation(
                    "exact quantile of an empty column is undefined".into(),
                ));
            }
            let mut sorted = values.to_vec();
            sorted.sort_by(f64::total_cmp);
            Ok(exact_quantile_sorted(&sorted, *q))
        }
        // The bare bucket total, which is how an exact accumulator's state is
        // read out: `SampleValue` with no item is the sum of the weights the
        // summary ingested, `Wildcard` is `COUNT(*)`.
        SketchQuery::PointCount {
            key: ColumnRef::SampleValue,
            value: None,
        } => Ok(values.iter().sum()),
        SketchQuery::PointCount {
            key: ColumnRef::Wildcard,
            value: None,
        } => Ok(values.len() as f64),
        // Everything else needs either item keys the weight column does not
        // carry, or a family v0 does not bind. Admission refuses these, so
        // reaching here is a seam bug rather than a user error.
        other => Err(EvalError::Validation(format!(
            "no exact answer for {other:?} from a weight column alone"
        ))),
    }
}

/// The value at rank `q·n`, on the same ruler [`rank_error`] measures against:
/// feeding this back in as the estimate yields a rank error of exactly zero.
fn exact_quantile_sorted(sorted: &[f64], q: f64) -> f64 {
    let n = sorted.len();
    let index = ((q * n as f64).floor() as usize).min(n - 1);
    sorted[index]
}

// ── guarantee checking ──────────────────────────────────────────────────────

/// Check one guarantee against a population of observations.
///
/// `observations` is one `(estimate, rank_err)` pair per (seed, probe): the
/// whole point is that a single pair cannot check a bound carrying δ≈0.01.
/// The estimate is carried for the caller's provenance; the rate is computed
/// from the errors.
pub fn check_guarantee(
    guarantee: &ResultGuarantee,
    q: f64,
    observations: &[(f64, f64)],
) -> GuaranteeCheck {
    let claimed_bound = guarantee.bound.evaluate();
    let failure_probability = guarantee.failure_probability.evaluate();

    let mut sum_err = 0.0_f64;
    let mut max_err = f64::NEG_INFINITY;
    let mut violations = 0usize;
    for (_estimate, err) in observations {
        sum_err += *err;
        if *err > max_err {
            max_err = *err;
        }
        if let Some(bound) = claimed_bound {
            if *err > bound {
                violations += 1;
            }
        }
    }

    let n = observations.len();
    let (mean_rank_err, max_rank_err) = if n == 0 {
        (f64::NAN, f64::NAN)
    } else {
        (sum_err / n as f64, max_err)
    };
    // No bound and no observations are two different ways of having checked
    // nothing, and both have to stay distinguishable from "checked, zero
    // violations" — which is why neither produces 0.0.
    let observed_violation_rate = match (claimed_bound, n) {
        (Some(_), 1..) => violations as f64 / n as f64,
        _ => f64::NAN,
    };

    let readout = sketch_readout(guarantee);
    let contract = readout.map(|(_, contract, _)| contract.to_owned());
    let implied_fit = readout.and_then(|(algorithm, _, params)| {
        // Only KLL's bound is the `2.296 / k^0.9723` curve; back-solving it
        // out of a CMS or HLL bound would be arithmetic about nothing.
        (algorithm == "Kll")
            .then(|| kll_k(params))
            .flatten()
            .zip(claimed_bound)
            .and_then(|(k, bound)| ImpliedKllFit::back_solve(k, bound))
    });

    GuaranteeCheck {
        q,
        metric: metric_name(&guarantee.metric),
        claimed_bound,
        failure_probability,
        seeds: n,
        observed_violation_rate,
        mean_rank_err,
        max_rank_err,
        contract,
        implied_fit,
    }
}

/// The first `SketchReadout` step of the provenance trail, as
/// `(algorithm, contract, params)`.
fn sketch_readout(guarantee: &ResultGuarantee) -> Option<(&str, &str, &serde_json::Value)> {
    guarantee.provenance.iter().find_map(|source| match source {
        GuaranteeSource::SketchReadout {
            algorithm,
            contract,
            params,
            ..
        } => Some((algorithm.as_str(), contract.as_str(), params)),
        _ => None,
    })
}

/// `k` out of a serialized `SketchParams::Kll` (`{"Kll": {"k": 269}}`).
fn kll_k(params: &serde_json::Value) -> Option<u32> {
    params.get("Kll")?.get("k")?.as_u64()?.try_into().ok()
}

/// `ErrorMetric` in its serialized spelling. Goes through serde rather than a
/// match because the enum is `#[non_exhaustive]`.
fn metric_name(metric: &ErrorMetric) -> String {
    serde_json::to_value(metric)
        .ok()
        .and_then(|value| value.as_str().map(str::to_owned))
        .unwrap_or_else(|| format!("{metric:?}"))
}

/// `NaN` ⇄ `null`. A rate that was never computed is absent from the JSON, not
/// zero, and it survives a round trip as the same `NaN` it went in as.
mod nan_null {
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(value: &f64, serializer: S) -> Result<S::Ok, S::Error> {
        if value.is_finite() {
            serializer.serialize_f64(*value)
        } else {
            serializer.serialize_none()
        }
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<f64, D::Error> {
        Ok(Option::<f64>::deserialize(deserializer)?.unwrap_or(f64::NAN))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aqpbm_core::accuracy::quantile::RankErrorGT;
    use aqpbm_core::{ColumnData, GeneratedTable, GroundTruth};
    use asap_types::post_asap::guarantee::{BoundExpr, ProbabilityExpr};

    /// `0, 1, … 9999` — the dataset every hand-computed number below is taken
    /// over.
    fn uniform_0_10000() -> Vec<f64> {
        (0..10_000).map(|i| i as f64).collect()
    }

    fn table(values: &[f64]) -> GeneratedTable {
        GeneratedTable {
            column_num: 1,
            column_title: vec!["value".into()],
            data: vec![ColumnData::Float64(values.to_vec())],
            row_num: values.len() as u64,
        }
    }

    /// The guarantee the planner actually attaches to `quantile(0.5,
    /// cpu_cores)` at `Epsilon(0.01)` — PLAN.md §1.8 quotes it verbatim.
    fn planner_kll_guarantee() -> ResultGuarantee {
        ResultGuarantee {
            metric: ErrorMetric::Rank,
            bound: BoundExpr::Constant {
                value: 0.009_966_065_608_321_138,
            },
            failure_probability: ProbabilityExpr::Constant { value: 0.01 },
            provenance: vec![GuaranteeSource::SketchReadout {
                algorithm: "Kll".into(),
                contract: "apache_datasketches_kll_empirical_99_a9b42755072b".into(),
                params: serde_json::json!({ "Kll": { "k": 269 } }),
                query: "Quantile { q: 0.5 }".into(),
            }],
        }
    }

    #[test]
    fn rank_error_matches_hand_computation_on_uniform_data() {
        let values = uniform_0_10000();

        // n = 10000, q = 0.5 -> target rank 5000. The estimate 5100.0 occupies
        // ranks [5100, 5101], so the near edge is 5100 and the error is
        // (5100 - 5000) / 10000.
        assert_eq!(rank_error(&values, 5100.0, 0.5), 0.01);

        // Symmetric on the other side: 4900.0 occupies [4900, 4901], the
        // target 5000 is above the far edge, so (5000 - 4901) / 10000.
        assert_eq!(rank_error(&values, 4900.0, 0.5), 0.0099);

        // The exact answer is exact: target 5000 lands inside [5000, 5001].
        assert_eq!(rank_error(&values, 5000.0, 0.5), 0.0);

        // Both grid ends, where the interval is one-sided.
        assert_eq!(rank_error(&values, 0.0, 0.0), 0.0);
        assert_eq!(rank_error(&values, 9999.0, 1.0), 0.0);
        // An estimate past the top of the data: ranks [10000, 10000], target
        // 10000 -> still zero; one below it costs one rank.
        assert_eq!(rank_error(&values, 9998.0, 1.0), 0.0001);
    }

    #[test]
    fn rank_error_is_range_based_over_ties() {
        // 100 copies of 7.0 and nothing else: every rank in [0, 100] is the
        // value's, so no quantile of it can be wrong.
        let values = vec![7.0; 100];
        for i in 0..=100 {
            assert_eq!(rank_error(&values, 7.0, i as f64 / 100.0), 0.0);
        }
    }

    #[test]
    fn rank_error_of_empty_input_is_zero() {
        assert_eq!(rank_error(&[], 1.0, 0.5), 0.0);
    }

    /// The equality that makes every number this crate reports comparable to
    /// every existing `MergedRecord`: on `RankErrorGT`'s own 101-point grid,
    /// our per-probe rank error folds to exactly the mean and max that
    /// comparator reports.
    #[test]
    fn rank_error_agrees_with_rank_error_gt_on_the_101_point_grid() {
        let values = uniform_0_10000();
        let table = table(&values);
        let gt = RankErrorGT { column: 0 };
        let truth = gt.truth(&table).expect("float column is numeric");
        let probes = gt.probes(&truth);
        assert_eq!(probes.len(), 101, "the grid is 101 points");

        // Deliberately wrong answers, so the agreement is about non-zero
        // errors rather than about both sides returning 0.
        let answers: Vec<f64> = probes
            .iter()
            .enumerate()
            .map(|(i, q)| exact_quantile_sorted(&truth, *q) + (i % 7) as f64 * 13.0)
            .collect();

        let theirs = gt.score(&truth, &probes, &answers);

        let ours: Vec<f64> = probes
            .iter()
            .zip(&answers)
            .map(|(q, est)| rank_error_sorted(&truth, *est, *q))
            .collect();
        let mean = ours.iter().sum::<f64>() / ours.len() as f64;
        let max = ours.iter().copied().fold(f64::NEG_INFINITY, f64::max);

        assert!(max > 0.0, "the perturbation has to produce real error");
        assert_eq!(theirs["mean_rank_err"], mean);
        assert_eq!(theirs["max_rank_err"], max);
        assert_eq!(theirs["grid_points"], 101.0);
    }

    #[test]
    fn exact_quantile_scores_zero_against_itself() {
        let values = uniform_0_10000();
        for i in 0..=100 {
            let q = i as f64 / 100.0;
            let answer = exact_answer(&values, &SketchQuery::Quantile { q }).unwrap();
            assert_eq!(
                rank_error(&values, answer, q),
                0.0,
                "exact answer at q = {q} must have zero rank error"
            );
        }
    }

    #[test]
    fn exact_answer_covers_the_two_readouts_v0_can_reach() {
        let values: Vec<f64> = (1..=100).map(|i| i as f64).collect();

        // A KLL readout.
        let median = exact_answer(&values, &SketchQuery::Quantile { q: 0.5 }).unwrap();
        assert_eq!(median, 51.0, "value at rank floor(0.5 * 100)");

        // An exact accumulator read out as a bucket total.
        let total = exact_answer(
            &values,
            &SketchQuery::PointCount {
                key: ColumnRef::SampleValue,
                value: None,
            },
        )
        .unwrap();
        assert_eq!(total, 5050.0);

        // Everything else is a seam bug, not a silent number.
        for unreachable in [
            SketchQuery::Cardinality,
            SketchQuery::FrequencyL2,
            SketchQuery::FrequencyEntropy,
            SketchQuery::TopK { k: 5 },
        ] {
            assert!(
                exact_answer(&values, &unreachable).is_err(),
                "{unreachable:?} must not produce a number"
            );
        }
    }

        #[test]
    fn exact_answer_refuses_what_it_cannot_compute() {
        let values = vec![1.0, 2.0];
        assert!(exact_answer(&values, &SketchQuery::TopK { k: 3 }).is_err());
        assert!(exact_answer(&values, &SketchQuery::Quantile { q: 1.5 }).is_err());
        assert!(exact_answer(&[], &SketchQuery::Quantile { q: 0.5 }).is_err());
        assert!(exact_answer(
            &values,
            &SketchQuery::PointCount {
                key: ColumnRef::Named("item".into()),
                value: Some("checkout".into()),
            },
        )
        .is_err());
    }

    #[test]
    fn unknown_bound_is_none_and_never_zero() {
        let guarantee = ResultGuarantee {
            metric: ErrorMetric::Cardinality,
            bound: BoundExpr::Product {
                factors: vec![
                    BoundExpr::Constant { value: 0.02 },
                    BoundExpr::Unknown {
                        statistic: "true_cardinality".into(),
                    },
                ],
            },
            failure_probability: ProbabilityExpr::Unknown {
                statistic: "hll_estimator_failure_probability".into(),
            },
            provenance: vec![],
        };
        let check = check_guarantee(&guarantee, 0.5, &[(1.0, 0.5), (1.0, 0.9)]);

        assert_eq!(check.claimed_bound, None);
        assert_ne!(check.claimed_bound, Some(0.0));
        assert_eq!(check.failure_probability, None);
        assert_ne!(check.failure_probability, Some(0.0));
        // Nothing was checked, so the violation rate is not 0.0 either.
        assert!(check.observed_violation_rate.is_nan());
        assert_eq!(check.mean_rank_err, 0.7);
        assert_eq!(check.contract, None);
        assert!(check.implied_fit.is_none());

        // And "unknown" survives the wire as null, not as a number.
        let json: serde_json::Value = serde_json::to_value(&check).unwrap();
        assert!(json["claimed_bound"].is_null());
        assert!(json["failure_probability"].is_null());
        assert!(json["observed_violation_rate"].is_null());
    }

    #[test]
    fn structural_zero_bound_stays_zero() {
        // The other half of the rule: an exact computation really does claim
        // zero, and that must not be confused with "unknown".
        let check = check_guarantee(&ResultGuarantee::exact("KeepPreAsap"), 0.5, &[(1.0, 0.0)]);
        assert_eq!(check.claimed_bound, Some(0.0));
        assert_eq!(check.failure_probability, Some(0.0));
        assert_eq!(check.observed_violation_rate, 0.0);
        assert_eq!(check.metric, "absolute_value");
    }

    #[test]
    fn violation_rate_counts_errors_above_the_bound() {
        let guarantee = planner_kll_guarantee();
        let bound = 0.009_966_065_608_321_138;
        // 100 observations, 3 of them over the bound. One exactly at the
        // bound is not a violation: the claim is `Pr[err > bound]`.
        let mut observations: Vec<(f64, f64)> = vec![(0.0, bound * 0.5); 96];
        observations.push((0.0, bound));
        observations.extend([(0.0, bound * 1.1), (0.0, bound * 2.0), (0.0, bound * 3.0)]);

        let check = check_guarantee(&guarantee, 0.5, &observations);
        assert_eq!(check.seeds, 100);
        assert_eq!(check.observed_violation_rate, 0.03);
        assert_eq!(check.failure_probability, Some(0.01));
        assert_eq!(check.max_rank_err, bound * 3.0);
        assert_eq!(check.metric, "rank");
    }

    #[test]
    fn no_observations_yields_no_rates() {
        let check = check_guarantee(&planner_kll_guarantee(), 0.5, &[]);
        assert_eq!(check.seeds, 0);
        assert!(check.observed_violation_rate.is_nan());
        assert!(check.mean_rank_err.is_nan());
        assert!(check.max_rank_err.is_nan());
        // The bound itself is still known — it is the sample that is missing.
        assert_eq!(check.claimed_bound, Some(0.009_966_065_608_321_138));
    }

    #[test]
    fn contract_is_copied_verbatim_and_the_fit_is_back_solved() {
        let check = check_guarantee(&planner_kll_guarantee(), 0.5, &[(0.0, 0.001)]);
        assert_eq!(
            check.contract.as_deref(),
            Some("apache_datasketches_kll_empirical_99_a9b42755072b")
        );

        let fit = check.implied_fit.expect("a KLL readout back-solves");
        assert_eq!(fit.k, 269);
        assert!(
            fit.agrees_with_reference,
            "planner bound implies coefficient {}, reference is {}",
            fit.coefficient_at_reference_exponent, fit.reference_coefficient
        );
        assert!((fit.coefficient_at_reference_exponent - 2.296).abs() < 1e-9);
        assert!((fit.exponent_at_reference_coefficient - 0.9723).abs() < 1e-9);
    }

    /// The reason the back-solve exists: the `contract` literal is hardcoded,
    /// so a coefficient change upstream keeps the same id — and only the
    /// implied fit notices.
    #[test]
    fn a_moved_coefficient_is_caught_even_though_the_contract_did_not_change() {
        let mut guarantee = planner_kll_guarantee();
        // Same k, same contract string, bound recomputed from 2.400 / k^0.9723.
        guarantee.bound = BoundExpr::Constant {
            value: 2.400 / 269f64.powf(REFERENCE_KLL_EXPONENT_99),
        };
        let check = check_guarantee(&guarantee, 0.5, &[(0.0, 0.001)]);
        assert_eq!(
            check.contract.as_deref(),
            Some("apache_datasketches_kll_empirical_99_a9b42755072b"),
            "the literal is unchanged, which is exactly the problem"
        );
        let fit = check.implied_fit.expect("a KLL readout back-solves");
        assert!(!fit.agrees_with_reference);
        assert!((fit.coefficient_at_reference_exponent - 2.400).abs() < 1e-9);
    }

    #[test]
    fn non_kll_readouts_do_not_back_solve_a_kll_curve() {
        let guarantee = ResultGuarantee {
            metric: ErrorMetric::Frequency,
            bound: BoundExpr::Constant {
                value: std::f64::consts::E / 2048.0,
            },
            failure_probability: ProbabilityExpr::Constant {
                value: (-5.0_f64).exp(),
            },
            provenance: vec![GuaranteeSource::SketchReadout {
                algorithm: "Cms".into(),
                contract: "count_min_l1_markov_v1".into(),
                params: serde_json::json!({ "Cms": { "width": 2048, "depth": 5 } }),
                query: "PointCount".into(),
            }],
        };
        let check = check_guarantee(&guarantee, f64::NAN, &[(0.0, 0.0)]);
        assert_eq!(check.contract.as_deref(), Some("count_min_l1_markov_v1"));
        assert!(check.implied_fit.is_none());
        assert!(check.q.is_nan());
        assert!(serde_json::to_value(&check).unwrap()["q"].is_null());
    }

    #[test]
    fn guarantee_check_round_trips_through_serde() {
        let check = check_guarantee(&planner_kll_guarantee(), 0.5, &[(5000.0, 0.002)]);
        let json = serde_json::to_string(&check).unwrap();
        let back: GuaranteeCheck = serde_json::from_str(&json).unwrap();
        assert_eq!(back.claimed_bound, check.claimed_bound);
        assert_eq!(back.metric, check.metric);
        assert_eq!(back.seeds, check.seeds);
        assert_eq!(back.observed_violation_rate, check.observed_violation_rate);
        assert_eq!(back.contract, check.contract);
        assert_eq!(
            back.implied_fit.map(|fit| fit.k),
            check.implied_fit.map(|fit| fit.k)
        );
    }
}
