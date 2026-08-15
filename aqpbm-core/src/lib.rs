//! `aqpbm-core` — the benchmark framework: everything needed to measure *a*
//! sketch, nothing that knows *which* sketches exist. See `docs/DESIGN.md` §3.1.
//!
//! Implement [`accumulator::Accumulator`], [`init::InitSketch`] and
//! [`init::BenchImpl`], then call [`cell::run_cell`] with a comparator and a
//! closure saying how *your* sketch is queried.

pub mod accumulator;
pub mod accuracy;
pub mod aggregation;
pub mod binfile;
pub mod cell;
pub mod config;
pub mod hot_loop;
pub mod init;
pub mod latency;
pub mod memory_footprint;
pub mod metrics;
pub mod ops;
pub mod probe;
pub mod report;
pub mod request;
pub mod runner;
pub mod workload;

// Only the open axis. The concrete per-algorithm params structs live with the
// implementations that consume them, in `sketch-bench::params`.
pub use config::{ParamSet, SketchParams};
// The generator is its own crate: independent product surface, no knowledge of
// sketches, and no file format — it hands back in-memory columns and this crate
// owns what reaches disk (`binfile`). Re-exported so callers find it here.
pub use accuracy::{Comparison, GroundTruth};
pub use aqpbm_datagen::{
    ColumnData, ColumnSpec, DataDistribution, DataGenError, ColumnItem, GeneratedTable,
    NormalParameter, StringOpts, TableDescription, UniformParameter, ZipfParameter,
    RULE_MONOTONIC_INCREASE, RULE_NONE,
};
pub use binfile::{BasicStats, BinMeta, BIN_META_SCHEMA_VERSION};
// The seam an implementation plugs into, and the two calls that drive it.
pub use accumulator::{Accumulator, MergeUnsupported};
pub use ops::SketchOps;
pub use cell::{run_cell, BenchItem, RowLabel, RunError, WorkloadData, WorkloadSpec};
pub use init::{BuildError, InitSketch};
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
// What a frontend asks for. The registry that answers it lives in the bundle
// crate; this is only the vocabulary the question is written in.
pub use request::{Capability, Numeric, Requirement};
pub use runner::{BenchConfig, BenchReport, BenchRunner, NoGT};
pub use workload::{
    BytesWorkload, I64Workload, Labeled, LabeledWorkload, StringWorkload, Workload,
    WorkloadDescription,
};
