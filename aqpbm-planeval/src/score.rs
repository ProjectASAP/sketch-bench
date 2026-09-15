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

use std::collections::BTreeMap;

use crate::run::Readout;
use crate::types::{Answer, EvalError};

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
    #[serde(with = "json_f64")]
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
    /// `NaN` — serialized as `null` — when there is no bound to exceed, no
    /// observation to test, or an observation whose error is itself `NaN`,
    /// because `0.0` there would read as "nothing was violated" when in fact
    /// nothing was checked.
    #[serde(with = "json_f64")]
    pub observed_violation_rate: f64,
    #[serde(with = "json_f64")]
    pub mean_error: f64,
    #[serde(with = "json_f64")]
    pub max_error: f64,
    pub unevaluatable: Option<UnevaluatableReason>,
    /// How many of the `seeds` observations carried a `NaN` error — an error
    /// no arithmetic produced, as distinct from a large one or an infinite
    /// one. Any such observation blocks the three numbers above rather than
    /// being folded in as a clean draw.
    #[serde(default)]
    pub uncomputed_errors: usize,
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
        let agrees_with_reference =
            (coefficient_at_reference_exponent - REFERENCE_KLL_COEFFICIENT_99).abs()
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

pub const PLANEVAL_GUARANTEE_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReadoutGuarantee {
    pub schema_version: u32,
    pub node: u32,
    pub group: String,
    pub query: String,
    pub check: GuaranteeCheck,
}

#[derive(Debug, Default)]
pub struct GuaranteeObservations {
    by_readout: BTreeMap<(u32, String, String), ObservedReadout>,
}

#[derive(Debug)]
struct ObservedReadout {
    q: f64,
    guarantee: ResultGuarantee,
    pairs: Vec<(f64, f64)>,
}

impl GuaranteeObservations {
    pub fn observe(&mut self, readout: &Readout) {
        let Some(guarantee) = readout.guarantee.as_ref() else {
            return;
        };
        let query = format!("{:?}", readout.query);
        let entry = self
            .by_readout
            .entry((readout.node.0, readout.group.clone(), query))
            .or_insert_with(|| ObservedReadout {
                q: match readout.query {
                    SketchQuery::Quantile { q } => q,
                    _ => f64::NAN,
                },
                guarantee: guarantee.clone(),
                pairs: Vec::new(),
            });
        let Answer::Scalar(estimate) = &readout.approximate else {
            return;
        };
        match &readout.observed_error {
            ObservedError::Measured { error, .. } => entry.pairs.push((*estimate, *error)),
            // Carried into the population as the `NaN` it is, rather than
            // dropped: an observation silently missing from the denominator
            // would let the survivors publish a clean rate over a population
            // that was never fully checked.
            ObservedError::Unevaluatable {
                reason: UnevaluatableReason::ErrorIsNotANumber,
                ..
            } => entry.pairs.push((*estimate, f64::NAN)),
            _ => {}
        }
    }

