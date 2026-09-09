//! What `--flat` does: fold one invocation's records into one row. The row holds
//! one slot per operation and a metric is a field inside a slot, because a
//! measurement is
//! named by both. Grouping records by identity is the caller's job.

use aqpbm_core::{
    BenchSection, InsertMetrics, MergeMetrics, MergedRecord, PrepareMetrics, QueryMetrics, Record,
};

/// Which slot a `Record` lands in, per its own `bench.operation` field. Every
/// name `--operations` can produce is matched explicitly; anything else — no
/// `bench`, no `operation`, or an unknown one — is an error rather than a guess.
fn operation_of(record: &Record) -> Result<&'static str, String> {
    let bench = record.bench.as_ref().ok_or_else(|| {
        format!(
            "flatten_record: {}/{} record has no bench section, so its pass is unknown",
            record.sketch, record.library
        )
    })?;
    match bench.operation.as_deref() {
        Some("insert") => Ok("insert"),
        Some("query") => Ok("query"),
        Some("merge") => Ok("merge"),
        Some("prepare") => Ok("prepare"),
        Some(other) => Err(format!(
            "flatten_record: {}/{} record has unrecognized bench.operation '{other}'",
            record.sketch, record.library
        )),
        None => Err(format!(
            "flatten_record: {}/{} record has bench section but no bench.operation set",
            record.sketch, record.library
        )),
    }
}

/// The primary result a benchmark record was asked to produce. CPU and memory
/// are deliberately not variants here: they are secondary telemetry attached
/// to one of these primary passes by the runner.
#[derive(Clone, Copy, PartialEq, Eq)]
enum PassMetric {
    Throughput,
    Latency,
    Accuracy,
}

fn metric_of(record: &Record) -> Result<PassMetric, String> {
    let bench = record.bench.as_ref().ok_or_else(|| {
        format!(
            "flatten_record: {}/{} record has no bench section, so its metric is unknown",
            record.sketch, record.library
        )
    })?;
    match bench.metric.as_deref() {
        Some("throughput") => Ok(PassMetric::Throughput),
        Some("latency") => Ok(PassMetric::Latency),
        Some("accuracy") => Ok(PassMetric::Accuracy),
        Some(other) => Err(format!(
            "flatten_record: {}/{} record has unrecognized bench.metric '{other}'",
            record.sketch, record.library
        )),
        None => Err(format!(
            "flatten_record: {}/{} record has bench section but no bench.metric set",
            record.sketch, record.library
        )),
    }
}

/// Reject a primary result attached to the wrong pass. Secondary telemetry is
/// intentionally excluded: the runner attaches CPU/memory/wall-clock data to
/// every pass, and flattening discards it unless that pass is its canonical
/// owner below.
fn validate_primary_ownership(
    record: &Record,
    metric: PassMetric,
    bench: &BenchSection,
) -> Result<(), String> {
    let metric_name = bench.metric.as_deref().unwrap_or("<missing>");
    for (field, present, owner) in [
        (
            "throughput_items_per_sec",
            bench.throughput_items_per_sec.is_some(),
            PassMetric::Throughput,
        ),
        (
            "latency_ns",
            bench.latency_ns.is_some(),
            PassMetric::Latency,
        ),
        ("accuracy", bench.accuracy.is_some(), PassMetric::Accuracy),
    ] {
        if present && metric != owner {
            return Err(format!(
                "flatten_record: {}/{} {metric_name} pass contains {field}, which belongs to a different pass",
                record.sketch, record.library
            ));
        }
    }
    Ok(())
}

