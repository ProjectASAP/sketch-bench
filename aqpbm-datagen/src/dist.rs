//! The distribution axis: *how* a column's values are spread, and the seed that
//! makes the spread reproducible. Three distributions, each carrying its own
//! parameters and its own seed — so two columns are independent unless they were
//! deliberately given the same seed.
//!
//! Every distribution draws an `f64`. Zipf's draws happen to be integral ranks,
//! but the raw stream is one type so the rule / shift / render pipeline in
//! [`crate::column`] is written once instead of three times.

use rand_distr::{Distribution as _, Normal, Uniform, Zipf};
use rand_xoshiro::Xoshiro256PlusPlus;
use serde::{Deserialize, Serialize};

use crate::error::DataGenError;

fn bad(msg: String) -> DataGenError {
    DataGenError::BadParam(msg)
}

/// Zipfian: rank `r` appears with probability `∝ r^-skewness`, over the ranks
/// `1..=population_size`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ZipfParameter {
    pub skewness: f64,
    pub population_size: u64,
    pub seed: u64,
}

/// Flat over `[lower_bound, upper_bound)`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UniformParameter {
    pub lower_bound: f64,
    pub upper_bound: f64,
    pub seed: u64,
}

/// Gaussian. Unbounded, which is why it has no domain (see [`DataDistribution::domain`]).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NormalParameter {
    pub mean: f64,
    pub standard_deviation: f64,
    pub seed: u64,
}

/// A column's distribution. Serialises internally tagged, so a spec file reads
/// `distribution: {kind: zipf, skewness: 1.1, population_size: 200, seed: 1}`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DataDistribution {
    Zipf(ZipfParameter),
    Uniform(UniformParameter),
    Normal(NormalParameter),
}

/// The half-open span a bounded distribution draws over: the lowest value it can
/// produce, and how many distinct integer positions the span holds. Both halves
/// are needed — the lower bound turns a draw into a 0-based rank for string
/// rendering, and the size is what a `cardinality` claim is checked against.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Domain {
    pub lower: f64,
    pub size: u64,
}

impl DataDistribution {
    /// A short tag for reports and error messages.
    pub fn tag(&self) -> &'static str {
        match self {
            DataDistribution::Zipf(_) => "zipf",
            DataDistribution::Uniform(_) => "uniform",
            DataDistribution::Normal(_) => "normal",
        }
    }

    /// The seed this column draws from.
    pub fn seed(&self) -> u64 {
        match self {
            DataDistribution::Zipf(p) => p.seed,
            DataDistribution::Uniform(p) => p.seed,
            DataDistribution::Normal(p) => p.seed,
        }
    }

    /// The bounded span, or `None` for Normal — which is unbounded, so a
    /// `cardinality` over it would be a claim this crate cannot keep.
    pub fn domain(&self) -> Option<Domain> {
        match self {
            DataDistribution::Zipf(p) => Some(Domain {
                // Ranks run `1..=population_size`, so rank 1 is the 0th position.
                lower: 1.0,
                size: p.population_size,
            }),
            DataDistribution::Uniform(p) => Some(Domain {
                lower: p.lower_bound,
                size: (p.upper_bound - p.lower_bound) as u64,
            }),
            DataDistribution::Normal(_) => None,
        }
    }

    /// Build the sampler, validating parameters eagerly so a bad description
    /// fails before any allocation.
    pub fn sampler(&self) -> Result<Sampler, DataGenError> {
        match self {
            DataDistribution::Zipf(p) => {
                if p.population_size == 0 {
                    return Err(bad("zipf: population_size must be > 0".into()));
                }
                Ok(Sampler::Zipf(
                    Zipf::new(p.population_size, p.skewness)
                        .map_err(|e| bad(format!("zipf: {e}")))?,
                ))
            }
            DataDistribution::Uniform(p) => {
                // `is_finite` first, so a NaN bound is named rather than
                // slipping through a comparison that is false either way.
                if !p.lower_bound.is_finite()
                    || !p.upper_bound.is_finite()
                    || p.lower_bound >= p.upper_bound
                {
                    return Err(bad(format!(
                        "uniform: lower_bound {} must be finite and below upper_bound {}",
                        p.lower_bound, p.upper_bound
                    )));
                }
                Ok(Sampler::Uniform(Uniform::new(p.lower_bound, p.upper_bound)))
            }
            DataDistribution::Normal(p) => {
                if !p.standard_deviation.is_finite() || p.standard_deviation <= 0.0 {
                    return Err(bad(format!(
                        "normal: standard_deviation must be finite and > 0, got {}",
                        p.standard_deviation
                    )));
                }
                Ok(Sampler::Normal(
                    Normal::new(p.mean, p.standard_deviation)
                        .map_err(|e| bad(format!("normal: {e}")))?,
                ))
            }
        }
    }
}

/// A prepared sampler. Built once via [`DataDistribution::sampler`], then drawn
/// from per value.
pub enum Sampler {
    Zipf(Zipf<f64>),
    Uniform(Uniform<f64>),
    Normal(Normal<f64>),
}

impl Sampler {
    #[inline]
    pub fn sample(&self, rng: &mut Xoshiro256PlusPlus) -> f64 {
        match self {
            Sampler::Zipf(d) => d.sample(rng),
            Sampler::Uniform(d) => d.sample(rng),
            Sampler::Normal(d) => d.sample(rng),
        }
    }
}
