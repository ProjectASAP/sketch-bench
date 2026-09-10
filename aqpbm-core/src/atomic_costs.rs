//! Reduce [`MergedRecord`]s down to the per-op cost document ASAPQuery's
//! optimizer loads (see ASAPQuery#524, sketch-bench#30). Lives in `aqpbm-core`
//! rather than `aqpbm-cli` because it is a JSONL-derived wire shape, same
//! reasoning as `MergedRecord` itself.
//!
//! No `subtract_cpu_secs` field: `asap_sketchlib` has no `subtract` yet
//! (asap_sketchlib#69), so there is nothing to measure. ASAPQuery's loader
//! treats a missing entry as "drop the candidate", which covers this too.
//!
//! Costs and accuracy are workload-dependent (external traces make that
//! literal, not hypothetical), so entries are grouped into one
//! [`AtomicCostProfile`] per distinct [`WorkloadDescription`] rather than
//! flattened into one bare array — a reader that ignored `workload` would
//! silently mix measurements that were never comparable.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::benchmark_result::{CpuTime, MergedRecord, RunStats, WorkloadDescription};

/// Bumped whenever a breaking change lands in [`AtomicCostDocument`],
/// [`AtomicCostProfile`] or [`AtomicCostEntry`]. Independent of
/// `benchmark_result::SCHEMA_VERSION`: this is a different wire interface
/// (ASAPQuery's loader), with its own compatibility lifecycle, not a
/// derivative of the report schema's.
pub const ATOMIC_COST_SCHEMA_VERSION: u32 = 2;

/// Immutable identity of the exporter-ready scenario on which a cost profile
/// was measured. This is deliberately profile-level provenance: all entries
/// in a profile share the same stream, while entries differ only by sketch
/// construction configuration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScenarioIdentity {
    /// SHA-256 of the canonical decompressed wrangled CSV payload.
    pub payload_sha256: String,
    pub exported_metric: String,
    pub grouping_labels: Vec<String>,
    pub source_time_range_us: [i64; 2],
}

/// The atomic-cost document ASAPQuery's loader reads: a version gate plus one
/// profile per workload the grid was measured against.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AtomicCostDocument {
    pub schema_version: u32,
    pub profiles: Vec<AtomicCostProfile>,
}

/// Every atomic-cost entry measured against one workload. Entries never mix
/// across profiles: `(sketch, sketch_config)` is only unique *within* a
/// profile, since the same construction point measured against two workloads
/// is two different real numbers, not a collision.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AtomicCostProfile {
    pub workload: WorkloadDescription,
    pub scenario: ScenarioIdentity,
    pub entries: Vec<AtomicCostEntry>,
}

/// One row: measured atomic costs for one (sketch algorithm, construction
/// params) point, at the grid resolution ASAPQuery's `candidate_gen.rs`
/// sweeps. `sketch_config` is `MergedRecord::sketch_config` reused verbatim,
/// so ASAPQuery's loader keys on it directly instead of re-deriving it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AtomicCostEntry {
    pub sketch: String,
    pub sketch_config: serde_json::Value,
    pub mem_bytes_per_instance: f64,
    pub insert_cpu_secs: f64,
    pub merge_cpu_secs: f64,
    pub query_cpu_secs: f64,
    /// The registry's comparator score for this (sketch, config) point,
    /// carried over verbatim from `MergedRecord::query.accuracy` — its keys
    /// are whatever that comparator's `GroundTruth::score` named them
    /// (`aqpbm_core::accuracy`), which differ by capability (frequency's
    /// `are_top10`/`l1_err`/… vs. cardinality's `relative_error` vs.
    /// rank-error's `mean_rank_err`). No single scalar covers all of them, so
    /// this stays a map rather than picking one field to promote.
    pub query_accuracy: BTreeMap<String, f64>,
}

