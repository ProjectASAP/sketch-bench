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
pub const SCHEMA_VERSION: u32 = 1;

/// A single record in the v1 JSONL report stream. One record
/// per benchmark / profile / runtime window.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Record {
    pub schema_version: u32,
    pub sketch: String,
    #[serde(rename = "impl")]
    pub impl_name: String,
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
}

/// MACRO (benchmark) section of a record. Every sub-field is
/// `Option`al so N/A metrics don't pollute the JSON with zero
/// placeholders.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BenchSection {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub throughput_items_per_sec: Option<RunStats>,
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
    pub heap_peak_kb: Option<u64>,
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
    fn schema_version_is_v1() {
        assert_eq!(SCHEMA_VERSION, 1);
    }
}
