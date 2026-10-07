//! `aqpbm-core` — the benchmark framework: everything needed to measure *a*
//! sketch, nothing that knows *which* sketches exist. See `docs/aqpbm-core.md`.
//! Hand a sketch's primed closures to [`measure()`](fn@measure); it times them.

pub mod accuracy;
pub mod atomic_costs;
pub mod benchmark_result;
pub mod erp;
pub mod error;
pub mod measure;
pub mod metrics;

// The generator is its own crate: independent product surface, no knowledge of
// sketches, and no file format — it hands back in-memory columns.
// Re-exported so callers find it here.
pub use accuracy::GroundTruth;
pub use aqpbm_datagen::{
    ColumnData, ColumnItem, ColumnSpec, DataDistribution, DataGenError, GeneratedTable,
    NormalParameter, ParetoParameter, StringOpts, TableDescription, UniformParameter,
    ZipfParameter, RULE_MONOTONIC_INCREASE, RULE_NONE,
};
pub use atomic_costs::{
    reduce_all, reduce_one, AtomicCostEntry, AtomicCostTable, MeasuredAt, MeasuredShape, SkipReason,
};
pub use benchmark_result::{
    BenchReport, BenchSection, CpuTime, ExternalWorkload, InsertMetrics, LatencySummary,
    MergeMetrics, MergedRecord, Mode, PrepareMetrics, QueryMetrics, Record, RunStats, Source,
    WorkloadDescription, SCHEMA_VERSION,
};
pub use erp::{
    erp_artifact, erp_record, ErpArtifact, ErpRecord, ErpResourceProfile, ERP_SCHEMA_VERSION,
};
pub use error::RunError;
pub use measure::{
    measure, record_calls, runs_for, MeasureConfig, Measurement, Pass, Report, RunOutcome,
    MIN_MERGE_SHARDS,
};
pub use metrics::{LatencyRecorder, LatencySnapshot, MetricsMask, RunMetrics};