/// Why one [`MergedRecord`] didn't produce a row. Not an error: the caller
/// logs these and moves on, same "missing entry, no crash" policy the table's
/// reader (ASAPQuery) uses on its side.
#[derive(Debug, Clone, PartialEq)]
pub enum SkipReason {
    MissingField(&'static str),
    /// Like `MissingField`, but for the per-op measurements in
    /// `cpu_secs_per_op`, which need to say *which* of an op's three
    /// sub-fields (rate/elapsed/cpu_time) was absent rather than just the op
    /// name — otherwise "missing insert" could mean any of the three, and
    /// the operator has to go diff the raw `--flat` JSONL by hand to find
    /// out which.
    MissingSubField(&'static str, &'static str),
    /// A rate/elapsed pair implies zero work — dividing by it would produce
    /// `inf`/`NaN` rather than a real cost.
    ZeroWork(&'static str),
    /// `rate`, `elapsed_ms` and `cpu_time_ms` didn't carry the same number of
    /// per-run samples, so there's no safe way to pair sample *i* of one with
    /// sample *i* of another — `fold.rs`'s `rate()`/`elapsed_ms()`/
    /// `cpu_time_ms()` each filter out unusable runs independently, so a run
    /// dropped by one but not another shifts every later index out of step.
    /// Pairing anyway would silently multiply/divide unrelated runs' numbers
    /// together, which is worse than skipping.
    MisalignedSamples(&'static str),
    /// A field was present but not shaped the way this reduction expects
    /// (e.g. `query_accuracy` holding something other than a flat object of
    /// numbers) — a comparator/schema bug, not a run that simply didn't
    /// measure the field.
    InvalidField(&'static str),
}

impl std::fmt::Display for SkipReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SkipReason::MissingField(field) => write!(f, "missing {field}"),
            SkipReason::MissingSubField(op, field) => write!(f, "missing {op} {field}"),
            SkipReason::ZeroWork(op) => write!(f, "{op} rate/elapsed imply zero work"),
            SkipReason::MisalignedSamples(op) => {
                write!(f, "{op} rate/elapsed/cpu_time sample counts disagree")
            }
            SkipReason::InvalidField(field) => write!(f, "{field} is not the expected shape"),
        }
    }
}

/// Seconds of CPU time (user + sys) per single operation, derived from a
/// rate/elapsed pair rather than from an assumed item or query count.
///
/// `rate` (items or folds per sec) and `elapsed_ms` don't serialize the raw
/// `work` count each run measured, only their own per-run samples. But
/// `rate.samples[i]` and `elapsed_ms.samples[i]` come from the *same* run
/// `i`, so `rate.samples[i] * elapsed_ms.samples[i]` recovers that run's
/// exact `work` count (rather than `rate.mean * elapsed_ms.mean`, which is
/// biased whenever elapsed time varies run to run — the mean of a ratio times
/// the mean of its denominator isn't the mean of the numerator). Summing
/// `work_i` and `cpu_i` separately and dividing the sums, instead of
/// averaging each run's `cpu_i / work_i`, matches what "total CPU time over
/// total ops" means physically.
///
/// This works uniformly for insert (rate = items/sec), query (rate =
/// items/sec) and merge (rate = folds/sec, so the result is cost per fold,
/// which is what `query_cost`'s `merges * merge_cpu_secs` wants).
fn cpu_secs_per_op(
    op: &'static str,
    rate: Option<&RunStats>,
    elapsed_ms: Option<&RunStats>,
    cpu_time_ms: Option<&CpuTime>,
) -> Result<f64, SkipReason> {
    let rate = rate.ok_or(SkipReason::MissingSubField(op, "rate"))?;
    let elapsed_ms = elapsed_ms.ok_or(SkipReason::MissingSubField(op, "elapsed"))?;
    let cpu_time_ms = cpu_time_ms.ok_or(SkipReason::MissingSubField(op, "cpu_time"))?;

    let n = rate.samples.len();
    if n == 0
        || elapsed_ms.samples.len() != n
        || cpu_time_ms.user_ms.samples.len() != n
        || cpu_time_ms.sys_ms.samples.len() != n
    {
        return Err(SkipReason::MisalignedSamples(op));
    }

    let mut work_total = 0.0;
    let mut cpu_ms_total = 0.0;
    for i in 0..n {
        work_total += rate.samples[i] * (elapsed_ms.samples[i] / 1000.0);
        cpu_ms_total += cpu_time_ms.user_ms.samples[i] + cpu_time_ms.sys_ms.samples[i];
    }

    if !work_total.is_finite() || work_total <= 0.0 {
        return Err(SkipReason::ZeroWork(op));
    }
    Ok((cpu_ms_total / 1000.0) / work_total)
}

/// Reduce one [`MergedRecord`] to one [`AtomicCostEntry`], or say which
/// required field was absent. A record from a `--flat` run that didn't
/// measure insert+query+merge+memory in one invocation is expected to fail
/// this — that's a driver bug (wrong `--operations`/`--metrics`), not a
/// candidate to silently drop, so the caller should treat `SkipReason` as
/// worth logging even though it isn't fatal.
pub fn reduce_one(record: &MergedRecord) -> Result<AtomicCostEntry, SkipReason> {
    let mem_bytes_per_instance = record
        .memory_bytes
        .ok_or(SkipReason::MissingField("memory_bytes"))? as f64;

    let insert_cpu_secs = cpu_secs_per_op(
        "insert",
        record.insert.throughput_items_per_sec.as_ref(),
        record.insert.wall_time_ms.as_ref(),
        record.insert.cpu_time_ms.as_ref(),
    )?;
    let query_cpu_secs = cpu_secs_per_op(
        "query",
        record.query.throughput_items_per_sec.as_ref(),
        record.query.wall_time_ms.as_ref(),
        record.query.cpu_time_ms.as_ref(),
    )?;
    let merge_cpu_secs = cpu_secs_per_op(
        "merge",
        record.merge.merge_folds_per_sec.as_ref(),
        record.merge.wall_time_ms.as_ref(),
        record.merge.cpu_time_ms.as_ref(),
    )?;

    let query_accuracy = query_accuracy(record.query.accuracy.as_ref())?;

    Ok(AtomicCostEntry {
        sketch: record.sketch.clone(),
        sketch_config: record
            .sketch_config
            .clone()
            .unwrap_or(serde_json::Value::Null),
        mem_bytes_per_instance,
        insert_cpu_secs,
        merge_cpu_secs,
        query_cpu_secs,
        query_accuracy,
    })
}

/// `MergedRecord::query.accuracy` down to the flat numeric map every
/// comparator's `GroundTruth::score` actually produces. Required like the
/// cost fields (missing skips the row, `SkipReason` says which): an entry
/// with no comparator never runs a query at all (registry's `comparator:
/// None` entries are all insert-only), so it is already excluded here by
/// `query_cpu_secs` above — requiring accuracy too costs nothing further and
/// keeps every kept row equally complete.
fn query_accuracy(
    accuracy: Option<&serde_json::Value>,
) -> Result<BTreeMap<String, f64>, SkipReason> {
    let accuracy = accuracy.ok_or(SkipReason::MissingField("query_accuracy"))?;
    let object = accuracy
        .as_object()
        .ok_or(SkipReason::InvalidField("query_accuracy"))?;
    object
        .iter()
        .map(|(k, v)| v.as_f64().map(|v| (k.clone(), v)))
        .collect::<Option<_>>()
        .ok_or(SkipReason::InvalidField("query_accuracy"))
}

/// Two records grouped into the same [`AtomicCostProfile`] reduced to the
/// same `(sketch, sketch_config)` point. ASAPQuery's loader keys entries by
/// that pair within a profile, so silently keeping one of the two (first- or
/// last-wins) would silently discard a real measurement — this is reported
/// as a hard error instead, same "don't guess" policy as [`SkipReason`], just
/// not one a single record can detect on its own.
#[derive(Debug, Clone, PartialEq)]
pub struct DuplicateEntry {
    pub workload: WorkloadDescription,
    pub sketch: String,
    pub sketch_config: serde_json::Value,
}

impl std::fmt::Display for DuplicateEntry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "duplicate atomic-cost entry for sketch={:?} sketch_config={} in workload {}",
            self.sketch,
            self.sketch_config,
            serde_json::to_string(&self.workload).unwrap_or_else(|_| "<unserializable>".into()),
        )
    }
}

