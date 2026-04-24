//! Exact-algorithm baselines per family.
//!
//! These are not sketches. They keep enough state to answer the
//! family's query with zero error, exposed through the same
//! [`sketch_core::sketch::Sketch`] trait so the bench runner can
//! time them on the exact same insert / query path as a sketch.
//!
//! Purposes:
//!
//! 1. **CLI comparability.** `sketchlib bench --sketch hll --impl
//!    exact,oxide,…` produces one v1 JSONL row per variant. The
//!    accuracy comparator reports zero error against the exact
//!    impl, which both (a) gives the CLI a throughput / CPU /
//!    memory reference point and (b) serves as a sanity check
//!    that the ground-truth path is wired correctly.
//!
//! 2. **Shared ground-truth algorithm.** The `accuracy/` harness
//!    crates previously each held their own copy of the same
//!    HashMap / HashSet / sorted-Vec logic in
//!    `accuracy/{family}/rust/src/baseline.rs`. Those now
//!    delegate to these types so there is a single source of
//!    truth for "what is the correct answer here".
//!
//! Dispatch constraint: the sweep driver marks these impls as
//! `Constraint::Unparameterized` — they ignore the family's
//! `ParamSet`, so the driver runs each exactly once per
//! invocation instead of once per config in the grid.

pub mod cms;
pub mod hll;
pub mod kll;

pub use cms::ExactCms;
pub use hll::ExactHll;
pub use kll::ExactKll;