    pub fn checks(&self) -> Vec<ReadoutGuarantee> {
        self.by_readout
            .iter()
            .map(|((node, group, query), observed)| ReadoutGuarantee {
                schema_version: PLANEVAL_GUARANTEE_SCHEMA_VERSION,
                node: *node,
                group: group.clone(),
                query: query.clone(),
                check: check_guarantee(&observed.guarantee, observed.q, &observed.pairs),
            })
            .collect()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UnevaluatableReason {
    NoQuantileInTheQuery,
    TrueDistinctCount,
    StreamL1Norm,
    StreamL2Norm,
    TrueTopKSet,
    UnrecognizedMetric,
    /// The metric's own arithmetic produced `NaN` — an estimate or a truth
    /// that is not a number. Distinct from `+inf`, which is the real answer
    /// "infinitely outside the bound" and stays a measurement.
    ErrorIsNotANumber,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "measurement", rename_all = "snake_case")]
pub enum ObservedError {
    NotVerified,
    NoGuarantee,
    Measured {
        metric: String,
        #[serde(with = "json_f64")]
        error: f64,
    },
    Unevaluatable {
        metric: String,
        reason: UnevaluatableReason,
    },
}

impl ObservedError {
    pub fn measured(&self) -> Option<f64> {
        match self {
            ObservedError::Measured { error, .. } => Some(*error),
            _ => None,
        }
    }
}

pub fn error_under_metric(
    metric: &ErrorMetric,
    values: &[f64],
    query: &SketchQuery,
    estimate: f64,
    truth: f64,
) -> Result<f64, UnevaluatableReason> {
    match metric {
        ErrorMetric::Rank if !is_sorted_by_total_cmp(values) => {
            let mut sorted = values.to_vec();
            sorted.sort_by(f64::total_cmp);
            error_under_metric_sorted(metric, &sorted, query, estimate, truth)
        }
        _ => error_under_metric_sorted(metric, values, query, estimate, truth),
    }
}

fn is_sorted_by_total_cmp(values: &[f64]) -> bool {
    values
        .windows(2)
        .all(|pair| pair[0].total_cmp(&pair[1]).is_le())
}

fn error_under_metric_sorted(
    metric: &ErrorMetric,
    sorted: &[f64],
    query: &SketchQuery,
    estimate: f64,
    truth: f64,
) -> Result<f64, UnevaluatableReason> {
    let q = match query {
        SketchQuery::Quantile { q } => *q,
        _ => f64::NAN,
    };
    if let Some(reason) = unevaluatable_reason(metric, q) {
        return Err(reason);
    }
    if !estimate.is_finite() {
        return Err(UnevaluatableReason::ErrorIsNotANumber);
    }
    match metric {
        ErrorMetric::Rank => Ok(rank_error_sorted(sorted, estimate, q)),
        ErrorMetric::AbsoluteValue => Ok((estimate - truth).abs()),
        ErrorMetric::RelativeValue => Ok(relative_error(estimate, truth)),
        _ => Err(UnevaluatableReason::UnrecognizedMetric),
    }
}

fn unevaluatable_reason(metric: &ErrorMetric, q: f64) -> Option<UnevaluatableReason> {
    match metric {
        ErrorMetric::Rank => q
            .is_nan()
            .then_some(UnevaluatableReason::NoQuantileInTheQuery),
        ErrorMetric::AbsoluteValue | ErrorMetric::RelativeValue => None,
        ErrorMetric::Cardinality => Some(UnevaluatableReason::TrueDistinctCount),
        ErrorMetric::Frequency => Some(UnevaluatableReason::StreamL1Norm),
        ErrorMetric::L2Frequency => Some(UnevaluatableReason::StreamL2Norm),
        ErrorMetric::TopKMembership => Some(UnevaluatableReason::TrueTopKSet),
        _ => Some(UnevaluatableReason::UnrecognizedMetric),
    }
}

fn relative_error(estimate: f64, truth: f64) -> f64 {
    let gap = (estimate - truth).abs();
    if truth == 0.0 {
        return if gap == 0.0 { 0.0 } else { f64::INFINITY };
    }
    gap / truth.abs()
}

pub fn observed_error(
    guarantee: Option<&ResultGuarantee>,
    values: &[f64],
    query: &SketchQuery,
    approximate: &Answer,
    truth: f64,
) -> ObservedError {
    scored(guarantee, approximate, |metric, estimate| {
        error_under_metric(metric, values, query, estimate, truth)
    })
}

fn scored(
    guarantee: Option<&ResultGuarantee>,
    approximate: &Answer,
    error: impl FnOnce(&ErrorMetric, f64) -> Result<f64, UnevaluatableReason>,
) -> ObservedError {
    let Some(guarantee) = guarantee else {
        return ObservedError::NoGuarantee;
    };
    let metric = metric_name(&guarantee.metric);
    let Answer::Scalar(estimate) = approximate else {
        return ObservedError::Unevaluatable {
            metric,
            reason: UnevaluatableReason::TrueTopKSet,
        };
    };
    match error(&guarantee.metric, *estimate) {
        // A `NaN` error is not a small error; it is the absence of one, and
        // `Measured` is this crate's word for "a number was computed".
        Ok(error) if error.is_nan() => ObservedError::Unevaluatable {
            metric,
            reason: UnevaluatableReason::ErrorIsNotANumber,
        },
        Ok(error) => ObservedError::Measured { metric, error },
        Err(reason) => ObservedError::Unevaluatable { metric, reason },
    }
}

// ── rank error ──────────────────────────────────────────────────────────────

/// Exact rank error of `estimate` against the true distribution of `values` at
/// quantile `q`, in units of `n`.
///
/// Numerically identical, for a finite estimate, to
/// `aqpbm_core::accuracy::quantile`'s `rank_err`, which `RankErrorGT` scores
/// every existing `MergedRecord` with. That
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
pub(crate) fn rank_error_sorted(sorted: &[f64], estimate: f64, q: f64) -> f64 {
    let nf = sorted.len() as f64;
    if nf == 0.0 {
        return 0.0;
    }
    if !estimate.is_finite() {
        return f64::NAN;
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

pub fn needs_a_sorted_column(query: &SketchQuery) -> bool {
    matches!(query, SketchQuery::Quantile { .. })
}

/// Exact answer for a readout, computed from the same rows the sketch
/// consumed — the `exact` arm of PLAN.md §1.10.3.
///
/// `sorted` is the column the node's `SummaryUpdate.weight` resolved to, sorted
/// by the caller whenever [`needs_a_sorted_column`] says so. The
/// frequency-flavoured queries are taken over the *value-frequency
/// distribution* of that column (a value's frequency is how many times it
/// occurred), matching `SketchQuery`'s own definitions, not over the numeric
/// values themselves.
pub fn exact_answer_sorted(sorted: &[f64], query: &SketchQuery) -> Result<f64, EvalError> {
    match query {
        SketchQuery::Quantile { q } => {
            if !(0.0..=1.0).contains(q) {
                return Err(EvalError::Validation(format!(
                    "quantile q = {q} is outside [0, 1]"
                )));
            }
            if sorted.is_empty() {
                return Err(EvalError::Validation(
                    "exact quantile of an empty column is undefined".into(),
                ));
            }
            Ok(exact_quantile_sorted(sorted, *q))
        }
        // The bare bucket total, which is how an exact accumulator's state is
        // read out: `SampleValue` with no item is the sum of the weights the
        // summary ingested, `Wildcard` is `COUNT(*)`.
        SketchQuery::PointCount {
            key: ColumnRef::SampleValue,
            value: None,
        } => Ok(sorted.iter().sum()),
        SketchQuery::PointCount {
            key: ColumnRef::Wildcard,
            value: None,
        } => Ok(sorted.len() as f64),
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
/// `observations` is one `(estimate, error)` pair per (seed, probe), each error
/// measured under `guarantee.metric`: the whole point is that a single pair
/// cannot check a bound carrying δ≈0.01. The estimate is carried for the
/// caller's provenance; the rate is computed from the errors.
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
    let mut uncomputed_errors = 0usize;
    for (_estimate, err) in observations {
        // `NaN > bound` is `false`, so an unguarded comparison would file an
        // error that does not exist as a draw that stayed inside the bound.
        // `+inf` is not that case: it is a real error, and it is a violation.
        if err.is_nan() {
            uncomputed_errors += 1;
            continue;
        }
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
    let (mean_error, max_error) = if n == 0 || uncomputed_errors > 0 {
        (f64::NAN, f64::NAN)
    } else {
        (sum_err / n as f64, max_err)
    };
    let unevaluatable = unevaluatable_reason(&guarantee.metric, q);
    // No bound, no observations, a metric the exact arm cannot compute, and an
    // observation whose error is itself NaN are four different ways of having
    // checked nothing, and all four have to stay distinguishable from
    // "checked, zero violations" — which is why none of them produces 0.0.
    let observed_violation_rate = match (claimed_bound, n, unevaluatable, uncomputed_errors) {
        (Some(_), 1.., None, 0) => violations as f64 / n as f64,
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
        mean_error,
        max_error,
        unevaluatable,
        uncomputed_errors,
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

/// An `f64` through JSON, which has no spelling for a non-finite number.
///
/// `NaN` ⇄ `null`: a rate that was never computed is absent from the JSON, not
/// zero, and it survives a round trip as the same `NaN` it went in as. The
/// infinities ⇄ `"inf"` / `"-inf"`, because an infinite error is a computed
/// answer — "infinitely outside the bound" — and writing it as `null` would
/// file the one result that most needs reading as a result that does not
/// exist.
pub(crate) mod json_f64 {
    use serde::de::{Error, Unexpected};
    use serde::{Deserialize, Deserializer, Serializer};

    const POSITIVE_INFINITY: &str = "inf";
    const NEGATIVE_INFINITY: &str = "-inf";

    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Encoded {
        Number(f64),
        Marker(String),
        NotComputed,
    }

    pub fn serialize<S: Serializer>(value: &f64, serializer: S) -> Result<S::Ok, S::Error> {
        if value.is_finite() {
            serializer.serialize_f64(*value)
        } else if *value == f64::INFINITY {
            serializer.serialize_str(POSITIVE_INFINITY)
        } else if *value == f64::NEG_INFINITY {
            serializer.serialize_str(NEGATIVE_INFINITY)
        } else {
            serializer.serialize_none()
        }
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<f64, D::Error> {
        match Encoded::deserialize(deserializer)? {
            Encoded::Number(value) => Ok(value),
            Encoded::Marker(marker) if marker == POSITIVE_INFINITY => Ok(f64::INFINITY),
            Encoded::Marker(marker) if marker == NEGATIVE_INFINITY => Ok(f64::NEG_INFINITY),
            Encoded::Marker(marker) => Err(D::Error::invalid_value(
                Unexpected::Str(&marker),
                &"a number, null, \"inf\" or \"-inf\"",
            )),
            Encoded::NotComputed => Ok(f64::NAN),
        }
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

    fn observed_readout(rank_error: Option<f64>, guarantee: Option<ResultGuarantee>) -> Readout {
        let observed_error = match rank_error {
            Some(error) => ObservedError::Measured {
                metric: "rank".to_string(),
                error,
            },
            None => ObservedError::NotVerified,
        };
        Readout {
            node: asap_types::post_asap::PostAsapNodeId(2),
            producer: asap_types::post_asap::PostAsapNodeId(1),
            group: String::new(),
            query: SketchQuery::Quantile { q: 0.5 },
            approximate: Answer::Scalar(12.0),
            exact: Some(12.0),
            observed_error,
            guarantee,
            observations: 200_000,
        }
    }

    #[test]
    fn observations_accumulate_across_seeds_into_one_check() {
        let mut observations = GuaranteeObservations::default();
        for err in [0.001, 0.002, 0.02, 0.003] {
            observations.observe(&observed_readout(Some(err), Some(planner_kll_guarantee())));
        }

        let checks = observations.checks();
        assert_eq!(checks.len(), 1, "one readout identity, one check");
        let check = &checks[0].check;
        assert_eq!(check.seeds, 4);
        assert_eq!(check.observed_violation_rate, 0.25);
        assert_eq!(check.max_error, 0.02);
        assert!(
            check
                .implied_fit
                .as_ref()
                .expect("Kll fit")
                .agrees_with_reference
        );
    }

    #[test]
    fn a_readout_without_a_guarantee_produces_no_check() {
        let mut observations = GuaranteeObservations::default();
        observations.observe(&observed_readout(Some(0.001), None));
        assert!(observations.checks().is_empty());
    }

    #[test]
    fn an_unverified_readout_is_counted_as_unchecked_not_as_zero_violations() {
        let mut observations = GuaranteeObservations::default();
        for _ in 0..3 {
            observations.observe(&observed_readout(None, Some(planner_kll_guarantee())));
        }

        let checks = observations.checks();
        let check = &checks[0].check;
        assert_eq!(check.seeds, 0);
        assert!(check.observed_violation_rate.is_nan());
        assert!(serde_json::to_value(check).unwrap()["observed_violation_rate"].is_null());
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
            let answer = exact_answer_sorted(&values, &SketchQuery::Quantile { q }).unwrap();
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
        let median = exact_answer_sorted(&values, &SketchQuery::Quantile { q: 0.5 }).unwrap();
        assert_eq!(median, 51.0, "value at rank floor(0.5 * 100)");

        // An exact accumulator read out as a bucket total.
        let total = exact_answer_sorted(
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
                exact_answer_sorted(&values, &unreachable).is_err(),
                "{unreachable:?} must not produce a number"
            );
        }
    }

    #[test]
    fn exact_answer_refuses_what_it_cannot_compute() {
        let values = vec![1.0, 2.0];
        assert!(exact_answer_sorted(&values, &SketchQuery::TopK { k: 3 }).is_err());
        assert!(exact_answer_sorted(&values, &SketchQuery::Quantile { q: 1.5 }).is_err());
        assert!(exact_answer_sorted(&[], &SketchQuery::Quantile { q: 0.5 }).is_err());
        assert!(exact_answer_sorted(
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
        assert_eq!(check.mean_error, 0.7);
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
        assert_eq!(check.max_error, bound * 3.0);
        assert_eq!(check.metric, "rank");
    }

    #[test]
    fn no_observations_yields_no_rates() {
        let check = check_guarantee(&planner_kll_guarantee(), 0.5, &[]);
        assert_eq!(check.seeds, 0);
        assert!(check.observed_violation_rate.is_nan());
        assert!(check.mean_error.is_nan());
        assert!(check.max_error.is_nan());
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

    fn planner_ddsketch_guarantee(alpha: f64) -> ResultGuarantee {
        ResultGuarantee {
            metric: ErrorMetric::RelativeValue,
            bound: BoundExpr::Constant { value: alpha },
            failure_probability: ProbabilityExpr::Zero,
            provenance: vec![GuaranteeSource::SketchReadout {
                algorithm: "DDSketch".into(),
                contract: "ddsketch_relative_error_alpha_v1".into(),
                params: serde_json::json!({ "DDSketch": { "alpha": alpha } }),
                query: "Quantile { q: 0.5 }".into(),
            }],
        }
    }

    fn guarantee_with_metric(metric: ErrorMetric, bound: f64) -> ResultGuarantee {
        ResultGuarantee {
            metric,
            bound: BoundExpr::Constant { value: bound },
            failure_probability: ProbabilityExpr::Constant { value: 0.01 },
            provenance: vec![],
        }
    }

    #[test]
    fn a_rank_error_is_the_same_whether_or_not_the_caller_sorted_the_column() {
        let sorted = uniform_0_10000();
        let mut shuffled = sorted.clone();
        shuffled.rotate_left(3_137);
        shuffled.swap(0, 9_999);
        assert!(!is_sorted_by_total_cmp(&shuffled));

        let query = SketchQuery::Quantile { q: 0.5 };
        let guarantee = guarantee_with_metric(ErrorMetric::Rank, 0.01);
        let approximate = Answer::Scalar(5_100.0);

        assert_eq!(
            error_under_metric(&ErrorMetric::Rank, &shuffled, &query, 5_100.0, 5_000.0),
            error_under_metric(&ErrorMetric::Rank, &sorted, &query, 5_100.0, 5_000.0),
            "the metric must not read a rank off an unsorted slice"
        );
        assert_eq!(
            error_under_metric(&ErrorMetric::Rank, &shuffled, &query, 5_100.0, 5_000.0),
            Ok(0.01)
        );
        assert_eq!(
            observed_error(Some(&guarantee), &shuffled, &query, &approximate, 5_000.0),
            observed_error(Some(&guarantee), &sorted, &query, &approximate, 5_000.0)
        );
    }

    #[test]
    fn a_relative_value_bound_is_measured_by_relative_error_not_by_rank_error() {
        let values = uniform_0_10000();
        let query = SketchQuery::Quantile { q: 0.5 };
        let truth = exact_answer_sorted(&values, &query).unwrap();
        assert_eq!(truth, 5000.0);
        let estimate = 5100.0;

        let rank = rank_error(&values, estimate, 0.5);
        let relative = error_under_metric(
            &ErrorMetric::RelativeValue,
            &values,
            &query,
            estimate,
            truth,
        )
        .expect("the retained column answers a relative-value error");
        assert_eq!(relative, 100.0 / 5000.0);
        assert_ne!(relative, rank);

        let alpha = 0.01;
        assert!(relative > alpha);
        assert!(rank <= alpha);

        let check = check_guarantee(
            &planner_ddsketch_guarantee(alpha),
            0.5,
            &[(estimate, relative)],
        );
        assert_eq!(check.metric, "relative_value");
        assert_eq!(check.unevaluatable, None);
        assert_eq!(check.claimed_bound, Some(alpha));
        assert_eq!(check.observed_violation_rate, 1.0);
        assert_eq!(check.max_error, relative);
    }

    /// `NaN > bound` is `false`, so an error that does not exist used to land
    /// in the denominator as a draw that stayed inside the bound, and the
    /// record published "checked, zero violations".
    #[test]
    fn a_nan_error_is_never_reported_as_a_clean_observation() {
        let check = check_guarantee(
            &guarantee_with_metric(ErrorMetric::RelativeValue, 0.01),
            f64::NAN,
            &[(1.0, f64::NAN)],
        );

        assert_eq!(check.seeds, 1);
        assert_eq!(check.uncomputed_errors, 1);
        assert!(
            check.observed_violation_rate.is_nan(),
            "a rate of {} claims an observation was checked and clean",
            check.observed_violation_rate
        );

        let json = serde_json::to_value(&check).expect("serializes");
        assert!(
            json["observed_violation_rate"].is_null(),
            "{json} reads as a checked rate"
        );
        assert_eq!(json["uncomputed_errors"], 1);
    }

    /// The mixed case: nine measurable observations must not vouch for the
    /// tenth, which produced no error at all.
    #[test]
    fn one_nan_among_clean_observations_blocks_the_whole_rate() {
        let mut observations = vec![(1.0, 0.001); 9];
        observations.push((1.0, f64::NAN));

        let check = check_guarantee(
            &guarantee_with_metric(ErrorMetric::RelativeValue, 0.01),
            f64::NAN,
            &observations,
        );

        assert_eq!(check.seeds, 10);
        assert_eq!(check.uncomputed_errors, 1);
        assert!(check.observed_violation_rate.is_nan());
        assert!(
            check.max_error.is_nan() && check.mean_error.is_nan(),
            "max {} / mean {} summarize nine of ten observations as if they were ten",
            check.max_error,
            check.mean_error
        );
    }

    /// The other side of the same coin: `+inf` is a computed answer — the
    /// estimate missed a truth of zero — and it is a violation, not a gap.
    #[test]
    fn an_infinite_relative_error_counts_as_a_violation() {
        assert_eq!(relative_error(3.0, 0.0), f64::INFINITY);

        let check = check_guarantee(
            &guarantee_with_metric(ErrorMetric::RelativeValue, 0.01),
            f64::NAN,
            &[(3.0, f64::INFINITY)],
        );

        assert_eq!(check.observed_violation_rate, 1.0);
        assert_eq!(check.uncomputed_errors, 0);
        assert_eq!(check.max_error, f64::INFINITY);
        assert_eq!(check.unevaluatable, None);

        // And it survives the JSON round trip as infinity rather than as the
        // `null` that means "no error was computed".
        let json = serde_json::to_string(&check).expect("serializes");
        let back: GuaranteeCheck = serde_json::from_str(&json).expect("round trips");
        assert_eq!(back.max_error, f64::INFINITY);
        assert!(json.contains("\"max_error\":\"inf\""), "{json}");
    }

    #[test]
    fn a_nan_error_is_unevaluatable_rather_than_a_measurement() {
        let guarantee = guarantee_with_metric(ErrorMetric::RelativeValue, 0.01);
        let query = SketchQuery::PointCount {
            key: ColumnRef::SampleValue,
            value: None,
        };

        let observed = observed_error(
            Some(&guarantee),
            &[1.0, 2.0],
            &query,
            &Answer::Scalar(f64::NAN),
            3.0,
        );

        assert_eq!(
            observed,
            ObservedError::Unevaluatable {
                metric: "relative_value".to_string(),
                reason: UnevaluatableReason::ErrorIsNotANumber,
            }
        );
        assert_eq!(observed.measured(), None);
    }

    /// An unevaluatable observation that simply vanished from `pairs` would
    /// shrink the denominator and let the survivors publish a clean rate.
    #[test]
    fn an_uncomputable_error_still_reaches_the_population_it_belongs_to() {
        let mut observations = GuaranteeObservations::default();
        for error in [Some(0.001), Some(0.002)] {
            observations.observe(&observed_readout(error, Some(planner_kll_guarantee())));
        }
        let mut uncomputable = observed_readout(Some(0.0), Some(planner_kll_guarantee()));
        uncomputable.observed_error = ObservedError::Unevaluatable {
            metric: "rank".to_string(),
            reason: UnevaluatableReason::ErrorIsNotANumber,
        };
        observations.observe(&uncomputable);

        let checks = observations.checks();
        assert_eq!(checks.len(), 1);
        let check = &checks[0].check;
        assert_eq!(check.seeds, 3, "the third observation was dropped");
        assert_eq!(check.uncomputed_errors, 1);
        assert!(check.observed_violation_rate.is_nan());
    }

    #[test]
    fn an_absolute_value_bound_is_measured_in_the_values_own_units() {
        let values = uniform_0_10000();
        let query = SketchQuery::Quantile { q: 0.5 };
        let error =
            error_under_metric(&ErrorMetric::AbsoluteValue, &values, &query, 5100.0, 5000.0)
                .expect("an absolute error needs nothing the column does not have");
        assert_eq!(error, 100.0);
    }

    #[test]
    fn a_rank_bound_on_a_readout_with_no_quantile_is_not_measured_as_zero() {
        let values = uniform_0_10000();
        let total = SketchQuery::PointCount {
            key: ColumnRef::SampleValue,
            value: None,
        };
        assert_eq!(
            error_under_metric(&ErrorMetric::Rank, &values, &total, 1.0, 1.0),
            Err(UnevaluatableReason::NoQuantileInTheQuery)
        );
    }

    #[test]
    fn the_metrics_the_retained_column_cannot_normalize_say_what_they_would_need() {
        let values = uniform_0_10000();
        let query = SketchQuery::Quantile { q: 0.5 };
        let approximate = Answer::Scalar(5100.0);

        for (metric, expected) in [
            (
                ErrorMetric::Cardinality,
                UnevaluatableReason::TrueDistinctCount,
            ),
            (ErrorMetric::Frequency, UnevaluatableReason::StreamL1Norm),
            (ErrorMetric::L2Frequency, UnevaluatableReason::StreamL2Norm),
            (
                ErrorMetric::TopKMembership,
                UnevaluatableReason::TrueTopKSet,
            ),
        ] {
            let guarantee = guarantee_with_metric(metric, 0.01);
            match observed_error(Some(&guarantee), &values, &query, &approximate, 5000.0) {
                ObservedError::Unevaluatable {
                    reason,
                    metric: name,
                } => {
                    assert_eq!(reason, expected, "{name}");
                }
                other => panic!("{metric:?} must not produce a number: {other:?}"),
            }
        }

        assert_eq!(
            observed_error(None, &values, &query, &approximate, 5000.0),
            ObservedError::NoGuarantee
        );
        assert!(matches!(
            observed_error(
                Some(&planner_kll_guarantee()),
                &values,
                &query,
                &approximate,
                5000.0
            ),
            ObservedError::Measured { .. }
        ));
    }

    #[test]
    fn an_unevaluatable_metric_is_reported_as_unevaluatable_not_as_zero_violations() {
        let guarantee = guarantee_with_metric(ErrorMetric::Cardinality, 0.0065);

        let mut observations = GuaranteeObservations::default();
        let mut readout = observed_readout(None, Some(guarantee.clone()));
        readout.observed_error = ObservedError::Unevaluatable {
            metric: "cardinality".to_string(),
            reason: UnevaluatableReason::TrueDistinctCount,
        };
        for _ in 0..4 {
            observations.observe(&readout);
        }

        let checks = observations.checks();
        let check = &checks[0].check;
        assert_eq!(
            check.claimed_bound,
            Some(0.0065),
            "the bound itself is known"
        );
        assert_eq!(
            check.seeds, 0,
            "an unevaluatable error is not an observation"
        );
        assert_eq!(
            check.unevaluatable,
            Some(UnevaluatableReason::TrueDistinctCount)
        );
        assert!(check.observed_violation_rate.is_nan());
        assert_ne!(check.observed_violation_rate, 0.0);

        let json = serde_json::to_value(check).unwrap();
        assert_eq!(json["unevaluatable"], "true_distinct_count");
        assert!(json["observed_violation_rate"].is_null());

        // And an error measured under some *other* rule cannot back-fill it.
        let smuggled = check_guarantee(&guarantee, f64::NAN, &[(1.0, 0.0), (1.0, 0.0)]);
        assert!(smuggled.observed_violation_rate.is_nan());
    }

    #[test]
    fn a_non_finite_measured_error_round_trips_instead_of_becoming_null() {
        for (error, spelling) in [
            (0.25_f64, "0.25"),
            (f64::INFINITY, "\"inf\""),
            (f64::NEG_INFINITY, "\"-inf\""),
            (f64::NAN, "null"),
        ] {
            let observed = ObservedError::Measured {
                metric: "relative_value".to_string(),
                error,
            };
            let json = serde_json::to_string(&observed).expect("serializes");
            assert!(json.contains(&format!("\"error\":{spelling}")), "{json}");

            let back: ObservedError = serde_json::from_str(&json)
                .unwrap_or_else(|e| panic!("{json} must read back, got {e}"));
            let ObservedError::Measured { error: read, .. } = back else {
                panic!("{json} came back as a different measurement");
            };
            assert_eq!(read.total_cmp(&error), std::cmp::Ordering::Equal, "{json}");
        }
    }

    #[test]
    fn a_non_finite_estimate_is_not_an_answer_and_is_never_measured() {
        let values = uniform_0_10000();
        let query = SketchQuery::Quantile { q: 0.5 };

        for estimate in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            for q in [0.5, 0.99] {
                assert!(
                    rank_error(&values, estimate, q).is_nan(),
                    "rank_error({estimate}, {q}) fabricated a number"
                );
            }
            assert_eq!(
                error_under_metric(&ErrorMetric::Rank, &values, &query, estimate, 5000.0),
                Err(UnevaluatableReason::ErrorIsNotANumber),
                "{estimate}"
            );

            let observed = observed_error(
                Some(&planner_kll_guarantee()),
                &values,
                &query,
                &Answer::Scalar(estimate),
                5000.0,
            );
            assert_eq!(
                observed,
                ObservedError::Unevaluatable {
                    metric: "rank".to_string(),
                    reason: UnevaluatableReason::ErrorIsNotANumber,
                },
                "{estimate}"
            );
            assert_eq!(observed.measured(), None, "{estimate}");
        }

        let total = SketchQuery::PointCount {
            key: ColumnRef::SampleValue,
            value: None,
        };
        let infinite = observed_error(
            Some(&guarantee_with_metric(ErrorMetric::RelativeValue, 0.01)),
            &values,
            &total,
            &Answer::Scalar(3.0),
            0.0,
        );
        assert_eq!(
            infinite.measured(),
            Some(f64::INFINITY),
            "an infinite error is a real result and must stay a measurement"
        );
    }

    #[test]
    fn the_guarantee_stream_carries_its_own_version() {
        let mut observations = GuaranteeObservations::default();
        observations.observe(&observed_readout(
            Some(0.001),
            Some(planner_kll_guarantee()),
        ));

        let checks = observations.checks();
        assert_eq!(checks[0].schema_version, PLANEVAL_GUARANTEE_SCHEMA_VERSION);

        let line = serde_json::to_string(&checks[0]).expect("serializes");
        assert!(!line.contains('\n'), "one guarantee, one line");
        assert!(line.contains("\"schema_version\":1"), "{line}");

        let back: ReadoutGuarantee =
            serde_json::from_str(&line).expect("the guarantee document round trips");
        assert_eq!(back.schema_version, PLANEVAL_GUARANTEE_SCHEMA_VERSION);
        assert_eq!(back.node, checks[0].node);
    }

    #[test]
    fn the_readout_error_round_trips_as_typed_data() {
        for error in [
            ObservedError::NotVerified,
            ObservedError::NoGuarantee,
            ObservedError::Measured {
                metric: "rank".to_string(),
                error: 0.002,
            },
            ObservedError::Unevaluatable {
                metric: "frequency".to_string(),
                reason: UnevaluatableReason::StreamL1Norm,
            },
        ] {
            let json = serde_json::to_string(&error).unwrap();
            let back: ObservedError = serde_json::from_str(&json).unwrap();
            assert_eq!(back, error);
            assert_eq!(
                error.measured().is_some(),
                matches!(error, ObservedError::Measured { .. })
            );
        }
    }
}
