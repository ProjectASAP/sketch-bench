//! Output data structure for `sketchlib merge` — the planned
//! replacement for `scripts/merge_passes.py`.
//!
//! One row of `merged.jsonl`. Field names on the wire match
//! `merge_passes.py`'s output exactly (existing consumers — the
//! leaderboard, `visualization/js/dataLoader.js` — don't need to
//! change).

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use sketch_core::{
    report::{CpuTime, Language, LatencySummary, Mode, RunStats, Source},
    WorkloadDesc,
};

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

    #[serde(skip_serializing_if = "Option::is_none")]
    pub sketch_config: Option<Value>,
    pub workload: WorkloadDesc,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub memory_bytes: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub accuracy: Option<Value>,

    #[serde(flatten)]
    pub insert: InsertMetrics,
    #[serde(flatten)]
    pub query: QueryMetrics,
    #[serde(flatten)]
    pub latency: LatencyMetrics,

    /// Safety net: any field that shows up under `bench` in the
    /// raw records but isn't one of the named fields above lands
    /// here instead of being dropped.
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct InsertMetrics {
    #[serde(rename = "insert_timestamp", skip_serializing_if = "Option::is_none")]
    pub timestamp: Option<DateTime<Utc>>,
    #[serde(rename = "insert_throughput_items_per_sec", skip_serializing_if = "Option::is_none")]
    pub throughput_items_per_sec: Option<RunStats>,
    #[serde(rename = "insert_throughput_samples", skip_serializing_if = "Option::is_none")]
    pub throughput_samples: Option<Vec<f64>>,
    #[serde(rename = "insert_cpu_time_ms", skip_serializing_if = "Option::is_none")]
    pub cpu_time_ms: Option<CpuTime>,
    #[serde(rename = "insert_wall_time_ms", skip_serializing_if = "Option::is_none")]
    pub wall_time_ms: Option<RunStats>,
    #[serde(rename = "insert_rss_peak_kb", skip_serializing_if = "Option::is_none")]
    pub rss_peak_kb: Option<u64>,
    #[serde(rename = "insert_heap_allocated_kb", skip_serializing_if = "Option::is_none")]
    pub heap_allocated_kb: Option<u64>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct QueryMetrics {
    #[serde(rename = "query_timestamp", skip_serializing_if = "Option::is_none")]
    pub timestamp: Option<DateTime<Utc>>,
    #[serde(rename = "query_throughput_items_per_sec", skip_serializing_if = "Option::is_none")]
    pub throughput_items_per_sec: Option<RunStats>,
    #[serde(rename = "query_cpu_time_ms", skip_serializing_if = "Option::is_none")]
    pub cpu_time_ms: Option<CpuTime>,
    #[serde(rename = "query_wall_time_ms", skip_serializing_if = "Option::is_none")]
    pub wall_time_ms: Option<RunStats>,
    #[serde(rename = "query_rss_peak_kb", skip_serializing_if = "Option::is_none")]
    pub rss_peak_kb: Option<u64>,
    #[serde(rename = "query_heap_allocated_kb", skip_serializing_if = "Option::is_none")]
    pub heap_allocated_kb: Option<u64>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LatencyMetrics {
    #[serde(rename = "latency_timestamp", skip_serializing_if = "Option::is_none")]
    pub timestamp: Option<DateTime<Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub latency_ns: Option<LatencySummary>,
}
