//! Ground-truth accuracy comparators. Four built-in families
//! (frequency, cardinality, quantile, top-k) — the sketch-domain
//! implementations of the generic [`GroundTruth`] trait.
//!
//! The trait itself, the [`Comparison`] it returns, and the
//! [`QueryCallSample`] telemetry it may carry are all generic
//! over the core `Sketch` trait, so they live in
//! `aqpbm-core::accuracy` / `aqpbm-core::metrics` and are
//! re-exported here where the concrete comparators that populate
//! them live.
//!
//! See `docs/DESIGN.md` §5.6.

pub use aqpbm_core::accuracy::{Comparison, GroundTruth};
pub use aqpbm_core::metrics::QueryCallSample;

pub mod cardinality;
pub mod frequency;
pub mod quantile;
#[cfg(feature = "accuracy-topk")]
pub mod topk;