/// Fold the `Record`s sharing one (sketch, impl, sketch_config, dataset) identity
/// into a single [`MergedRecord`] — one record per measurement, grouped by the caller.
/// Every `BenchSection` field is bound by name, so an unplaced one fails the build.
pub fn flatten_record(records: &[Record]) -> Result<MergedRecord, String> {
    let base = records
        .first()
        .expect("flatten_record requires at least one record");

    let mut out = MergedRecord {
        schema_version: base.schema_version,
        sketch: base.sketch.clone(),
        library: base.library.clone(),
        language: base.language,
        mode: base.mode,
        runs: base.runs,
        source: base.source,
        sketch_config: base.sketch_config.clone(),
        input_dataset: base.input_dataset.clone(),
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
            latency_ns,
            cpu_time_ms,
            wall_time_ms,
            rss_peak_kb,
            heap_allocated_kb,
            memory_bytes,
            heap_bytes_net,
            heap_bytes_peak,
            accuracy,
            merge_folds_per_sec,
            merge_shards,
            merge_supported,
        } = bench;

        let metric = metric_of(record)?;
        validate_primary_ownership(record, metric, bench)?;
        if operation == "merge" && metric == PassMetric::Latency {
            return Err(format!(
                "flatten_record: {}/{} merge latency cannot be flattened because MergedRecord has no merge latency field",
                record.sketch, record.library
            ));
        }

        // A flattened field has exactly one canonical primary-pass owner.
        // Secondary telemetry on a latency/accuracy record is deliberately
        // discarded: it was collected under a different measurement protocol
        // and cannot be paired safely with throughput's work samples.
        macro_rules! keep_shared {
            ($slot:expr, $v:expr) => {
                if let Some(v) = $v {
                    $slot.get_or_insert(v.clone());
                }
            };
        }
        let owns_cost_telemetry = metric == PassMetric::Throughput
            || (operation == "prepare" && metric == PassMetric::Latency);
        if metric == PassMetric::Throughput {
            if let Some(mb) = memory_bytes {
                out.memory_bytes.get_or_insert(*mb);
            }
            if let Some(hb) = heap_bytes_net {
                out.heap_bytes_net.get_or_insert(*hb);
            }
            if let Some(hp) = heap_bytes_peak {
                out.heap_bytes_peak.get_or_insert(*hp);
            }
        }
        // `throughput_items_per_sec`, `latency_ns`, `accuracy` and the
        // `merge_*` fields are each owned by exactly one metric-pass per
        // operation — nothing legitimately produces a second value for one
        // of these within a slot. A second value is a genuine duplicate, so
        // it is refused by field name rather than silently resolved.
        macro_rules! keep_owned {
            ($slot:expr, $v:expr, $field:literal) => {
                if $v.is_some() {
                    if $slot.is_some() {
                        return Err(format!(
                            "flatten_record: {}/{} {operation} has two values for {}",
                            record.sketch, record.library, $field
                        ));
                    }
                    $slot = $v.clone();
                }
            };
        }
        match operation {
            "insert" => {
                if metric == PassMetric::Throughput {
                    out.insert.timestamp = Some(record.timestamp);
                    keep_owned!(
                        out.insert.throughput_items_per_sec,
                        *throughput_items_per_sec,
                        "throughput_items_per_sec"
                    );
                }
                if metric == PassMetric::Latency {
                    out.insert.timestamp = Some(record.timestamp);
                    keep_owned!(out.insert.latency_ns, *latency_ns, "latency_ns");
                }
                if owns_cost_telemetry {
                    keep_shared!(out.insert.cpu_time_ms, cpu_time_ms);
                    keep_shared!(out.insert.wall_time_ms, wall_time_ms);
                    keep_shared!(out.insert.rss_peak_kb, rss_peak_kb);
                    keep_shared!(out.insert.heap_allocated_kb, heap_allocated_kb);
                }
            }
            "query" => {
                if metric == PassMetric::Throughput {
                    out.query.timestamp = Some(record.timestamp);
                    keep_owned!(
                        out.query.throughput_items_per_sec,
                        *throughput_items_per_sec,
                        "throughput_items_per_sec"
                    );
                }
                if metric == PassMetric::Accuracy {
                    out.query.timestamp = Some(record.timestamp);
                    keep_owned!(out.query.accuracy, accuracy.clone(), "accuracy");
                }
                if metric == PassMetric::Latency {
                    out.query.timestamp = Some(record.timestamp);
                    keep_owned!(out.query.latency_ns, *latency_ns, "latency_ns");
                }
                if owns_cost_telemetry {
                    keep_shared!(out.query.cpu_time_ms, cpu_time_ms);
                    keep_shared!(out.query.wall_time_ms, wall_time_ms);
                    keep_shared!(out.query.rss_peak_kb, rss_peak_kb);
                    keep_shared!(out.query.heap_allocated_kb, heap_allocated_kb);
                }
            }
            "merge" => {
                if metric == PassMetric::Throughput {
                    out.merge.timestamp = Some(record.timestamp);
                    keep_owned!(
                        out.merge.merge_folds_per_sec,
                        *merge_folds_per_sec,
                        "merge_folds_per_sec"
                    );
                    keep_owned!(out.merge.merge_shards, *merge_shards, "merge_shards");
                    keep_owned!(
                        out.merge.merge_supported,
                        *merge_supported,
                        "merge_supported"
                    );
                }
                if owns_cost_telemetry {
                    keep_shared!(out.merge.cpu_time_ms, cpu_time_ms);
                    keep_shared!(out.merge.wall_time_ms, wall_time_ms);
                    keep_shared!(out.merge.rss_peak_kb, rss_peak_kb);
                    keep_shared!(out.merge.heap_allocated_kb, heap_allocated_kb);
                }
            }
            "prepare" => {
                if metric == PassMetric::Latency {
                    out.prepare.timestamp = Some(record.timestamp);
                    keep_shared!(out.prepare.cpu_time_ms, cpu_time_ms);
                    keep_shared!(out.prepare.wall_time_ms, wall_time_ms);
                    keep_shared!(out.prepare.rss_peak_kb, rss_peak_kb);
                    keep_shared!(out.prepare.heap_allocated_kb, heap_allocated_kb);
                }
            }
            _ => unreachable!("operation_of only returns insert/query/merge/prepare"),
        }
    }

    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use aqpbm_core::benchmark_result::schema::{LatencySummary, Mode, RunStats};
    use aqpbm_datagen::{ColumnSpec, DataDistribution, TableDescription, UniformParameter};

    fn stats(mean: f64) -> RunStats {
        RunStats {
            mean,
            stddev: 0.0,
            ci95: None,
            n: 1,
            samples: vec![mean],
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
        let wd = TableDescription::single(
            "key",
            ColumnSpec {
                distribution: DataDistribution::Uniform(UniformParameter {
                    lower_bound: 0.0,
                    upper_bound: 100.0,
                    seed: 1,
                }),
                shift: None,
                cardinality: None,
                special_rule: aqpbm_datagen::RULE_NONE,
                data_type: "i64".into(),
                string: None,
            },
            1000,
        );
        let mut rec = Record::new("cms", "oxide", wd, Mode::Bench, 1);
        rec.bench = Some(BenchSection {
            operation: Some(operation.into()),
            metric: Some(metric.into()),
            ..bench
        });
        rec
    }

    /// Two measurements of one metric over two operations. Keyed on the metric they
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
                    throughput_items_per_sec: Some(stats(700.0)),
                    ..Default::default()
                },
            ),
        ];
        let out = flatten_record(&rows).expect("both measurements are named");
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
        let out = flatten_record(&rows).expect("both measurements are named");
        assert_eq!(out.insert.latency_ns.unwrap().count, 30_000);
        assert_eq!(out.query.latency_ns.unwrap().count, 2_381);
    }

    /// Merge latency has no representation in `MergedRecord`; accepting it
    /// would silently lose the requested primary result.
    #[test]
    fn merge_latency_without_an_output_field_is_refused() {
        let rows = vec![
            record(
                "merge",
                "latency",
                BenchSection {
                    wall_time_ms: Some(stats(0.15)),
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
        let err = flatten_record(&rows).expect_err("merge latency must not be silently dropped");
        assert!(err.contains("merge latency"), "{err}");
    }

    /// The deferred build has its own slot, so a caller asking only about
    /// inserting does not read it off the insert record.
    #[test]
    fn prepare_carries_its_own_time() {
        let rows = vec![record(
            "prepare",
            "latency",
            BenchSection {
                wall_time_ms: Some(stats(0.005)),
                ..Default::default()
            },
        )];
        let out = flatten_record(&rows).expect("the measurement is named");
        assert_eq!(out.prepare.wall_time_ms.unwrap().mean, 0.005);
    }

    /// A record whose operation this function does not place is an error, not
    /// a guess: a silently dropped measurement reads as one nobody asked for.
    #[test]
    fn an_unplaceable_operation_is_refused_by_name() {
        let rows = vec![record("teleport", "latency", BenchSection::default())];
        let err = flatten_record(&rows).expect_err("no slot holds it");
        assert!(err.contains("teleport"), "{err}");
    }

    /// `cpu_time_ms`/`wall_time_ms`/`rss_peak_kb`/`heap_allocated_kb` ride
    /// along with every square regardless of which metric drove it — an
    /// ordinary run crossing `insert` with both `throughput` and `latency`
    /// has both squares set `wall_time_ms`. That's expected, not a
    /// duplicate, so the first one measured wins silently.
    #[test]
    fn shared_timing_fields_on_two_squares_of_one_operation_first_wins() {
        let rows = vec![
            record(
                "insert",
                "throughput",
                BenchSection {
                    wall_time_ms: Some(stats(1.0)),
                    ..Default::default()
                },
            ),
            record(
                "insert",
                "latency",
                BenchSection {
                    wall_time_ms: Some(stats(2.0)),
                    ..Default::default()
                },
            ),
        ];
        let out = flatten_record(&rows).expect("shared fields never collide");
        assert_eq!(out.insert.wall_time_ms.unwrap().mean, 1.0);
    }

    #[test]
    fn accuracy_first_does_not_own_query_timing() {
        let rows = vec![
            record(
                "query",
                "accuracy",
                BenchSection {
                    accuracy: Some(serde_json::json!({"mean_rank_err": 0.01})),
                    wall_time_ms: Some(stats(1.0)),
                    ..Default::default()
                },
            ),
            record(
                "query",
                "throughput",
                BenchSection {
                    throughput_items_per_sec: Some(stats(1_000.0)),
                    wall_time_ms: Some(stats(5.0)),
                    ..Default::default()
                },
            ),
        ];

        let out = flatten_record(&rows).expect("each pass owns a distinct field set");
        assert_eq!(
            out.query.accuracy,
            Some(serde_json::json!({"mean_rank_err": 0.01}))
        );
        assert_eq!(out.query.wall_time_ms.unwrap().mean, 5.0);
    }

    #[test]
    fn primary_field_on_the_wrong_pass_is_refused_by_name() {
        let rows = vec![record(
            "query",
            "accuracy",
            BenchSection {
                throughput_items_per_sec: Some(stats(1_000.0)),
                ..Default::default()
            },
        )];

        let err = flatten_record(&rows).expect_err("accuracy cannot own throughput");
        assert!(err.contains("accuracy"), "{err}");
        assert!(err.contains("throughput_items_per_sec"), "{err}");
    }

    /// `throughput_items_per_sec` is owned by exactly one metric-pass per
    /// operation — nothing legitimately produces it twice for `insert`. A
    /// second value here is a genuine duplicate, refused by field name
    /// rather than the second value silently overwriting or being silently
    /// dropped.
    #[test]
    fn two_squares_colliding_on_an_owned_field_is_an_error() {
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
                "insert",
                "throughput",
                BenchSection {
                    throughput_items_per_sec: Some(stats(901.0)),
                    ..Default::default()
                },
            ),
        ];
        let err = flatten_record(&rows).expect_err("both squares set throughput_items_per_sec");
        assert!(err.contains("throughput_items_per_sec"), "{err}");
    }
}
