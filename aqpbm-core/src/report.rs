//! The JSONL report schema — the single serialised record shape
//! shared by offline benchmarks, runtime samplers, and the
//! visualisation layer. Current version: [`SCHEMA_VERSION`].
//!
//! See `docs/DESIGN.md` §4.4.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::workload::WorkloadDescription;

/// Bumped whenever a breaking field change lands. Readers
/// should refuse to process records with a mismatched version.
pub const SCHEMA_VERSION: u32 = 3;

/// A single record in the JSONL report stream. One record
/// per benchmark / profile / runtime window.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Record {
    pub schema_version: u32,
    /// The algorithm this run measured, structural variant included
    /// (`cms-fastpath-vector2d`, not `cms`).
    pub sketch: String,
    /// The family [`Self::sketch`] belongs to. Rows sharing it answer the same
    /// question from the same knobs, so this is what a cross-library comparison
    /// groups by; `sketch` is what a variant comparison groups by. Additive and
    /// optional, so records written before it still read.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub family: Option<String>,
    #[serde(rename = "impl")]
    pub impl_name: String,
    /// Implementation language, so readers can split Rust from C++ records when
    /// both tracks dump into one JSONL stream. Defaults to `rust`.
    #[serde(default)]
    pub language: Language,
    /// Algorithm-specific construction params used for this run, populated by
    /// `bench` from the cell's `ParamSet`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sketch_config: Option<serde_json::Value>,
    pub workload: WorkloadDescription,
    pub mode: Mode,
    pub runs: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bench: Option<BenchSection>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub profile: Option<ProfileSection>,
    pub source: Source,
    pub timestamp: DateTime<Utc>,
}

/// Implementation language of the run that produced this record.
/// Used by readers to split apples-to-apples comparisons.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum Language {
    #[default]
    Rust,
    Cpp,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    Bench,
    Profile,
    Runtime,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum Source {
    /// Run produced by the `approxbench` CLI.
    Cli,
    /// Run produced by an embedded `sketch-runtime::Sampler`
    /// in one of the downstream apps.
    AsapFusion,
    DataCollector,
    AsapQuery,
    /// Run produced by a `cpp-bench` binary (Google Benchmark
    /// based C++ track). See `docs/SCHEMA_V1.md`.
    CppBench,
}

/// MACRO (benchmark) section of a record. Every sub-field is
/// `Option`al so N/A metrics don't pollute the JSON with zero
/// placeholders.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BenchSection {
    /// Which pass produced this record — `"throughput"`, `"latency"`,
    /// `"accuracy"` or `"merge"`. One invocation emits several records per
    /// (sketch, impl, config, workload); group by this before pooling any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pass: Option<String>,
    /// **Ingest rate**: `items / insert_wall`, `Accumulator::prepare` excluded.
    /// Deferred-build rows buffer in `update`, so this times their `Vec::push`
    /// — compare [`Self::build_throughput_items_per_sec`] instead.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub throughput_items_per_sec: Option<RunStats>,
    /// Per-run ingest-rate samples (items/sec, one entry per measured run).
    /// Kept alongside the aggregate so consumers can render box plots /
    /// CDFs without re-running the bench.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub throughput_samples: Option<Vec<f64>>,
    /// **Accumulator-build rate**: `items / (insert_wall + finalize_wall)`, the
    /// rate a *ready-to-answer* sketch is produced at. The cross-algorithm column;
    /// equals the ingest rate wherever `prepare` is free.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub build_throughput_items_per_sec: Option<RunStats>,
    /// Wall time of `Accumulator::prepare` per run — the deferred build cost
    /// separating the two throughput columns. `0.0` means finalize really is a
    /// no-op, which is a measurement, not a gap.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finalize_time_ms: Option<RunStats>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub query_throughput_items_per_sec: Option<RunStats>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub latency_ns: Option<LatencySummary>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cpu_time_ms: Option<CpuTime>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub wall_time_ms: Option<RunStats>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rss_peak_kb: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub heap_allocated_kb: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub memory_bytes: Option<u64>,
    /// Net bytes the tracking allocator attributes to this sketch's lifetime.
    /// **Measured**, where [`Self::memory_bytes`] is derived from the row's own
    /// formula, so the two answer the same question by different means and a
    /// wide gap between them is a formula that has drifted from the structure it
    /// describes. Needs `heap-track` on the linking binary; absent otherwise.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub heap_bytes_net: Option<u64>,
    /// High-water mark of the same counter across construction and insert, so a
    /// resize or an intermediate buffer is visible instead of being smoothed
    /// away by the steady-state figure.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub heap_bytes_peak: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub accuracy: Option<serde_json::Value>,
    /// Wall time to fold `merge_shards` sketches into one, per run; absent
    /// unless the merge pass ran. Scales with sketch *state*, not stream
    /// length, so compare against `memory_bytes`, not insert throughput.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub merge_time_ms: Option<RunStats>,
    /// How many shards were folded. Present whenever the merge pass ran, even
    /// if the implementation turned out not to support merging.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub merge_shards: Option<usize>,
    /// `false` when the implementation provides no merge. Recorded rather
    /// than omitted so a capability gap is visible in the output instead of
    /// showing up as a missing row.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub merge_supported: Option<bool>,
}

