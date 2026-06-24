//! AQP query parsing/lowering.
//!
//! Phase 2 intentionally starts by reusing DataFusion's SQL parser and
//! `LogicalPlan` DAG. This crate owns only the benchmark-specific layer:
//! a registered benchmark schema plus a small lowering pass from
//! DataFusion plans into AQP tasks the current sketch benchmark can run.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::io::Read;
use std::sync::Arc;
use std::time::Instant;

use anyhow::{bail, Context as AnyhowContext, Result};
use arrow_schema::{DataType, Field, Schema, SchemaRef};
use datafusion_common::config::ConfigOptions;
use datafusion_common::{plan_err, ScalarValue, TableReference};
use datafusion_expr::expr::AggregateFunction as DfAggregateFunction;
use datafusion_expr::{
    Aggregate, AggregateUDF, Expr, LogicalPlan, LogicalTableSource, ScalarUDF, TableSource,
    WindowUDF,
};
use datafusion_sql::parser::DFParser;
use datafusion_sql::planner::{ContextProvider, SqlToRel};
use rand::Rng;
use rand_distr::{Distribution, Zipf};
use rand_xoshiro::rand_core::SeedableRng;
use rand_xoshiro::Xoshiro256PlusPlus;
use serde::{Deserialize, Serialize};

/// One query as understood by the AQP benchmark frontend.
#[derive(Debug, Clone)]
pub struct AqpQuery {
    pub sql: String,
    pub plan: LogicalPlan,
    pub plan_display: String,
    pub task: AqpTask,
}

/// First lowered task shape.
///
/// This is deliberately narrower than DataFusion's full plan language. The
/// parser accepts SQL, DataFusion builds the DAG, then we admit only the query
/// shapes that the benchmark execution layer can currently account for.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum AqpTask {
    CountDistinct {
        table: String,
        column: String,
        group_by: Vec<String>,
    },
    Quantile {
        table: String,
        column: String,
        quantile: f64,
        group_by: Vec<String>,
    },
    Frequency {
        table: String,
        column: String,
    },
}

impl AqpTask {
    pub fn runnable_by_current_bench(&self) -> bool {
        matches!(admit_task(self).status, AdmissionStatus::Approximated)
    }

    pub fn workload_intent(&self) -> WorkloadIntent {
        WorkloadIntent::from_task(self)
    }
}

const UNGROUPED: &str = "__all__";
const DEFAULT_KLL_K: i32 = 200;
const DEFAULT_FREQ_ROWS: usize = 5;
const DEFAULT_FREQ_COLS: usize = 2048;
const FREQ_HEAVY_HITTER_TOP_K: usize = 1_000;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyntheticEventsConfig {
    pub rows: usize,
    pub regions: usize,
    pub user_cardinality: u64,
    pub user_distribution: UserDistribution,
    pub region_skew: f64,
    pub seed: u64,
}