impl std::error::Error for DuplicateEntry {}

/// Reduce every record that has what it takes, skipping (and reporting) the
/// rest, then group the survivors into one [`AtomicCostProfile`] per exact
/// [`WorkloadDescription`] — grouped by its canonical JSON encoding (stable
/// since `serde_json::Map` here is a `BTreeMap`, not insertion-order), so two
/// records only share a profile when their workload is identical field for
/// field. External windows are never merged: `window_start_ns`,
/// `window_end_ns` and `records_loaded` are part of that encoding, so two
/// windows over the same dataset stay distinct profiles.
///
/// Profiles are emitted sorted by that same canonical encoding, and entries
/// within a profile follow `records` order — both independent of any
/// hash-map iteration, so the document is byte-for-byte reproducible from the
/// same input regardless of process or platform.
///
/// A duplicate `(sketch, sketch_config)` within one profile fails the whole
/// reduction: it means the input already lost information (e.g. the same
/// grid point measured twice), and there is no default that isn't a guess
/// about which measurement to keep. Skipped records are still returned
/// alongside the error, so the caller can report both.
pub fn reduce_all(
    records: &[MergedRecord],
    scenario: ScenarioIdentity,
) -> (
    Result<AtomicCostDocument, DuplicateEntry>,
    Vec<(usize, SkipReason)>,
) {
    let mut skipped = Vec::new();
    let mut groups: BTreeMap<String, (WorkloadDescription, Vec<AtomicCostEntry>)> = BTreeMap::new();
    let mut duplicate = None;

    for (i, record) in records.iter().enumerate() {
        let entry = match reduce_one(record) {
            Ok(entry) => entry,
            Err(reason) => {
                skipped.push((i, reason));
                continue;
            }
        };

        let key = serde_json::to_string(&record.input_dataset)
            .expect("WorkloadDescription -> JSON should not fail");
        let (_, entries) = groups
            .entry(key)
            .or_insert_with(|| (record.input_dataset.clone(), Vec::new()));

        let is_duplicate = entries
            .iter()
            .any(|e| e.sketch == entry.sketch && e.sketch_config == entry.sketch_config);
        if is_duplicate {
            duplicate.get_or_insert(DuplicateEntry {
                workload: record.input_dataset.clone(),
                sketch: entry.sketch,
                sketch_config: entry.sketch_config,
            });
            continue;
        }
        entries.push(entry);
    }

    if let Some(duplicate) = duplicate {
        return (Err(duplicate), skipped);
    }

    let profiles = groups
        .into_values()
        .map(|(workload, entries)| AtomicCostProfile {
            workload,
            scenario: scenario.clone(),
            entries,
        })
        .collect();
    (
        Ok(AtomicCostDocument {
            schema_version: ATOMIC_COST_SCHEMA_VERSION,
            profiles,
        }),
        skipped,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::benchmark_result::{
        ExternalWorkload, InsertMetrics, Language, MergeMetrics, Mode, PrepareMetrics,
        QueryMetrics, Source, SCHEMA_VERSION,
    };
    use aqpbm_datagen::TableDescription;

    fn stats(mean: f64) -> RunStats {
        stats_n(mean, 5)
    }

    /// Same, but with an explicit sample count — for tests that need a
    /// specific `samples.len()` (e.g. to simulate one field's fold dropping
    /// more runs than another's).
    fn stats_n(mean: f64, n: usize) -> RunStats {
        RunStats {
            mean,
            stddev: 0.0,
            ci95: None,
            n,
            samples: vec![mean; n],
        }
    }

    fn cpu(user_mean: f64, sys_mean: f64) -> CpuTime {
        CpuTime {
            user_ms: stats(user_mean),
            sys_ms: stats(sys_mean),
        }
    }

    fn dataset() -> TableDescription {
        TableDescription::single(
            "key",
            aqpbm_datagen::ColumnSpec {
                distribution: aqpbm_datagen::DataDistribution::Uniform(
                    aqpbm_datagen::UniformParameter {
                        lower_bound: 0.0,
                        upper_bound: 100_000.0,
                        seed: 42,
                    },
                ),
                shift: None,
                cardinality: Some(100_000),
                special_rule: aqpbm_datagen::RULE_NONE,
                data_type: "i64".into(),
                string: None,
            },
            1_000_000,
        )
    }

    /// A fully-populated record: 1M items inserted in 1000ms of wall time at
    /// 500ms of CPU time (500us/item), 10k queries in 100ms wall / 40ms cpu
    /// (4us/query), 3 folds in 30ms wall / 30ms cpu (10ms/fold).
    fn full_record() -> MergedRecord {
        MergedRecord {
            schema_version: SCHEMA_VERSION,
            sketch: "cms".into(),
            library: "lib".into(),
            language: Language::Rust,
            mode: Mode::Bench,
            runs: 5,
            source: Source::Cli,
            sketch_config: Some(
                serde_json::json!({"algorithm": "cms", "params": {"rows": 3, "cols": 1024}}),
            ),
            input_dataset: dataset().into(),
            memory_bytes: Some(12_288),
            heap_bytes_net: None,
            heap_bytes_peak: None,
            insert: InsertMetrics {
                throughput_items_per_sec: Some(stats(1_000_000.0)),
                wall_time_ms: Some(stats(1000.0)),
                cpu_time_ms: Some(cpu(400.0, 100.0)),
                ..Default::default()
            },
            query: QueryMetrics {
                throughput_items_per_sec: Some(stats(100_000.0)),
                wall_time_ms: Some(stats(100.0)),
                cpu_time_ms: Some(cpu(30.0, 10.0)),
                accuracy: Some(serde_json::json!({"relative_error_mean": 0.01})),
                ..Default::default()
            },
            merge: MergeMetrics {
                merge_folds_per_sec: Some(stats(100.0)),
                wall_time_ms: Some(stats(30.0)),
                cpu_time_ms: Some(cpu(20.0, 10.0)),
                ..Default::default()
            },
            prepare: PrepareMetrics::default(),
        }
    }

    #[test]
    fn reduces_a_full_record_to_per_op_seconds() {
        let entry = reduce_one(&full_record()).expect("fully populated record");
        assert_eq!(entry.sketch, "cms");
        assert_eq!(entry.mem_bytes_per_instance, 12_288.0);

        // insert: work = 1_000_000/sec * 1.0s = 1_000_000 items;
        // cpu = 0.5s total -> 0.5us/item.
        assert!((entry.insert_cpu_secs - 0.5e-6).abs() < 1e-12);

        // query: work = 100_000/sec * 0.1s = 10_000 queries;
        // cpu = 0.04s total -> 4us/query.
        assert!((entry.query_cpu_secs - 4e-6).abs() < 1e-12);

        // merge: work = 100 folds/sec * 0.03s = 3 folds;
        // cpu = 0.03s total -> 10ms/fold.
        assert!((entry.merge_cpu_secs - 10e-3).abs() < 1e-9);

        assert_eq!(
            entry.query_accuracy,
            BTreeMap::from([("relative_error_mean".to_string(), 0.01)])
        );
    }

    #[test]
    fn missing_query_accuracy_is_skipped_not_defaulted() {
        // No comparator ran (e.g. `--metrics` didn't include `accuracy`) --
        // silently shipping an empty map would read as "perfect accuracy" to
        // anything skimming the table.
        let mut record = full_record();
        record.query.accuracy = None;
        assert_eq!(
            reduce_one(&record),
            Err(SkipReason::MissingField("query_accuracy"))
        );
    }

    #[test]
    fn non_numeric_query_accuracy_is_skipped_not_coerced() {
        // A comparator producing something other than its documented flat
        // `BTreeMap<String, f64>` is a bug worth surfacing, not silently
        // dropping the offending key.
        let mut record = full_record();
        record.query.accuracy = Some(serde_json::json!({"relative_error_mean": "not a number"}));
        assert_eq!(
            reduce_one(&record),
            Err(SkipReason::InvalidField("query_accuracy"))
        );
    }

    #[test]
    fn missing_memory_bytes_is_skipped_not_defaulted() {
        let mut record = full_record();
        record.memory_bytes = None;
        assert_eq!(
            reduce_one(&record),
            Err(SkipReason::MissingField("memory_bytes"))
        );
    }

    #[test]
    fn missing_merge_measurement_skips_the_whole_record() {
        // A grid point costed for insert/query but not merge (e.g. --operations
        // didn't include merge) must not silently ship a 0.0 merge_cpu_secs --
        // ASAPQuery would read that as "merging is free".
        let mut record = full_record();
        record.merge.cpu_time_ms = None;
        assert_eq!(
            reduce_one(&record),
            Err(SkipReason::MissingSubField("merge", "cpu_time"))
        );
    }

    #[test]
    fn zero_elapsed_time_is_skipped_not_divided() {
        let mut record = full_record();
        record.insert.wall_time_ms = Some(stats(0.0));
        assert_eq!(reduce_one(&record), Err(SkipReason::ZeroWork("insert")));
    }

    #[test]
    fn mismatched_sample_counts_are_skipped_not_mispaired() {
        // Simulates fold.rs's rate() dropping one run (e.g. a stray
        // elapsed_ns == 0) that elapsed_ms()/cpu_time_ms() kept: rate has 4
        // samples, elapsed_ms and cpu_time_ms have 5. Pairing by index
        // anyway would silently multiply/divide unrelated runs' numbers
        // together instead of the intended same-run pair.
        let mut record = full_record();
        record.insert.throughput_items_per_sec = Some(stats_n(1_000_000.0, 4));
        assert_eq!(
            reduce_one(&record),
            Err(SkipReason::MisalignedSamples("insert"))
        );
    }

    /// An external-trace variant of [`full_record`], same measurements, a
    /// different workload — for tests exercising grouping rather than
    /// reduction. `window` lets a test carve out a second, disjoint window
    /// over the same dataset without repeating every field.
    fn external_record(window: (i64, i64), records_loaded: u64) -> MergedRecord {
        let mut record = full_record();
        record.input_dataset = WorkloadDescription::External(ExternalWorkload {
            source: "boom".into(),
            dataset: "datadog_boom/data/boom_benchmark/ds-1-T".into(),
            mode: "scalar".into(),
            key_columns: Vec::new(),
            group_columns: Vec::new(),
            variate: Some(3),
            value_column: "target".into(),
            window_start_ns: window.0,
            window_end_ns: window.1,
            records_loaded,
            source_timestamp_unit: "frequency-derived".into(),
            timestamp_unit: "nanoseconds".into(),
        });
        record
    }

    fn scenario() -> ScenarioIdentity {
        ScenarioIdentity {
            payload_sha256: "a".repeat(64),
            exported_metric: "google_mean_cpu_usage_rate_0".into(),
            grouping_labels: vec!["job_id".into(), "task_index".into(), "machine_id".into()],
            source_time_range_us: [1_313_535_000_000, 1_313_715_000_000],
        }
    }

    #[test]
    fn reduce_all_partitions_ok_and_skipped_records() {
        let good = full_record();
        let mut bad = full_record();
        bad.memory_bytes = None;

        let (document, skipped) = reduce_all(&[good, bad], scenario());
        let document = document.expect("no duplicates");
        assert_eq!(document.schema_version, ATOMIC_COST_SCHEMA_VERSION);
        assert_eq!(document.profiles.len(), 1);
        assert_eq!(document.profiles[0].entries.len(), 1);
        assert_eq!(skipped, vec![(1, SkipReason::MissingField("memory_bytes"))]);
    }

    #[test]
    fn synthetic_workload_round_trips_through_the_document() {
        let (document, skipped) = reduce_all(&[full_record()], scenario());
        let document = document.expect("no duplicates");
        assert!(skipped.is_empty());

        let json = serde_json::to_string(&document).unwrap();
        let back: AtomicCostDocument = serde_json::from_str(&json).unwrap();
        assert_eq!(back, document);
        assert_eq!(back.profiles[0].workload, dataset().into());
    }

    #[test]
    fn external_workload_round_trips_with_window_and_provenance() {
        let record = external_record((10, 20), 4);
        let (document, skipped) = reduce_all(&[record], scenario());
        let document = document.expect("no duplicates");
        assert!(skipped.is_empty());

        let json = serde_json::to_string(&document).unwrap();
        let back: AtomicCostDocument = serde_json::from_str(&json).unwrap();
        assert_eq!(back, document);

        let WorkloadDescription::External(workload) = &back.profiles[0].workload else {
            panic!("expected external workload");
        };
        assert_eq!(workload.window_start_ns, 10);
        assert_eq!(workload.window_end_ns, 20);
        assert_eq!(workload.records_loaded, 4);
        assert_eq!(workload.variate, Some(3));
    }

    #[test]
    fn records_from_different_workloads_become_different_profiles() {
        let synthetic = full_record();
        let external = external_record((10, 20), 4);

        let (document, skipped) = reduce_all(&[synthetic, external], scenario());
        let document = document.expect("no duplicates");
        assert!(skipped.is_empty());
        assert_eq!(document.profiles.len(), 2);
    }

    #[test]
    fn records_from_different_external_windows_become_different_profiles() {
        // Same dataset, disjoint windows: window_start_ns/window_end_ns/
        // records_loaded are provenance, but different provenance can mean
        // different measured behaviour, so each window keeps its own profile
        // rather than collapsing into the dataset it was drawn from.
        let window_a = external_record((0, 10), 4);
        let window_b = external_record((10, 20), 4);

        let (document, skipped) = reduce_all(&[window_a, window_b], scenario());
        let document = document.expect("no duplicates");
        assert!(skipped.is_empty());
        assert_eq!(document.profiles.len(), 2);
    }

    #[test]
    fn duplicate_sketch_and_config_within_one_profile_fails_loudly() {
        let first = full_record();
        let second = full_record();

        let (document, _skipped) = reduce_all(&[first, second], scenario());
        let err = document.expect_err("duplicate must be rejected, not silently merged");
        assert_eq!(err.sketch, "cms");
        assert_eq!(err.workload, dataset().into());
    }

    #[test]
    fn same_sketch_and_config_in_different_profiles_is_allowed() {
        let synthetic = full_record();
        let external = external_record((10, 20), 4);
        assert_eq!(synthetic.sketch, external.sketch);
        assert_eq!(synthetic.sketch_config, external.sketch_config);

        let (document, skipped) = reduce_all(&[synthetic, external], scenario());
        let document = document.expect("same (sketch, sketch_config) across profiles is fine");
        assert!(skipped.is_empty());
        assert_eq!(document.profiles.len(), 2);
        assert_eq!(document.profiles[0].entries.len(), 1);
        assert_eq!(document.profiles[1].entries.len(), 1);
    }

    #[test]
    fn atomic_cost_entry_serialises_to_the_documented_shape() {
        // Pinned so a field rename here can't silently break ASAPQuery's
        // loader, which deserializes this exact shape.
        let entry = AtomicCostEntry {
            sketch: "cms".into(),
            sketch_config: serde_json::json!({"algorithm": "cms", "params": {"rows": 3, "cols": 1024}}),
            mem_bytes_per_instance: 12288.0,
            insert_cpu_secs: 5e-7,
            merge_cpu_secs: 1e-2,
            query_cpu_secs: 4e-6,
            query_accuracy: BTreeMap::from([("relative_error_mean".to_string(), 0.01)]),
        };
        let json = serde_json::to_string(&entry).unwrap();
        assert_eq!(
            json,
            r#"{"sketch":"cms","sketch_config":{"algorithm":"cms","params":{"cols":1024,"rows":3}},"mem_bytes_per_instance":12288.0,"insert_cpu_secs":5e-7,"merge_cpu_secs":0.01,"query_cpu_secs":4e-6,"query_accuracy":{"relative_error_mean":0.01}}"#
        );
    }

    #[test]
    fn atomic_cost_document_serialises_to_the_documented_shape() {
        // Pinned end-to-end (document -> profile -> entry) so ASAPQuery's
        // loader shape is exercised as a whole, not just its innermost row.
        let (document, skipped) = reduce_all(&[full_record()], scenario());
        let document = document.expect("no duplicates");
        assert!(skipped.is_empty());

        let json = serde_json::to_string(&document).unwrap();
        assert_eq!(
            json,
            r#"{"schema_version":2,"profiles":[{"workload":{"synthetic":{"description":{"column_num":1,"column_label":["key"],"column_spec":[{"distribution":{"kind":"uniform","lower_bound":0.0,"upper_bound":100000.0,"seed":42},"cardinality":100000,"special_rule":0,"data_type":"i64"}],"row_num":1000000}}},"scenario":{"payload_sha256":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","exported_metric":"google_mean_cpu_usage_rate_0","grouping_labels":["job_id","task_index","machine_id"],"source_time_range_us":[1313535000000,1313715000000]},"entries":[{"sketch":"cms","sketch_config":{"algorithm":"cms","params":{"cols":1024,"rows":3}},"mem_bytes_per_instance":12288.0,"insert_cpu_secs":5e-7,"merge_cpu_secs":0.01,"query_cpu_secs":4e-6,"query_accuracy":{"relative_error_mean":0.01}}]}]}"#
        );
    }

    #[test]
    fn unknown_document_level_field_is_rejected() {
        let json = r#"{"schema_version":1,"profiles":[],"extra":true}"#;
        assert!(serde_json::from_str::<AtomicCostDocument>(json).is_err());
    }

    #[test]
    fn unknown_profile_level_field_is_rejected() {
        let json = serde_json::json!({
            "workload": {"synthetic": {"description": dataset()}},
            "entries": [],
            "extra": true,
        })
        .to_string();
        assert!(serde_json::from_str::<AtomicCostProfile>(&json).is_err());
    }

    #[test]
    fn unknown_entry_level_field_is_rejected() {
        let json = serde_json::json!({
            "sketch": "cms",
            "sketch_config": serde_json::Value::Null,
            "mem_bytes_per_instance": 1.0,
            "insert_cpu_secs": 1.0,
            "merge_cpu_secs": 1.0,
            "query_cpu_secs": 1.0,
            "query_accuracy": {},
            "extra": true,
        })
        .to_string();
        assert!(serde_json::from_str::<AtomicCostEntry>(&json).is_err());
    }

    #[test]
    fn entry_missing_query_accuracy_is_rejected() {
        // No default and no Option: an entry that never measured accuracy
        // must not deserialize into one that looks like it scored perfectly.
        let json = serde_json::json!({
            "sketch": "cms",
            "sketch_config": serde_json::Value::Null,
            "mem_bytes_per_instance": 1.0,
            "insert_cpu_secs": 1.0,
            "merge_cpu_secs": 1.0,
            "query_cpu_secs": 1.0,
        })
        .to_string();
        assert!(serde_json::from_str::<AtomicCostEntry>(&json).is_err());
    }
}
