//! `aqpbm-core` — the benchmark framework: everything needed to measure *a*
//! sketch, nothing that knows *which* sketches exist. See `docs/aqpbm-core.md`.
//! Hand a sketch's closures to [`measurement`]; [`measure()`](fn@measure) times them.

pub mod accuracy;
pub mod benchmark_result;
pub mod error;
pub mod input_dataset;
pub mod measure;
pub mod measurement;
pub mod metrics;

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
// `InputDataSetData` is what a frontend generates and a row materialises: the
// hand-off between the two halves of `InputDataSetSpec::build`, so both ends of
// it need to name the type. See `input_dataset::spec`.
pub use input_dataset::{
    BenchItem, I64InputDataSet, InputDataSet, InputDataSetData, InputDataSetDescription,
    InputDataSetSpec, Labeled, LabeledInputDataSet,
};
pub use measure::{measure, MeasureConfig, RunOutcome, Timed};
pub use measurement::{Measurement, MIN_MERGE_SHARDS};
pub use metrics::{LatencyRecorder, LatencySnapshot, MetricsMask, RunMetrics};
