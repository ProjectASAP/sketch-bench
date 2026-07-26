//! Ground-truth accuracy comparators: the sketch-domain implementations of
//! the generic [`GroundTruth`] trait, one family per module (cardinality,
//! frequency, quantile, top-k).
//!
//! The trait itself, the [`Comparison`] it returns and the [`QueryCallSample`]
//! telemetry it may carry know nothing about sketch families, so they live in
//! `aqpbm-core` and are re-exported here beside the comparators that populate
//! them.
//!
//! See `docs/DESIGN.md` §5.6.

pub use aqpbm_core::accuracy::{Comparison, GroundTruth};
pub use aqpbm_core::metrics::QueryCallSample;

pub mod cardinality;
pub mod frequency;
pub mod quantile;
pub mod statistic;
pub mod topk;

// The capability traits that declare which sketch answers which statistic.
pub use statistic::{CardinalityOps, FrequencyOps, QuantileOps, TopKOps};
