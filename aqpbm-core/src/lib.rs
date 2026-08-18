//! `aqpbm-core` — the benchmark framework: everything needed to measure *a*
//! sketch, nothing that knows *which* sketches exist. See `docs/aqpbm-core.md`.
//! Hand a sketch's closures to [`target::Target`]; [`measure()`](fn@measure) times them.

pub mod accuracy;
pub mod benchmark_result;
pub mod error;
pub mod input_dataset;
pub mod measure;
pub mod metrics;
pub mod target;

/// Column shapes and the generate-then-materialise step, shared by the tests of
/// every module that touches an input dataset.
#[cfg(test)]
pub(crate) mod test_support;

// The generator is its own crate: independent product surface, no knowledge of
// sketches, and no file format — it hands back in-memory columns.
// Re-exported so callers find it here.
pub use accuracy::GroundTruth;
pub use aqpbm_datagen::{
    ColumnData, ColumnItem, ColumnSpec, DataDistribution, DataGenError, GeneratedTable,
    NormalParameter, StringOpts, TableDescription, UniformParameter, ZipfParameter,
    RULE_MONOTONIC_INCREASE, RULE_NONE,
};
pub use benchmark_result::{
    BenchReport, BenchSection, CpuTime, InsertMetrics, LatencySummary, MergeMetrics, MergedRecord,
    Mode, PrepareMetrics, QueryMetrics, Record, RunStats, Source, SCHEMA_VERSION,
};
pub use error::RunError;
// `InputDataSetData` is deliberately absent: it is the intermediate inside
// `InputDataSetSpec::build`, not a step a caller performs. See `input_dataset::spec`.
pub use input_dataset::{
    BenchItem, I64InputDataSet, InputDataSet, InputDataSetDescription, InputDataSetSpec, Labeled,
    LabeledInputDataSet,
};
pub use measure::{measure, MeasureConfig, RunOutcome, Timed};
pub use metrics::{LatencyRecorder, LatencySnapshot, MetricsMask, RunMetrics};
pub use target::{
    open_target_scored, open_target_unscored, Measurement, NoScore, Opening, Target,
    MIN_MERGE_SHARDS,
};
