//! Weighting schemes for the finite-domain skewed-categorical shape.
//!
//! A `WeightSpec` resolves to one positive weight per category; the
//! generator draws category indices proportionally.

use serde::{Deserialize, Serialize};

use crate::error::SketchCoreError;

/// How to weight a fixed set of `k` categories. Extend by adding a
/// variant plus an arm in [`WeightSpec::resolve`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum WeightSpec {
    /// Every category equally likely.
    Uniform,
    /// Zipfian by category *index*: weight of category `i` is
    /// `1 / (i + 1)^s`, so earlier categories are heavier.
    Zipf { s: f64 },
    /// Explicit per-category weights; length must equal the number of
    /// categories.
    Explicit { weights: Vec<f64> },
}

impl WeightSpec {
    /// Resolve to exactly `k` positive, finite weights.
    pub fn resolve(&self, k: usize) -> Result<Vec<f64>, SketchCoreError> {
        let weights = match self {
            WeightSpec::Uniform => vec![1.0; k],
            WeightSpec::Zipf { s } => (0..k)
                .map(|i| 1.0 / ((i as f64) + 1.0).powf(*s))
                .collect(),
            WeightSpec::Explicit { weights } => {
                if weights.len() != k {
                    return Err(SketchCoreError::BadParam(format!(
                        "explicit weights: expected {k}, got {}",
                        weights.len()
                    )));
                }
                weights.clone()
            }
        };
        if weights.iter().any(|w| !w.is_finite() || *w <= 0.0) {
            return Err(SketchCoreError::BadParam(
                "weights must all be finite and > 0".into(),
            ));
        }
        Ok(weights)
    }
}
