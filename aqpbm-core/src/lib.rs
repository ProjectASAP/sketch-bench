//! `aqpbm-core` — the benchmark framework: everything needed to measure *a*
//! sketch, nothing that knows *which* sketches exist. See `docs/aqpbm-core.md`.
//! Hand a sketch's closures to [`ops::squares_for`]; [`measure()`](fn@measure) times them.

pub mod accuracy;
pub mod binfile;
pub mod build_error;
pub mod config;
pub mod dataset;
pub mod latency;
pub mod measure;
pub mod metrics;
pub mod ops;
pub mod report;
pub mod request;
pub mod run_error;
pub mod run_stats;
pub mod runner;

/// Column shapes and the generate-then-materialise step, shared by the tests of
/// every module that touches a dataset or its file format.
#[cfg(test)]
pub(crate) mod test_support;

// Only the open axis. The concrete per-algorithm params structs live with the
// implementations that consume them, in `sketch-bench::params`.
pub use config::{ParamSet, SketchParams};
// The generator is its own crate: independent product surface, no knowledge of
// sketches, and no file format — it hands back in-memory columns and this crate
// owns what reaches disk (`binfile`). Re-exported so callers find it here.
pub use accuracy::GroundTruth;
pub use aqpbm_datagen::{
    ColumnData, ColumnItem, ColumnSpec, DataDistribution, DataGenError, GeneratedTable,
    NormalParameter, StringOpts, TableDescription, UniformParameter, ZipfParameter,
    RULE_MONOTONIC_INCREASE, RULE_NONE,
};
pub use binfile::{BasicStats, BinMeta, BIN_META_SCHEMA_VERSION};
pub use build_error::BuildError;
pub use latency::{LatencyRecorder, LatencySnapshot};
pub use measure::{measure, MeasureConfig, RunOutcome, Timed};
pub use metrics::{MetricsMask, RunMetrics};
pub use ops::{open_target_scored, open_target_unscored, Body, NoScore, Target, MIN_MERGE_SHARDS};
pub use report::{
    BenchSection, CpuTime, InsertMetrics, LatencySummary, MergeMetrics, MergedRecord, Mode,
    PrepareMetrics, QueryMetrics, Record, RunStats, Source, SCHEMA_VERSION,
};
pub use run_error::RunError;
// What a frontend asks for. The registry that answers it lives in the bundle
// crate; this is only the vocabulary the question is written in.
// `DatasetData` is deliberately absent: it is the intermediate inside
// `DatasetSpec::build`, not a step a caller performs. See `dataset::spec`.
pub use dataset::{
    BenchItem, Dataset, DatasetDescription, DatasetSpec, I64Dataset, Labeled, LabeledDataset,
};
pub use request::{Capability, Numeric, Requirement};
pub use runner::BenchReport;
