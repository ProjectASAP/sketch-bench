//! Output data structure + flatten logic for a planned `approxbench merge`
//! command — the intended Rust replacement for `scripts/merge_passes.py`.
//!
//! The CLI emits one raw [`Record`] per metric pass (see the comment at
//! `main.rs`'s `run_bench`: "A downstream group-by on (sketch, impl,
//! sketch_config, workload) merges them back."). [`flatten_record`] is that
//! downstream step: given the 2-3 [`Record`]s that share one identity
//! (one insert pass, one query/accuracy pass, optionally one latency pass),
//! it folds them into a single [`MergedRecord`] row. Field names on the
//! wire match `merge_passes.py`'s current output, so existing consumers
//! don't need to change.
//!
//! Grouping records by identity, and splitting a re-run (the same pass
//! appearing twice) into two separate calls to `flatten_record` instead of
//! silently overwriting, is the caller's job — not implemented here yet.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use aqpbm_core::report::{CpuTime, Language, LatencySummary, Mode, RunStats, Source};
use aqpbm_core::{Record, WorkloadDescription};

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

    pub sketch_config: Option<Value>,
    pub workload: WorkloadDescription,

    pub memory_bytes: Option<u64>,
    pub accuracy: Option<Value>,

    #[serde(flatten)]
    pub insert: InsertMetrics,
    #[serde(flatten)]
    pub query: QueryMetrics,
    #[serde(flatten)]
    pub latency: LatencyMetrics,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct InsertMetrics {
    #[serde(rename = "insert_timestamp")]
    pub timestamp: Option<DateTime<Utc>>,
    #[serde(rename = "insert_throughput_items_per_sec")]
    pub throughput_items_per_sec: Option<RunStats>,
    #[serde(rename = "insert_throughput_samples")]
    pub throughput_samples: Option<Vec<f64>>,
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

/// Which pass a `Record` belongs to. Prefers the schema's own
/// `bench.pass` field (added in schema v3) when present; falls back to
/// sniffing which throughput-shaped field is set, for records written
/// before that field existed.
fn pass_of(record: &Record) -> &'static str {
    let bench = match record.bench.as_ref() {
        Some(b) => b,
        None => return "query",
    };
    if let Some(p) = bench.pass.as_deref() {
        match p {
            "throughput" => return "insert",
            "latency" => return "latency",
            "accuracy" => return "query",
            _ => {} // "merge" (shard-merge pass) or unrecognised: fall through to sniffing
        }
    }
    if bench.throughput_items_per_sec.is_some() {
        "insert"
    } else if bench.latency_ns.is_some() {
        "latency"
    } else {
        // query_throughput_items_per_sec set, or a memory-only second
        // pass with neither — both belong in the query/accuracy slot.
        "query"
    }
}

/// Fold the 2-3 `Record`s that share one (sketch, impl, sketch_config,
/// workload) identity into a single [`MergedRecord`]. Callers are
/// responsible for grouping records by identity and for handling repeat
/// runs (the same pass appearing twice) *before* calling this — pass it
/// exactly one insert / one query / one latency record, not a rerun's
/// worth of duplicates.
///
/// Returns `Err` naming the field if a record's `bench` section carries a
/// field this function doesn't explicitly know how to place. This is
/// deliberate: silently dropping or silently absorbing an unrecognized
/// field would hide a schema drift (a new metric nobody taught this
/// function about, or a bug) behind output that still "looks fine" —
/// failing loudly forces it to be noticed and handled.
pub fn flatten_record(records: &[Record]) -> Result<MergedRecord, String> {
    let base = records.first().expect("flatten_record requires at least one record");

    let mut out = MergedRecord {
        schema_version: base.schema_version,
        sketch: base.sketch.clone(),
        impl_name: base.impl_name.clone(),
        language: base.language,
        mode: base.mode,
        runs: base.runs,
        source: base.source,
        sketch_config: base.sketch_config.clone(),
        workload: base.workload.clone(),
        memory_bytes: None,
        accuracy: None,
        insert: InsertMetrics::default(),
        query: QueryMetrics::default(),
        latency: LatencyMetrics::default(),
    };

    for record in records {
        let phase = pass_of(record);
        let Some(bench) = record.bench.as_ref() else {
            continue;
        };

        // memory_bytes: insert's structural footprint wins if present;
        // otherwise take whatever pass offers it first.
        if let Some(mb) = bench.memory_bytes {
            if phase == "insert" || out.memory_bytes.is_none() {
                out.memory_bytes = Some(mb);
            }
        }
        if bench.accuracy.is_some() {
            out.accuracy = bench.accuracy.clone();
        }

        match phase {
            "insert" => {
                out.insert.timestamp = Some(record.timestamp);
                out.insert.throughput_items_per_sec = bench.throughput_items_per_sec;
                out.insert.throughput_samples = bench.throughput_samples.clone();
                out.insert.cpu_time_ms = bench.cpu_time_ms;
                out.insert.wall_time_ms = bench.wall_time_ms;
                out.insert.rss_peak_kb = bench.rss_peak_kb;
                out.insert.heap_allocated_kb = bench.heap_allocated_kb;
            }
            "query" => {
                out.query.timestamp = Some(record.timestamp);
                out.query.throughput_items_per_sec = bench.query_throughput_items_per_sec;
                out.query.cpu_time_ms = bench.cpu_time_ms;
                out.query.wall_time_ms = bench.wall_time_ms;
                out.query.rss_peak_kb = bench.rss_peak_kb;
                out.query.heap_allocated_kb = bench.heap_allocated_kb;
            }
            "latency" => {
                out.latency.timestamp = Some(record.timestamp);
                out.latency.latency_ns = bench.latency_ns;
            }
            _ => {}
        }

        // Fail loudly on any bench field this function doesn't place
        // explicitly above, instead of dropping it or silently absorbing
        // it into a catch-all.
        let bench_value = serde_json::to_value(bench)
            .map_err(|e| format!("flatten_record: could not inspect bench section: {e}"))?;
        if let Value::Object(bench_map) = bench_value {
            let named: &[&str] = &[
                "pass",
                "throughput_items_per_sec",
                "throughput_samples",
                "query_throughput_items_per_sec",
                "latency_ns",
                "cpu_time_ms",
                "wall_time_ms",
                "rss_peak_kb",
                "heap_allocated_kb",
                "memory_bytes",
                "accuracy",
            ];
            for (k, v) in bench_map {
                if named.contains(&k.as_str()) || v.is_null() {
                    continue;
                }
                return Err(format!(
                    "flatten_record: unrecognized bench field '{k}' on {}/{} (pass={phase}) — \
                     add explicit handling in flatten_record before merging this data",
                    out.sketch, out.impl_name
                ));
            }
        }
    }

    Ok(out)
}
