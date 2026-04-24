//! Exact-baseline wrappers used by `dispatch.rs`.
//!
//! The algorithms live in [`sketch_bench::baselines`], organised
//! by statistic (cardinality / frequency / quantile). This file
//! only re-exports them so the dispatch macros can reference
//! `exact::ExactCardinality`, `exact::ExactFrequency`,
//! `exact::ExactQuantile` in the same shape as every other
//! wrapper. See `sketch_bench::baselines` for the full statistic
//! → sketches → baseline mapping.

pub use sketch_bench::baselines::{ExactCardinality, ExactFrequency, ExactQuantile};
