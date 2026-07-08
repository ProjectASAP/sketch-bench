//! v1 JSONL report schema — the single serialised record shape
//! shared by offline benchmarks, runtime samplers, and the
//! visualisation layer.
//!
//! See `docs/DESIGN.md` §4.4.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::workload::WorkloadDesc;

/// Bumped whenever a breaking field change lands. Readers
/// should refuse to process records with a mismatched version.
pub const SCHEMA_VERSION: u32 = 2;

/// A single record in the v1 JSONL report stream. One record
/// per benchmark / profile / runtime window.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Record {
    pub schema_version: u32,
    pub sketch: String,
    #[serde(rename = "impl")]
    pub impl_name: String,
    /// Implementation language. Lets readers split Rust vs C++
    /// records when both tracks dump into one JSONL stream
    /// (see `docs/archive/SCHEMA_V1.md`). Defaults to `rust` so older
    /// records deserialise unchanged.
    #[serde(default)]
    pub language: Language,
    /// Family-specific construction params used for this run.
    /// Populated by `bench` when it knows the `ParamSet`; absent
    /// from legacy records. See `docs/BENCH_SWEEP.md` §5.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sketch_config: Option<serde_json::Value>,
    pub workload: WorkloadDesc,
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
    /// Run produced by the `sketchlib` CLI.
    Cli,
    /// Run produced by an embedded `sketch-runtime::Sampler`
    /// in one of the downstream apps.
    AsapFusion,
    DataCollector,
    AsapQuery,
    /// Run produced by a `cpp-bench` binary (Google Benchmark
    /// based C++ track). See `docs/archive/SCHEMA_V1.md`.
    CppBench,
}

/// MACRO (benchmark) section of a record. Every sub-field is
/// `Option`al so N/A metrics don't pollute the JSON with zero
/// placeholders.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BenchSection {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub throughput_items_per_sec: Option<RunStats>,
    /// Per-run throughput samples (items/sec, one entry per measured run).
    /// Kept alongside the aggregate so consumers can render box plots /
    /// CDFs without re-running the bench.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub throughput_samples: Option<Vec<f64>>,
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub accuracy: Option<serde_json::Value>,
}

/// MICRO (profile) section of a record — filled by
/// `sketch-profile`. Kept here so readers don't need two
/// crates to deserialise the same JSONL file.
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

/// Aggregate across N runs: mean / stddev / 95% CI.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct RunStats {
    pub mean: f64,
    pub stddev: f64,
    pub ci95: [f64; 2],
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

impl Record {
    pub fn new(
        sketch: impl Into<String>,
        impl_name: impl Into<String>,
        workload: WorkloadDesc,
        mode: Mode,
        runs: usize,
    ) -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            sketch: sketch.into(),
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
        let wd = WorkloadDesc {
            shape: "zipf".into(),
            size: 1_000_000,
            cardinality: Some(10_000),
            zipf_s: Some(1.1),
            source_path: None,
            seed: Some(42),
        };
        let mut rec = Record::new("hll", "oxide", wd, Mode::Bench, 10);
        rec.bench = Some(BenchSection {
            throughput_items_per_sec: Some(RunStats {
                mean: 4.2e7,
                stddev: 1.1e6,
                ci95: [4.15e7, 4.25e7],
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
        let wd = WorkloadDesc {
            shape: "uniform".into(),
            size: 1000,
            cardinality: Some(100),
            zipf_s: None,
            source_path: None,
            seed: Some(1),
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

    #[test]
    fn schema_version_is_v2() {
        assert_eq!(SCHEMA_VERSION, 2);
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

    /// Wire-format compat test: a JSONL line shaped exactly like
    /// what `cpp-bench/common/record_v1.cpp` emits must deserialise
    /// into a `Record` with the right fields. Catches field-name
    /// drift between the two emitters without needing to compile
    /// the C++ side.
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
        let wd = WorkloadDesc {
            shape: "file".into(),
            size: 1_000_000,
            cardinality: None,
            zipf_s: None,
            source_path: Some("input/benchmark_data_1m_int64.bin".into()),
            seed: None,
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
