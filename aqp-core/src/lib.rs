//! AQP query parsing/lowering.
//!
//! Phase 2 intentionally starts by reusing DataFusion's SQL parser and
//! `LogicalPlan` DAG. This crate owns only the benchmark-specific layer:
//! a registered benchmark schema plus a small lowering pass from
//! DataFusion plans into AQP tasks the current sketch benchmark can run.

use std::collections::{HashMap, HashSet};
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
        match self {
            AqpTask::CountDistinct { group_by, .. } => {
                group_by.is_empty() || is_region_group_by(group_by)
            }
            AqpTask::Quantile { group_by, .. } => {
                group_by.is_empty() || is_region_group_by(group_by)
            }
            AqpTask::Frequency { column, .. } => column == "user_id",
        }
    }
}

const UNGROUPED: &str = "__all__";
const DEFAULT_KLL_K: i32 = 200;
const DEFAULT_FREQ_ROWS: usize = 5;
const DEFAULT_FREQ_COLS: usize = 2048;

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
    pub task: AqpTask,
    pub data_source: SyntheticEventsConfig,
    pub exact: BackendReport,
    pub sketch: BackendReport,
    pub accuracy: AccuracySummary,
    #[serde(default)]
    pub sketches: Vec<SketchComparison>,
    pub notes: Vec<String>,
}

