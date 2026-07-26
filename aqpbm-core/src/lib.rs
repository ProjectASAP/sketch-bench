//! `aqpbm-core` — the domain-agnostic benchmark engine + the
//! shared types every sketchlib-tool crate is built on.
//!
//! It carries the abstractions (`Sketch` §4.1, the `Probe` decorator §4.2,
//! the materialised workloads §4.3, the v1 JSONL report schema §4.4) **and**
//! the machinery that turns them into measurements: the metric recorders +
//! `MetricsMask`, the `BenchRunner`, the N-run `aggregate`, the `GroundTruth`
//! comparator. None of it names a sketch family — the concrete wrappers and
//! per-family comparators live in `sketch-bench`, generation in
//! `aqpbm-datagen`. See `docs/DESIGN.md` §3.1 for the dependency diagram.

pub mod accuracy;
pub mod aggregation;
pub mod config;
pub mod hot_loop;
pub mod latency;
pub mod metrics;
pub mod probe;
pub mod report;
pub mod runner;
pub mod sketch;
pub mod workload;

// Only the open axis. The concrete per-family params structs live
// with the implementations that consume them, in `aqpbm-cli::params`
// — `aqpbm-core` names no sketch family.
pub use config::{ParamSet, SketchParams};
// The generator is its own crate: it has an independent product surface
// (`sketchlib workload generate`), an on-disk format contract, and no
// knowledge of sketches. Re-exported here so the names stay where callers
// already look for them.
pub use aqpbm_datagen::{
    BasicStats, DType, Distribution, FixedWidth, GenMeta, GenSpec, GenValue, Generator, Shape,
    SketchError, TimeUnit, GEN_META_SCHEMA_VERSION,
};
pub use accuracy::{Comparison, GroundTruth};
pub use latency::{LatencyRecorder, LatencySnapshot};
pub use metrics::{FullSink, MetricsMask, RunMetrics};
pub use probe::{MetricsSink, NoopSink, Probe};
pub use runner::{BenchConfig, BenchReport, BenchRunner, NoGT};
pub use report::{
    BenchSection, CpuTime, ExternalReports, HwCounters, LatencySummary, Mode, ProfileSection,
    Record, RunStats, Source, SCHEMA_VERSION,
};
pub use sketch::{MergeUnsupported, Sketch};
pub use workload::{BytesWorkload, I64Workload, StringWorkload, Workload, WorkloadDesc};
