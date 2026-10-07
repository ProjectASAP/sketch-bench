//! Each sketch algorithm's published accuracy guarantee, at 95% confidence,
//! in the metric the optimizer checks it by. [`crate::saturation`] falls back
//! to it where the study measured nothing: past an unsaturated curve, past
//! the measured merge counts or N, or with no merge curve for a point.
//!
//! The bounds hold for any stream length, so they don't depend on N. They are
//! looser than the measured curves.

use std::collections::BTreeMap;

use aqpbm_core::MeasuredShape;

use crate::analytical_cost_model::TOPK_ENTRIES;

/// One-sided confidence the bounds are stated at.
pub const CONFIDENCE: f64 = 0.95;

/// The bound on `sketch`'s error with `params` on data shaped `shape`,
/// answering from `merges` merged instances, or `None` when the algorithm
/// has none at [`CONFIDENCE`]:
///
/// - `kll-percall`: normalized rank error of one quantile. DataSketches
///   publishes `2.296 / k^0.9723` at 99%; KLL's error scales with
///   `sqrt(ln(1/δ))`, which gives the 95% value. KLL merges without loss of
///   this guarantee.
/// - `dd`: relative value error `α`, deterministic while the bucket limit
///   isn't reached; merges exactly.
/// - `hll`: relative error `1.96 · 1.04 / sqrt(2^lg_k)`; merges exactly.
/// - `cms-heap-topk-fastpath-vector2d`: precision@k on Zipf(θ) over `K`
///   keys. Each CMS estimate exceeds the true count by at most `e/w · N`
///   with probability `1 − e^−d`, so a top-k key whose share `p_i` beats
///   the (k+1)-th's by more than `e/w` stays ranked: precision is the
///   fraction of the top `k` that do. Needs `1 − e^−d ≥ 95%`. A merged
///   answer has none: each shard's heap keeps only its own top `k`.
pub fn bound(
    sketch: &str,
    params: &BTreeMap<String, f64>,
    shape: MeasuredShape,
    merges: u64,
) -> Option<f64> {
    match sketch {
        "kll-percall" => {
            let k = params.get("k")?;
            let at_99 = 2.296 / k.powf(0.9723);
            Some(at_99 * ((1.0 / (1.0 - CONFIDENCE)).ln() / 100f64.ln()).sqrt())
        }
        "dd" => params.get("alpha").copied(),
        "hll" => {
            let registers = 2f64.powf(*params.get("lg_k")?);
            Some(1.96 * 1.04 / registers.sqrt())
        }
        "cms-heap-topk-fastpath-vector2d" if merges <= 1 => {
            let (rows, cols) = (params.get("rows")?, params.get("cols")?);
            if 1.0 - (-rows).exp() < CONFIDENCE {
                return None;
            }
            let MeasuredShape::Zipf { skew, keys } = shape else {
                return None;
            };
            Some(topk_precision(skew, keys, std::f64::consts::E / cols))
        }
        _ => None,
    }
}

/// The share of the top [`TOPK_ENTRIES`] keys of Zipf(`skew`) over `keys`
/// keys whose probability exceeds the next key's by more than `slack`.
fn topk_precision(skew: f64, keys: f64, slack: f64) -> f64 {
    let k = TOPK_ENTRIES as usize;
    let keys = keys as usize;
    if keys <= k {
        return 1.0;
    }
    let weight = |rank: usize| (rank as f64).powf(-skew);
    let total: f64 = (1..=keys).map(weight).sum();
    let next = weight(k + 1) / total;
    let ranked = (1..=k)
        .filter(|&i| weight(i) / total - next > slack)
        .count();
    ranked as f64 / k as f64
}

#[cfg(test)]
mod tests {
    use super::*;

    fn params(pairs: &[(&str, f64)]) -> BTreeMap<String, f64> {
        pairs.iter().map(|&(k, v)| (k.to_string(), v)).collect()
    }

    const PARETO: MeasuredShape = MeasuredShape::Pareto { tail_index: 2.0 };

    #[test]
    fn kll_hll_and_dd_bounds_ignore_n_and_merges() {
        let kll = bound("kll-percall", &params(&[("k", 200.0)]), PARETO, 64).unwrap();
        // 1.32% at 99%, scaled by sqrt(ln 20 / ln 100).
        assert!((kll - 0.01068).abs() < 1e-4, "{kll}");
        assert_eq!(
            bound("dd", &params(&[("alpha", 0.01)]), PARETO, 1000),
            Some(0.01)
        );
        let hll = bound("hll", &params(&[("lg_k", 12.0)]), PARETO, 4).unwrap();
        assert!((hll - 1.96 * 1.04 / 64.0).abs() < 1e-12);
    }

    #[test]
    fn topk_precision_needs_the_rows_and_one_window() {
        let zipf = MeasuredShape::Zipf {
            skew: 1.1,
            keys: 1e4,
        };
        let wide = params(&[("rows", 3.0), ("cols", 16384.0)]);
        // e/w = 1.7e-4 separates all but the 32nd key of Zipf 1.1 over 1e4
        // keys from the 33rd.
        assert_eq!(
            bound("cms-heap-topk-fastpath-vector2d", &wide, zipf, 1),
            Some(31.0 / 32.0)
        );
        let narrow = params(&[("rows", 3.0), ("cols", 256.0)]);
        let p = bound("cms-heap-topk-fastpath-vector2d", &narrow, zipf, 1).unwrap();
        assert!(p < 1.0, "{p}");
        // Two rows: 1 − e^−2 = 86% < 95%.
        let two = params(&[("rows", 2.0), ("cols", 16384.0)]);
        assert_eq!(
            bound("cms-heap-topk-fastpath-vector2d", &two, zipf, 1),
            None
        );
        // Merged heaps: no guarantee.
        assert_eq!(
            bound("cms-heap-topk-fastpath-vector2d", &wide, zipf, 4),
            None
        );
    }

    #[test]
    fn sketches_without_a_published_bound_have_none() {
        assert_eq!(bound("univmon-topk", &BTreeMap::new(), PARETO, 1), None);
    }
}