impl AqpMvpReport {
    pub fn to_jsonl(&self) -> Result<String> {
        Ok(serde_json::to_string(self)?)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackendReport {
    pub policy: String,
    pub implementation: String,
    pub groups: usize,
    pub rows: usize,
    pub elapsed_ns: u128,
    pub throughput_rows_per_sec: f64,
    pub memory_bytes_estimate: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SketchComparison {
    pub sketch: BackendReport,
    pub accuracy: AccuracySummary,
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
    let sketches = vec![SketchComparison {
        sketch: sketch_report.clone(),
        accuracy: accuracy.clone(),
    }];

    Ok(AqpMvpReport {
        schema_version: 1,
        mode: "aqp_mvp".to_string(),
        task: task.clone(),
        data_source: source,
        exact: exact_report,
        sketch: sketch_report,
        accuracy,
        sketches,
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
        BackendReport {
            policy: "exact".to_string(),
            implementation: "HashMap<region, HashSet<user_id>>".to_string(),
            groups: result.len(),
            rows: events.rows.len(),
            elapsed_ns,
            throughput_rows_per_sec: rows_per_sec(events.rows.len(), elapsed_ns),
            memory_bytes_estimate,
        },
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
        BackendReport {
            policy: "sketch".to_string(),
            implementation: "HashMap<region, asap_sketchlib::HyperLogLog<Classic>>".to_string(),
            groups: result.len(),
            rows: events.rows.len(),
            elapsed_ns,
            throughput_rows_per_sec: rows_per_sec(events.rows.len(), elapsed_ns),
            memory_bytes_estimate,
        },
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
    let (exact_report, exact) = run_exact_quantile(&events, column, *quantile, grouped);
    let (sketch_report, sketch) = run_kll_quantile(&events, column, *quantile, grouped);
    let accuracy = compare_grouped(&exact, &sketch);
    let sketches = vec![SketchComparison {
        sketch: sketch_report.clone(),
        accuracy: accuracy.clone(),
    }];

    Ok(AqpMvpReport {
        schema_version: 1,
        mode: "aqp_mvp".to_string(),
        task: task.clone(),
        data_source: source,
        exact: exact_report,
        sketch: sketch_report,
        accuracy,
        sketches,
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
) -> (BackendReport, HashMap<String, f64>) {
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
    let mut memory_bytes_estimate = 0u64;
    for (group, mut values) in groups {
        memory_bytes_estimate += (values.capacity() * std::mem::size_of::<f64>()) as u64;
        values.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        result.insert(group, exact_quantile_sorted(&values, quantile));
    }
    let elapsed_ns = start.elapsed().as_nanos();

    (
        BackendReport {
            policy: "exact".to_string(),
            implementation: format!("HashMap<group, Vec<{column}>> sort/type7"),
            groups: result.len(),
            rows: events.rows.len(),
            elapsed_ns,
            throughput_rows_per_sec: rows_per_sec(events.rows.len(), elapsed_ns),
            memory_bytes_estimate,
        },
        result,
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
        BackendReport {
            policy: "sketch".to_string(),
            implementation: format!("HashMap<group, asap_sketchlib::KLL<f64>(k={DEFAULT_KLL_K})>"),
            groups: result.len(),
            rows: events.rows.len(),
            elapsed_ns,
            throughput_rows_per_sec: rows_per_sec(events.rows.len(), elapsed_ns),
            memory_bytes_estimate,
        },
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
    let (cs_report, cs) = run_countsketch_frequency(&events);
    let cs_accuracy = compare_grouped(&exact, &cs);
    let sketches = vec![
        SketchComparison {
            sketch: cms_report.clone(),
            accuracy: cms_accuracy.clone(),
        },
        SketchComparison {
            sketch: cs_report,
            accuracy: cs_accuracy,
        },
    ];

    Ok(AqpMvpReport {
        schema_version: 1,
        mode: "aqp_mvp".to_string(),
        task: task.clone(),
        data_source: source,
        exact: exact_report,
        sketch: cms_report,
        accuracy: cms_accuracy,
        sketches,
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
        BackendReport {
            policy: "exact".to_string(),
            implementation: "HashMap<user_id, count>".to_string(),
            groups: result.len(),
            rows: events.rows.len(),
            elapsed_ns,
            throughput_rows_per_sec: rows_per_sec(events.rows.len(), elapsed_ns),
            memory_bytes_estimate,
        },
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
        BackendReport {
            policy: "sketch".to_string(),
            implementation: format!(
                "asap_sketchlib::CountMin<Vector2D<i32>, FastPath>({DEFAULT_FREQ_ROWS}x{DEFAULT_FREQ_COLS})"
            ),
            groups: result.len(),
            rows: events.rows.len(),
            elapsed_ns,
            throughput_rows_per_sec: rows_per_sec(events.rows.len(), elapsed_ns),
            memory_bytes_estimate,
        },
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
        BackendReport {
            policy: "sketch".to_string(),
            implementation: format!(
                "asap_sketchlib::Count<Vector2D<i32>, FastPath>({DEFAULT_FREQ_ROWS}x{DEFAULT_FREQ_COLS})"
            ),
            groups: result.len(),
            rows: events.rows.len(),
            elapsed_ns,
            throughput_rows_per_sec: rows_per_sec(events.rows.len(), elapsed_ns),
            memory_bytes_estimate,
        },
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
            "unsupported AQP plan root: {}. First slice supports COUNT(DISTINCT ...) aggregate queries.",
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
        assert_eq!(report.exact.groups, 4);
        assert_eq!(report.sketch.groups, 4);
        assert_eq!(report.accuracy.compared_groups, 4);
        assert!(report.exact.throughput_rows_per_sec > 0.0);
        assert!(report.sketch.throughput_rows_per_sec > 0.0);
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
        assert_eq!(report.exact.groups, 4);
        assert_eq!(report.sketch.groups, 4);
        assert_eq!(report.sketches.len(), 1);
        assert!(report.sketch.implementation.contains("KLL"));
        assert_eq!(report.accuracy.compared_groups, 4);
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
        assert!(report.exact.groups > 0);
        assert_eq!(report.sketches.len(), 2);
        assert!(report.sketches[0]
            .sketch
            .implementation
            .contains("CountMin"));
        assert!(report.sketches[1].sketch.implementation.contains("Count"));
        assert_eq!(report.accuracy.compared_groups, report.exact.groups);
    }

    #[test]
    fn rejects_filters_until_execution_support_exists() {
        let err =
            parse_aqp_sql("SELECT COUNT(DISTINCT user_id) AS users FROM events WHERE user_id > 10")
                .unwrap_err();
        assert!(err.to_string().contains("unsupported AQP input operator"));
    }
}
