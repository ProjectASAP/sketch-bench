//! `aqpbm-core` — the benchmark framework. Everything needed to measure *a*
//! sketch; nothing that knows *which* sketches exist.
//!
//! # Benchmarking your own sketch
//!
//! This crate is the whole seam. Implement four things on your type and the
//! runners below will drive it, score it, and emit the same v1 JSONL records
//! the `sketchlib bench` CLI does:
//!
//! 1. [`accumulator::Accumulator`] — how an item is ingested (§4.1).
//! 2. [`init::InitSketch`] — how to build one from a [`config::ParamSet`], or
//!    why not.
//! 3. [`init::BenchImpl`] — what the row is called.
//! 4. One capability trait per statistic you answer
//!    ([`accuracy::CardinalityOps`], [`accuracy::FrequencyOps`],
//!    [`accuracy::QuantileOps`], [`accuracy::TopKOps`]) — this is what makes
//!    the matching comparator applicable to you, and it is compiler-checked.
//!
//! Then call [`cell::run_cell`] for the timed half and [`cell::score_cell`]
//! for the accuracy half. No registration step: nothing in this crate holds a
//! list of implementations, so there is nothing to add yourself to.
//!
//! # What lives here
//!
//! The abstractions (`Accumulator` §4.1, the `Probe` decorator §4.2, the
//! materialised workloads §4.3, the v1 JSONL report schema §4.4) **and** the
//! machinery that turns them into measurements: the metric recorders +
//! `MetricsMask`, the `BenchRunner`, the N-run `aggregate`, the `GroundTruth`
//! comparators.
//!
//! None of it names a sketch family. The wrapped implementations and the
//! catalog that lists them are `sketch-bench`'s job; generation is
//! `aqpbm-datagen`'s. See `docs/DESIGN.md` §3.1 for the dependency diagram.

pub mod accumulator;
pub mod accuracy;
pub mod aggregation;
pub mod cell;
pub mod config;
pub mod hot_loop;
pub mod init;
pub mod latency;
pub mod memory_footprint;
pub mod metrics;
pub mod probe;
pub mod report;
pub mod runner;
pub mod workload;

// Only the open axis. The concrete per-family params structs live
// with the implementations that consume them, in `aqpbm-cli::params`
// — `aqpbm-core` names no sketch family.
pub use config::{ParamSet, SketchParams};
// The generator is its own crate: it has an independent product surface
// (`sketchlib workload generate`), an on-disk format contract, and no
// knowledge of sketches. Re-exported here so the names stay where callers
// already look for them.
pub use accuracy::{CardinalityOps, Comparison, FrequencyOps, GroundTruth, QuantileOps, TopKOps};
pub use aqpbm_datagen::{
    BasicStats, Distribution, FixedWidth, GenMeta, GenSpec, GenValue, Generator, Shape,
    SketchError, TimeUnit, GEN_META_SCHEMA_VERSION,
};
// The seam an implementation plugs into, and the two calls that drive it.
pub use accumulator::{Accumulator, MergeUnsupported};
pub use cell::{run_cell, score_cell, AccuracyCfg, BenchItem, RunError, WorkloadSpec};
pub use init::{BenchImpl, BuildError, InitSketch};
pub use latency::{LatencyRecorder, LatencySnapshot};
pub use memory_footprint::MemoryFootprint;
pub use metrics::{FullSink, MetricsMask, RunMetrics};
pub use probe::{MetricsSink, NoopSink, Probe};
pub use report::{
    BenchSection, CpuTime, ExternalReports, HwCounters, LatencySummary, Mode, ProfileSection,
    Record, RunStats, Source, SCHEMA_VERSION,
};
pub use runner::{BenchConfig, BenchReport, BenchRunner, NoGT};
pub use workload::{BytesWorkload, I64Workload, StringWorkload, Workload, WorkloadDescription};
