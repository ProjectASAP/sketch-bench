//! Flatten logic for a planned `approxbench merge` command — the intended
//! Rust replacement for `scripts/merge_passes.py`.
//!
//! The CLI emits one raw [`Record`] per metric pass (see the comment at
//! `main.rs`'s `run_bench`: "A downstream group-by on (sketch, impl,
//! sketch_config, workload) merges them back."). [`flatten_record`] is that
//! downstream step: given the 2-4 [`Record`]s that share one identity
//! (one insert pass, one query/accuracy pass, optionally a latency pass,
//! optionally a merge pass), it folds them into a single [`MergedRecord`]
//! row. [`MergedRecord`] itself lives in `aqpbm_core` alongside [`Record`]
//! — it's a JSONL wire shape, not CLI-specific logic.
//!
//! Grouping records by identity, and splitting a re-run (the same pass
//! appearing twice) into two separate calls to `flatten_record` instead of
//! silently overwriting, is the caller's job — not implemented here yet.

use aqpbm_core::{
    BenchSection, InsertMetrics, LatencyMetrics, MergeMetrics, MergedRecord, QueryMetrics, Record,
};

/// Which pass a `Record` belongs to, per its own `bench.pass` field
/// (schema v3+). Every name `--metrics` can produce (`throughput`,
/// `latency`, `accuracy`, `merge`) is matched explicitly; anything else —
/// a missing `bench` section, a missing `pass` value, or a `pass` value
/// none of these arms names — is an error rather than a guess. Which of
/// these slots get shown, and how, is the leaderboard's call, not this
/// function's: its job is only to keep the flattened record complete.
fn pass_of(record: &Record) -> Result<&'static str, String> {
    let bench = record.bench.as_ref().ok_or_else(|| {
        format!(
            "flatten_record: {}/{} record has no bench section, so its pass is unknown",
            record.sketch, record.impl_name
        )
    })?;
    match bench.pass.as_deref() {
        Some("throughput") => Ok("insert"),
        Some("latency") => Ok("latency"),
        Some("accuracy") => Ok("query"),
        Some("merge") => Ok("merge"),
        Some(other) => Err(format!(
            "flatten_record: {}/{} record has unrecognized bench.pass '{other}'",
            record.sketch, record.impl_name
        )),
        None => Err(format!(
            "flatten_record: {}/{} record has bench section but no bench.pass set",
            record.sketch, record.impl_name
        )),
    }
}

/// Fold the 2-3 `Record`s that share one (sketch, impl, sketch_config,
/// workload) identity into a single [`MergedRecord`]. Callers are
/// responsible for grouping records by identity and for handling repeat
/// runs (the same pass appearing twice) *before* calling this — pass it
/// exactly one insert / one query / one latency record, not a rerun's
/// worth of duplicates.
///
/// Every `BenchSection` field is bound by name below (not `..`), so a
/// field this function doesn't yet place is a **compile error**, not a
/// silently dropped or silently absorbed value: adding a field to
/// `BenchSection` without teaching this function about it fails the build,
/// which is stronger than a runtime check and can't drift out of date the
/// way a hand-maintained list of field names could.
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
        heap_bytes_net: None,
        heap_bytes_peak: None,
        accuracy: None,
        insert: InsertMetrics::default(),
        query: QueryMetrics::default(),
        latency: LatencyMetrics::default(),
        merge: MergeMetrics::default(),
    };

    for record in records {
        let phase = pass_of(record)?;
        let Some(bench) = record.bench.as_ref() else {
            continue;
        };

        // Named field-by-field, not `..`: a field added to `BenchSection`
        // that isn't listed here fails the build instead of being
        // silently dropped. See the doc comment above.
        let BenchSection {
            pass: _,
            throughput_items_per_sec,
            throughput_samples,
            build_throughput_items_per_sec,
            finalize_time_ms,
            query_throughput_items_per_sec,
            latency_ns,
            cpu_time_ms,
            wall_time_ms,
            rss_peak_kb,
            heap_allocated_kb,
            memory_bytes,
            heap_bytes_net,
            heap_bytes_peak,
            accuracy,
            merge_time_ms,
            merge_shards,
            merge_supported,
        } = bench;

        // Structural footprint fields: pass-invariant (the sketch has one
        // size, however many passes measure it), so first value wins
        // rather than favoring any particular pass. `heap_bytes_net` is
        // kept separate from `memory_bytes` rather than overriding it —
        // see the field doc on `MergedRecord::heap_bytes_net`.
        if let Some(mb) = memory_bytes {
            out.memory_bytes.get_or_insert(*mb);
        }
        if let Some(hb) = heap_bytes_net {
            out.heap_bytes_net.get_or_insert(*hb);
        }
        if let Some(hp) = heap_bytes_peak {
            out.heap_bytes_peak.get_or_insert(*hp);
        }
        if accuracy.is_some() {
            out.accuracy = accuracy.clone();
        }

        match phase {
            "insert" => {
                out.insert.timestamp = Some(record.timestamp);
                out.insert.throughput_items_per_sec = *throughput_items_per_sec;
                out.insert.throughput_samples = throughput_samples.clone();
                out.insert.build_throughput_items_per_sec = *build_throughput_items_per_sec;
                out.insert.finalize_time_ms = *finalize_time_ms;
                out.insert.cpu_time_ms = *cpu_time_ms;
                out.insert.wall_time_ms = *wall_time_ms;
                out.insert.rss_peak_kb = *rss_peak_kb;
                out.insert.heap_allocated_kb = *heap_allocated_kb;
            }
            "query" => {
                out.query.timestamp = Some(record.timestamp);
                out.query.throughput_items_per_sec = *query_throughput_items_per_sec;
                out.query.cpu_time_ms = *cpu_time_ms;
                out.query.wall_time_ms = *wall_time_ms;
                out.query.rss_peak_kb = *rss_peak_kb;
                out.query.heap_allocated_kb = *heap_allocated_kb;
            }
            "latency" => {
                out.latency.timestamp = Some(record.timestamp);
                out.latency.latency_ns = *latency_ns;
            }
            "merge" => {
                out.merge.timestamp = Some(record.timestamp);
                out.merge.merge_time_ms = *merge_time_ms;
                out.merge.merge_shards = *merge_shards;
                out.merge.merge_supported = *merge_supported;
            }
            _ => unreachable!("pass_of only returns insert/query/latency/merge"),
        }
    }

    Ok(out)
}
