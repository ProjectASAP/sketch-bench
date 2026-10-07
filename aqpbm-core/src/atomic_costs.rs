//! Reduce [`MergedRecord`]s down to the flat per-op cost table ASAPQuery's
//! optimizer loads (see ASAPQuery#524, sketch-bench#30). Lives in `aqpbm-core`
//! rather than `aqpbm-cli` because it is a JSONL-derived wire shape, same
//! reasoning as `MergedRecord` itself.
//!
//! No `subtract_cpu_secs` field: `asap_sketchlib` has no `subtract` yet
//! (asap_sketchlib#69), so there is nothing to measure. ASAPQuery's loader
//! treats a missing entry as "drop the candidate", which covers this too.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use aqpbm_datagen::{DataDistribution, TableDescription, RULE_MONOTONIC_INCREASE};

use crate::accuracy::aggregate::GROUPS_PER_INSTANCE;
use crate::benchmark_result::{CpuTime, MergedRecord, RunStats, WorkloadDescription};

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
    /// The `query_accuracy` key that is this row's accuracy, named by whoever
    /// ran the export (the comparator doesn't say which of its scores is the
    /// one). [`reduce_one`] refuses a record whose scores lack it.
    pub accuracy_metric: String,
    /// The conditions the row was measured under.
    pub measured_at: MeasuredAt,
}

impl AtomicCostEntry {
    /// `query_accuracy[accuracy_metric]`; present in every reduced row.
    pub fn accuracy(&self) -> Option<f64> {
        self.query_accuracy.get(&self.accuracy_metric).copied()
    }
}

/// What one benchmark instance saw. A consumer applying the row to an instance
/// of a different size, key count or value range can tell how far it is
/// extrapolating (sketch-bench#147). Read off the record's workload, so a
/// field the workload doesn't determine is `None`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MeasuredAt {
    /// Items (rows) inserted into the instance.
    pub items_per_instance: u64,
    /// The measured group count for a grouped accumulator; otherwise the
    /// product of the key columns' populations, an upper bound on the distinct
    /// keys actually drawn.
    pub keys_per_instance: Option<u64>,
    /// `[lower, upper]` of the value column's distribution, plus its shift.
    /// `None` for a monotonic (counter) column, whose values are a running sum
    /// of draws rather than the draws themselves.
    pub value_range: Option<[f64; 2]>,
    /// Items in each sketch the merge benchmark folded in:
    /// `items_per_instance / merge_shards`.
    pub merge_operand_items: Option<u64>,
    /// The value column's distribution as generated, e.g. Zipf with its skew.
    pub distribution: Option<DataDistribution>,
}

impl MeasuredAt {
    /// The data shape the row was measured at, keyed the way the saturation
    /// study keys its grid points, or `None` for a distribution the study
    /// doesn't sweep. A single-column workload's Zipf population is its `K`.
    pub fn data_shape(&self) -> Option<MeasuredShape> {
        match self.distribution.as_ref()? {
            DataDistribution::Zipf(p) => Some(MeasuredShape::Zipf {
                skew: p.skewness,
                keys: p.population_size as f64,
            }),
            DataDistribution::Pareto(p) => Some(MeasuredShape::Pareto {
                tail_index: p.alpha,
            }),
            DataDistribution::Uniform(_) | DataDistribution::Normal(_) => None,
        }
    }
}

/// The data parameters a sketch's accuracy was measured at. A cost-table
/// row ([`MeasuredAt::data_shape`]) and a saturation-study grid point share
/// this key, so a row is one point on that grid point's curve.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum MeasuredShape {
    /// Keys drawn Zipf(`skew`) over `keys` distinct keys (frequency, top-k,
    /// cardinality).
    Zipf { skew: f64, keys: f64 },
    /// Values drawn Pareto with tail index `tail_index` (quantiles).
    Pareto { tail_index: f64 },
}

pub type AtomicCostTable = Vec<AtomicCostEntry>;

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
    /// The comparator's scores lack the row's `accuracy_metric`, e.g. DD
    /// omits its error when every true quantile is 0.
    MissingMetric(String),
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
            SkipReason::MissingMetric(metric) => write!(f, "query_accuracy has no {metric}"),
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
/// worth logging even though it isn't fatal. `accuracy_metric` names the
/// row's accuracy among the comparator's scores.
pub fn reduce_one(
    record: &MergedRecord,
    accuracy_metric: &str,
) -> Result<AtomicCostEntry, SkipReason> {
    let entry = reduce_scores(record)?;
    if !entry.query_accuracy.contains_key(accuracy_metric) {
        return Err(SkipReason::MissingMetric(accuracy_metric.to_string()));
    }
    Ok(AtomicCostEntry {
        accuracy_metric: accuracy_metric.to_string(),
        ..entry
    })
}

