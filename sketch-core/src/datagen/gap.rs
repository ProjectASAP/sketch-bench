//! Inter-arrival gap distributions for the monotonic-timestamp shape.
//!
//! A gap is a non-negative integer number of *ticks* (in the units the
//! timestamp shape declares). The timestamp generator sums gaps to
//! build a monotonically non-decreasing sequence.

use rand_distr::{Distribution, Exp, Geometric, Poisson};
use rand_xoshiro::Xoshiro256PlusPlus;
use serde::{Deserialize, Serialize};

use crate::error::SketchCoreError;

/// A serde-tagged inter-arrival gap distribution. Extend by adding a
/// variant here plus an arm in [`GapDist::build`] / [`GapSampler`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum GapDist {
    /// Fixed gap of `step` ticks (a regular clock).
    Constant { step: u64 },
    /// Geometric number of ticks with success probability `p`.
    Geometric { p: f64 },
    /// Exponential inter-arrival time with rate `lambda`, rounded to the
    /// nearest tick (a Poisson process in continuous time).
    Exponential { lambda: f64 },
    /// Poisson-distributed ticks with mean `lambda`.
    Poisson { lambda: f64 },
}

impl GapDist {
    /// Construct the sampler, validating parameters eagerly.
    pub fn build(&self) -> Result<GapSampler, SketchCoreError> {
        Ok(match *self {
            GapDist::Constant { step } => GapSampler::Constant(step),
            GapDist::Geometric { p } => GapSampler::Geometric(
                Geometric::new(p).map_err(|e| SketchCoreError::BadParam(format!("geometric gap: {e}")))?,
            ),
            GapDist::Exponential { lambda } => GapSampler::Exponential(
                Exp::new(lambda).map_err(|e| SketchCoreError::BadParam(format!("exponential gap: {e}")))?,
            ),
            GapDist::Poisson { lambda } => GapSampler::Poisson(
                Poisson::new(lambda).map_err(|e| SketchCoreError::BadParam(format!("poisson gap: {e}")))?,
            ),
        })
    }
}

/// A prepared gap sampler. Constructed once via [`GapDist::build`], then
/// sampled per element.
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
            GapSampler::Exponential(d) => {
                let t: f64 = d.sample(rng);
                t.round().max(0.0) as u64
            }
            GapSampler::Poisson(d) => {
                let t: f64 = d.sample(rng);
                t.round().max(0.0) as u64
            }
        }
    }
}
