//! The distribution axis.
//!
//! A [`Distribution`] says *how* values are spread; it says nothing
//! about what they mean. The structure that consumes it (keys /
//! categorical / monotonic — see [`super::shape::Shape`]) decides that,
//! and picks the realization engine its domain can afford:
//!
//! * a large key space samples directly (no per-value table),
//! * a small finite domain resolves to an explicit weight table,
//! * a monotonic series samples non-negative integer gaps.
//!
//! Because the engine is the *structure's* concern, each distribution
//! (uniform, zipf, …) is declared exactly once here instead of once per
//! structure — which is what removes the old `Shape::Zipf` /
//! `WeightSpec::Zipf` / `GapDist` duplication.

use rand_distr::{Distribution as _, Exp, Geometric, Poisson, Uniform, Zipf};
use rand_xoshiro::Xoshiro256PlusPlus;
use serde::{Deserialize, Serialize};

use crate::error::SketchError;

fn bad(msg: String) -> SketchError {
    SketchError::BadParam(msg)
}

/// A statistical shape, independent of the domain it is drawn over.
/// Extend by adding a variant plus the arm(s) in whichever realization
/// method(s) it supports.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Distribution {
    /// Flat: every outcome equally likely.
    Uniform,
    /// Zipfian: outcome `r` has probability `∝ r^-s`.
    Zipf { s: f64 },
    /// Geometric number of ticks with success probability `p`.
    Geometric { p: f64 },
    /// Exponential inter-arrival time with rate `lambda`, rounded to the
    /// nearest tick.
    Exponential { lambda: f64 },
    /// Poisson-distributed ticks with mean `lambda`.
    Poisson { lambda: f64 },
    /// A single fixed value (a degenerate distribution).
    Constant { value: u64 },
    /// Explicit per-outcome weights; only meaningful over a finite
    /// domain whose size equals `weights.len()`.
    Explicit { weights: Vec<f64> },
}

impl Distribution {
    /// A short tag for reports and error messages.
    pub fn tag(&self) -> &'static str {
        match self {
            Distribution::Uniform => "uniform",
            Distribution::Zipf { .. } => "zipf",
            Distribution::Geometric { .. } => "geometric",
            Distribution::Exponential { .. } => "exponential",
            Distribution::Poisson { .. } => "poisson",
            Distribution::Constant { .. } => "constant",
            Distribution::Explicit { .. } => "explicit",
        }
    }

    /// Realization for the **keys** structure: sample directly over a
    /// large `[0, cardinality)` domain, with no per-value table. Only
    /// the range-valued distributions apply here.
    pub fn key_sampler(&self, cardinality: u64) -> Result<KeySampler, SketchError> {
        match self {
            Distribution::Uniform => Ok(KeySampler::Uniform(Uniform::new(0u64, cardinality))),
            Distribution::Zipf { s } => Ok(KeySampler::Zipf(
                Zipf::new(cardinality, *s).map_err(|e| bad(format!("zipf: {e}")))?,
            )),
            Distribution::Constant { value } => Ok(KeySampler::Constant(*value)),
            other => Err(bad(format!(
                "{} is not a key distribution (use uniform, zipf, or constant)",
                other.tag()
            ))),
        }
    }

    /// Realization for the **categorical** structure: resolve to exactly
    /// `k` positive, finite weights over a small finite domain. Only the
    /// finitely-supported distributions apply here.
    pub fn weights(&self, k: usize) -> Result<Vec<f64>, SketchError> {
        let weights = match self {
            Distribution::Uniform => vec![1.0; k],
            // Zipf pmf evaluated analytically at k points — bounded to
            // the domain, unlike the sampler used by `key_sampler`.
            Distribution::Zipf { s } => (0..k).map(|i| 1.0 / ((i as f64) + 1.0).powf(*s)).collect(),
            Distribution::Explicit { weights } => {
                if weights.len() != k {
                    return Err(bad(format!(
                        "explicit weights: expected {k}, got {}",
                        weights.len()
                    )));
                }
                weights.clone()
            }
            other => {
                return Err(bad(format!(
                    "{} is not a categorical distribution (use uniform, zipf, or explicit)",
                    other.tag()
                )))
            }
        };
        if weights.iter().any(|w| !w.is_finite() || *w <= 0.0) {
            return Err(bad("weights must all be finite and > 0".into()));
        }
        Ok(weights)
    }

    /// Realization for the **monotonic** structure: a non-negative
    /// integer gap sampler. Only the count/interval distributions apply.
    pub fn gap_sampler(&self) -> Result<GapSampler, SketchError> {
        Ok(match *self {
            Distribution::Constant { value } => GapSampler::Constant(value),
            Distribution::Geometric { p } => GapSampler::Geometric(
                Geometric::new(p).map_err(|e| bad(format!("geometric gap: {e}")))?,
            ),
            Distribution::Exponential { lambda } => GapSampler::Exponential(
                Exp::new(lambda).map_err(|e| bad(format!("exponential gap: {e}")))?,
            ),
            Distribution::Poisson { lambda } => GapSampler::Poisson(
                Poisson::new(lambda).map_err(|e| bad(format!("poisson gap: {e}")))?,
            ),
            ref other => {
                return Err(bad(format!(
                    "{} is not a gap distribution (use constant, geometric, exp, or poisson)",
                    other.tag()
                )))
            }
        })
    }
}

/// A prepared key sampler over `[0, cardinality)` (or a constant).
/// Built once via [`Distribution::key_sampler`], then sampled per value.
pub enum KeySampler {
    Uniform(Uniform<u64>),
    Zipf(Zipf<f64>),
    Constant(u64),
}

impl KeySampler {
    pub fn sample(&self, rng: &mut Xoshiro256PlusPlus) -> u64 {
        match self {
            KeySampler::Uniform(d) => d.sample(rng),
            KeySampler::Zipf(d) => d.sample(rng) as u64,
            KeySampler::Constant(v) => *v,
        }
    }
}

/// A prepared gap sampler. Built once via [`Distribution::gap_sampler`],
/// then sampled per element.
pub enum GapSampler {
    Constant(u64),
    Geometric(Geometric),
    Exponential(Exp<f64>),
    Poisson(Poisson<f64>),
}

impl GapSampler {
    /// Draw a non-negative integer gap in ticks. Continuous
    /// distributions are rounded to the nearest tick and floored at 0.
    pub fn sample_ticks(&self, rng: &mut Xoshiro256PlusPlus) -> u64 {
        match self {
            GapSampler::Constant(step) => *step,
            GapSampler::Geometric(d) => d.sample(rng),
            GapSampler::Exponential(d) => d.sample(rng).round().max(0.0) as u64,
            GapSampler::Poisson(d) => d.sample(rng).round().max(0.0) as u64,
        }
    }
}