/// [`reduce_one`] without naming the row's metric (`accuracy_metric` is
/// empty), for a consumer that keeps every score (ERP).
pub(crate) fn reduce_scores(record: &MergedRecord) -> Result<AtomicCostEntry, SkipReason> {
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

    let query_accuracy = accuracy_map("query_accuracy", record.query.accuracy.as_ref())?;

    // A multi-subpopulation accumulator holds every group in one instance, so
    // its footprint and fold were measured at the bench spec's group count.
    // Priced per group here, so the optimizer's `card(group-by) ×` counts each
    // group once. Query is already per group (one probe per group); insert is
    // per item.
    let groups_measured = query_accuracy.get(GROUPS_PER_INSTANCE).copied();
    let groups = groups_measured.unwrap_or(1.0).max(1.0);
    let mem_bytes_per_instance = mem_bytes_per_instance / groups;
    let merge_cpu_secs = merge_cpu_secs / groups;

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
        measured_at: measured_at(record, groups_measured),
        query_accuracy,
        accuracy_metric: String::new(),
    })
}

/// The record's measurement conditions. In a table description the last
/// column holds the values and the ones before it are the keys; a
/// single-column table's one column is both.
fn measured_at(record: &MergedRecord, groups_measured: Option<f64>) -> MeasuredAt {
    let (items, columns) = match &record.input_dataset {
        WorkloadDescription::Synthetic { description, .. } => {
            (description.row_num, Some(description))
        }
        WorkloadDescription::External(external) => (external.records_loaded, None),
    };
    let value = columns.and_then(|d: &TableDescription| d.column_spec.last());
    let keys = columns.and_then(|d| match d.column_spec.split_last() {
        Some((value, [])) => value.distribution.domain().map(|d| d.size),
        // Saturating: wide label domains multiply past u64; the bound stays a bound.
        Some((_, labels)) => labels.iter().try_fold(1u64, |keys, c| {
            Some(keys.saturating_mul(c.distribution.domain()?.size))
        }),
        None => None,
    });
    MeasuredAt {
        items_per_instance: items,
        keys_per_instance: groups_measured.map(|g| g as u64).or(keys),
        value_range: value
            .filter(|c| c.special_rule & RULE_MONOTONIC_INCREASE == 0)
            .and_then(|c| {
                let shift = c.shift.unwrap_or(0.0);
                match &c.distribution {
                    DataDistribution::Zipf(p) => Some([1.0, p.population_size as f64]),
                    DataDistribution::Uniform(p) => Some([p.lower_bound, p.upper_bound]),
                    DataDistribution::Normal(_) | DataDistribution::Pareto(_) => None,
                }
                .map(|[lo, hi]| [lo + shift, hi + shift])
            }),
        merge_operand_items: record
            .merge
            .merge_shards
            .filter(|&m| m > 0)
            .map(|m| items / m as u64),
        distribution: value.map(|c| c.distribution.clone()),
    }
}

/// `MergedRecord::query.accuracy` down to the flat numeric map every
/// comparator's `GroundTruth::score` actually produces. Required like the
/// cost fields (missing skips the row, `SkipReason` says which): an entry
/// with no comparator never runs a query at all (registry's `comparator:
/// None` entries are all insert-only), so it is already excluded here by
/// `query_cpu_secs` above — requiring accuracy too costs nothing further and
/// keeps every kept row equally complete.
fn accuracy_map(
    field: &'static str,
    accuracy: Option<&serde_json::Value>,
) -> Result<BTreeMap<String, f64>, SkipReason> {
    let accuracy = accuracy.ok_or(SkipReason::MissingField(field))?;
    let object = accuracy
        .as_object()
        .ok_or(SkipReason::InvalidField(field))?;
    object
        .iter()
        .map(|(k, v)| v.as_f64().map(|v| (k.clone(), v)))
        .collect::<Option<_>>()
        .ok_or(SkipReason::InvalidField(field))
}

