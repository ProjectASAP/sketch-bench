//! The JSONL report schema — the single serialised record shape
//! shared by offline benchmarks, runtime samplers, and the
//! visualisation layer. Current version: [`SCHEMA_VERSION`].

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use aqpbm_datagen::{BurstSpec, TableDescription};

/// Bumped whenever a breaking field change lands. Readers
/// should refuse to process records with a mismatched version.
pub const SCHEMA_VERSION: u32 = 5;

/// The provenance and selection semantics of the input records.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum WorkloadDescription {
    /// A table generated from the existing datagen description.
    Synthetic {
        description: TableDescription,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        burst: Option<BurstSpec>,
    },
    /// A bounded selection from an external trace.
    External(ExternalWorkload),
}

impl From<TableDescription> for WorkloadDescription {
    fn from(description: TableDescription) -> Self {
        Self::Synthetic {
            description,
            burst: None,
        }
    }
}

/// Metadata written with an external workload. Paths are logical dataset
/// identities, never machine-specific absolute paths.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExternalWorkload {
    pub source: String,
    pub dataset: String,
    pub mode: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub key_columns: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub group_columns: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub variate: Option<usize>,
    pub value_column: String,
    pub window_start_ns: i64,
    pub window_end_ns: i64,
    pub records_loaded: u64,
    pub source_timestamp_unit: String,
    pub timestamp_unit: String,
}

/// A single record in the JSONL report stream. One record
/// per benchmark / runtime window.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Record {
    pub schema_version: u32,
    /// The algorithm this run measured, structural variant included
    /// (`cms-fastpath-vector2d`, not `cms`).
    pub sketch: String,
    /// The algorithm [`Self::sketch`] belongs to. Rows sharing it answer the same
    /// question from the same knobs, so this is what a cross-library comparison
    /// groups by; `sketch` is what a variant comparison groups by. Additive and
    /// optional, so records written before it still read.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub algorithm: Option<String>,
    #[serde(rename = "impl")]
    pub library: String,
    /// Implementation language, so readers can split Rust from C++ records when
    /// both tracks dump into one JSONL stream. Defaults to `rust`.
    #[serde(default)]
    pub language: Language,
    /// Algorithm-specific construction params used for this run, populated by
    /// `bench` from the run's construction parameters.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sketch_config: Option<serde_json::Value>,
    /// The data this run was measured over. The wire name stays `workload` while
    /// the Rust name does not: it is the group key `scripts/merge_passes.py`
    /// pools by, so renaming the field would be a schema break for a word.
    #[serde(rename = "workload")]
    pub input_dataset: WorkloadDescription,
    pub mode: Mode,
    pub runs: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bench: Option<BenchSection>,
    pub source: Source,
    pub timestamp: DateTime<Utc>,
}

/// Implementation language of the run that produced this record.
/// Used by readers to split apples-to-apples comparisons.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum Language {
    #[default]
    Rust,
    Cpp,
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    Bench,
    Runtime,
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum Source {
    /// Run produced by the `approxbench` CLI.
    Cli,
    /// Run produced by an embedded sampler in one of the downstream apps.
    /// No producer in this workspace emits these; they are read, not written.
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
    /// `"throughput"`, `"latency"` or `"accuracy"`
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metric: Option<String>,
    /// `"insert"`, `"query"`, `"merge"`, `"prepare"`
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub operation: Option<String>,
    /// `items / wall_time_ms`, with `prepare` excluded
    #[serde(skip_serializing_if = "Option::is_none")]
    pub throughput_items_per_sec: Option<RunStats>,
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
    /// Folds per second. A fold is `merge_shards - 1` merge calls, so the unit
    /// is folds: merge consumes sketches, not a stream, and items per second
    /// would have no denominator here. Scales with sketch *state*, not stream
    /// length, so compare against `memory_bytes`, not insert throughput.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub merge_folds_per_sec: Option<RunStats>,
    /// How many shards were folded. Present whenever merge was measured, even
    /// if the implementation turned out not to support merging.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub merge_shards: Option<usize>,
    /// Present, and `true`, whenever a merge was measured. Never `false`: a row
    /// that provides no merge is refused where the closures are built, before anything
    /// is timed, so the gap shows up as an error naming the row rather than as a
    /// record carrying a `false`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub merge_supported: Option<bool>,
}