/// MICRO (profile) section of a record — reserved for `sketch-profile`
/// (`docs/DESIGN.md` §2.2), which is not built yet. Declared here so readers
/// don't need two crates to deserialise the same JSONL file.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ProfileSection {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hw_counters: Option<HwCounters>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub external: Option<ExternalReports>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct HwCounters {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub l1d_miss_rate: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub llc_miss_rate: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub branch_miss_rate: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dtlb_miss_rate: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub itlb_miss_rate: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ipc: Option<f64>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ExternalReports {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub perf_record: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cachegrind_report: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub flamegraph: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub heaptrack_report: Option<String>,
}

/// Aggregate across N samples: mean / stddev / optional 95% CI.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct RunStats {
    pub mean: f64,
    pub stddev: f64,
    /// 95% CI on the mean — **present only when the samples are statistically
    /// independent**, i.e. `--repeats R` (R > 1) over R processes. Iterations of
    /// one `--runs N` share too much for an interval over them to mean anything.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ci95: Option<[f64; 2]>,
    /// Number of samples behind `mean`. Iterations within one process when
    /// `ci95` is absent; independent processes when it is present.
    pub n: usize,
}

/// CPU time split into user vs sys.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct CpuTime {
    pub user_ms: RunStats,
    pub sys_ms: RunStats,
}

/// Latency percentile summary extracted from an
/// `hdrhistogram::Histogram`.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct LatencySummary {
    pub p50: u64,
    pub p95: u64,
    pub p99: u64,
    pub p999: u64,
    pub max: u64,
    pub count: u64,
}