/// Reduce every record that has what it takes, skipping (and reporting) the
/// rest. Order follows `records`. `accuracy_metrics` maps each variant to its
/// accuracy key; a variant missing from it is skipped.
pub fn reduce_all(
    records: &[MergedRecord],
    accuracy_metrics: &BTreeMap<String, String>,
) -> (AtomicCostTable, Vec<(usize, SkipReason)>) {
    let mut table = Vec::with_capacity(records.len());
    let mut skipped = Vec::new();
    for (i, record) in records.iter().enumerate() {
        let entry = accuracy_metrics
            .get(&record.sketch)
            .ok_or(SkipReason::MissingField("accuracy_metric"))
            .and_then(|metric| reduce_one(record, metric));
        match entry {
            Ok(entry) => table.push(entry),
            Err(reason) => skipped.push((i, reason)),
        }
    }
    (table, skipped)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::benchmark_result::{
        InsertMetrics, Language, MergeMetrics, Mode, PrepareMetrics, QueryMetrics, Source,
        SCHEMA_VERSION,
    };
    use aqpbm_datagen::TableDescription;

    const METRIC: &str = "relative_error_mean";

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

    /// A grouped accumulator's memory and fold are priced per group; insert
    /// and query are not touched.
    #[test]
    fn grouped_records_are_priced_per_group() {
        let mut record = full_record();
        record.query.accuracy =
            Some(serde_json::json!({"relative_error_mean": 0.0, "groups_per_instance": 4.0}));
        let entry = reduce_one(&record, METRIC).expect("fully populated record");
        assert_eq!(entry.mem_bytes_per_instance, 12_288.0 / 4.0);
        assert!((entry.merge_cpu_secs - 10e-3 / 4.0).abs() < 1e-12);
        assert!((entry.insert_cpu_secs - 0.5e-6).abs() < 1e-12);
        assert!((entry.query_cpu_secs - 4e-6).abs() < 1e-12);
    }

    /// A one-column table: its column gives the keys and the values, and the
    /// merge operand is the stream split `merge_shards` ways.
    #[test]
    fn measured_at_reads_a_single_column_workload() {
        let mut record = full_record();
        record.merge.merge_shards = Some(16);
        let entry = reduce_one(&record, METRIC).expect("fully populated record");
        let at = entry.measured_at;
        assert_eq!(at.items_per_instance, 1_000_000);
        assert_eq!(at.keys_per_instance, Some(100_000));
        assert_eq!(at.value_range, Some([0.0, 100_000.0]));
        assert_eq!(at.merge_operand_items, Some(62_500));
        assert_eq!(
            at.distribution,
            Some(dataset().column_spec[0].distribution.clone())
        );
    }

    /// A multi-column table: the last column is the value, the rest are keys.
    /// A measured group count wins over the key populations' bound.
    #[test]
    fn measured_at_reads_a_labelled_workload() {
        let zipf = |population_size| aqpbm_datagen::ColumnSpec {
            distribution: aqpbm_datagen::DataDistribution::Zipf(aqpbm_datagen::ZipfParameter {
                skewness: 1.1,
                population_size,
                seed: 1,
            }),
            ..dataset().column_spec[0].clone()
        };
        let mut description = dataset();
        description.column_num = 3;
        description.column_label = vec!["a".into(), "b".into(), "value".into()];
        description.column_spec = vec![zipf(50), zipf(1000), dataset().column_spec[0].clone()];
        description.column_spec[2].shift = Some(10.0);
        let mut record = full_record();
        record.input_dataset = description.into();

        let at = reduce_one(&record, METRIC).unwrap().measured_at;
        assert_eq!(at.keys_per_instance, Some(50_000));
        assert_eq!(at.value_range, Some([10.0, 100_010.0]));
        assert_eq!(at.merge_operand_items, None);

        record.query.accuracy =
            Some(serde_json::json!({"relative_error_mean": 0.0, "groups_per_instance": 9_876.0}));
        let at = reduce_one(&record, METRIC).unwrap().measured_at;
        assert_eq!(at.keys_per_instance, Some(9_876));
    }

    /// A counter column emits a running sum of its draws, so the draws'
    /// bounds say nothing about the values.
    #[test]
    fn measured_at_has_no_value_range_for_a_counter_column() {
        let mut description = dataset();
        description.column_spec[0].special_rule = aqpbm_datagen::RULE_MONOTONIC_INCREASE;
        let mut record = full_record();
        record.input_dataset = description.into();
        let at = reduce_one(&record, METRIC).unwrap().measured_at;
        assert_eq!(at.value_range, None);
        assert_eq!(at.items_per_instance, 1_000_000);
    }

    /// A row without its metric is an error: tables are regenerated, not
    /// migrated.
    #[test]
    fn a_row_without_accuracy_metric_is_rejected() {
        let json = r#"{"sketch":"cms","sketch_config":null,"mem_bytes_per_instance":1.0,"insert_cpu_secs":1.0,"merge_cpu_secs":1.0,"query_cpu_secs":1.0,"query_accuracy":{}}"#;
        let err = serde_json::from_str::<AtomicCostEntry>(json).unwrap_err();
        assert!(err.to_string().contains("accuracy_metric"), "{err}");
    }

    #[test]
    fn a_record_without_its_metric_is_skipped() {
        assert_eq!(
            reduce_one(&full_record(), "mean_rank_err"),
            Err(SkipReason::MissingMetric("mean_rank_err".into()))
        );
    }

    /// A misspelled or newer field is an error, not silently dropped data.
    #[test]
    fn a_row_with_an_unknown_field_is_rejected() {
        let json = r#"{"sketch":"cms","sketch_config":null,"mem_bytes_per_instance":1.0,"insert_cpu_secs":1.0,"merge_cpu_secs":1.0,"query_cpu_secs":1.0,"query_accuracy":{},"accuracy_metric":"x","merge_acuracy":{}}"#;
        let err = serde_json::from_str::<AtomicCostEntry>(json).unwrap_err();
        assert!(err.to_string().contains("merge_acuracy"), "{err}");
    }

    #[test]
    fn measured_at_with_an_unknown_field_is_rejected() {
        let json = r#"{"sketch":"cms","sketch_config":null,"mem_bytes_per_instance":1.0,"insert_cpu_secs":1.0,"merge_cpu_secs":1.0,"query_cpu_secs":1.0,"query_accuracy":{},"accuracy_metric":"x","measured_at":{"items_per_instance":1,"keys_per_instance":null,"value_rnage":null,"merge_operand_items":null,"distribution":null}}"#;
        let err = serde_json::from_str::<AtomicCostEntry>(json).unwrap_err();
        assert!(err.to_string().contains("value_rnage"), "{err}");
    }

    #[test]
    fn a_distribution_with_an_unknown_field_is_rejected() {
        let json = r#"{"sketch":"cms","sketch_config":null,"mem_bytes_per_instance":1.0,"insert_cpu_secs":1.0,"merge_cpu_secs":1.0,"query_cpu_secs":1.0,"query_accuracy":{},"accuracy_metric":"x","measured_at":{"items_per_instance":1,"keys_per_instance":null,"value_range":null,"merge_operand_items":null,"distribution":{"kind":"zipf","skewnes":1.1,"skewness":1.1,"population_size":10,"seed":1}}}"#;
        let err = serde_json::from_str::<AtomicCostEntry>(json).unwrap_err();
        assert!(err.to_string().contains("skewnes"), "{err}");
    }

    #[test]
    fn reduces_a_full_record_to_per_op_seconds() {
        let entry = reduce_one(&full_record(), METRIC).expect("fully populated record");
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
            reduce_one(&record, METRIC),
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
            reduce_one(&record, METRIC),
            Err(SkipReason::InvalidField("query_accuracy"))
        );
    }

    #[test]
    fn missing_memory_bytes_is_skipped_not_defaulted() {
        let mut record = full_record();
        record.memory_bytes = None;
        assert_eq!(
            reduce_one(&record, METRIC),
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
            reduce_one(&record, METRIC),
            Err(SkipReason::MissingSubField("merge", "cpu_time"))
        );
    }

    #[test]
    fn zero_elapsed_time_is_skipped_not_divided() {
        let mut record = full_record();
        record.insert.wall_time_ms = Some(stats(0.0));
        assert_eq!(
            reduce_one(&record, METRIC),
            Err(SkipReason::ZeroWork("insert"))
        );
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
            reduce_one(&record, METRIC),
            Err(SkipReason::MisalignedSamples("insert"))
        );
    }

    #[test]
    fn reduce_all_partitions_ok_and_skipped_records() {
        let good = full_record();
        let mut bad = full_record();
        bad.memory_bytes = None;

        let mut unnamed = full_record();
        unnamed.sketch = "hll".into();

        let metrics = BTreeMap::from([("cms".to_string(), METRIC.to_string())]);
        let (table, skipped) = reduce_all(&[good, bad, unnamed], &metrics);
        assert_eq!(table.len(), 1);
        assert_eq!(table[0].accuracy(), Some(0.01));
        assert_eq!(
            skipped,
            vec![
                (1, SkipReason::MissingField("memory_bytes")),
                (2, SkipReason::MissingField("accuracy_metric")),
            ]
        );
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
            accuracy_metric: "relative_error_mean".into(),
            measured_at: MeasuredAt {
                items_per_instance: 1_000_000,
                keys_per_instance: Some(10_000),
                value_range: None,
                merge_operand_items: None,
                distribution: None,
            },
        };
        let json = serde_json::to_string(&entry).unwrap();
        assert_eq!(
            json,
            r#"{"sketch":"cms","sketch_config":{"algorithm":"cms","params":{"cols":1024,"rows":3}},"mem_bytes_per_instance":12288.0,"insert_cpu_secs":5e-7,"merge_cpu_secs":0.01,"query_cpu_secs":4e-6,"query_accuracy":{"relative_error_mean":0.01},"accuracy_metric":"relative_error_mean","measured_at":{"items_per_instance":1000000,"keys_per_instance":10000,"value_range":null,"merge_operand_items":null,"distribution":null}}"#
        );
    }
}
