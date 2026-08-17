//! `aqpbm-core` — the benchmark framework: everything needed to measure *a*
//! sketch, nothing that knows *which* sketches exist. See `docs/DESIGN.md` §3.1.
//!
//! Write the handful of closures that drive your sketch — each takes the sketch
//! and its data as parameters and captures nothing — and hand them to
//! [`ops::squares_for`]. It returns one [`ops::Body`] per square, which
//! [`measure`] then times.

pub mod accumulator;
pub mod accuracy;
pub mod aggregation;
pub mod binfile;
pub mod build_error;
pub mod cell;
pub mod config;
pub mod latency;
pub mod measure;
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
    ColumnData, ColumnItem, ColumnSpec, DataDistribution, DataGenError, GeneratedTable,
    NormalParameter, StringOpts, TableDescription, UniformParameter, ZipfParameter,
    RULE_MONOTONIC_INCREASE, RULE_NONE,
};
pub use binfile::{BasicStats, BinMeta, BIN_META_SCHEMA_VERSION};
// The seam an implementation plugs into, and the two calls that drive it.
pub use accumulator::{Accumulator, MergeUnsupported};
pub use cell::{BenchItem, RunError, WorkloadData, WorkloadSpec};
pub use latency::{LatencyRecorder, LatencySnapshot};
pub use build_error::BuildError;
pub use measure::{measure, MeasureConfig, RunOutcome, Timed};
pub use ops::{squares_for, squares_for_unscored, Body, NoScore, MIN_MERGE_SHARDS};
pub use metrics::{MetricsMask, RunMetrics};
pub use probe::{MetricsSink, NoopSink, Probe};
pub use report::{
    BenchSection, CpuTime, ExternalReports, HwCounters, InsertMetrics, LatencySummary,
    MergeMetrics, MergedRecord, Mode, PrepareMetrics, ProfileSection, QueryMetrics, Record,
    RunStats, Source, SCHEMA_VERSION,
};
// What a frontend asks for. The registry that answers it lives in the bundle
// crate; this is only the vocabulary the question is written in.
pub use request::{Capability, Numeric, Requirement};
pub use runner::BenchReport;
pub use workload::{
    BytesWorkload, I64Workload, Labeled, LabeledWorkload, StringWorkload, Workload,
    WorkloadDescription,
};
