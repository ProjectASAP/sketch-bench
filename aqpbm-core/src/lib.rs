//! `aqpbm-core` — shared types across the sketchlib-tool crates.
//!
//! Contains the `Sketch` trait (§4.1), the `Probe` decorator
//! (§4.2), the materialised workload types (§4.3), and the v1
//! JSONL report schema (§4.4). Generation itself lives in
//! `sketch-datagen`. All other sketchlib-tool crates and every
//! downstream ASAP app depend here — see `docs/DESIGN.md` §3.1
//! for the full dependency-direction diagram.

pub mod aggregation;
pub mod config;
pub mod hot_loop;
pub mod latency;
pub mod metrics;
pub mod probe;
pub mod report;
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
pub use latency::{LatencyRecorder, LatencySnapshot};
pub use metrics::MetricsMask;
pub use probe::{MetricsSink, NoopSink, Probe};
pub use report::{
    BenchSection, CpuTime, ExternalReports, HwCounters, LatencySummary, Mode, ProfileSection,
    Record, RunStats, Source, SCHEMA_VERSION,
};
pub use sketch::{MergeUnsupported, Sketch};
pub use workload::{BytesWorkload, I64Workload, StringWorkload, Workload, WorkloadDesc};
