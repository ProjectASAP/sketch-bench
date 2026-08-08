//! What `--flat` does: fold one cell's records into one row.
//!
//! The row holds one slot per operation, and a metric is a field inside a
//! slot. A square is named by both, so a slot keyed on either name alone
//! would hold two squares at once.
//!
//! [`MergedRecord`] lives in `aqpbm_core` beside [`Record`], being a wire
//! shape and not CLI logic. Grouping records by identity is the caller's job.

use aqpbm_core::{
    BenchSection, InsertMetrics, MergeMetrics, MergedRecord, PrepareMetrics, QueryMetrics, Record,
};

/// Which slot a `Record` lands in, per its own `bench.operation` field. Every
/// name `--operations` can produce is matched explicitly; anything else — a
/// missing `bench` section, a missing `operation` value, or one none of these
/// arms names — is an error rather than a guess. Which of these slots get
/// shown, and how, is the leaderboard's call, not this function's: its job is
/// only to keep the flattened record complete.
fn operation_of(record: &Record) -> Result<&'static str, String> {
    let bench = record.bench.as_ref().ok_or_else(|| {
        format!(
            "flatten_record: {}/{} record has no bench section, so its pass is unknown",
            record.sketch, record.impl_name
        )
    })?;
    match bench.operation.as_deref() {
        Some("insert") => Ok("insert"),
        Some("query") => Ok("query"),
        Some("merge") => Ok("merge"),
        Some("prepare") => Ok("prepare"),
        Some(other) => Err(format!(
            "flatten_record: {}/{} record has unrecognized bench.operation '{other}'",
            record.sketch, record.impl_name
        )),
        None => Err(format!(
            "flatten_record: {}/{} record has bench section but no bench.operation set",
            record.sketch, record.impl_name
        )),
    }
}