/// Aggregate across N samples: mean / stddev / optional 95% CI, plus the raw
/// samples the aggregate was computed from.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunStats {
    pub mean: f64,
    pub stddev: f64,
    /// 95% CI on the mean — **present only when the samples are statistically
    /// independent**, i.e. `--repeat-experiment R` (R > 1) over R processes.
    /// Iterations of
    /// one `--runs N` share too much for an interval over them to mean anything.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ci95: Option<[f64; 2]>,
    /// Number of samples behind `mean`. Iterations within one process when
    /// `ci95` is absent; independent processes when it is present.
    pub n: usize,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub samples: Vec<f64>,
}

/// CPU time split into user vs sys.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CpuTime {
    pub user_ms: RunStats,
    pub sys_ms: RunStats,
}

/// Latency percentile summary extracted from an
/// `hdrhistogram::Histogram`.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct LatencySummary {
    pub p50: u64,
    pub p95: u64,
    pub p99: u64,
    pub p999: u64,
    pub max: u64,
    pub count: u64,
}

/// Flattened form of the [`Record`]s that share one (sketch, impl,
/// sketch_config, dataset) identity: one row per invocation, where the record
/// stream writes one per measurement. Built by `aqpbm-cli`'s `flatten_record`.
/// Lives beside [`Record`] because it is a JSONL wire shape, not CLI logic.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MergedRecord {
    pub schema_version: u32,
    pub sketch: String,
    #[serde(rename = "impl")]
    pub library: String,
    pub language: Language,
    pub mode: Mode,
    pub runs: usize,
    pub source: Source,

    pub sketch_config: Option<serde_json::Value>,
    /// The data this run was measured over. The wire name stays `workload` while
    /// the Rust name does not: it is the group key `scripts/merge_passes.py`
    /// pools by, so renaming the field would be a schema break for a word.
    #[serde(rename = "workload")]
    pub input_dataset: WorkloadDescription,

    pub memory_bytes: Option<u64>,
    /// Net bytes the tracking allocator attributes to this sketch's lifetime —
    /// **measured**, where [`Self::memory_bytes`] is a self-reported formula.
    /// Its own field, so a gap between the two stays visible. Needs `heap-track`.
    pub heap_bytes_net: Option<u64>,
    /// High-water mark of the same counter across construction and insert.
    pub heap_bytes_peak: Option<u64>,

    // One slot per operation, and within a slot one field per metric. A
    // measurement is named by both, so a flattened row that named only one of
    // them had two measurements landing in the same place.
    #[serde(flatten)]
    pub insert: InsertMetrics,
    #[serde(flatten)]
    pub query: QueryMetrics,
    #[serde(flatten)]
    pub merge: MergeMetrics,
    #[serde(flatten)]
    pub prepare: PrepareMetrics,
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct InsertMetrics {
    #[serde(rename = "insert_timestamp")]
    pub timestamp: Option<DateTime<Utc>>,
    #[serde(rename = "insert_throughput_items_per_sec")]
    pub throughput_items_per_sec: Option<RunStats>,
    #[serde(rename = "insert_latency_ns")]
    pub latency_ns: Option<LatencySummary>,
    #[serde(rename = "insert_cpu_time_ms")]
    pub cpu_time_ms: Option<CpuTime>,
    #[serde(rename = "insert_wall_time_ms")]
    pub wall_time_ms: Option<RunStats>,
    #[serde(rename = "insert_rss_peak_kb")]
    pub rss_peak_kb: Option<u64>,
    #[serde(rename = "insert_heap_allocated_kb")]
    pub heap_allocated_kb: Option<u64>,
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct QueryMetrics {
    #[serde(rename = "query_timestamp")]
    pub timestamp: Option<DateTime<Utc>>,
    #[serde(rename = "query_throughput_items_per_sec")]
    pub throughput_items_per_sec: Option<RunStats>,
    #[serde(rename = "query_latency_ns")]
    pub latency_ns: Option<LatencySummary>,
    /// The comparator's scores. Only this operation has them: accuracy is
    /// what an answer can be scored for, and query is what produces one.
    #[serde(rename = "query_accuracy")]
    pub accuracy: Option<serde_json::Value>,
    #[serde(rename = "query_cpu_time_ms")]
    pub cpu_time_ms: Option<CpuTime>,
    #[serde(rename = "query_wall_time_ms")]
    pub wall_time_ms: Option<RunStats>,
    #[serde(rename = "query_rss_peak_kb")]
    pub rss_peak_kb: Option<u64>,
    #[serde(rename = "query_heap_allocated_kb")]
    pub heap_allocated_kb: Option<u64>,
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MergeMetrics {
    #[serde(rename = "merge_timestamp")]
    pub timestamp: Option<DateTime<Utc>>,
    /// How many folds a second: the throughput reading of the same clock.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub merge_folds_per_sec: Option<RunStats>,
    pub merge_shards: Option<usize>,
    pub merge_supported: Option<bool>,
    #[serde(rename = "merge_cpu_time_ms")]
    pub cpu_time_ms: Option<CpuTime>,
    #[serde(rename = "merge_wall_time_ms")]
    pub wall_time_ms: Option<RunStats>,
    #[serde(rename = "merge_rss_peak_kb")]
    pub rss_peak_kb: Option<u64>,
    #[serde(rename = "merge_heap_allocated_kb")]
    pub heap_allocated_kb: Option<u64>,
}

/// The deferred build. Only a latency: a build happens once per sketch, so
/// there is no rate to state and nothing it answers to be scored against.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PrepareMetrics {
    #[serde(rename = "prepare_timestamp")]
    pub timestamp: Option<DateTime<Utc>>,
    #[serde(rename = "prepare_cpu_time_ms")]
    pub cpu_time_ms: Option<CpuTime>,
    #[serde(rename = "prepare_wall_time_ms")]
    pub wall_time_ms: Option<RunStats>,
    #[serde(rename = "prepare_rss_peak_kb")]
    pub rss_peak_kb: Option<u64>,
    #[serde(rename = "prepare_heap_allocated_kb")]
    pub heap_allocated_kb: Option<u64>,
}

impl Record {
    pub fn new(
        sketch: impl Into<String>,
        library: impl Into<String>,
        input_dataset: impl Into<WorkloadDescription>,
        mode: Mode,
        runs: usize,
    ) -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            sketch: sketch.into(),
            algorithm: None,
            library: library.into(),
            language: Language::Rust,
            sketch_config: None,
            input_dataset: input_dataset.into(),
            mode,
            runs,
            bench: None,
            source: Source::Cli,
            timestamp: Utc::now(),
        }
    }

    pub fn to_jsonl(&self) -> String {
        serde_json::to_string(self).expect("Record -> JSON should not fail")
    }
}

