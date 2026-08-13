//! How a ground truth is named, and how a registration builds one.
//!
//! Next to the ground truths rather than next to the registry: what a
//! `SubpopFrequencyGT` is built with is a fact about that struct, and every
//! choice below is visible beside the struct it configures.

use crate::accumulator::Accumulator;
use crate::config::ParamSet;

use super::cardinality::CardinalityGT;
use super::frequency::FrequencyGT;
use super::quantile::{RankErrorGT, RelativeErrorGT};
use super::subpopulation::{SubpopCardinalityGT, SubpopFrequencyGT, SubpopRankErrorGT};
use super::topk::TopkGT;
use super::GroundTruth;

/// Not generic over `S`, so a name can be read without naming a sketch type —
/// which a row whose sketch type is chosen at run time needs.
pub trait GroundTruthName {
    const NAME: &'static str;
}

/// A [`GroundTruth`] that builds itself from the row's params. A trait, not a
/// `fn` argument, so it is named as a *type* and the row stays `const`.
pub trait GroundTruthCalculator<S: Accumulator>: GroundTruth<S> + GroundTruthName {
    fn build(params: &ParamSet) -> Self;
}

macro_rules! ground_truth_name {
    ($ty:ty, $name:literal) => {
        impl GroundTruthName for $ty {
            const NAME: &'static str = $name;
        }
    };
}

ground_truth_name!(CardinalityGT, "cardinality");
ground_truth_name!(FrequencyGT, "frequency");
ground_truth_name!(SubpopFrequencyGT, "subpop-frequency");
ground_truth_name!(SubpopCardinalityGT, "subpop-cardinality");
ground_truth_name!(SubpopRankErrorGT, "subpop-rank-error");
ground_truth_name!(RankErrorGT, "rank-error");
ground_truth_name!(RelativeErrorGT, "relative-error");
ground_truth_name!(TopkGT, "topk");

impl<S: Accumulator> GroundTruthCalculator<S> for CardinalityGT
where
    Self: GroundTruth<S>,
{
    fn build(_params: &ParamSet) -> Self {
        CardinalityGT
    }
}

impl<S: Accumulator> GroundTruthCalculator<S> for FrequencyGT
where
    Self: GroundTruth<S>,
{
    fn build(_params: &ParamSet) -> Self {
        FrequencyGT
    }
}

impl<S: Accumulator> GroundTruthCalculator<S> for SubpopFrequencyGT
where
    Self: GroundTruth<S>,
{
    /// Column 0. Another column, or a deeper subset, is a second row.
    fn build(_params: &ParamSet) -> Self {
        SubpopFrequencyGT { label_column: 0 }
    }
}

impl<S: Accumulator> GroundTruthCalculator<S> for SubpopCardinalityGT
where
    Self: GroundTruth<S>,
{
    fn build(_params: &ParamSet) -> Self {
        SubpopCardinalityGT { label_column: 0 }
    }
}

impl<S: Accumulator> GroundTruthCalculator<S> for SubpopRankErrorGT
where
    Self: GroundTruth<S>,
{
    fn build(_params: &ParamSet) -> Self {
        SubpopRankErrorGT { label_column: 0 }
    }
}

impl<S: Accumulator> GroundTruthCalculator<S> for RankErrorGT
where
    Self: GroundTruth<S>,
{
    fn build(_params: &ParamSet) -> Self {
        RankErrorGT {}
    }
}

impl<S: Accumulator> GroundTruthCalculator<S> for RelativeErrorGT
where
    Self: GroundTruth<S>,
{
    fn build(_params: &ParamSet) -> Self {
        RelativeErrorGT {}
    }
}

impl<S: Accumulator> GroundTruthCalculator<S> for TopkGT
where
    Self: GroundTruth<S>,
{
    /// Scores against the same `k` the sketch was built with — a different prefix
    /// would measure the mismatch, not the sketch. Infallible because the timed
    /// half runs first, so an unreadable `k` has already failed the build.
    fn build(params: &ParamSet) -> Self {
        TopkGT {
            k: params
                .field::<usize>("k")
                .expect("the row built, so its params parse and carry `k`"),
        }
    }
}
