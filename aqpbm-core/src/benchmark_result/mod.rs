//! What a measurement produced: the Welford fold over a run population, the
//! shaping of that fold into one record, and the JSONL schema the record is
//! written in. See `docs/aqpbm-core.md` §Results.

pub mod bench_report;
pub mod fold;
pub mod schema;
pub mod welford;

pub use bench_report::BenchReport;
pub use fold::MemoryMaxima;
pub use schema::{
    BenchSection, CpuTime, ExternalWorkload, InsertMetrics, Language, LatencySummary, MergeMetrics,
    MergeSplit, MergedRecord, Mode, PrepareMetrics, QueryMetrics, Record, RunStats, Source,
    WorkloadDescription, SCHEMA_VERSION,
};
pub use welford::Welford;