/// The JSON Schema for [`MergedRecord`] — the `--flat` JSONL row — as the
/// pretty-printed text that belongs in `schema/merged_record.schema.json`.
///
/// Exists so out-of-tree readers of this stream can validate against a schema
/// *derived from these structs* instead of a hand-copied field list. The
/// leaderboard's `benchmarkRecord.js` carried such a copy — 150 lines of field
/// names, optionality and types — and it silently fell behind.
///
/// `schema_export_is_current` keeps the checked-in file honest.
#[cfg(feature = "schema")]
pub fn merged_record_schema_json() -> String {
    let mut schema = schemars::schema_for!(MergedRecord);

    // Widen `required` to every field serde actually writes.
    //
    // schemars derives `required` from the Rust type, so an `Option<T>` is
    // optional — 8 of 44 fields here. But optionality in `MergedRecord` is
    // about the *value*, not the key: almost nothing carries
    // `skip_serializing_if`, so serde emits every field on every line, null
    // included. The key set is therefore an exact contract, and "is it
    // intact?" is the cheapest corruption check a reader has — the one thing
    // the leaderboard's hand-written list got right and a stock schema throws
    // away.
    //
    // Which keys those are is asked of serde rather than listed here: an
    // all-`None` record is serialised and its keys observed. A field that
    // gains or loses `skip_serializing_if` moves in this set on its own, so
    // there is no second list to keep in step with the first.
    let probe = MergedRecord {
        schema_version: SCHEMA_VERSION,
        sketch: String::new(),
        library: String::new(),
        language: Language::default(),
        mode: Mode::Bench,
        runs: 0,
        source: Source::Cli,
        sketch_config: None,
        // Only this record's own key set is read below, so the dataset just
        // has to be constructible — its shape is described by schemars from
        // `TableDescription` itself.
        input_dataset: WorkloadDescription::Synthetic {
            description: TableDescription {
                column_num: 0,
                column_label: Vec::new(),
                column_spec: Vec::new(),
                column_connected: Vec::new(),
                row_num: 0,
            },
            burst: None,
        },
        memory_bytes: None,
        heap_bytes_net: None,
        heap_bytes_peak: None,
        insert: InsertMetrics::default(),
        query: QueryMetrics::default(),
        merge: MergeMetrics::default(),
        prepare: PrepareMetrics::default(),
    };
    let serde_json::Value::Object(always_written) =
        serde_json::to_value(&probe).expect("MergedRecord -> JSON should not fail")
    else {
        unreachable!("MergedRecord serialises as an object")
    };
    let mut required: Vec<String> = always_written.keys().cloned().collect();
    required.sort();
    schema.insert("required".to_string(), serde_json::json!(required));

    // `additionalProperties` is deliberately left unset. An unknown key means
    // the reader is older than the writer, which is not corruption — the
    // schema grows additively — so the severity of that is the reader's call.
    // The leaderboard warns on it, using this document's `properties` as the
    // known set.

    // Stamped into the document so a reader can refuse a schema from a
    // different generation of the format without inspecting its fields —
    // the same gate `Record::schema_version` gives an individual row.
    schema.insert(
        "x-schema-version".to_string(),
        serde_json::json!(SCHEMA_VERSION),
    );

    let mut text = serde_json::to_string_pretty(&schema).expect("schema -> JSON should not fail");
    text.push('\n');
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A one-column description, the shape most records carry.
    fn table() -> TableDescription {
        TableDescription::single(
            "key",
            aqpbm_datagen::ColumnSpec {
                distribution: aqpbm_datagen::DataDistribution::Zipf(aqpbm_datagen::ZipfParameter {
                    skewness: 1.1,
                    population_size: 10_000,
                    seed: 42,
                }),
                shift: None,
                cardinality: None,
                special_rule: aqpbm_datagen::RULE_NONE,
                data_type: "i64".into(),
                string: None,
            },
            1_000_000,
        )
    }

    #[test]
    fn record_roundtrips_through_json() {
        let mut rec = Record::new("hll", "oxide", table(), Mode::Bench, 10);
        rec.bench = Some(BenchSection {
            metric: None,
            throughput_items_per_sec: Some(RunStats {
                mean: 4.2e7,
                stddev: 1.1e6,
                ci95: None,
                n: 10,
                samples: vec![4.1e7, 4.3e7],
            }),
            ..Default::default()
        });
        let s = rec.to_jsonl();
        let back: Record = serde_json::from_str(&s).unwrap();
        assert_eq!(back.sketch, "hll");
        assert_eq!(back.library, "oxide");
        assert_eq!(back.mode, Mode::Bench);
        assert!(back.bench.is_some());
    }

    #[test]
    fn external_workload_preserves_selected_variate() {
        let workload = WorkloadDescription::External(ExternalWorkload {
            source: "boom".into(),
            dataset: "datadog_boom/data/boom_benchmark/ds-1-T".into(),
            mode: "scalar".into(),
            key_columns: Vec::new(),
            group_columns: Vec::new(),
            variate: Some(3),
            value_column: "target".into(),
            window_start_ns: 10,
            window_end_ns: 20,
            records_loaded: 4,
            source_timestamp_unit: "frequency-derived".into(),
            timestamp_unit: "nanoseconds".into(),
        });
        let rec = Record::new("hll", "oxide", workload, Mode::Bench, 1);
        let back: Record = serde_json::from_str(&rec.to_jsonl()).unwrap();
        let WorkloadDescription::External(workload) = back.input_dataset else {
            panic!("expected external workload");
        };
        assert_eq!(workload.variate, Some(3));
    }

    /// A gate, not a behaviour test: bumping `SCHEMA_VERSION` turns this red on
    /// purpose. The number leaves the repo — in every JSONL record, and on the
    /// wire as `RuntimeRecord.schema_version` — so a bump is a contract change.
    #[test]
    fn schema_version_is_v5() {
        assert_eq!(SCHEMA_VERSION, 5);
    }

    #[test]
    fn language_defaults_to_rust_when_absent() {
        // A v2 JSONL record produced before the `language` field
        // existed must still deserialise, with language = Rust.
        let legacy = serde_json::json!({
            "schema_version": 2,
            "sketch": "hll",
            "impl": "oxide",
            "workload": {"synthetic": {"description": table()}},
            "mode": "bench",
            "runs": 1,
            "source": "cli",
            "timestamp": "2025-01-01T00:00:00Z"
        })
        .to_string();
        let rec: Record = serde_json::from_str(&legacy).unwrap();
        assert_eq!(rec.language, Language::Rust);
    }

    /// Wire-format compat: a JSONL line shaped like what the C++ emitter writes
    /// must deserialise into a `Record` with the right fields, catching
    /// field-name drift without compiling that side.
    #[test]
    fn cpp_bench_jsonl_parses() {
        let cpp_emitted = format!(
            r#"{{"schema_version":2,"sketch":"kll","impl":"datasketches","language":"cpp","workload":{},"mode":"bench","runs":10,"bench":{{"throughput_items_per_sec":{{"mean":42000000,"stddev":1100000,"ci95":[41500000,42500000],"n":10}},"latency_ns":{{"p50":17,"p95":41,"p99":60,"p999":95,"max":312,"count":1000}},"accuracy":{{"queries":[0.5,0.95,0.99],"abs_rank_err":{{"mean":0.0021,"max":0.0084}},"rel_rank_err":{{"mean":0.0043,"max":0.019}}}}}},"source":"cpp-bench","timestamp":"2026-05-13T07:14:22.123456Z"}}"#,
            serde_json::to_string(&WorkloadDescription::from(table())).unwrap()
        );
        let rec: Record = serde_json::from_str(&cpp_emitted).unwrap();
        assert_eq!(rec.sketch, "kll");
        assert_eq!(rec.library, "datasketches");
        assert_eq!(rec.language, Language::Cpp);
        assert_eq!(rec.source, Source::CppBench);
        let bench = rec.bench.as_ref().unwrap();
        let tput = bench.throughput_items_per_sec.as_ref().unwrap();
        assert_eq!(tput.n, 10);
        let lat = bench.latency_ns.unwrap();
        assert_eq!(lat.p99, 60);
        let acc = bench.accuracy.as_ref().unwrap();
        assert!(acc.get("queries").is_some());
        assert!(acc.get("abs_rank_err").is_some());
    }

    /// The mechanism that makes the exported schema trustworthy.
    ///
    /// `schema/merged_record.schema.json` is checked in and copied into the
    /// leaderboard, where it replaces a hand-written field list. A copy is only
    /// worth more than the list it replaced if it cannot quietly fall behind,
    /// so this test regenerates the schema and compares. Change `MergedRecord`
    /// without re-exporting and `cargo test` goes red, naming the command.
    ///
    /// Regenerate with:
    ///
    /// ```sh
    /// UPDATE_SCHEMA=1 cargo test -p aqpbm-core --features schema schema_export_is_current
    /// ```
    ///
    /// Deliberately *not* an `--example` binary: an exporter nobody is forced
    /// to run is exactly the failure mode this replaces.
    #[cfg(feature = "schema")]
    #[test]
    fn schema_export_is_current() {
        use std::path::PathBuf;

        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("schema")
            .join("merged_record.schema.json");
        let generated = merged_record_schema_json();

        if std::env::var_os("UPDATE_SCHEMA").is_some() {
            std::fs::create_dir_all(path.parent().unwrap()).expect("create schema dir");
            std::fs::write(&path, &generated).expect("write schema");
            eprintln!("wrote {}", path.display());
            return;
        }

        let checked_in = std::fs::read_to_string(&path).unwrap_or_else(|e| {
            panic!(
                "cannot read {}: {e}\n\
                 regenerate with: UPDATE_SCHEMA=1 cargo test -p aqpbm-core \
                 --features schema schema_export_is_current",
                path.display()
            )
        });

        // Compared as parsed JSON, not as text: whitespace and key order are
        // serde_json's business, and a reformat is not a schema change.
        let a: serde_json::Value = serde_json::from_str(&checked_in).expect("checked-in schema");
        let b: serde_json::Value = serde_json::from_str(&generated).expect("generated schema");
        assert_eq!(
            a,
            b,
            "{} is stale — MergedRecord changed without re-exporting.\n\
             regenerate with: UPDATE_SCHEMA=1 cargo test -p aqpbm-core \
             --features schema schema_export_is_current\n\
             then copy the file into the leaderboard's src/utils/.",
            path.display()
        );
    }

    #[test]
    fn cpp_record_roundtrips() {
        let mut rec = Record::new("kll", "datasketches", table(), Mode::Bench, 10);
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
