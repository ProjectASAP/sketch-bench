//! `sketch-bench` — macro-metric benchmark library.
//!
//! Entrypoint: [`BenchRunner`] drives a workload through a
//! fresh-sketch factory, collects `RunMetrics` per run,
//! aggregates into a [`BenchReport`] that serialises as a v1
//! JSONL record.
//!
//! See `docs/DESIGN.md` §5 for the full contract.

pub mod accuracy;
pub mod aggregation;
pub mod catalog;
pub mod cell;
pub mod config;
pub mod init;
pub mod legacy_csv;
pub mod params;
pub mod runner;
pub mod wrappers;

pub use config::{BenchConfig, MetricsMask};
pub use init::{BuildError, InitSketch};
// The metric primitives + per-run record moved to `aqpbm-core`;
// re-exported here so `sketch_bench::{FullSink, RunMetrics, ...}`
// still resolve for existing callers.
pub use aqpbm_core::latency::LatencySnapshot;
pub use aqpbm_core::metrics::{FullSink, RunMetrics};
pub use runner::{BenchReport, BenchRunner, NoGT};
