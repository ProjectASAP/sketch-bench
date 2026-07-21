//! `sketch-core` — shared types across the sketchlib-tool crates.
//!
//! Contains the `Sketch` trait (§4.1), the `Probe` decorator
//! (§4.2), workload generators (§4.3), and the v1 JSONL report
//! schema (§4.4). All other sketchlib-tool crates and every
//! downstream ASAP app depend here — see `docs/DESIGN.md` §3.1
//! for the full dependency-direction diagram.

pub mod config;
pub mod datagen;
pub mod error;
pub mod probe;
pub mod report;
pub mod sketch;
pub mod workload;

pub use config::{
    CmsParams, CountSketchParams, DdParams, ElasticParams, HllParams, KllParams, NitroParams,
    ParamSet, UnivMonParams,
};
pub use datagen::{
    BasicStats, Column, ColumnGenerator, DType, Distribution, GenMeta, GenSpec, Shape, TimeUnit,
    GEN_META_SCHEMA_VERSION,
};
pub use error::SketchCoreError;
pub use probe::{MetricsSink, NoopSink, Probe};
pub use report::{
    BenchSection, CpuTime, ExternalReports, HwCounters, LatencySummary, Mode, ProfileSection,
    Record, RunStats, Source, SCHEMA_VERSION,
};
pub use sketch::Sketch;
pub use workload::{
    BytesFromI64, FileI64, StringFromI64, UniformI64, Workload, WorkloadDesc, ZipfI64,
};