/// Flattened form of the 2-4 [`Record`]s that share one (sketch, impl,
/// sketch_config, workload) identity — one row per cell instead of one row
/// per pass. Built by `aqpbm-cli`'s `flatten_record` from a throughput
/// pass, a query/accuracy pass, an optional latency pass, and an optional
/// merge pass. Field names on the wire match `scripts/merge_passes.py`'s
/// current output, so existing consumers don't need to change. Lives next
/// to [`Record`] rather than in the CLI crate since it's a JSONL wire
/// shape like `Record`, not CLI-specific logic.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MergedRecord {
    pub schema_version: u32,
    pub sketch: String,
    #[serde(rename = "impl")]
    pub impl_name: String,
    pub language: Language,
    pub mode: Mode,
    pub runs: usize,
    pub source: Source,

    pub sketch_config: Option<serde_json::Value>,
    pub workload: WorkloadDescription,

    pub memory_bytes: Option<u64>,
    /// Net bytes the tracking allocator attributes to this sketch's
    /// lifetime — **measured**, where [`Self::memory_bytes`] is a
    /// self-reported formula. Only present when the source binary was built
    /// with `heap-track`; kept as its own field (not merged into
    /// `memory_bytes`) so a gap between the two stays visible instead of
    /// being silently papered over.
    pub heap_bytes_net: Option<u64>,
    /// High-water mark of the same counter across construction and insert.
    pub heap_bytes_peak: Option<u64>,
    pub accuracy: Option<serde_json::Value>,

    #[serde(flatten)]
    pub insert: InsertMetrics,
    #[serde(flatten)]
    pub query: QueryMetrics,
    #[serde(flatten)]
    pub latency: LatencyMetrics,
    #[serde(flatten)]
    pub merge: MergeMetrics,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct InsertMetrics {
    #[serde(rename = "insert_timestamp")]
    pub timestamp: Option<DateTime<Utc>>,
    #[serde(rename = "insert_throughput_items_per_sec")]
    pub throughput_items_per_sec: Option<RunStats>,
    #[serde(rename = "insert_throughput_samples")]
    pub throughput_samples: Option<Vec<f64>>,
    #[serde(rename = "insert_build_throughput_items_per_sec")]
    pub build_throughput_items_per_sec: Option<RunStats>,
    #[serde(rename = "insert_finalize_time_ms")]
    pub finalize_time_ms: Option<RunStats>,
    #[serde(rename = "insert_cpu_time_ms")]
    pub cpu_time_ms: Option<CpuTime>,
    #[serde(rename = "insert_wall_time_ms")]
    pub wall_time_ms: Option<RunStats>,
    #[serde(rename = "insert_rss_peak_kb")]
    pub rss_peak_kb: Option<u64>,
    #[serde(rename = "insert_heap_allocated_kb")]
    pub heap_allocated_kb: Option<u64>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct QueryMetrics {
    #[serde(rename = "query_timestamp")]
    pub timestamp: Option<DateTime<Utc>>,
    #[serde(rename = "query_throughput_items_per_sec")]
    pub throughput_items_per_sec: Option<RunStats>,
    #[serde(rename = "query_cpu_time_ms")]
    pub cpu_time_ms: Option<CpuTime>,
    #[serde(rename = "query_wall_time_ms")]
    pub wall_time_ms: Option<RunStats>,
    #[serde(rename = "query_rss_peak_kb")]
    pub rss_peak_kb: Option<u64>,
    #[serde(rename = "query_heap_allocated_kb")]
    pub heap_allocated_kb: Option<u64>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LatencyMetrics {
    #[serde(rename = "latency_timestamp")]
    pub timestamp: Option<DateTime<Utc>>,
    pub latency_ns: Option<LatencySummary>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MergeMetrics {
    #[serde(rename = "merge_timestamp")]
    pub timestamp: Option<DateTime<Utc>>,
    pub merge_time_ms: Option<RunStats>,
    pub merge_shards: Option<usize>,
    pub merge_supported: Option<bool>,
}

impl Record {
    pub fn new(
        sketch: impl Into<String>,
        impl_name: impl Into<String>,
        workload: WorkloadDescription,
        mode: Mode,
        runs: usize,
    ) -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            sketch: sketch.into(),
            family: None,
            impl_name: impl_name.into(),
            language: Language::Rust,
            sketch_config: None,
            workload,
            mode,
            runs,
            bench: None,
            profile: None,
            source: Source::Cli,
            timestamp: Utc::now(),
        }
    }

    pub fn to_jsonl(&self) -> String {
        serde_json::to_string(self).expect("Record -> JSON should not fail")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn record_roundtrips_through_json() {
        let wd = WorkloadDescription {
            shape: "zipf".into(),
            size: 1_000_000,
            cardinality: Some(10_000),
            zipf_s: Some(1.1),
            source_path: None,
            seed: Some(42),
            spec: None,
        };
        let mut rec = Record::new("hll", "oxide", wd, Mode::Bench, 10);
        rec.bench = Some(BenchSection {
            pass: None,
            throughput_items_per_sec: Some(RunStats {
                mean: 4.2e7,
                stddev: 1.1e6,
                ci95: None,
                n: 10,
            }),
            ..Default::default()
        });
        let s = rec.to_jsonl();
        let back: Record = serde_json::from_str(&s).unwrap();
        assert_eq!(back.sketch, "hll");
        assert_eq!(back.impl_name, "oxide");
        assert_eq!(back.mode, Mode::Bench);
        assert!(back.bench.is_some());
    }

    #[test]
    fn profile_section_roundtrips() {
        let wd = WorkloadDescription {
            shape: "uniform".into(),
            size: 1000,
            cardinality: Some(100),
            zipf_s: None,
            source_path: None,
            seed: Some(1),
            spec: None,
        };
        let mut rec = Record::new("cms", "oxide", wd, Mode::Profile, 1);
        rec.profile = Some(ProfileSection {
            hw_counters: Some(HwCounters {
                ipc: Some(3.2),
                l1d_miss_rate: Some(0.018),
                ..Default::default()
            }),
            external: None,
        });
        let s = rec.to_jsonl();
        let back: Record = serde_json::from_str(&s).unwrap();
        assert_eq!(back.profile.unwrap().hw_counters.unwrap().ipc, Some(3.2));
    }

    /// A gate, not a behaviour test: bumping `SCHEMA_VERSION` turns this red on
    /// purpose. The number leaves the repo — in every JSONL record, and on the
    /// wire as `RuntimeRecord.schema_version` — so a bump is a contract change.
    #[test]
    fn schema_version_is_v3() {
        assert_eq!(SCHEMA_VERSION, 3);
    }

    #[test]
    fn language_defaults_to_rust_when_absent() {
        // A v2 JSONL record produced before the `language` field
        // existed must still deserialise, with language = Rust.
        let legacy = r#"{
            "schema_version": 2,
            "sketch": "hll",
            "impl": "oxide",
            "workload": {"shape": "uniform", "size": 100, "seed": 1},
            "mode": "bench",
            "runs": 1,
            "source": "cli",
            "timestamp": "2025-01-01T00:00:00Z"
        }"#;
        let rec: Record = serde_json::from_str(legacy).unwrap();
        assert_eq!(rec.language, Language::Rust);
    }

    /// Wire-format compat: a JSONL line shaped like what the C++ emitter writes
    /// must deserialise into a `Record` with the right fields, catching
    /// field-name drift without compiling that side.
    #[test]
    fn cpp_bench_jsonl_parses() {
        let cpp_emitted = r#"{"schema_version":2,"sketch":"kll","impl":"datasketches","language":"cpp","workload":{"shape":"file","size":1000000,"source_path":"input/benchmark_data_1m_int64.bin"},"mode":"bench","runs":10,"bench":{"throughput_items_per_sec":{"mean":42000000,"stddev":1100000,"ci95":[41500000,42500000],"n":10},"latency_ns":{"p50":17,"p95":41,"p99":60,"p999":95,"max":312,"count":1000},"accuracy":{"queries":[0.5,0.95,0.99],"abs_rank_err":{"mean":0.0021,"max":0.0084},"rel_rank_err":{"mean":0.0043,"max":0.019}}},"source":"cpp-bench","timestamp":"2026-05-13T07:14:22.123456Z"}"#;
        let rec: Record = serde_json::from_str(cpp_emitted).unwrap();
        assert_eq!(rec.sketch, "kll");
        assert_eq!(rec.impl_name, "datasketches");
        assert_eq!(rec.language, Language::Cpp);
        assert_eq!(rec.source, Source::CppBench);
        let bench = rec.bench.as_ref().unwrap();
        let tput = bench.throughput_items_per_sec.unwrap();
        assert_eq!(tput.n, 10);
        let lat = bench.latency_ns.unwrap();
        assert_eq!(lat.p99, 60);
        let acc = bench.accuracy.as_ref().unwrap();
        assert!(acc.get("queries").is_some());
        assert!(acc.get("abs_rank_err").is_some());
    }

    #[test]
    fn cpp_record_roundtrips() {
        let wd = WorkloadDescription {
            shape: "file".into(),
            size: 1_000_000,
            cardinality: None,
            zipf_s: None,
            source_path: Some("input/benchmark_data_1m_int64.bin".into()),
            seed: None,
            spec: None,
        };
        let mut rec = Record::new("kll", "datasketches", wd, Mode::Bench, 10);
        rec.language = Language::Cpp;
        rec.source = Source::CppBench;
        let s = rec.to_jsonl();
        assert!(s.contains("\"language\":\"cpp\""));
        assert!(s.contains("\"source\":\"cpp-bench\""));
        let back: Record = serde_json::from_str(&s).unwrap();
        assert_eq!(back.language, Language::Cpp);
        assert_eq!(back.source, Source::CppBench);
    }
}
