//! Exact-baseline wrappers used by `dispatch.rs`.
//!
//! The algorithms live in [`sketch_bench::baselines`] so both
//! the CLI and the `accuracy/` harness crates share a single
//! implementation. This file only re-exports the types so the
//! dispatch macros can reference them as `exact::ExactHll`,
//! `exact::ExactCms`, `exact::ExactKll` in the same shape as
//! every other wrapper.

pub use sketch_bench::baselines::{ExactCms, ExactHll, ExactKll};
