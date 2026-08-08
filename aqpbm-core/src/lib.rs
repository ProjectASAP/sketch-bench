//! `aqpbm-core` — the benchmark framework: everything needed to measure *a*
//! sketch, nothing that knows *which* sketches exist. See `docs/DESIGN.md` §3.1.
//!
//! Implement [`accumulator::Accumulator`], [`init::InitSketch`], [`init::BenchImpl`]
//! + one capability trait per statistic, then call [`cell::run_cell`].

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

// Only the open axis. The concrete per-algorithm params structs live with the
// implementations that consume them, in `sketch-bench::params`.
pub use config::{ParamSet, SketchParams};
// The generator is its own crate: independent product surface, its own on-disk
// format contract, no knowledge of sketches. Re-exported so callers find it here.
pub use accuracy::{
    CardinalityOps, Comparison, FrequencyOps, GroundTruth, QuantileOps, SubpopFrequencyOps, TopKOps,
};
pub use aqpbm_datagen::{
    BasicStats, Distribution, FixedWidth, GenMeta, GenSpec, GenValue, Generator, Shape,
    SketchError, TimeUnit, GEN_META_SCHEMA_VERSION,
};
// The seam an implementation plugs into, and the two calls that drive it.
pub use accumulator::{Accumulator, MergeUnsupported};
pub use cell::{run_cell, BenchItem, RunError, WorkloadSpec};
pub use init::{BenchImpl, BuildError, InitSketch};
pub use latency::{LatencyRecorder, LatencySnapshot};
pub use memory_footprint::MemoryFootprint;
pub use metrics::{FullSink, MetricsMask, RunMetrics};
pub use probe::{MetricsSink, NoopSink, Probe};
pub use report::{
    BenchSection, CpuTime, ExternalReports, HwCounters, InsertMetrics,
    LatencySummary, MergeMetrics, MergedRecord, Mode, PrepareMetrics, ProfileSection,
    QueryMetrics, Record,
    RunStats, Source, SCHEMA_VERSION,
};
pub use runner::{BenchConfig, BenchReport, BenchRunner, NoGT};
pub use workload::{
    BytesWorkload, I64Workload, Labeled, LabeledWorkload, StringWorkload, Workload,
    WorkloadDescription,
};