/// Fold the `Record`s that share one (sketch, impl, sketch_config, workload)
/// identity into a single [`MergedRecord`]. Callers are responsible for
/// grouping records by identity before calling this: pass it one record per
/// square, not a rerun's worth of duplicates.
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
        insert: InsertMetrics::default(),
        query: QueryMetrics::default(),
        merge: MergeMetrics::default(),
        prepare: PrepareMetrics::default(),
    };

    for record in records {
        let operation = operation_of(record)?;
        let Some(bench) = record.bench.as_ref() else {
            continue;
        };

        // Named field-by-field, not `..`: a field added to `BenchSection`
        // that isn't listed here fails the build instead of being
        // silently dropped. See the doc comment above.
        let BenchSection {
            metric: _,
            operation: _,
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
            merge_folds_per_sec,
            merge_shards,
            merge_supported,
        } = bench;

        // A sketch has one size however many squares measured it, so the
        // first value wins and no square is favoured. `heap_bytes_net` stays
        // separate from `memory_bytes`: see the field doc on
        // `MergedRecord::heap_bytes_net`.
        if let Some(mb) = memory_bytes {
            out.memory_bytes.get_or_insert(*mb);
        }
        if let Some(hb) = heap_bytes_net {
            out.heap_bytes_net.get_or_insert(*hb);
        }
        if let Some(hp) = heap_bytes_peak {
            out.heap_bytes_peak.get_or_insert(*hp);
        }
        // Assigned with `get_or_insert`-style guards on the fields two squares
        // of one operation both carry: `(insert, throughput)` and
        // `(insert, latency)` each bring their own `wall_time_ms`, and whichever
        // arrives second must not blank what the first placed.
        macro_rules! keep {
            ($slot:expr, $v:expr) => {
                if $v.is_some() {
                    $slot = $v.clone();
                }
            };
        }
        match operation {
            "insert" => {
                out.insert.timestamp = Some(record.timestamp);
                keep!(out.insert.throughput_items_per_sec, *throughput_items_per_sec);
                keep!(out.insert.throughput_samples, throughput_samples.clone());
                keep!(
                    out.insert.build_throughput_items_per_sec,
                    *build_throughput_items_per_sec
                );
                keep!(out.insert.latency_ns, *latency_ns);
                keep!(out.insert.cpu_time_ms, *cpu_time_ms);
                keep!(out.insert.wall_time_ms, *wall_time_ms);
                keep!(out.insert.rss_peak_kb, *rss_peak_kb);
                keep!(out.insert.heap_allocated_kb, *heap_allocated_kb);
            }
            "query" => {
                out.query.timestamp = Some(record.timestamp);
                keep!(
                    out.query.throughput_items_per_sec,
                    *query_throughput_items_per_sec
                );
                keep!(out.query.latency_ns, *latency_ns);
                keep!(out.query.accuracy, accuracy.clone());
                keep!(out.query.cpu_time_ms, *cpu_time_ms);
                keep!(out.query.wall_time_ms, *wall_time_ms);
                keep!(out.query.rss_peak_kb, *rss_peak_kb);
                keep!(out.query.heap_allocated_kb, *heap_allocated_kb);
            }
            "merge" => {
                out.merge.timestamp = Some(record.timestamp);
                keep!(out.merge.merge_time_ms, *merge_time_ms);
                keep!(out.merge.merge_folds_per_sec, *merge_folds_per_sec);
                keep!(out.merge.merge_shards, *merge_shards);
                keep!(out.merge.merge_supported, *merge_supported);
                keep!(out.merge.cpu_time_ms, *cpu_time_ms);
                keep!(out.merge.wall_time_ms, *wall_time_ms);
                keep!(out.merge.rss_peak_kb, *rss_peak_kb);
                keep!(out.merge.heap_allocated_kb, *heap_allocated_kb);
            }
            "prepare" => {
                out.prepare.timestamp = Some(record.timestamp);
                keep!(out.prepare.finalize_time_ms, *finalize_time_ms);
                keep!(out.prepare.cpu_time_ms, *cpu_time_ms);
                keep!(out.prepare.wall_time_ms, *wall_time_ms);
                keep!(out.prepare.rss_peak_kb, *rss_peak_kb);
                keep!(out.prepare.heap_allocated_kb, *heap_allocated_kb);
            }
            _ => unreachable!("operation_of only returns insert/query/merge/prepare"),
        }
    }

    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use aqpbm_core::report::{LatencySummary, Mode, RunStats};
    use aqpbm_core::WorkloadDescription;

    fn stats(mean: f64) -> RunStats {
        RunStats {
            mean,
            stddev: 0.0,
            ci95: None,
            n: 1,
        }
    }

    fn latency(count: u64) -> LatencySummary {
        LatencySummary {
            p50: 10,
            p95: 20,
            p99: 30,
            p999: 40,
            max: 50,
            count,
        }
    }

    fn record(operation: &str, metric: &str, bench: BenchSection) -> Record {
        let wd = WorkloadDescription {
            shape: "uniform".into(),
            size: 1000,
            cardinality: Some(100),
            zipf_s: None,
            source_path: None,
            seed: Some(1),
            spec: None,
        };
        let mut rec = Record::new("cms", "oxide", wd, Mode::Bench, 1);
        rec.bench = Some(BenchSection {
            operation: Some(operation.into()),
            metric: Some(metric.into()),
            ..bench
        });
        rec
    }

    /// Two squares of one metric over two operations. Keyed on the metric they
    /// were the same slot, and the second silently blanked the first.
    #[test]
    fn one_metric_over_two_operations_keeps_both_numbers() {
        let rows = vec![
            record(
                "insert",
                "throughput",
                BenchSection {
                    throughput_items_per_sec: Some(stats(900.0)),
                    ..Default::default()
                },
            ),
            record(
                "query",
                "throughput",
                BenchSection {
                    query_throughput_items_per_sec: Some(stats(700.0)),
                    ..Default::default()
                },
            ),
        ];
        let out = flatten_record(&rows).expect("both squares are named");
        assert_eq!(out.insert.throughput_items_per_sec.unwrap().mean, 900.0);
        assert_eq!(out.query.throughput_items_per_sec.unwrap().mean, 700.0);
    }

    /// Both latencies are `latency_ns` in their own record, so the flattened
    /// row is where they must stop being one field. `count` tells them apart:
    /// inserts are counted per item, probes per question.
    #[test]
    fn insert_and_query_latency_are_two_fields() {
        let rows = vec![
            record(
                "insert",
                "latency",
                BenchSection {
                    latency_ns: Some(latency(30_000)),
                    ..Default::default()
                },
            ),
            record(
                "query",
                "latency",
                BenchSection {
                    latency_ns: Some(latency(2_381)),
                    ..Default::default()
                },
            ),
        ];
        let out = flatten_record(&rows).expect("both squares are named");
        assert_eq!(out.insert.latency_ns.unwrap().count, 30_000);
        assert_eq!(out.query.latency_ns.unwrap().count, 2_381);
    }

    /// Two squares of one operation. Both carry a `wall_time_ms`, and the one
    /// arriving second must not blank the field the first filled.
    #[test]
    fn two_squares_of_one_operation_share_a_slot_without_erasing() {
        let rows = vec![
            record(
                "merge",
                "latency",
                BenchSection {
                    merge_time_ms: Some(stats(0.15)),
                    merge_shards: Some(4),
                    ..Default::default()
                },
            ),
            record(
                "merge",
                "throughput",
                BenchSection {
                    merge_folds_per_sec: Some(stats(19_496.0)),
                    ..Default::default()
                },
            ),
        ];
        let out = flatten_record(&rows).expect("both squares are named");
        assert_eq!(out.merge.merge_time_ms.unwrap().mean, 0.15);
        assert_eq!(out.merge.merge_folds_per_sec.unwrap().mean, 19_496.0);
        assert_eq!(out.merge.merge_shards, Some(4));
    }

    /// The deferred build has its own slot, so it is no longer read off the
    /// insert record by a caller who asked only about inserting.
    #[test]
    fn prepare_carries_its_own_finalize_time() {
        let rows = vec![record(
            "prepare",
            "latency",
            BenchSection {
                finalize_time_ms: Some(stats(0.005)),
                ..Default::default()
            },
        )];
        let out = flatten_record(&rows).expect("the square is named");
        assert_eq!(out.prepare.finalize_time_ms.unwrap().mean, 0.005);
    }

    /// A record whose operation this function does not place is an error, not
    /// a guess: a silently dropped square reads as a square nobody asked for.
    #[test]
    fn an_unplaceable_operation_is_refused_by_name() {
        let rows = vec![record("teleport", "latency", BenchSection::default())];
        let err = flatten_record(&rows).expect_err("no slot holds it");
        assert!(err.contains("teleport"), "{err}");
    }
}