impl Default for SyntheticEventsConfig {
    fn default() -> Self {
        Self {
            rows: 1_000_000,
            regions: 16,
            user_cardinality: 100_000,
            user_distribution: UserDistribution::Uniform,
            region_skew: 0.0,
            seed: 42,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum UserDistribution {
    Uniform,
    Zipf { s: f64 },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventRow {
    pub region: String,
    pub user_id: i64,
    pub value: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyntheticEvents {
    pub config: SyntheticEventsConfig,
    pub rows: Vec<EventRow>,
}

impl SyntheticEvents {
    pub fn generate(config: SyntheticEventsConfig) -> Result<Self> {
        if config.regions == 0 {
            bail!("synthetic events need at least one region");
        }
        if config.user_cardinality == 0 {
            bail!("synthetic events need user_cardinality > 0");
        }
        if !(0.0..1.0).contains(&config.region_skew) && config.region_skew != 0.0 {
            bail!("region_skew must be in [0.0, 1.0)");
        }

        let mut rng = Xoshiro256PlusPlus::seed_from_u64(config.seed);
        let zipf = match config.user_distribution {
            UserDistribution::Uniform => None,
            UserDistribution::Zipf { s } => {
                Some(Zipf::new(config.user_cardinality, s).map_err(|e| anyhow::anyhow!("{e}"))?)
            }
        };

        let mut rows = Vec::with_capacity(config.rows);
        for _ in 0..config.rows {
            let region_idx = sample_region(config.regions, config.region_skew, &mut rng);
            let user_id = match zipf {
                Some(ref z) => z.sample(&mut rng) as i64,
                None => rng.gen_range(0..config.user_cardinality) as i64,
            };
            rows.push(EventRow {
                region: format!("region_{region_idx:03}"),
                user_id,
                value: user_id as f64,
            });
        }

        Ok(Self { config, rows })
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorkloadIntent {
    pub intent_id: String,
    pub family: String,
    pub table: String,
    pub measure_column: String,
    pub group_by: Vec<String>,
    pub parameters: serde_json::Value,
}

impl WorkloadIntent {
    pub fn from_task(task: &AqpTask) -> Self {
        match task {
            AqpTask::CountDistinct {
                table,
                column,
                group_by,
            } => Self {
                intent_id: if group_by.is_empty() {
                    "count_distinct.v1".to_string()
                } else {
                    "grouped_count_distinct.v1".to_string()
                },
                family: "cardinality".to_string(),
                table: table.clone(),
                measure_column: column.clone(),
                group_by: group_by.clone(),
                parameters: serde_json::json!({ "distinct": true }),
            },
            AqpTask::Quantile {
                table,
                column,
                quantile,
                group_by,
            } => Self {
                intent_id: if group_by.is_empty() {
                    "quantile.v1".to_string()
                } else {
                    "grouped_quantile.v1".to_string()
                },
                family: "quantile".to_string(),
                table: table.clone(),
                measure_column: column.clone(),
                group_by: group_by.clone(),
                parameters: serde_json::json!({ "quantile": quantile }),
            },
            AqpTask::Frequency { table, column } => Self {
                intent_id: "frequency_by_key.v1".to_string(),
                family: "frequency".to_string(),
                table: table.clone(),
                measure_column: "count".to_string(),
                group_by: vec![column.clone()],
                parameters: serde_json::json!({ "aggregate": "count_star" }),
            },
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DataProfile {
    pub source_id: String,
    pub relation: String,
    pub row_count: usize,
    pub regions: usize,
    pub user_cardinality: u64,
    pub user_distribution: UserDistribution,
    pub region_skew: f64,
    pub seed: u64,
}

impl DataProfile {
    pub fn synthetic_events(config: &SyntheticEventsConfig) -> Self {
        Self {
            source_id: "synthetic_events.v1".to_string(),
            relation: "events".to_string(),
            row_count: config.rows,
            regions: config.regions,
            user_cardinality: config.user_cardinality,
            user_distribution: config.user_distribution,
            region_skew: config.region_skew,
            seed: config.seed,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ApproximationRequirements {
    pub target_relative_error: Option<f64>,
    pub target_rank_error: Option<f64>,
    pub latency_budget_ns: Option<u64>,
    pub memory_budget_bytes: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AdmissionStatus {
    Approximated,
    ExactFallback,
    Rejected,
    Unsupported,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdmissionReport {
    pub status: AdmissionStatus,
    pub reason: String,
    pub execution_backend: Option<String>,
    pub fallback: Option<String>,
}

pub fn admit_task(task: &AqpTask) -> AdmissionReport {
    let accepted = |reason: &str| AdmissionReport {
        status: AdmissionStatus::Approximated,
        reason: reason.to_string(),
        execution_backend: Some("local_exact_plus_asap_sketchlib.v1".to_string()),
        fallback: None,
    };
    let unsupported = |reason: String| AdmissionReport {
        status: AdmissionStatus::Unsupported,
        reason,
        execution_backend: None,
        fallback: None,
    };

    match task {
        AqpTask::CountDistinct {
            table,
            column,
            group_by,
        } if table == "events"
            && column == "user_id"
            && (group_by.is_empty() || is_region_group_by(group_by)) =>
        {
            accepted("admitted COUNT(DISTINCT user_id) over events, optionally GROUP BY region")
        }
        AqpTask::Quantile {
            table,
            column,
            quantile,
            group_by,
        } if table == "events"
            && matches!(column.as_str(), "user_id" | "value")
            && (group_by.is_empty() || is_region_group_by(group_by))
            && (0.0..=1.0).contains(quantile)
            && quantile.is_finite() =>
        {
            accepted("admitted approx_median/approx_percentile_cont over events, optionally GROUP BY region")
        }
        AqpTask::Frequency { table, column } if table == "events" && column == "user_id" => {
            accepted("admitted COUNT(*) GROUP BY user_id over events")
        }
        _ => unsupported(format!(
            "unsupported AQP MVP task {task:?}; supported shapes are COUNT(DISTINCT user_id) over events optionally GROUP BY region, approx_median/approx_percentile_cont over events.user_id or events.value optionally GROUP BY region, and COUNT(*) GROUP BY user_id"
        )),
    }
}

fn sample_region(regions: usize, region_skew: f64, rng: &mut Xoshiro256PlusPlus) -> usize {
    if regions == 1 {
        return 0;
    }
    if region_skew > 0.0 && rng.gen_bool(region_skew) {
        0
    } else {
        rng.gen_range(0..regions)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AqpMvpReport {
    pub schema_version: u32,
    pub mode: String,
    pub workload_intent: WorkloadIntent,
    pub data_profile: DataProfile,
    pub approximation_requirements: ApproximationRequirements,
    pub admission: AdmissionReport,
    pub exact_oracle: BackendReport,
    pub approximate_backends: Vec<SketchComparison>,
    pub accuracy_metrics: AccuracyMetrics,
    pub notes: Vec<String>,
}

impl AqpMvpReport {
    pub fn to_jsonl(&self) -> Result<String> {
        Ok(serde_json::to_string(self)?)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackendReport {
    pub role: String,
    pub backend_id: String,
    pub policy: String,
    pub implementation: String,
    pub cost_metrics: CostMetrics,
}

impl BackendReport {
    fn new(
        role: impl Into<String>,
        backend_id: impl Into<String>,
        policy: impl Into<String>,
        implementation: impl Into<String>,
        groups: usize,
        rows: usize,
        elapsed_ns: u128,
        memory_bytes_estimate: u64,
    ) -> Self {
        let throughput_rows_per_sec = rows_per_sec(rows, elapsed_ns);
        Self {
            role: role.into(),
            backend_id: backend_id.into(),
            policy: policy.into(),
            implementation: implementation.into(),
            cost_metrics: CostMetrics {
                rows,
                groups,
                elapsed_ns,
                throughput_rows_per_sec,
                memory_bytes_estimate,
            },
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CostMetrics {
    pub rows: usize,
    pub groups: usize,
    pub elapsed_ns: u128,
    pub throughput_rows_per_sec: f64,
    pub memory_bytes_estimate: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SketchComparison {
    pub sketch: BackendReport,
    pub accuracy: AccuracySummary,
    pub accuracy_metrics: AccuracyMetrics,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AccuracySummary {
    pub compared_groups: usize,
    pub missing_groups: usize,
    pub mean_relative_error: f64,
    pub p95_relative_error: f64,
    pub max_relative_error: f64,
    pub worst_group: Option<GroupError>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GroupError {
    pub group: String,
    pub exact: f64,
    pub estimate: f64,
    pub relative_error: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AccuracyMetrics {
    pub metric_kind: String,
    pub compared_items: usize,
    pub missing_items: usize,
    pub relative_error: Option<AccuracySummary>,
    pub rank_error: Option<RankErrorSummary>,
    pub frequency_heavy_hitters: Option<FrequencyHeavyHitterSummary>,
}

impl AccuracyMetrics {
    fn relative(metric_kind: impl Into<String>, summary: AccuracySummary) -> Self {
        Self {
            metric_kind: metric_kind.into(),
            compared_items: summary.compared_groups,
            missing_items: summary.missing_groups,
            relative_error: Some(summary),
            rank_error: None,
            frequency_heavy_hitters: None,
        }
    }

    fn quantile(rank_error: RankErrorSummary, value_error: AccuracySummary) -> Self {
        Self {
            metric_kind: "quantile_rank_error".to_string(),
            compared_items: rank_error.compared_groups,
            missing_items: rank_error.missing_groups,
            relative_error: Some(value_error),
            rank_error: Some(rank_error),
            frequency_heavy_hitters: None,
        }
    }

    fn frequency(all_keys: AccuracySummary, heavy_hitters: FrequencyHeavyHitterSummary) -> Self {
        Self {
            metric_kind: "frequency_relative_error".to_string(),
            compared_items: all_keys.compared_groups,
            missing_items: all_keys.missing_groups,
            relative_error: Some(all_keys),
            rank_error: None,
            frequency_heavy_hitters: Some(heavy_hitters),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RankErrorSummary {
    pub quantile: f64,
    pub compared_groups: usize,
    pub missing_groups: usize,
    pub mean_rank_error: f64,
    pub p95_rank_error: f64,
    pub max_rank_error: f64,
    pub worst_group: Option<RankGroupError>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RankGroupError {
    pub group: String,
    pub target_quantile: f64,
    pub estimate: f64,
    pub lower_rank: f64,
    pub upper_rank: f64,
    pub rank_error: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FrequencyHeavyHitterSummary {
    pub top_k: usize,
    pub compared_keys: usize,
    pub mean_relative_error: f64,
    pub p95_relative_error: f64,
    pub max_relative_error: f64,
    pub worst_key: Option<GroupError>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AsapQueryImportManifest {
    pub case_id: String,
    pub track: String,
    pub benchmark_context: String,
    pub task: AsapQueryTaskSpec,
    pub data_condition: AsapQueryDataCondition,
    pub requirements: AsapQueryRequirements,
    pub options: AsapQueryOptions,
    pub provenance: AsapQueryProvenance,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AsapQueryTaskSpec {
    pub task_id: String,
    pub family: String,
    pub query_language: String,
    pub table: String,
    pub time_column: String,
    pub value_column: String,
    pub group_by: Vec<String>,
    pub quantile: f64,
    pub window_size_secs: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AsapQueryDataCondition {
    pub dataset_id: String,
    pub description: String,
    pub row_count: Option<u64>,
    pub window_count: Option<u64>,
    pub group_count: Option<u64>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AsapQueryRequirements {
    pub latency_budget_ms: Option<f64>,
    pub target_relative_error: Option<f64>,
    pub require_result_rows_match: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AsapQueryOptions {
    pub approximate_option_id: String,
    pub baseline_option_id: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AsapQueryProvenance {
    pub asapquery_repo_path: Option<String>,
    pub benchmark_dir: Option<String>,
    pub asap_query_file: Option<String>,
    pub baseline_query_file: Option<String>,
    pub query_suite_file: Option<String>,
    pub streaming_config: Option<String>,
    pub inference_config: Option<String>,
}

impl AsapQueryImportManifest {
    pub fn from_toml_str(input: &str) -> Result<Self> {
        let doc = MiniToml::parse(input)?;
        let task = AsapQueryTaskSpec {
            task_id: doc.required("task", "task_id")?,
            family: doc.required("task", "family")?,
            query_language: doc
                .optional("task", "query_language")
                .unwrap_or_else(|| "SQL".to_string()),
            table: doc.required("task", "table")?,
            time_column: doc.required("task", "time_column")?,
            value_column: doc.required("task", "value_column")?,
            group_by: doc.array("task", "group_by")?,
            quantile: doc.f64("task", "quantile")?,
            window_size_secs: doc.u64("task", "window_size_secs")?,
        };
        let data_condition = AsapQueryDataCondition {
            dataset_id: doc.required("data_condition", "dataset_id")?,
            description: doc
                .optional("data_condition", "description")
                .unwrap_or_default(),
            row_count: doc.optional_u64("data_condition", "row_count")?,
            window_count: doc.optional_u64("data_condition", "window_count")?,
            group_count: doc.optional_u64("data_condition", "group_count")?,
        };
        let requirements = AsapQueryRequirements {
            latency_budget_ms: doc.optional_f64("requirements", "latency_budget_ms")?,
            target_relative_error: doc.optional_f64("requirements", "target_relative_error")?,
            require_result_rows_match: doc.bool_or(
                "requirements",
                "require_result_rows_match",
                true,
            )?,
        };
        let options = AsapQueryOptions {
            approximate_option_id: doc.required("options", "approximate_option_id")?,
            baseline_option_id: doc.required("options", "baseline_option_id")?,
        };
        let provenance = AsapQueryProvenance {
            asapquery_repo_path: doc.optional("provenance", "asapquery_repo_path"),
            benchmark_dir: doc.optional("provenance", "benchmark_dir"),
            asap_query_file: doc.optional("provenance", "asap_query_file"),
            baseline_query_file: doc.optional("provenance", "baseline_query_file"),
            query_suite_file: doc.optional("provenance", "query_suite_file"),
            streaming_config: doc.optional("provenance", "streaming_config"),
            inference_config: doc.optional("provenance", "inference_config"),
        };
        Ok(Self {
            case_id: doc.required("case", "case_id")?,
            track: doc.required("case", "track")?,
            benchmark_context: doc.required("case", "benchmark_context")?,
            task,
            data_condition,
            requirements,
            options,
            provenance,
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AsapQueryImportReport {
    pub schema_version: u32,
    pub mode: String,
    pub case_id: String,
    pub track: String,
    pub benchmark_context: String,
    pub task: AsapQueryTaskSpec,
    pub data_condition: AsapQueryDataCondition,
    pub requirements: AsapQueryRequirements,
    pub options: AsapQueryOptions,
    pub records: Vec<AsapQueryAqpRecord>,
    pub provenance: AsapQueryProvenance,
    pub notes: Vec<String>,
}

impl AsapQueryImportReport {
    pub fn to_jsonl(&self) -> Result<String> {
        Ok(serde_json::to_string(self)?)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AsapQueryAqpRecord {
    pub record_kind: String,
    pub query_id: String,
    pub option_run: Option<AsapQueryOptionRun>,
    pub comparison: Option<AsapQueryPairComparison>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AsapQueryOptionRun {
    pub option_id: String,
    pub role: String,
    pub admission_status: AsapQueryAdmissionStatus,
    pub error: Option<String>,
    pub cost_metrics: AsapQueryCostMetrics,
    pub result_observation: AsapQueryResultObservation,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AsapQueryAdmissionStatus {
    Approximated,
    Baseline,
    Timeout,
    RejectedOrRuntimeError,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AsapQueryCostMetrics {
    pub latency_ms: f64,
    pub serving_ms: Option<f64>,
    pub pipeline_ms: Option<f64>,
    pub result_rows: usize,
    pub latency_budget_met: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AsapQueryResultObservation {
    pub result_text_kind: String,
    pub result_text_present: bool,
    pub result_rows_reported: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AsapQueryPairComparison {
    pub approximate_option_id: String,
    pub baseline_option_id: String,
    pub status: String,
    pub fidelity_status: String,
    pub latency_speedup: Option<f64>,
    pub latency_delta_ms: Option<f64>,
    pub result_row_delta: Option<i64>,
    pub numeric_error: Option<AsapQueryNumericErrorSummary>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AsapQueryNumericErrorSummary {
    pub compared_values: usize,
    pub mean_absolute_error: f64,
    pub max_absolute_error: f64,
    pub mean_relative_error: f64,
    pub max_relative_error: f64,
}

pub fn import_asapquery_csv_readers<R1: Read, R2: Read>(
    manifest: AsapQueryImportManifest,
    asap_csv: R1,
    baseline_csv: R2,
) -> Result<AsapQueryImportReport> {
    let asap_rows = read_asapquery_rows(asap_csv).context("read ASAPQuery approximate CSV")?;
    let baseline_rows = read_asapquery_rows(baseline_csv).context("read ASAPQuery baseline CSV")?;
    Ok(import_asapquery_rows(manifest, asap_rows, baseline_rows))
}

pub fn import_asapquery_rows(
    manifest: AsapQueryImportManifest,
    asap_rows: Vec<AsapQueryCsvRow>,
    baseline_rows: Vec<AsapQueryCsvRow>,
) -> AsapQueryImportReport {
    let mut by_query: BTreeMap<String, (Option<AsapQueryCsvRow>, Option<AsapQueryCsvRow>)> =
        BTreeMap::new();
    for row in asap_rows {
        let query_id = row.query_id.clone();
        by_query.entry(query_id).or_default().0 = Some(row);
    }
    for row in baseline_rows {
        let query_id = row.query_id.clone();
        by_query.entry(query_id).or_default().1 = Some(row);
    }

    let mut records = Vec::new();
    for (query_id, (asap, baseline)) in by_query {
        if let Some(row) = &asap {
            records.push(AsapQueryAqpRecord {
                record_kind: "option_run".to_string(),
                query_id: query_id.clone(),
                option_run: Some(option_run(
                    &manifest.options.approximate_option_id,
                    "approximate",
                    row,
                    manifest.requirements.latency_budget_ms,
                )),
                comparison: None,
            });
        }
        if let Some(row) = &baseline {
            records.push(AsapQueryAqpRecord {
                record_kind: "option_run".to_string(),
                query_id: query_id.clone(),
                option_run: Some(option_run(
                    &manifest.options.baseline_option_id,
                    "baseline",
                    row,
                    manifest.requirements.latency_budget_ms,
                )),
                comparison: None,
            });
        }
        records.push(AsapQueryAqpRecord {
            record_kind: "paired_comparison".to_string(),
            query_id,
            option_run: None,
            comparison: Some(pair_comparison(&manifest, asap.as_ref(), baseline.as_ref())),
        });
    }

    AsapQueryImportReport {
        schema_version: 1,
        mode: "external_asapquery_import".to_string(),
        case_id: manifest.case_id.clone(),
        track: manifest.track.clone(),
        benchmark_context: manifest.benchmark_context.clone(),
        task: manifest.task.clone(),
        data_condition: manifest.data_condition.clone(),
        requirements: manifest.requirements.clone(),
        options: manifest.options.clone(),
        records,
        provenance: manifest.provenance.clone(),
        notes: vec![
            "Imported from external ASAPQuery benchmark CSVs; sketch-bench did not run the external systems.".to_string(),
            "ClickHouse baseline is recorded as a baseline option, not automatically as an exact oracle.".to_string(),
        ],
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct AsapQueryCsvRow {
    pub query_id: String,
    pub latency_ms: f64,
    pub serving_ms: Option<f64>,
    pub pipeline_ms: Option<f64>,
    pub result_rows: usize,
    pub result_text: Option<String>,
    pub result_text_kind: String,
    pub error: Option<String>,
    pub mode: Option<String>,
}

pub fn read_asapquery_rows<R: Read>(reader: R) -> Result<Vec<AsapQueryCsvRow>> {
    let mut csv_reader = csv::Reader::from_reader(reader);
    let headers = csv_reader.headers()?.clone();
    let mut rows = Vec::new();
    for record in csv_reader.records() {
        let record = record?;
        let get = |name: &str| -> Option<&str> {
            headers
                .iter()
                .position(|h| h == name)
                .and_then(|idx| record.get(idx))
                .map(str::trim)
        };
        let query_id = get("query_id")
            .filter(|v| !v.is_empty())
            .context("ASAPQuery CSV row missing query_id")?
            .to_string();
        let latency_ms = parse_optional_f64(get("latency_ms")).unwrap_or(0.0);
        let result_rows = parse_optional_usize(get("result_rows")).unwrap_or(0);
        let result_text =
            first_non_empty(&[get("result_full"), get("result"), get("result_preview")]);
        let result_text_kind = if get("result_full").is_some_and(|v| !v.is_empty()) {
            "full"
        } else if get("result").is_some_and(|v| !v.is_empty()) {
            "full"
        } else if get("result_preview").is_some_and(|v| !v.is_empty()) {
            "preview"
        } else {
            "none"
        }
        .to_string();
        rows.push(AsapQueryCsvRow {
            query_id,
            latency_ms,
            serving_ms: parse_optional_f64(get("serving_ms")),
            pipeline_ms: parse_optional_f64(get("pipeline_ms")),
            result_rows,
            result_text,
            result_text_kind,
            error: get("error")
                .filter(|v| !v.is_empty())
                .map(ToString::to_string),
            mode: get("mode")
                .filter(|v| !v.is_empty())
                .map(ToString::to_string),
        });
    }
    Ok(rows)
}

fn option_run(
    option_id: &str,
    role: &str,
    row: &AsapQueryCsvRow,
    latency_budget_ms: Option<f64>,
) -> AsapQueryOptionRun {
    let admission_status = match (role, row.error.as_deref()) {
        (_, Some(err)) if err.eq_ignore_ascii_case("timeout") => AsapQueryAdmissionStatus::Timeout,
        (_, Some(_)) => AsapQueryAdmissionStatus::RejectedOrRuntimeError,
        ("baseline", None) => AsapQueryAdmissionStatus::Baseline,
        _ => AsapQueryAdmissionStatus::Approximated,
    };
    AsapQueryOptionRun {
        option_id: option_id.to_string(),
        role: role.to_string(),
        admission_status,
        error: row.error.clone(),
        cost_metrics: AsapQueryCostMetrics {
            latency_ms: row.latency_ms,
            serving_ms: row.serving_ms,
            pipeline_ms: row.pipeline_ms,
            result_rows: row.result_rows,
            latency_budget_met: latency_budget_ms.map(|budget| row.latency_ms <= budget),
        },
        result_observation: AsapQueryResultObservation {
            result_text_kind: row.result_text_kind.clone(),
            result_text_present: row.result_text.as_ref().is_some_and(|v| !v.is_empty()),
            result_rows_reported: row.result_rows,
        },
    }
}

fn pair_comparison(
    manifest: &AsapQueryImportManifest,
    asap: Option<&AsapQueryCsvRow>,
    baseline: Option<&AsapQueryCsvRow>,
) -> AsapQueryPairComparison {
    let mut status = "compared".to_string();
    let fidelity_status;
    let mut row_delta = None;
    let mut numeric_error = None;

    match (asap, baseline) {
        (None, _) | (_, None) => {
            status = "missing_counterpart".to_string();
            fidelity_status = "missing_counterpart".to_string();
        }
        (Some(a), Some(b)) if a.error.is_some() || b.error.is_some() => {
            status = "option_error".to_string();
            fidelity_status = "option_error".to_string();
        }
        (Some(a), Some(b)) => {
            row_delta = Some(a.result_rows as i64 - b.result_rows as i64);
            if manifest.requirements.require_result_rows_match && a.result_rows != b.result_rows {
                fidelity_status = "row_count_mismatch".to_string();
            } else if a.result_text_kind != "full" || b.result_text_kind != "full" {
                fidelity_status = "unavailable_result_truncated".to_string();
            } else {
                let a_values = parse_numeric_result_values(a.result_text.as_deref().unwrap_or(""));
                let b_values = parse_numeric_result_values(b.result_text.as_deref().unwrap_or(""));
                if a_values.len() != b_values.len() || a_values.is_empty() {
                    fidelity_status = "numeric_result_unparseable".to_string();
                } else {
                    numeric_error = Some(numeric_error_summary(&a_values, &b_values));
                    fidelity_status = "numeric_value_compared".to_string();
                }
            }
        }
    }

    let (latency_speedup, latency_delta_ms) = match (asap, baseline) {
        (Some(a), Some(b)) if a.error.is_none() && b.error.is_none() && a.latency_ms > 0.0 => (
            Some(b.latency_ms / a.latency_ms),
            Some(a.latency_ms - b.latency_ms),
        ),
        _ => (None, None),
    };

    AsapQueryPairComparison {
        approximate_option_id: manifest.options.approximate_option_id.clone(),
        baseline_option_id: manifest.options.baseline_option_id.clone(),
        status,
        fidelity_status,
        latency_speedup,
        latency_delta_ms,
        result_row_delta: row_delta,
        numeric_error,
    }
}

fn parse_numeric_result_values(text: &str) -> Vec<f64> {
    text.lines()
        .filter_map(|line| {
            line.split(['\t', ',', ' '])
                .find_map(|token| token.trim().parse::<f64>().ok())
        })
        .collect()
}

fn numeric_error_summary(
    approximate_values: &[f64],
    baseline_values: &[f64],
) -> AsapQueryNumericErrorSummary {
    let mut abs_errors = Vec::with_capacity(approximate_values.len());
    let mut rel_errors = Vec::with_capacity(approximate_values.len());
    for (approx, base) in approximate_values.iter().zip(baseline_values.iter()) {
        let abs = (approx - base).abs();
        abs_errors.push(abs);
        rel_errors.push(if *base == 0.0 { 0.0 } else { abs / base.abs() });
    }
    AsapQueryNumericErrorSummary {
        compared_values: abs_errors.len(),
        mean_absolute_error: mean(&abs_errors),
        max_absolute_error: abs_errors.iter().copied().fold(0.0, f64::max),
        mean_relative_error: mean(&rel_errors),
        max_relative_error: rel_errors.iter().copied().fold(0.0, f64::max),
    }
}

fn mean(values: &[f64]) -> f64 {
    if values.is_empty() {
        0.0
    } else {
        values.iter().sum::<f64>() / values.len() as f64
    }
}

fn parse_optional_f64(value: Option<&str>) -> Option<f64> {
    value.filter(|v| !v.is_empty())?.parse().ok()
}

fn parse_optional_usize(value: Option<&str>) -> Option<usize> {
    value.filter(|v| !v.is_empty())?.parse().ok()
}

fn first_non_empty(values: &[Option<&str>]) -> Option<String> {
    values
        .iter()
        .flatten()
        .find(|v| !v.is_empty())
        .map(|v| (*v).to_string())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AsapQueryPromqlImportReport {
    pub schema_version: u32,
    pub mode: String,
    pub case_id: String,
    pub track: String,
    pub benchmark_context: String,
    pub task: AsapQueryTaskSpec,
    pub data_condition: AsapQueryDataCondition,
    pub requirements: AsapQueryRequirements,
    pub options: AsapQueryOptions,
    pub records: Vec<AsapQueryPromqlAqpRecord>,
    pub provenance: AsapQueryProvenance,
    pub notes: Vec<String>,
}

impl AsapQueryPromqlImportReport {
    pub fn to_jsonl(&self) -> Result<String> {
        Ok(serde_json::to_string(self)?)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AsapQueryPromqlAqpRecord {
    pub record_kind: String,
    pub query_id: String,
    pub query_expr: Option<String>,
    pub declared_approximate: Option<bool>,
    pub option_run: Option<AsapQueryPromqlOptionRun>,
    pub comparison: Option<AsapQueryPromqlPairComparison>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AsapQueryPromqlOptionRun {
    pub option_id: String,
    pub role: String,
    pub admission_status: AsapQueryPromqlAdmissionStatus,
    pub error: Option<String>,
    pub cost_metrics: AsapQueryPromqlCostMetrics,
    pub result_observation: AsapQueryPromqlResultObservation,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AsapQueryPromqlAdmissionStatus {
    Approximated,
    ExactFallback,
    Baseline,
    OptionError,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AsapQueryPromqlCostMetrics {
    pub latency_samples_ms: Vec<f64>,
    pub failed_runs: usize,
    pub min_latency_ms: Option<f64>,
    pub mean_latency_ms: Option<f64>,
    pub p50_latency_ms: Option<f64>,
    pub p95_latency_ms: Option<f64>,
    pub max_latency_ms: Option<f64>,
    pub latency_budget_met: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AsapQueryPromqlResultObservation {
    pub result_series: usize,
    pub result_values: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AsapQueryPromqlPairComparison {
    pub approximate_option_id: String,
    pub baseline_option_id: String,
    pub status: String,
    pub fidelity_status: String,
    pub native_approximate: Option<bool>,
    pub latency_speedup_p95: Option<f64>,
    pub latency_delta_p95_ms: Option<f64>,
    pub result_series_delta: Option<i64>,
    pub numeric_error: Option<AsapQueryPromqlNumericErrorSummary>,
    pub requirement_status: AsapQueryPromqlRequirementStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AsapQueryPromqlRequirementStatus {
    pub latency_budget_met: Option<bool>,
    pub fidelity_budget_met: Option<bool>,
    pub result_series_match_met: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AsapQueryPromqlNumericErrorSummary {
    pub compared_series: usize,
    pub missing_series: usize,
    pub extra_series: usize,
    pub mean_absolute_error: f64,
    pub max_absolute_error: f64,
    pub mean_relative_error: f64,
    pub max_relative_error: f64,
    pub worst_label_set: Option<String>,
}

#[derive(Debug, Deserialize)]
struct AsapQueryPromqlSuite {
    queries: Vec<AsapQueryPromqlSuiteQuery>,
}

#[derive(Debug, Clone, Deserialize)]
struct AsapQueryPromqlSuiteQuery {
    id: String,
    expr: String,
    #[serde(default)]
    approximate: bool,
}

#[derive(Debug, Deserialize)]
struct AsapQueryPromqlResultFile {
    results: BTreeMap<String, AsapQueryPromqlQueryResult>,
}

#[derive(Debug, Clone, Deserialize)]
struct AsapQueryPromqlQueryResult {
    status: String,
    #[serde(default)]
    approximate: bool,
    #[serde(default)]
    latencies_ms: Vec<Option<f64>>,
    #[serde(default)]
    data: Vec<serde_json::Value>,
    error: Option<String>,
}

pub fn import_asapquery_promql_json_readers<R1: Read, R2: Read, R3: Read>(
    manifest: AsapQueryImportManifest,
    baseline_json: R1,
    asap_json: R2,
    query_suite_json: R3,
) -> Result<AsapQueryPromqlImportReport> {
    let baseline: AsapQueryPromqlResultFile =
        serde_json::from_reader(baseline_json).context("read ASAPQuery PromQL baseline JSON")?;
    let asap: AsapQueryPromqlResultFile =
        serde_json::from_reader(asap_json).context("read ASAPQuery PromQL ASAP JSON")?;
    let suite: AsapQueryPromqlSuite =
        serde_json::from_reader(query_suite_json).context("read ASAPQuery PromQL query suite")?;
    Ok(import_asapquery_promql_results(
        manifest,
        baseline.results,
        asap.results,
        suite.queries,
    ))
}

fn import_asapquery_promql_results(
    manifest: AsapQueryImportManifest,
    baseline_results: BTreeMap<String, AsapQueryPromqlQueryResult>,
    asap_results: BTreeMap<String, AsapQueryPromqlQueryResult>,
    suite_queries: Vec<AsapQueryPromqlSuiteQuery>,
) -> AsapQueryPromqlImportReport {
    let suite_by_id = suite_queries
        .into_iter()
        .map(|query| (query.id.clone(), query))
        .collect::<BTreeMap<_, _>>();
    let mut all_ids = suite_by_id.keys().cloned().collect::<BTreeSet<_>>();
    all_ids.extend(baseline_results.keys().cloned());
    all_ids.extend(asap_results.keys().cloned());

    let mut records = Vec::new();
    for query_id in all_ids {
        let query = suite_by_id.get(&query_id);
        let query_expr = query.map(|q| q.expr.clone());
        let declared_approximate = query.map(|q| q.approximate);
        let baseline = baseline_results.get(&query_id);
        let asap = asap_results.get(&query_id);

        if let Some(result) = baseline {
            records.push(AsapQueryPromqlAqpRecord {
                record_kind: "option_run".to_string(),
                query_id: query_id.clone(),
                query_expr: query_expr.clone(),
                declared_approximate,
                option_run: Some(promql_option_run(
                    &manifest.options.baseline_option_id,
                    "baseline",
                    result,
                    false,
                    manifest.requirements.latency_budget_ms,
                )),
                comparison: None,
            });
        }
        if let Some(result) = asap {
            records.push(AsapQueryPromqlAqpRecord {
                record_kind: "option_run".to_string(),
                query_id: query_id.clone(),
                query_expr: query_expr.clone(),
                declared_approximate,
                option_run: Some(promql_option_run(
                    &manifest.options.approximate_option_id,
                    "asapquery",
                    result,
                    result.approximate,
                    manifest.requirements.latency_budget_ms,
                )),
                comparison: None,
            });
        }
        records.push(AsapQueryPromqlAqpRecord {
            record_kind: "paired_comparison".to_string(),
            query_id,
            query_expr,
            declared_approximate,
            option_run: None,
            comparison: Some(promql_pair_comparison(
                &manifest,
                asap,
                baseline,
                declared_approximate,
            )),
        });
    }

    AsapQueryPromqlImportReport {
        schema_version: 1,
        mode: "external_asapquery_promql_import".to_string(),
        case_id: manifest.case_id.clone(),
        track: manifest.track.clone(),
        benchmark_context: manifest.benchmark_context.clone(),
        task: manifest.task.clone(),
        data_condition: manifest.data_condition.clone(),
        requirements: manifest.requirements.clone(),
        options: manifest.options.clone(),
        records,
        provenance: manifest.provenance.clone(),
        notes: vec![
            "Imported from ASAPQuery quickstart PromQL JSON reports; sketch-bench did not modify ASAPQuery.".to_string(),
            "Prometheus is recorded as the exact baseline for this PromQL track; ASAPQuery rows are split into approximate native execution and exact fallback behavior.".to_string(),
            "Latency, admission, and fidelity are separate AQP outcomes because the bundled ASAPQuery comparator treats some approximate fidelity violations as informational warnings.".to_string(),
        ],
    }
}

fn promql_option_run(
    option_id: &str,
    role: &str,
    result: &AsapQueryPromqlQueryResult,
    native_approximate: bool,
    latency_budget_ms: Option<f64>,
) -> AsapQueryPromqlOptionRun {
    let success = result.status == "success" && result.error.is_none();
    let admission_status = if !success {
        AsapQueryPromqlAdmissionStatus::OptionError
    } else if role == "baseline" {
        AsapQueryPromqlAdmissionStatus::Baseline
    } else if native_approximate {
        AsapQueryPromqlAdmissionStatus::Approximated
    } else {
        AsapQueryPromqlAdmissionStatus::ExactFallback
    };
    let latencies = valid_promql_latencies(&result.latencies_ms);
    let stats = latency_stats(&latencies);
    AsapQueryPromqlOptionRun {
        option_id: option_id.to_string(),
        role: role.to_string(),
        admission_status,
        error: result.error.clone(),
        cost_metrics: AsapQueryPromqlCostMetrics {
            latency_samples_ms: latencies,
            failed_runs: result.latencies_ms.iter().filter(|v| v.is_none()).count(),
            min_latency_ms: stats.min,
            mean_latency_ms: stats.mean,
            p50_latency_ms: stats.p50,
            p95_latency_ms: stats.p95,
            max_latency_ms: stats.max,
            latency_budget_met: latency_budget_ms
                .zip(stats.p95)
                .map(|(budget, p95)| p95 <= budget),
        },
        result_observation: AsapQueryPromqlResultObservation {
            result_series: result.data.len(),
            result_values: result
                .data
                .iter()
                .filter(|entry| promql_entry_value(entry).is_some())
                .count(),
        },
    }
}

fn promql_pair_comparison(
    manifest: &AsapQueryImportManifest,
    asap: Option<&AsapQueryPromqlQueryResult>,
    baseline: Option<&AsapQueryPromqlQueryResult>,
    declared_approximate: Option<bool>,
) -> AsapQueryPromqlPairComparison {
    let mut status = "compared".to_string();
    let fidelity_status;
    let mut numeric_error = None;
    let mut result_series_delta = None;

    match (asap, baseline) {
        (None, _) | (_, None) => {
            status = "missing_counterpart".to_string();
            fidelity_status = "missing_counterpart".to_string();
        }
        (Some(a), Some(b)) if a.status != "success" || b.status != "success" => {
            status = "option_error".to_string();
            fidelity_status = "option_error".to_string();
        }
        (Some(a), Some(b)) => {
            result_series_delta = Some(a.data.len() as i64 - b.data.len() as i64);
            let summary = promql_numeric_error_summary(&a.data, &b.data);
            fidelity_status = if summary.compared_series == 0
                && summary.missing_series == 0
                && summary.extra_series == 0
            {
                "both_empty".to_string()
            } else if summary.compared_series == 0 {
                "numeric_result_unparseable".to_string()
            } else if (summary.missing_series > 0 || summary.extra_series > 0)
                && manifest.requirements.require_result_rows_match
            {
                "label_set_mismatch".to_string()
            } else {
                "numeric_value_compared".to_string()
            };
            numeric_error = Some(summary);
        }
    }

    let asap_p95 = asap.and_then(|a| latency_stats(&valid_promql_latencies(&a.latencies_ms)).p95);
    let baseline_p95 =
        baseline.and_then(|b| latency_stats(&valid_promql_latencies(&b.latencies_ms)).p95);
    let latency_speedup_p95 = match (asap_p95, baseline_p95) {
        (Some(a), Some(b)) if a > 0.0 => Some(b / a),
        _ => None,
    };
    let latency_delta_p95_ms = match (asap_p95, baseline_p95) {
        (Some(a), Some(b)) => Some(a - b),
        _ => None,
    };

    let fidelity_budget_met = numeric_error.as_ref().and_then(|summary| {
        manifest.requirements.target_relative_error.map(|target| {
            summary.max_relative_error <= target
                && (!manifest.requirements.require_result_rows_match
                    || (summary.missing_series == 0 && summary.extra_series == 0))
        })
    });
    let result_series_match_met = result_series_delta.map(|delta| delta == 0);

    AsapQueryPromqlPairComparison {
        approximate_option_id: manifest.options.approximate_option_id.clone(),
        baseline_option_id: manifest.options.baseline_option_id.clone(),
        status,
        fidelity_status,
        native_approximate: asap.map(|a| a.approximate).or(declared_approximate),
        latency_speedup_p95,
        latency_delta_p95_ms,
        result_series_delta,
        numeric_error,
        requirement_status: AsapQueryPromqlRequirementStatus {
            latency_budget_met: manifest
                .requirements
                .latency_budget_ms
                .zip(asap_p95)
                .map(|(budget, p95)| p95 <= budget),
            fidelity_budget_met,
            result_series_match_met,
        },
    }
}

fn promql_numeric_error_summary(
    asap_data: &[serde_json::Value],
    baseline_data: &[serde_json::Value],
) -> AsapQueryPromqlNumericErrorSummary {
    let asap_map = promql_value_map(asap_data);
    let baseline_map = promql_value_map(baseline_data);
    if asap_map.len() == 1 && baseline_map.len() == 1 {
        let (a_key, a_value) = asap_map.iter().next().expect("len checked");
        let (_, b_value) = baseline_map.iter().next().expect("len checked");
        let abs = (a_value - b_value).abs();
        let rel = relative_error(*a_value, *b_value);
        return AsapQueryPromqlNumericErrorSummary {
            compared_series: 1,
            missing_series: 0,
            extra_series: 0,
            mean_absolute_error: abs,
            max_absolute_error: abs,
            mean_relative_error: rel,
            max_relative_error: rel,
            worst_label_set: Some(a_key.clone()),
        };
    }

    let mut abs_errors = Vec::new();
    let mut rel_errors = Vec::new();
    let mut worst_label_set = None;
    let mut max_relative_error = 0.0;
    let mut missing_series = 0usize;
    for (key, baseline_value) in &baseline_map {
        let Some(asap_value) = asap_map.get(key) else {
            missing_series += 1;
            continue;
        };
        let abs = (asap_value - baseline_value).abs();
        let rel = relative_error(*asap_value, *baseline_value);
        if rel >= max_relative_error {
            max_relative_error = rel;
            worst_label_set = Some(key.clone());
        }
        abs_errors.push(abs);
        rel_errors.push(rel);
    }
    let extra_series = asap_map
        .keys()
        .filter(|key| !baseline_map.contains_key(*key))
        .count();

    AsapQueryPromqlNumericErrorSummary {
        compared_series: abs_errors.len(),
        missing_series,
        extra_series,
        mean_absolute_error: mean(&abs_errors),
        max_absolute_error: abs_errors.iter().copied().fold(0.0, f64::max),
        mean_relative_error: mean(&rel_errors),
        max_relative_error,
        worst_label_set,
    }
}

fn promql_value_map(data: &[serde_json::Value]) -> BTreeMap<String, f64> {
    data.iter()
        .filter_map(|entry| Some((promql_label_key(entry)?, promql_entry_value(entry)?)))
        .collect()
}

fn promql_label_key(entry: &serde_json::Value) -> Option<String> {
    let metric = entry.get("metric")?.as_object()?;
    let ordered = metric
        .iter()
        .map(|(key, value)| (key.clone(), value.as_str().unwrap_or("").to_string()))
        .collect::<BTreeMap<_, _>>();
    serde_json::to_string(&ordered).ok()
}

fn promql_entry_value(entry: &serde_json::Value) -> Option<f64> {
    entry.get("value")?.get(1)?.as_str()?.parse().ok()
}

fn relative_error(approximate: f64, baseline: f64) -> f64 {
    (approximate - baseline).abs() / baseline.abs().max(1e-9)
}

#[derive(Debug, Clone, Copy)]
struct LatencyStats {
    min: Option<f64>,
    mean: Option<f64>,
    p50: Option<f64>,
    p95: Option<f64>,
    max: Option<f64>,
}

fn valid_promql_latencies(latencies: &[Option<f64>]) -> Vec<f64> {
    latencies.iter().flatten().copied().collect()
}

fn latency_stats(values: &[f64]) -> LatencyStats {
    if values.is_empty() {
        return LatencyStats {
            min: None,
            mean: None,
            p50: None,
            p95: None,
            max: None,
        };
    }
    let mut sorted = values.to_vec();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    LatencyStats {
        min: sorted.first().copied(),
        mean: Some(mean(&sorted)),
        p50: Some(percentile_f64(&sorted, 50.0)),
        p95: Some(percentile_f64(&sorted, 95.0)),
        max: sorted.last().copied(),
    }
}

fn percentile_f64(sorted_values: &[f64], pct: f64) -> f64 {
    if sorted_values.len() == 1 {
        return sorted_values[0];
    }
    let idx = (pct / 100.0) * (sorted_values.len() - 1) as f64;
    let lo = idx.floor() as usize;
    let hi = idx.ceil() as usize;
    if lo == hi {
        sorted_values[lo]
    } else {
        let frac = idx - lo as f64;
        sorted_values[lo] * (1.0 - frac) + sorted_values[hi] * frac
    }
}

#[derive(Debug, Default)]
struct MiniToml {
    values: HashMap<(String, String), String>,
}

impl MiniToml {
    fn parse(input: &str) -> Result<Self> {
        let mut current_section = String::new();
        let mut values = HashMap::new();
        for (idx, raw_line) in input.lines().enumerate() {
            let line = raw_line.split('#').next().unwrap_or("").trim();
            if line.is_empty() {
                continue;
            }
            if line.starts_with('[') && line.ends_with(']') {
                current_section = line[1..line.len() - 1].trim().to_string();
                continue;
            }
            let Some((key, value)) = line.split_once('=') else {
                bail!("invalid manifest line {}: expected key = value", idx + 1);
            };
            if current_section.is_empty() {
                bail!("invalid manifest line {}: key outside a section", idx + 1);
            }
            values.insert(
                (current_section.clone(), key.trim().to_string()),
                value.trim().trim_end_matches(',').to_string(),
            );
        }
        Ok(Self { values })
    }

    fn required(&self, section: &str, key: &str) -> Result<String> {
        self.optional(section, key)
            .with_context(|| format!("manifest missing [{section}].{key}"))
    }

    fn optional(&self, section: &str, key: &str) -> Option<String> {
        self.values
            .get(&(section.to_string(), key.to_string()))
            .map(|v| parse_toml_string(v))
    }

    fn array(&self, section: &str, key: &str) -> Result<Vec<String>> {
        let raw = self
            .values
            .get(&(section.to_string(), key.to_string()))
            .with_context(|| format!("manifest missing [{section}].{key}"))?;
        parse_toml_array(raw)
    }

    fn f64(&self, section: &str, key: &str) -> Result<f64> {
        self.required(section, key)?
            .parse()
            .with_context(|| format!("manifest [{section}].{key} must be a number"))
    }

    fn u64(&self, section: &str, key: &str) -> Result<u64> {
        self.required(section, key)?
            .parse()
            .with_context(|| format!("manifest [{section}].{key} must be an integer"))
    }

    fn optional_f64(&self, section: &str, key: &str) -> Result<Option<f64>> {
        self.optional(section, key)
            .map(|v| {
                v.parse()
                    .with_context(|| format!("manifest [{section}].{key} must be a number"))
            })
            .transpose()
    }

    fn optional_u64(&self, section: &str, key: &str) -> Result<Option<u64>> {
        self.optional(section, key)
            .map(|v| {
                v.parse()
                    .with_context(|| format!("manifest [{section}].{key} must be an integer"))
            })
            .transpose()
    }

    fn bool_or(&self, section: &str, key: &str, default: bool) -> Result<bool> {
        self.optional(section, key)
            .map(|v| {
                v.parse()
                    .with_context(|| format!("manifest [{section}].{key} must be true or false"))
            })
            .unwrap_or(Ok(default))
    }
}

fn parse_toml_string(raw: &str) -> String {
    raw.trim().trim_matches('"').trim_matches('\'').to_string()
}

fn parse_toml_array(raw: &str) -> Result<Vec<String>> {
    let raw = raw.trim();
    if !raw.starts_with('[') || !raw.ends_with(']') {
        bail!("manifest array value must use [..] syntax");
    }
    let body = &raw[1..raw.len() - 1];
    if body.trim().is_empty() {
        return Ok(Vec::new());
    }
    Ok(body
        .split(',')
        .map(parse_toml_string)
        .filter(|v| !v.is_empty())
        .collect())
}

pub fn run_grouped_count_distinct_mvp(
    task: &AqpTask,
    source: SyntheticEventsConfig,
) -> Result<AqpMvpReport> {
    run_aqp_mvp(task, source)
}

pub fn run_aqp_mvp(task: &AqpTask, source: SyntheticEventsConfig) -> Result<AqpMvpReport> {
    match task {
        AqpTask::CountDistinct { .. } => run_count_distinct_mvp(task, source),
        AqpTask::Quantile { .. } => run_quantile_mvp(task, source),
        AqpTask::Frequency { .. } => run_frequency_mvp(task, source),
    }
}

fn run_count_distinct_mvp(task: &AqpTask, source: SyntheticEventsConfig) -> Result<AqpMvpReport> {
    let AqpTask::CountDistinct {
        table,
        column,
        group_by,
    } = task
    else {
        bail!("expected COUNT(DISTINCT) task, got {task:?}");
    };
    if table != "events"
        || column != "user_id"
        || !(group_by.is_empty() || is_region_group_by(group_by))
    {
        bail!(
            "AQP MVP supports COUNT(DISTINCT user_id) over events, optionally GROUP BY region; got {task:?}"
        );
    }

    let events = SyntheticEvents::generate(source.clone())?;
    let grouped = is_region_group_by(group_by);
    let (exact_report, exact) = run_exact_count_distinct(&events, grouped);
    let (sketch_report, sketch) = run_hll_count_distinct(&events, grouped);
    let accuracy = compare_grouped(&exact, &sketch);
    let accuracy_metrics =
        AccuracyMetrics::relative("cardinality_relative_error", accuracy.clone());
    let sketches = vec![SketchComparison {
        sketch: sketch_report.clone(),
        accuracy: accuracy.clone(),
        accuracy_metrics: accuracy_metrics.clone(),
    }];

    Ok(AqpMvpReport {
        schema_version: 1,
        mode: "aqp_mvp".to_string(),
        workload_intent: task.workload_intent(),
        data_profile: DataProfile::synthetic_events(&source),
        approximation_requirements: ApproximationRequirements::default(),
        admission: admit_task(task),
        exact_oracle: exact_report.clone(),
        approximate_backends: sketches.clone(),
        accuracy_metrics: accuracy_metrics.clone(),
        notes: vec![
            "This MVP benchmarks admitted query shapes, not general SQL: DataFusion SQL -> LogicalPlan -> AQP task -> exact and asap_sketchlib backends.".to_string(),
            "Sampling backend, budget enforcement, shard merge, and multi-operator composed error are intentionally left for the next phase.".to_string(),
        ],
    })
}

fn is_region_group_by(group_by: &[String]) -> bool {
    group_by.len() == 1 && group_by[0] == "region"
}

fn run_exact_count_distinct(
    events: &SyntheticEvents,
    grouped: bool,
) -> (BackendReport, HashMap<String, f64>) {
    let start = Instant::now();
    let mut groups: HashMap<String, HashSet<i64>> = HashMap::new();
    for row in &events.rows {
        let group = group_key(row, grouped);
        groups.entry(group).or_default().insert(row.user_id);
    }
    let elapsed_ns = start.elapsed().as_nanos();
    let result: HashMap<String, f64> = groups
        .iter()
        .map(|(region, users)| (region.clone(), users.len() as f64))
        .collect();
    let memory_bytes_estimate = groups
        .values()
        .map(|users| users.capacity() * std::mem::size_of::<i64>())
        .sum::<usize>() as u64;

    (
        BackendReport::new(
            "exact_oracle",
            "oracle.exact.hashset_count_distinct.v1",
            "exact",
            "HashMap<region, HashSet<user_id>>",
            result.len(),
            events.rows.len(),
            elapsed_ns,
            memory_bytes_estimate,
        ),
        result,
    )
}

fn run_hll_count_distinct(
    events: &SyntheticEvents,
    grouped: bool,
) -> (BackendReport, HashMap<String, f64>) {
    let start = Instant::now();
    let mut groups: HashMap<String, asap_sketchlib::HyperLogLog<asap_sketchlib::Classic>> =
        HashMap::new();
    for row in &events.rows {
        let group = group_key(row, grouped);
        groups
            .entry(group)
            .or_insert_with(asap_sketchlib::HyperLogLog::<asap_sketchlib::Classic>::new)
            .insert(&asap_sketchlib::DataInput::I64(row.user_id));
    }
    let elapsed_ns = start.elapsed().as_nanos();
    let result: HashMap<String, f64> = groups
        .iter()
        .map(|(region, sketch)| (region.clone(), sketch.estimate() as f64))
        .collect();
    let memory_bytes_estimate = (groups.len() * (1usize << 14)) as u64;

    (
        BackendReport::new(
            "approximate_backend",
            "sketch.asap_sketchlib.hll_classic.v1",
            "sketch",
            "HashMap<region, asap_sketchlib::HyperLogLog<Classic>>",
            result.len(),
            events.rows.len(),
            elapsed_ns,
            memory_bytes_estimate,
        ),
        result,
    )
}

fn run_quantile_mvp(task: &AqpTask, source: SyntheticEventsConfig) -> Result<AqpMvpReport> {
    let AqpTask::Quantile {
        table,
        column,
        quantile,
        group_by,
    } = task
    else {
        bail!("expected quantile task, got {task:?}");
    };
    if table != "events"
        || !matches!(column.as_str(), "user_id" | "value")
        || !(group_by.is_empty() || is_region_group_by(group_by))
    {
        bail!(
            "AQP MVP supports approx_median/approx_percentile_cont over events.user_id or events.value, optionally GROUP BY region; got {task:?}"
        );
    }
    validate_quantile(*quantile)?;

    let events = SyntheticEvents::generate(source.clone())?;
    let grouped = is_region_group_by(group_by);
    let (exact_report, exact, sorted_groups) =
        run_exact_quantile(&events, column, *quantile, grouped);
    let (sketch_report, sketch) = run_kll_quantile(&events, column, *quantile, grouped);
    let value_accuracy = compare_grouped(&exact, &sketch);
    let rank_accuracy = compare_quantile_rank(&sorted_groups, &sketch, *quantile);
    let accuracy_metrics = AccuracyMetrics::quantile(rank_accuracy.clone(), value_accuracy.clone());
    let sketches = vec![SketchComparison {
        sketch: sketch_report.clone(),
        accuracy: value_accuracy.clone(),
        accuracy_metrics: accuracy_metrics.clone(),
    }];

    Ok(AqpMvpReport {
        schema_version: 1,
        mode: "aqp_mvp".to_string(),
        workload_intent: task.workload_intent(),
        data_profile: DataProfile::synthetic_events(&source),
        approximation_requirements: ApproximationRequirements::default(),
        admission: admit_task(task),
        exact_oracle: exact_report.clone(),
        approximate_backends: sketches.clone(),
        accuracy_metrics: accuracy_metrics.clone(),
        notes: vec![
            "Quantile AQP MVP maps approx_median/approx_percentile_cont SQL shapes to exact sort and asap_sketchlib KLL backends.".to_string(),
            "DataFusion is used for parsing and logical-plan admission only; sketch-bench owns execution.".to_string(),
        ],
    })
}

fn run_exact_quantile(
    events: &SyntheticEvents,
    column: &str,
    quantile: f64,
    grouped: bool,
) -> (
    BackendReport,
    HashMap<String, f64>,
    HashMap<String, Vec<f64>>,
) {
    let start = Instant::now();
    let mut groups: HashMap<String, Vec<f64>> = HashMap::new();
    for row in &events.rows {
        let group = group_key(row, grouped);
        groups
            .entry(group)
            .or_default()
            .push(numeric_column(row, column));
    }
    let mut result = HashMap::with_capacity(groups.len());
    let mut sorted_groups = HashMap::with_capacity(groups.len());
    let mut memory_bytes_estimate = 0u64;
    for (group, mut values) in groups {
        memory_bytes_estimate += (values.capacity() * std::mem::size_of::<f64>()) as u64;
        values.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        result.insert(group.clone(), exact_quantile_sorted(&values, quantile));
        sorted_groups.insert(group, values);
    }
    let elapsed_ns = start.elapsed().as_nanos();

    (
        BackendReport::new(
            "exact_oracle",
            "oracle.exact.sorted_vec_quantile.type7.v1",
            "exact",
            format!("HashMap<group, Vec<{column}>> sort/type7"),
            result.len(),
            events.rows.len(),
            elapsed_ns,
            memory_bytes_estimate,
        ),
        result,
        sorted_groups,
    )
}

fn run_kll_quantile(
    events: &SyntheticEvents,
    column: &str,
    quantile: f64,
    grouped: bool,
) -> (BackendReport, HashMap<String, f64>) {
    let start = Instant::now();
    let mut groups: HashMap<String, asap_sketchlib::KLL<f64>> = HashMap::new();
    for row in &events.rows {
        let group = group_key(row, grouped);
        let value = numeric_column(row, column);
        groups
            .entry(group)
            .or_insert_with(|| asap_sketchlib::KLL::<f64>::init_kll(DEFAULT_KLL_K))
            .update(&value);
    }
    let elapsed_ns = start.elapsed().as_nanos();
    let result: HashMap<String, f64> = groups
        .iter()
        .map(|(group, sketch)| (group.clone(), sketch.quantile(quantile)))
        .collect();
    let memory_bytes_estimate =
        (groups.len() * DEFAULT_KLL_K as usize * std::mem::size_of::<f64>() * 4) as u64;

    (
        BackendReport::new(
            "approximate_backend",
            format!("sketch.asap_sketchlib.kll.k{DEFAULT_KLL_K}.v1"),
            "sketch",
            format!("HashMap<group, asap_sketchlib::KLL<f64>(k={DEFAULT_KLL_K})>"),
            result.len(),
            events.rows.len(),
            elapsed_ns,
            memory_bytes_estimate,
        ),
        result,
    )
}

fn run_frequency_mvp(task: &AqpTask, source: SyntheticEventsConfig) -> Result<AqpMvpReport> {
    let AqpTask::Frequency { table, column } = task else {
        bail!("expected frequency task, got {task:?}");
    };
    if table != "events" || column != "user_id" {
        bail!("AQP MVP supports frequency only for events.user_id; got {task:?}");
    }

    let events = SyntheticEvents::generate(source.clone())?;
    let (exact_report, exact) = run_exact_frequency(&events);
    let (cms_report, cms) = run_countmin_frequency(&events);
    let cms_accuracy = compare_grouped(&exact, &cms);
    let cms_heavy_hitters = compare_frequency_heavy_hitters(&exact, &cms, FREQ_HEAVY_HITTER_TOP_K);
    let cms_accuracy_metrics = AccuracyMetrics::frequency(cms_accuracy.clone(), cms_heavy_hitters);
    let (cs_report, cs) = run_countsketch_frequency(&events);
    let cs_accuracy = compare_grouped(&exact, &cs);
    let cs_heavy_hitters = compare_frequency_heavy_hitters(&exact, &cs, FREQ_HEAVY_HITTER_TOP_K);
    let cs_accuracy_metrics = AccuracyMetrics::frequency(cs_accuracy.clone(), cs_heavy_hitters);
    let sketches = vec![
        SketchComparison {
            sketch: cms_report.clone(),
            accuracy: cms_accuracy.clone(),
            accuracy_metrics: cms_accuracy_metrics.clone(),
        },
        SketchComparison {
            sketch: cs_report,
            accuracy: cs_accuracy,
            accuracy_metrics: cs_accuracy_metrics,
        },
    ];

    Ok(AqpMvpReport {
        schema_version: 1,
        mode: "aqp_mvp".to_string(),
        workload_intent: task.workload_intent(),
        data_profile: DataProfile::synthetic_events(&source),
        approximation_requirements: ApproximationRequirements::default(),
        admission: admit_task(task),
        exact_oracle: exact_report.clone(),
        approximate_backends: sketches.clone(),
        accuracy_metrics: cms_accuracy_metrics.clone(),
        notes: vec![
            "Frequency AQP MVP maps GROUP BY user_id COUNT(*) to exact counts plus asap_sketchlib CountMin and CountSketch backends.".to_string(),
            "The sketch backends are queried for every exact key so the report compares point-frequency error over the admitted key set.".to_string(),
        ],
    })
}

fn run_exact_frequency(events: &SyntheticEvents) -> (BackendReport, HashMap<String, f64>) {
    let start = Instant::now();
    let mut counts: HashMap<i64, u64> = HashMap::new();
    for row in &events.rows {
        *counts.entry(row.user_id).or_default() += 1;
    }
    let elapsed_ns = start.elapsed().as_nanos();
    let result: HashMap<String, f64> = counts
        .iter()
        .map(|(key, count)| (key.to_string(), *count as f64))
        .collect();
    let memory_bytes_estimate =
        (counts.capacity() * (std::mem::size_of::<i64>() + std::mem::size_of::<u64>())) as u64;

    (
        BackendReport::new(
            "exact_oracle",
            "oracle.exact.hashmap_frequency.v1",
            "exact",
            "HashMap<user_id, count>",
            result.len(),
            events.rows.len(),
            elapsed_ns,
            memory_bytes_estimate,
        ),
        result,
    )
}

fn run_countmin_frequency(events: &SyntheticEvents) -> (BackendReport, HashMap<String, f64>) {
    let start = Instant::now();
    let mut sketch = asap_sketchlib::CountMin::<
        asap_sketchlib::Vector2D<i32>,
        asap_sketchlib::FastPath,
    >::with_dimensions(DEFAULT_FREQ_ROWS, DEFAULT_FREQ_COLS);
    let mut keys = HashSet::new();
    for row in &events.rows {
        let input = asap_sketchlib::DataInput::I64(row.user_id);
        sketch.insert(&input);
        keys.insert(row.user_id);
    }
    let elapsed_ns = start.elapsed().as_nanos();
    let result = keys
        .iter()
        .map(|key| {
            let estimate = sketch.estimate(&asap_sketchlib::DataInput::I64(*key)) as f64;
            (key.to_string(), estimate)
        })
        .collect::<HashMap<_, _>>();
    let memory_bytes_estimate =
        (DEFAULT_FREQ_ROWS * DEFAULT_FREQ_COLS * std::mem::size_of::<i32>()) as u64;

    (
        BackendReport::new(
            "approximate_backend",
            format!(
                "sketch.asap_sketchlib.countmin.fastpath.{DEFAULT_FREQ_ROWS}x{DEFAULT_FREQ_COLS}.v1"
            ),
            "sketch",
            format!(
                "asap_sketchlib::CountMin<Vector2D<i32>, FastPath>({DEFAULT_FREQ_ROWS}x{DEFAULT_FREQ_COLS})"
            ),
            result.len(),
            events.rows.len(),
            elapsed_ns,
            memory_bytes_estimate,
        ),
        result,
    )
}

fn run_countsketch_frequency(events: &SyntheticEvents) -> (BackendReport, HashMap<String, f64>) {
    let start = Instant::now();
    let mut sketch =
        asap_sketchlib::Count::<asap_sketchlib::Vector2D<i32>, asap_sketchlib::FastPath>::with_dimensions(
            DEFAULT_FREQ_ROWS,
            DEFAULT_FREQ_COLS,
        );
    let mut keys = HashSet::new();
    for row in &events.rows {
        let input = asap_sketchlib::DataInput::I64(row.user_id);
        sketch.insert(&input);
        keys.insert(row.user_id);
    }
    let elapsed_ns = start.elapsed().as_nanos();
    let result = keys
        .iter()
        .map(|key| {
            let estimate = sketch
                .estimate(&asap_sketchlib::DataInput::I64(*key))
                .max(0.0);
            (key.to_string(), estimate)
        })
        .collect::<HashMap<_, _>>();
    let memory_bytes_estimate =
        (DEFAULT_FREQ_ROWS * DEFAULT_FREQ_COLS * std::mem::size_of::<i32>()) as u64;

    (
        BackendReport::new(
            "approximate_backend",
            format!(
                "sketch.asap_sketchlib.countsketch.fastpath.{DEFAULT_FREQ_ROWS}x{DEFAULT_FREQ_COLS}.v1"
            ),
            "sketch",
            format!(
                "asap_sketchlib::Count<Vector2D<i32>, FastPath>({DEFAULT_FREQ_ROWS}x{DEFAULT_FREQ_COLS})"
            ),
            result.len(),
            events.rows.len(),
            elapsed_ns,
            memory_bytes_estimate,
        ),
        result,
    )
}

fn compare_grouped(exact: &HashMap<String, f64>, sketch: &HashMap<String, f64>) -> AccuracySummary {
    let mut errors = Vec::with_capacity(exact.len());
    let mut missing = 0usize;

    for (group, &truth) in exact {
        match sketch.get(group) {
            Some(&estimate) => {
                let relative_error = if truth == 0.0 {
                    0.0
                } else {
                    (estimate - truth).abs() / truth.abs()
                };
                errors.push(GroupError {
                    group: group.clone(),
                    exact: truth,
                    estimate,
                    relative_error,
                });
            }
            None => missing += 1,
        }
    }

    errors.sort_by(|a, b| {
        a.relative_error
            .partial_cmp(&b.relative_error)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    let mean = if errors.is_empty() {
        0.0
    } else {
        errors.iter().map(|e| e.relative_error).sum::<f64>() / errors.len() as f64
    };
    let p95 = percentile_error(&errors, 0.95);
    let worst_group = errors.last().cloned();
    let max = worst_group
        .as_ref()
        .map(|e| e.relative_error)
        .unwrap_or(0.0);

    AccuracySummary {
        compared_groups: errors.len(),
        missing_groups: missing,
        mean_relative_error: mean,
        p95_relative_error: p95,
        max_relative_error: max,
        worst_group,
    }
}

fn compare_quantile_rank(
    sorted_groups: &HashMap<String, Vec<f64>>,
    sketch: &HashMap<String, f64>,
    quantile: f64,
) -> RankErrorSummary {
    let mut errors = Vec::with_capacity(sorted_groups.len());
    let mut missing = 0usize;

    for (group, values) in sorted_groups {
        let Some(&estimate) = sketch.get(group) else {
            missing += 1;
            continue;
        };
        if values.is_empty() {
            continue;
        }
        let n = values.len() as f64;
        let lower_rank = values.partition_point(|&v| v < estimate) as f64 / n;
        let upper_rank = values.partition_point(|&v| v <= estimate) as f64 / n;
        let rank_error = if quantile < lower_rank {
            lower_rank - quantile
        } else if quantile > upper_rank {
            quantile - upper_rank
        } else {
            0.0
        };
        errors.push(RankGroupError {
            group: group.clone(),
            target_quantile: quantile,
            estimate,
            lower_rank,
            upper_rank,
            rank_error,
        });
    }

    errors.sort_by(|a, b| {
        a.rank_error
            .partial_cmp(&b.rank_error)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    let mean = if errors.is_empty() {
        0.0
    } else {
        errors.iter().map(|e| e.rank_error).sum::<f64>() / errors.len() as f64
    };
    let p95 = percentile_rank_error(&errors, 0.95);
    let worst_group = errors.last().cloned();
    let max = worst_group.as_ref().map(|e| e.rank_error).unwrap_or(0.0);

    RankErrorSummary {
        quantile,
        compared_groups: errors.len(),
        missing_groups: missing,
        mean_rank_error: mean,
        p95_rank_error: p95,
        max_rank_error: max,
        worst_group,
    }
}

fn compare_frequency_heavy_hitters(
    exact: &HashMap<String, f64>,
    sketch: &HashMap<String, f64>,
    top_k: usize,
) -> FrequencyHeavyHitterSummary {
    let mut keys = exact.iter().collect::<Vec<_>>();
    keys.sort_by(|a, b| {
        b.1.partial_cmp(a.1)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.0.cmp(b.0))
    });
    keys.truncate(top_k.min(keys.len()));

    let mut errors = Vec::with_capacity(keys.len());
    for (key, &truth) in keys {
        let estimate = sketch.get(key).copied().unwrap_or(0.0);
        let relative_error = if truth == 0.0 {
            0.0
        } else {
            (estimate - truth).abs() / truth.abs()
        };
        errors.push(GroupError {
            group: key.clone(),
            exact: truth,
            estimate,
            relative_error,
        });
    }

    errors.sort_by(|a, b| {
        a.relative_error
            .partial_cmp(&b.relative_error)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    let mean = if errors.is_empty() {
        0.0
    } else {
        errors.iter().map(|e| e.relative_error).sum::<f64>() / errors.len() as f64
    };
    let p95 = percentile_error(&errors, 0.95);
    let worst_key = errors.last().cloned();
    let max = worst_key.as_ref().map(|e| e.relative_error).unwrap_or(0.0);

    FrequencyHeavyHitterSummary {
        top_k,
        compared_keys: errors.len(),
        mean_relative_error: mean,
        p95_relative_error: p95,
        max_relative_error: max,
        worst_key,
    }
}

fn group_key(row: &EventRow, grouped: bool) -> String {
    if grouped {
        row.region.clone()
    } else {
        UNGROUPED.to_string()
    }
}

fn numeric_column(row: &EventRow, column: &str) -> f64 {
    match column {
        "user_id" => row.user_id as f64,
        "value" => row.value,
        _ => f64::NAN,
    }
}

fn exact_quantile_sorted(values: &[f64], q: f64) -> f64 {
    if values.is_empty() {
        return f64::NAN;
    }
    if values.len() == 1 {
        return values[0];
    }
    let pos = q * (values.len() - 1) as f64;
    let lo = pos.floor() as usize;
    let hi = pos.ceil() as usize;
    if lo == hi {
        values[lo]
    } else {
        let frac = pos - lo as f64;
        values[lo] * (1.0 - frac) + values[hi] * frac
    }
}

fn validate_quantile(q: f64) -> Result<()> {
    if !(0.0..=1.0).contains(&q) || !q.is_finite() {
        bail!("quantile must be finite and in [0, 1], got {q}");
    }
    Ok(())
}

fn percentile_error(errors: &[GroupError], q: f64) -> f64 {
    if errors.is_empty() {
        return 0.0;
    }
    let idx = ((errors.len() - 1) as f64 * q).round() as usize;
    errors[idx.min(errors.len() - 1)].relative_error
}

fn percentile_rank_error(errors: &[RankGroupError], q: f64) -> f64 {
    if errors.is_empty() {
        return 0.0;
    }
    let idx = ((errors.len() - 1) as f64 * q).round() as usize;
    errors[idx.min(errors.len() - 1)].rank_error
}

fn rows_per_sec(rows: usize, elapsed_ns: u128) -> f64 {
    if elapsed_ns == 0 {
        0.0
    } else {
        rows as f64 * 1_000_000_000.0 / elapsed_ns as f64
    }
}

/// Schema used by the starter query corpus.
///
/// `user_id` is intentionally `Int64` so the current `Workload<Item = i64>`
/// benchmark path can run `COUNT(DISTINCT user_id)` without a relational
/// executor yet.
pub fn events_schema() -> SchemaRef {
    Arc::new(Schema::new(vec![
        Field::new("user_id", DataType::Int64, false),
        Field::new("region", DataType::Utf8, false),
        Field::new("ts", DataType::Int64, false),
        Field::new("value", DataType::Float64, true),
    ]))
}

/// Parse SQL with DataFusion and lower the resulting `LogicalPlan`.
pub fn parse_aqp_sql(sql: impl Into<String>) -> Result<AqpQuery> {
    let sql = sql.into();
    let mut statements = DFParser::parse_sql(&sql)
        .with_context(|| format!("DataFusion could not parse SQL:\n{sql}"))?;
    if statements.len() != 1 {
        bail!(
            "expected exactly one SQL statement in AQP query file, found {}",
            statements.len()
        );
    }
    let context = AqpContextProvider::default();
    let planner = SqlToRel::new(&context);
    let plan = planner
        .statement_to_plan(statements.pop_front().expect("length checked above"))
        .with_context(|| format!("DataFusion could not build a LogicalPlan for SQL:\n{sql}"))?;
    let plan_display = plan.display_indent_schema().to_string();
    let task = lower_plan(&plan)?;

    Ok(AqpQuery {
        sql,
        plan,
        plan_display,
        task,
    })
}

struct AqpContextProvider {
    options: ConfigOptions,
    events: Arc<dyn TableSource>,
}

impl Default for AqpContextProvider {
    fn default() -> Self {
        Self {
            options: ConfigOptions::default(),
            events: Arc::new(LogicalTableSource::new(events_schema())),
        }
    }
}

impl ContextProvider for AqpContextProvider {
    fn get_table_source(
        &self,
        name: TableReference,
    ) -> datafusion_common::Result<Arc<dyn TableSource>> {
        if name.table() == "events" {
            Ok(Arc::clone(&self.events))
        } else {
            plan_err!("Table not found: {}", name.table())
        }
    }

    fn get_function_meta(&self, _name: &str) -> Option<Arc<ScalarUDF>> {
        None
    }

    fn get_aggregate_meta(&self, name: &str) -> Option<Arc<AggregateUDF>> {
        match name {
            "count" => Some(datafusion_functions_aggregate::count::count_udaf()),
            "approx_median" => {
                Some(datafusion_functions_aggregate::approx_median::approx_median_udaf())
            }
            "approx_percentile_cont" => Some(
                datafusion_functions_aggregate::approx_percentile_cont::approx_percentile_cont_udaf(
                ),
            ),
            _ => None,
        }
    }

    fn get_window_meta(&self, _name: &str) -> Option<Arc<WindowUDF>> {
        None
    }

    fn get_variable_type(&self, _variable_names: &[String]) -> Option<DataType> {
        None
    }

    fn options(&self) -> &ConfigOptions {
        &self.options
    }

    fn udf_names(&self) -> Vec<String> {
        Vec::new()
    }

    fn udaf_names(&self) -> Vec<String> {
        vec![
            "count".to_string(),
            "approx_median".to_string(),
            "approx_percentile_cont".to_string(),
        ]
    }

    fn udwf_names(&self) -> Vec<String> {
        Vec::new()
    }
}

/// Lower the supported DataFusion plan subset into an AQP benchmark task.
pub fn lower_plan(plan: &LogicalPlan) -> Result<AqpTask> {
    match plan {
        LogicalPlan::Aggregate(agg) => lower_aggregate(agg),
        LogicalPlan::Projection(proj) => lower_plan(&proj.input),
        LogicalPlan::SubqueryAlias(alias) => lower_plan(&alias.input),
        other => bail!(
            "unsupported AQP plan root: {}. AQP MVP admits aggregate queries only: COUNT(DISTINCT user_id), approx_median/approx_percentile_cont, and COUNT(*) GROUP BY user_id over events.",
            plan_kind(other)
        ),
    }
}

fn lower_aggregate(agg: &Aggregate) -> Result<AqpTask> {
    if agg.aggr_expr.len() != 1 {
        bail!(
            "expected exactly one aggregate expression, found {}",
            agg.aggr_expr.len()
        );
    }

    ensure_supported_aggregate_input(&agg.input)?;

    let group_by = agg
        .group_expr
        .iter()
        .map(group_column_name)
        .collect::<Result<Vec<_>>>()?;
    let table = find_table_name(&agg.input).unwrap_or_else(|| "events".to_string());
    let aggr = agg.aggr_expr[0].clone().unalias();

    let Expr::AggregateFunction(func) = &aggr else {
        bail!("unsupported aggregate expression: {aggr}. Expected an aggregate function");
    };

    let name = func.func.name().to_ascii_lowercase();
    match name.as_str() {
        "count" if func.params.distinct => {
            let column = single_column_name(&aggr).with_context(|| {
                format!("could not find the DISTINCT column in aggregate expression: {aggr}")
            })?;
            Ok(AqpTask::CountDistinct {
                table,
                column,
                group_by,
            })
        }
        "count" => {
            if group_by.len() != 1 {
                bail!("frequency query expects COUNT(*) with exactly one GROUP BY column");
            }
            Ok(AqpTask::Frequency {
                table,
                column: group_by[0].clone(),
            })
        }
        "approx_median" => {
            if func.params.args.len() != 1 {
                bail!("approx_median query expects exactly one column argument");
            }
            Ok(AqpTask::Quantile {
                table,
                column: single_column_name(&func.params.args[0])?,
                quantile: 0.5,
                group_by,
            })
        }
        "approx_percentile_cont" => {
            let (column, quantile) = lower_approx_percentile_args(func)?;
            Ok(AqpTask::Quantile {
                table,
                column,
                quantile,
                group_by,
            })
        }
        _ => bail!(
            "unsupported aggregate expression: {aggr}. Expected COUNT(DISTINCT), COUNT(*) GROUP BY key, approx_median, or approx_percentile_cont"
        ),
    }
}

fn lower_approx_percentile_args(func: &DfAggregateFunction) -> Result<(String, f64)> {
    if func.params.args.len() >= 2 {
        let column = single_column_name(&func.params.args[0])?;
        let quantile = literal_f64(&func.params.args[1])?;
        validate_quantile(quantile)?;
        return Ok((column, quantile));
    }
    if func.params.args.len() == 1 && func.params.order_by.len() == 1 {
        let quantile = literal_f64(&func.params.args[0])?;
        let column = single_column_name(&func.params.order_by[0].expr)?;
        validate_quantile(quantile)?;
        return Ok((column, quantile));
    }
    bail!("approx_percentile_cont expects (column, quantile) or quantile WITHIN GROUP (ORDER BY column)")
}

fn literal_f64(expr: &Expr) -> Result<f64> {
    match expr {
        Expr::Literal(ScalarValue::Float64(Some(v)), _) => Ok(*v),
        Expr::Literal(ScalarValue::Float32(Some(v)), _) => Ok(*v as f64),
        Expr::Literal(ScalarValue::Decimal128(Some(v), _, scale), _) => {
            Ok(*v as f64 / 10f64.powi(*scale as i32))
        }
        other => bail!("expected a numeric literal, got {other}"),
    }
}

fn group_column_name(expr: &Expr) -> Result<String> {
    single_column_name(expr).with_context(|| {
        format!("unsupported GROUP BY expression: {expr}. Expected a plain column")
    })
}

fn single_column_name(expr: &Expr) -> Result<String> {
    let refs = expr.column_refs();
    if refs.len() != 1 {
        bail!(
            "expected exactly one column reference in expression {expr}, found {}",
            refs.len()
        );
    }
    let col = refs.into_iter().next().expect("len checked above");
    Ok(col.name.clone())
}

fn find_table_name(plan: &LogicalPlan) -> Option<String> {
    match plan {
        LogicalPlan::TableScan(scan) => Some(scan.table_name.to_string()),
        LogicalPlan::Filter(filter) => find_table_name(&filter.input),
        LogicalPlan::Projection(proj) => find_table_name(&proj.input),
        LogicalPlan::SubqueryAlias(alias) => find_table_name(&alias.input),
        _ => plan.inputs().into_iter().find_map(find_table_name),
    }
}

fn ensure_supported_aggregate_input(plan: &LogicalPlan) -> Result<()> {
    match plan {
        LogicalPlan::TableScan(_) => Ok(()),
        LogicalPlan::Projection(proj) => ensure_supported_aggregate_input(&proj.input),
        LogicalPlan::SubqueryAlias(alias) => ensure_supported_aggregate_input(&alias.input),
        other => bail!(
            "unsupported AQP input operator below aggregate: {}. Filters, joins, and computed expressions are not executed by the MVP path.",
            plan_kind(other)
        ),
    }
}

fn plan_kind(plan: &LogicalPlan) -> &'static str {
    match plan {
        LogicalPlan::Projection(_) => "Projection",
        LogicalPlan::Filter(_) => "Filter",
        LogicalPlan::Window(_) => "Window",
        LogicalPlan::Aggregate(_) => "Aggregate",
        LogicalPlan::Sort(_) => "Sort",
        LogicalPlan::Join(_) => "Join",
        LogicalPlan::Repartition(_) => "Repartition",
        LogicalPlan::Union(_) => "Union",
        LogicalPlan::TableScan(_) => "TableScan",
        LogicalPlan::EmptyRelation(_) => "EmptyRelation",
        LogicalPlan::Subquery(_) => "Subquery",
        LogicalPlan::SubqueryAlias(_) => "SubqueryAlias",
        LogicalPlan::Limit(_) => "Limit",
        LogicalPlan::Statement(_) => "Statement",
        LogicalPlan::Values(_) => "Values",
        LogicalPlan::Explain(_) => "Explain",
        LogicalPlan::Analyze(_) => "Analyze",
        LogicalPlan::Extension(_) => "Extension",
        LogicalPlan::Distinct(_) => "Distinct",
        LogicalPlan::Dml(_) => "Dml",
        LogicalPlan::Ddl(_) => "Ddl",
        LogicalPlan::Copy(_) => "Copy",
        LogicalPlan::DescribeTable(_) => "DescribeTable",
        LogicalPlan::Unnest(_) => "Unnest",
        LogicalPlan::RecursiveQuery(_) => "RecursiveQuery",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn asapquery_manifest() -> AsapQueryImportManifest {
        AsapQueryImportManifest::from_toml_str(
            r#"
[case]
case_id = "asapquery_h2o_quantile_v1"
track = "SQL AQP"
benchmark_context = "ClickHouse-compatible SQL query serving"

[task]
task_id = "grouped_p95_quantile_10s"
family = "quantile"
query_language = "SQL"
table = "h2o_groupby"
time_column = "timestamp"
value_column = "v1"
group_by = ["id1", "id2"]
quantile = 0.95
window_size_secs = 10

[data_condition]
dataset_id = "h2o_groupby_10m_100_groups"
description = "H2O groupby dataset streamed with synthetic timestamps"
row_count = 10000000
window_count = 10000
group_count = 100

[requirements]
latency_budget_ms = 1000
target_relative_error = 0.01
require_result_rows_match = true

[options]
approximate_option_id = "asapquery_kll_k200"
baseline_option_id = "clickhouse_quantile"

[provenance]
benchmark_dir = "ASAPQuery/asap-tools/execution-utilities/asap_benchmark_pipeline"
asap_query_file = "asap_quantile_queries.sql"
baseline_query_file = "clickhouse_quantile_queries.sql"
streaming_config = "streaming_config.yaml"
inference_config = "inference_config.yaml"
"#,
        )
        .unwrap()
    }

    #[test]
    fn parses_count_distinct() {
        let q = parse_aqp_sql("SELECT COUNT(DISTINCT user_id) AS users FROM events").unwrap();
        assert_eq!(
            q.task,
            AqpTask::CountDistinct {
                table: "events".to_string(),
                column: "user_id".to_string(),
                group_by: vec![],
            }
        );
        assert!(q.task.runnable_by_current_bench());
    }

    #[test]
    fn parses_grouped_count_distinct() {
        let q = parse_aqp_sql(
            "SELECT region, COUNT(DISTINCT user_id) AS users FROM events GROUP BY region",
        )
        .unwrap();
        assert_eq!(
            q.task,
            AqpTask::CountDistinct {
                table: "events".to_string(),
                column: "user_id".to_string(),
                group_by: vec!["region".to_string()],
            }
        );
        assert!(q.task.runnable_by_current_bench());
    }

    #[test]
    fn grouped_mvp_runs_exact_and_sketch() {
        let q = parse_aqp_sql(
            "SELECT region, COUNT(DISTINCT user_id) AS users FROM events GROUP BY region",
        )
        .unwrap();
        let report = run_grouped_count_distinct_mvp(
            &q.task,
            SyntheticEventsConfig {
                rows: 10_000,
                regions: 4,
                user_cardinality: 1_000,
                user_distribution: UserDistribution::Uniform,
                region_skew: 0.25,
                seed: 7,
            },
        )
        .unwrap();
        assert_eq!(report.exact_oracle.cost_metrics.groups, 4);
        assert_eq!(report.approximate_backends[0].sketch.cost_metrics.groups, 4);
        assert_eq!(
            report
                .accuracy_metrics
                .relative_error
                .as_ref()
                .unwrap()
                .compared_groups,
            4
        );
        assert_eq!(
            report.workload_intent.intent_id,
            "grouped_count_distinct.v1"
        );
        assert_eq!(report.data_profile.source_id, "synthetic_events.v1");
        assert_eq!(report.admission.status, AdmissionStatus::Approximated);
        assert_eq!(
            report.exact_oracle.backend_id,
            "oracle.exact.hashset_count_distinct.v1"
        );
        assert_eq!(
            report.approximate_backends[0].sketch.backend_id,
            "sketch.asap_sketchlib.hll_classic.v1"
        );
        assert_eq!(
            report.accuracy_metrics.metric_kind,
            "cardinality_relative_error"
        );
        assert!(report.exact_oracle.cost_metrics.throughput_rows_per_sec > 0.0);
        assert!(
            report.approximate_backends[0]
                .sketch
                .cost_metrics
                .throughput_rows_per_sec
                > 0.0
        );
    }

    #[test]
    fn parses_grouped_approx_median() {
        let q =
            parse_aqp_sql("SELECT region, approx_median(value) AS p50 FROM events GROUP BY region")
                .unwrap();
        assert_eq!(
            q.task,
            AqpTask::Quantile {
                table: "events".to_string(),
                column: "value".to_string(),
                quantile: 0.5,
                group_by: vec!["region".to_string()],
            }
        );
        assert!(q.task.runnable_by_current_bench());
    }

    #[test]
    fn parses_approx_percentile_cont() {
        let q =
            parse_aqp_sql("SELECT approx_percentile_cont(value, 0.95) AS p95 FROM events").unwrap();
        assert_eq!(
            q.task,
            AqpTask::Quantile {
                table: "events".to_string(),
                column: "value".to_string(),
                quantile: 0.95,
                group_by: vec![],
            }
        );
        assert!(q.task.runnable_by_current_bench());
    }

    #[test]
    fn parses_frequency_count_by_user() {
        let q = parse_aqp_sql("SELECT user_id, COUNT(*) AS frequency FROM events GROUP BY user_id")
            .unwrap();
        assert_eq!(
            q.task,
            AqpTask::Frequency {
                table: "events".to_string(),
                column: "user_id".to_string(),
            }
        );
        assert!(q.task.runnable_by_current_bench());
    }

    #[test]
    fn quantile_mvp_runs_exact_and_kll() {
        let q =
            parse_aqp_sql("SELECT region, approx_median(value) AS p50 FROM events GROUP BY region")
                .unwrap();
        let report = run_aqp_mvp(
            &q.task,
            SyntheticEventsConfig {
                rows: 10_000,
                regions: 4,
                user_cardinality: 1_000,
                user_distribution: UserDistribution::Uniform,
                region_skew: 0.25,
                seed: 8,
            },
        )
        .unwrap();
        assert_eq!(report.exact_oracle.cost_metrics.groups, 4);
        assert_eq!(report.approximate_backends[0].sketch.cost_metrics.groups, 4);
        assert_eq!(report.approximate_backends.len(), 1);
        assert!(report.approximate_backends[0]
            .sketch
            .implementation
            .contains("KLL"));
        assert_eq!(
            report
                .accuracy_metrics
                .relative_error
                .as_ref()
                .unwrap()
                .compared_groups,
            4
        );
        assert_eq!(report.workload_intent.intent_id, "grouped_quantile.v1");
        let rank = report
            .accuracy_metrics
            .rank_error
            .as_ref()
            .expect("quantile report should include rank error");
        assert_eq!(rank.compared_groups, 4);
        assert!(rank.max_rank_error >= 0.0);
    }

    #[test]
    fn frequency_mvp_runs_exact_countmin_and_countsketch() {
        let q = parse_aqp_sql("SELECT user_id, COUNT(*) AS frequency FROM events GROUP BY user_id")
            .unwrap();
        let report = run_aqp_mvp(
            &q.task,
            SyntheticEventsConfig {
                rows: 10_000,
                regions: 4,
                user_cardinality: 1_000,
                user_distribution: UserDistribution::Zipf { s: 1.1 },
                region_skew: 0.0,
                seed: 9,
            },
        )
        .unwrap();
        assert!(report.exact_oracle.cost_metrics.groups > 0);
        assert_eq!(report.approximate_backends.len(), 2);
        assert!(report.approximate_backends[0]
            .sketch
            .implementation
            .contains("CountMin"));
        assert!(report.approximate_backends[1]
            .sketch
            .implementation
            .contains("Count"));
        assert_eq!(
            report
                .accuracy_metrics
                .relative_error
                .as_ref()
                .unwrap()
                .compared_groups,
            report.exact_oracle.cost_metrics.groups
        );
        assert_eq!(report.workload_intent.intent_id, "frequency_by_key.v1");
        let heavy = report
            .accuracy_metrics
            .frequency_heavy_hitters
            .as_ref()
            .expect("frequency report should include heavy-hitter accuracy");
        assert_eq!(heavy.top_k, FREQ_HEAVY_HITTER_TOP_K);
        assert!(heavy.compared_keys > 0);
    }

    #[test]
    fn admission_reports_unsupported_task_clearly() {
        let task = AqpTask::CountDistinct {
            table: "events".to_string(),
            column: "value".to_string(),
            group_by: vec!["region".to_string()],
        };
        let admission = admit_task(&task);
        assert_eq!(admission.status, AdmissionStatus::Unsupported);
        assert!(admission.reason.contains("supported shapes"));
        assert!(!task.runnable_by_current_bench());
    }

    #[test]
    fn rejects_filters_until_execution_support_exists() {
        let err =
            parse_aqp_sql("SELECT COUNT(DISTINCT user_id) AS users FROM events WHERE user_id > 10")
                .unwrap_err();
        assert!(err.to_string().contains("unsupported AQP input operator"));
    }

    #[test]
    fn parses_asapquery_manifest_subset() {
        let manifest = asapquery_manifest();
        assert_eq!(manifest.case_id, "asapquery_h2o_quantile_v1");
        assert_eq!(manifest.task.group_by, vec!["id1", "id2"]);
        assert_eq!(manifest.task.quantile, 0.95);
        assert_eq!(manifest.requirements.latency_budget_ms, Some(1000.0));
        assert_eq!(manifest.options.approximate_option_id, "asapquery_kll_k200");
    }

    #[test]
    fn imports_asapquery_preview_csv_without_claiming_fidelity() {
        let asap_csv = "\
query_id,latency_ms,serving_ms,pipeline_ms,result_rows,result_preview,error,mode
T000,10.0,8.0,2.0,2,\"1.0 | 2.0\",,asap
";
        let baseline_csv = "\
query_id,latency_ms,serving_ms,pipeline_ms,result_rows,result_preview,error,mode
T000,100.0,100.0,0.0,2,\"1.0 | 2.0\",,baseline
";
        let report = import_asapquery_csv_readers(
            asapquery_manifest(),
            Cursor::new(asap_csv),
            Cursor::new(baseline_csv),
        )
        .unwrap();
        assert_eq!(report.records.len(), 3);
        let comparison = report
            .records
            .iter()
            .find(|r| r.record_kind == "paired_comparison")
            .unwrap()
            .comparison
            .as_ref()
            .unwrap();
        assert_eq!(comparison.status, "compared");
        assert_eq!(comparison.fidelity_status, "unavailable_result_truncated");
        assert_eq!(comparison.latency_speedup, Some(10.0));
    }

    #[test]
    fn imports_asapquery_error_and_missing_counterpart() {
        let asap_csv = "\
query_id,latency_ms,serving_ms,pipeline_ms,result_rows,result_preview,error,mode
T000,30000.0,30000.0,0.0,0,,Timeout,asap
T001,12.0,12.0,0.0,1,1.0,,asap
";
        let baseline_csv = "\
query_id,latency_ms,serving_ms,pipeline_ms,result_rows,result_preview,error,mode
T000,100.0,100.0,0.0,1,1.0,,baseline
";
        let report = import_asapquery_csv_readers(
            asapquery_manifest(),
            Cursor::new(asap_csv),
            Cursor::new(baseline_csv),
        )
        .unwrap();
        let timeout_run = report
            .records
            .iter()
            .find(|r| r.query_id == "T000" && r.option_run.is_some())
            .unwrap()
            .option_run
            .as_ref()
            .unwrap();
        assert!(matches!(
            timeout_run.admission_status,
            AsapQueryAdmissionStatus::Timeout
        ));
        let missing = report
            .records
            .iter()
            .find(|r| r.query_id == "T001" && r.record_kind == "paired_comparison")
            .unwrap()
            .comparison
            .as_ref()
            .unwrap();
        assert_eq!(missing.status, "missing_counterpart");
    }

    #[test]
    fn imports_asapquery_full_results_and_computes_numeric_error() {
        let asap_csv = "\
query_id,latency_ms,result_rows,result_full,error,mode
T000,10.0,2,\"10.0\tid001\n20.0\tid002\",,asap
";
        let baseline_csv = "\
query_id,latency_ms,result_rows,result_full,error,mode
T000,20.0,2,\"11.0\tid001\n18.0\tid002\",,baseline
";
        let report = import_asapquery_csv_readers(
            asapquery_manifest(),
            Cursor::new(asap_csv),
            Cursor::new(baseline_csv),
        )
        .unwrap();
        let comparison = report
            .records
            .iter()
            .find(|r| r.record_kind == "paired_comparison")
            .unwrap()
            .comparison
            .as_ref()
            .unwrap();
        assert_eq!(comparison.fidelity_status, "numeric_value_compared");
        let numeric = comparison.numeric_error.as_ref().unwrap();
        assert_eq!(numeric.compared_values, 2);
        assert_eq!(numeric.max_absolute_error, 2.0);
        assert!(numeric.mean_relative_error > 0.0);
    }

    #[test]
    fn imports_asapquery_promql_json_with_latency_and_fidelity() {
        let manifest = AsapQueryImportManifest::from_toml_str(
            r#"
[case]
case_id = "asapquery_promql_quickstart_v1"
track = "PromQL AQP"
benchmark_context = "Prometheus-compatible telemetry query serving"

[task]
task_id = "promql_sensor_reading_suite"
family = "mixed_promql_aggregates"
query_language = "PromQL"
table = "sensor_reading"
time_column = "time"
value_column = "sensor_reading"
group_by = ["pattern"]
quantile = 0.95
window_size_secs = 0

[data_condition]
dataset_id = "asapquery_quickstart_fake_exporters"
description = "ASAPQuery quickstart fake exporters"
group_count = 2

[requirements]
latency_budget_ms = 100
target_relative_error = 0.05
require_result_rows_match = true

[options]
approximate_option_id = "asapquery_promql_precompute"
baseline_option_id = "prometheus_exact"

[provenance]
query_suite_file = "benchmarks/queries/promql_suite.json"
"#,
        )
        .unwrap();
        let suite = r#"{
  "queries": [
    {"id": "q95_by_pattern", "expr": "quantile by (pattern) (0.95, sensor_reading)", "approximate": true}
  ]
}"#;
        let baseline = r#"{
  "results": {
    "q95_by_pattern": {
      "status": "success",
      "latencies_ms": [100.0, 110.0, 120.0],
      "data": [
        {"metric": {"pattern": "a"}, "value": [1, "10.0"]},
        {"metric": {"pattern": "b"}, "value": [1, "20.0"]}
      ],
      "error": null
    }
  }
}"#;
        let asap = r#"{
  "results": {
    "q95_by_pattern": {
      "status": "success",
      "approximate": true,
      "latencies_ms": [4.0, 5.0, 6.0],
      "data": [
        {"metric": {"pattern": "a"}, "value": [1, "10.5"]},
        {"metric": {"pattern": "b"}, "value": [1, "19.0"]}
      ],
      "error": null
    }
  }
}"#;
        let report = import_asapquery_promql_json_readers(
            manifest,
            Cursor::new(baseline),
            Cursor::new(asap),
            Cursor::new(suite),
        )
        .unwrap();
        assert_eq!(report.records.len(), 3);
        let asap_run = report
            .records
            .iter()
            .find_map(|record| {
                record
                    .option_run
                    .as_ref()
                    .filter(|run| run.role == "asapquery")
            })
            .unwrap();
        assert_eq!(
            asap_run.admission_status,
            AsapQueryPromqlAdmissionStatus::Approximated
        );
        assert_eq!(asap_run.cost_metrics.p95_latency_ms, Some(5.9));
        let comparison = report
            .records
            .iter()
            .find(|record| record.record_kind == "paired_comparison")
            .unwrap()
            .comparison
            .as_ref()
            .unwrap();
        assert_eq!(comparison.fidelity_status, "numeric_value_compared");
        assert!(comparison.latency_speedup_p95.unwrap() > 18.0);
        let numeric = comparison.numeric_error.as_ref().unwrap();
        assert_eq!(numeric.compared_series, 2);
        assert_eq!(numeric.missing_series, 0);
        assert_eq!(numeric.extra_series, 0);
        assert!(numeric.max_relative_error <= 0.05);
        assert_eq!(
            comparison.requirement_status.fidelity_budget_met,
            Some(true)
        );
    }
}
