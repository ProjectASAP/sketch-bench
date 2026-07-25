//! `sketch-bench` — the sketch domain layered on the `aqpbm-core`
//! benchmark engine: the wrapped sketch implementations, the
//! per-family accuracy comparators, the catalog that names which
//! `(family, impl)` pairs exist, and the legacy CSV rendering.
//!
//! The engine itself — [`BenchRunner`] driving a workload through a
//! fresh-sketch factory, collecting `RunMetrics` per run and
//! aggregating into a [`BenchReport`] v1 JSONL record — lives in
//! `aqpbm-core` and is re-exported here.
//!
//! See `docs/DESIGN.md` §5 for the full contract.

pub mod accuracy;
pub mod catalog;
pub mod cell;
pub mod init;
pub mod legacy_csv;
pub mod params;
pub mod wrappers;

pub use init::{BuildError, InitSketch};
// The benchmark engine — runner, config, metric records, and the
// generic accuracy abstraction — lives in `aqpbm-core`. Re-exported
// here so `sketch_bench::{BenchRunner, BenchConfig, RunMetrics, ...}`
// still name the library's entry points for existing callers.
pub use aqpbm_core::latency::LatencySnapshot;
pub use aqpbm_core::metrics::{FullSink, MetricsMask, RunMetrics};
pub use aqpbm_core::runner::{BenchConfig, BenchReport, BenchRunner, NoGT};
