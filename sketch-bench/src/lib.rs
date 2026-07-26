//! `sketch-bench` — the sketch domain layered on the `aqpbm-core`
//! benchmark engine: the wrapped sketch implementations, the
//! per-family accuracy comparators, the catalog that names which
//! `(family, impl)` pairs exist, and the legacy CSV rendering.
//!
//! The engine itself ([`BenchRunner`], [`BenchReport`]) lives in
//! `aqpbm-core` and is re-exported here. See `docs/DESIGN.md` §5.

pub mod accuracy;
pub mod catalog;
pub mod cell;
pub mod init;
pub mod legacy_csv;
pub mod params;
pub mod wrappers;

pub use init::{BuildError, InitSketch};
// The benchmark engine lives in `aqpbm-core`; these four are re-exported
// because this crate and the CLI name them through `sketch_bench::`.
// Anything else the engine exposes is reached at `aqpbm_core::` directly.
pub use aqpbm_core::metrics::MetricsMask;
pub use aqpbm_core::runner::{BenchConfig, BenchReport, BenchRunner};
