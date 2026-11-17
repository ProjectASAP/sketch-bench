//! Translates a plan file into ClickHouse: one MergeTree table per metric,
//! one AggregatingMergeTree table and materialized view per stored window
//! chain, and one SQL query per query root, evaluated at the query parameter
//! `{t:Int64}` (epoch milliseconds).
//!
//! The translator is the capability check. A first pass, in node id order,
//! refuses every operator and field it has no rule for on the node that
//! carries it; then each root is translated from these node patterns, and any
//! other pattern is refused:
//!
//! * a stored chain of exact Sum/Count/Min/Max (per series or grouped) or
//!   Rate/Increase (per series) over an ingestion-time Scan, read by
//!   `TimeRange` + `SummaryMerge` + `FinalizeExactAccumulator`;
//! * a query-time `SummaryAgg` of the same kinds over `TimeRange` + `Scan`
//!   (Sum/Count/Min/Max also over another pattern, per series or grouped),
//!   optionally regrouped by a `SummaryMerge`, then
//!   `FinalizeExactAccumulator`;
//! * an instant `TimeRange` over a query-time `Scan`: each series' latest
//!   sample in the lookback;
//! * `Aggregate` (`sum`, `count`, `min`, `max`, `avg`, `quantile`, `top_k`)
//!   over a query-time `TimeRange` + `Scan` or over another pattern;
//! * `Project` of columns and casts, and arithmetic `BinaryOp` between two
//!   patterns with the same labels.
//!
//! Windows are right-closed, `(t − range, t]`, like the planner's. A query
//! that reads stored windows is right only at a `t` that is a window end, so
//! it fails (`throwIf`) at any other `t`;
//! [`ClickHouseDeployment::query_time_steps_ms`] gives the step `t` must be a
//! multiple of. Rate and increase follow Prometheus `extrapolatedRate`
//! (`promql/functions.go`, v3.15.1).
//!
//! A stored counter chain keeps `deltaSumTimestamp` states, which add 0 for
//! the step across a counter reset (a [`Substitution`]). Each stored row is
//! read as one time-ordered segment of one series and the segments are
//! joined in time order, so the answer does not depend on how ClickHouse
//! merges states. Each segment must be in time order: insert each series'
//! rows in time order, in time-ordered chunks, one after another.

use std::collections::BTreeMap;

use serde_json::Value;

use crate::plan_file::{
    Lifetime, Measure, PlanFile, PlanNode, PlanOperator, Predicate, RangeKind, Reduction, Source,
    SummaryFamily, Timing,
};
use crate::{NodeId, SharingLost, Substitution, TranslationError};

/// The plan's series identity column, a label column like any other. It has
/// one value per series, so it is a plain `String`: `LowCardinality` pays
/// off only below about ten thousand distinct values.
pub const SERIES_IDENTITY: &str = "$promql_series_identity";

/// The settings every `INSERT` into a source table uses: a NULL value is an
/// error instead of being stored as 0 (PromQL samples are never NULL), in
/// a `VALUES` insert and in an `INSERT … SELECT`. A JSON input format reads
/// a `null` Float64 as NaN even so, so the ingest does not use one.
pub const INSERT_SETTINGS: &str = "input_format_null_as_default = 0, insert_null_as_default = 0";

/// What a plan becomes on ClickHouse.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClickHouseDeployment {
    /// `CREATE TABLE` statements: one per metric, then one per stored chain.
    pub tables: Vec<String>,
    /// The source table of each metric, in the order of `tables`: where its
    /// samples are inserted.
    pub sources: Vec<SourceTable>,
    /// The tables of the stored chains (`v<node>`), in the order of `tables`.
    pub stored_tables: Vec<String>,
    /// `CREATE MATERIALIZED VIEW` statements, one per stored chain.
    pub materialized_views: Vec<String>,
    /// One `SELECT` per query root, in root order, with the parameter `{t:Int64}`.
    pub queries: Vec<String>,
    /// Per query, the step its evaluation time `t` must be a multiple of: the
    /// window widths of the stored chains it reads (1 when it reads none).
    pub query_time_steps_ms: Vec<u64>,
    /// How long each table's rows must be kept, from the plan's
    /// `retention_ms`. Not applied by [`Self::script`]; see
    /// [`Self::ttl_statements`].
    pub table_retention: Vec<TableRetention>,
    pub substitutions: Vec<Substitution>,
    pub sharing_lost: Vec<SharingLost>,
}

/// A metric's source table: samples of `metric` (their `__name__`) are
/// inserted into `table`, with one column per label of `label_columns`, in
/// that order, the series identity last.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceTable {
    pub metric: String,
    pub table: String,
    pub label_columns: Vec<String>,
}

/// A table's retention: rows whose time is older than `retention_ms` before
/// the newest evaluation are no longer read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TableRetention {
    pub table: String,
    /// The SQL of a row's time as a `DateTime`, e.g. `toDateTime(ts)`.
    pub row_time_sql: String,
    pub retention_ms: u64,
}

impl ClickHouseDeployment {
    /// Every statement, in order (tables, views, queries), each ending with `;`.
    pub fn script(&self) -> String {
        self.tables
            .iter()
            .chain(&self.materialized_views)
            .chain(&self.queries)
            .map(|statement| format!("{statement};\n\n"))
            .collect()
    }

    /// One `ALTER TABLE … MODIFY TTL` per retained table, rounded up to whole
    /// seconds. A TTL deletes rows older than `now()` minus the retention, so
    /// apply these only when the data's timestamps are wall-clock time:
    /// under replayed data they would delete the replayed rows at once.
    pub fn ttl_statements(&self) -> Vec<String> {
        self.table_retention
            .iter()
            .map(|retention| {
                format!(
                    "ALTER TABLE {} MODIFY TTL {} + toIntervalSecond({})",
                    retention.table,
                    retention.row_time_sql,
                    retention.retention_ms.div_ceil(1000)
                )
            })
            .collect()
    }
}

/// Translates `plan`; the first node with no rule is refused.
pub fn translate(plan: &PlanFile) -> Result<ClickHouseDeployment, TranslationError> {
    for node in &plan.nodes {
        refuse_unsupported_fields(node)?;
    }
    let mut translator = Translator {
        plan,
        chains: BTreeMap::new(),
    };
    let mut queries = Vec::new();
    let mut query_time_steps_ms = Vec::new();
    for &root in &plan.roots {
        let relation = translator.relation(root)?;
        if relation.unfinalized_kind.is_some() {
            return Err(unsupported(root, "a summary state as a query result"));
        }
        queries.push(relation.sql);
        query_time_steps_ms.push(relation.step_ms);
    }
    let (sources, mut tables): (Vec<SourceTable>, Vec<String>) =
        source_tables(plan).into_iter().unzip();
    let mut stored_tables = Vec::new();
    let mut table_retention = source_retention(plan);
    let mut materialized_views = Vec::new();
    let mut substitutions = Vec::new();
    for chain in translator.chains.values() {
        tables.push(chain.table());
        stored_tables.push(chain.table_name());
        materialized_views.push(chain.view());
        if let Some(retention_ms) = chain.retention_ms {
            table_retention.push(TableRetention {
                table: chain.table_name(),
                row_time_sql: "toDateTime(intDiv(window_end_ms, 1000))".into(),
                retention_ms,
            });
        }
        if chain.kind.is_counter() {
            substitutions.push(Substitution {
                node: chain.node,
                rule: "clickhouse.stored_rate.delta_sum_timestamp".into(),
                guarantee_difference: "a counter reset adds 0 for the step across it, where \
                                       PromQL adds the first value after the reset"
                    .into(),
            });
        }
    }
    Ok(ClickHouseDeployment {
        sources,
        stored_tables,
        tables,
        materialized_views,
        queries,
        query_time_steps_ms,
        table_retention,
        substitutions,
        sharing_lost: sharing_lost(plan),
    })
}

fn unsupported(node: NodeId, reason: impl Into<String>) -> TranslationError {
    TranslationError::UnsupportedShape {
        node,
        reason: reason.into(),
    }
}

/// The first pass: an operator or field with no rule, on the node carrying it.
fn refuse_unsupported_fields(node: &PlanNode) -> Result<(), TranslationError> {
    let refuse = |reason: &str| Err(unsupported(node.id, reason));
    if node.timing == Timing::QueryTime && node.lifetime == Lifetime::AcrossEvaluations {
        return refuse("query-time result kept across evaluations");
    }
    let refuse_without = |reduction: &Reduction| match reduction {
        Reduction::Reduce { without: true, .. } => refuse("without"),
        _ => Ok(()),
    };
    match &node.operator {
        PlanOperator::Other { kind } => refuse(&format!("operator {kind}")),
        PlanOperator::Scan { source, predicates } => {
            if *source == Source::Table {
                return refuse("table source");
            }
            for predicate in predicates {
                let Predicate::LabelEquals { column, .. } = predicate else {
                    return refuse("predicate");
                };
                let on_label = node
                    .output_columns
                    .get(*column)
                    .is_some_and(|name| node.label_columns.contains(name));
                if !on_label {
                    return refuse("predicate on a non-label column");
                }
            }
            Ok(())
        }
        PlanOperator::SummaryTumblingWindowChain {
            family,
            reduction,
            input,
            filter,
            ..
        }
        | PlanOperator::SummaryAgg {
            family,
            reduction,
            input,
            filter,
            ..
        } => {
            if filter.is_some() {
                return refuse("filter");
            }
            if *input != serde_json::json!({"item": null, "weight": {"Column": "SampleValue"}}) {
                return refuse("summary input");
            }
            refuse_without(reduction)?;
            match family {
                SummaryFamily::Exact(kind) if ExactKind::of(kind).is_some() => Ok(()),
                SummaryFamily::Exact(kind) => refuse(&format!("exact {kind}")),
                SummaryFamily::Sketch(algorithm) => refuse(&format!("sketch {algorithm}")),
            }
        }
        PlanOperator::Aggregate {
            measure_filters,
            having,
            reduction,
            ..
        } => {
            if measure_filters.iter().any(Option::is_some) {
                return refuse("filter");
            }
            if having.is_some() {
                return refuse("having");
            }
            refuse_without(reduction)
        }
        PlanOperator::BinaryOp {
            operator,
            return_bool,
            ..
        } => {
            if *return_bool {
                return refuse("bool comparison");
            }
            if !operator
                .get("vector_match")
                .is_none_or(serde_json::Value::is_null)
            {
                return refuse("vector matching");
            }
            if operator.get("checked_relative_division") == Some(&Value::Bool(true)) {
                return refuse("checked relative division");
            }
            match arithmetic_operator(operator) {
                Some(_) => Ok(()),
                None => refuse(&format!("binary operator {}", operator["kind"])),
            }
        }
        PlanOperator::SummaryEstimate { .. } => refuse("summary estimate"),
        PlanOperator::TimeRange { .. }
        | PlanOperator::SummaryMerge { .. }
        | PlanOperator::FinalizeExactAccumulator { .. }
        | PlanOperator::Project { .. } => Ok(()),
    }
}

/// The SQL operator of an arithmetic `BinaryOperator` this translator
/// supports: `{"kind": {"Arithmetic": "Div"}, ...}` is `/`.
fn arithmetic_operator(operator: &Value) -> Option<&'static str> {
    match operator.get("kind")?.get("Arithmetic")?.as_str()? {
        "Add" => Some("+"),
        "Sub" => Some("-"),
        "Mul" => Some("*"),
        "Div" => Some("/"),
        _ => None,
    }
}

/// The exact kinds this translator builds, with their ClickHouse functions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ExactKind {
    Sum,
    Count,
    Min,
    Max,
    Rate,
    Increase,
}

impl ExactKind {
    fn of(kind: &str) -> Option<ExactKind> {
        Some(match kind {
            "Sum" => ExactKind::Sum,
            "Count" => ExactKind::Count,
            "Min" => ExactKind::Min,
            "Max" => ExactKind::Max,
            "Rate" => ExactKind::Rate,
            "Increase" => ExactKind::Increase,
            _ => return None,
        })
    }

    /// The kind of an exact `family` the first pass accepted.
    fn of_checked(family: &SummaryFamily) -> ExactKind {
        match family {
            SummaryFamily::Exact(kind) => ExactKind::of(kind),
            SummaryFamily::Sketch(_) => None,
        }
        .expect("the first pass refuses every other family")
    }

    fn is_counter(self) -> bool {
        matches!(self, ExactKind::Rate | ExactKind::Increase)
    }

    /// The aggregate function of a plain kind (`None` for a counter kind).
    fn function(self) -> Option<&'static str> {
        match self {
            ExactKind::Sum => Some("sum"),
            ExactKind::Count => Some("count"),
            ExactKind::Min => Some("min"),
            ExactKind::Max => Some("max"),
            ExactKind::Rate | ExactKind::Increase => None,
        }
    }

    /// The function that merges partial values of a plain kind: a count of
    /// counts is their sum.
    fn merge_function(self) -> Option<&'static str> {
        match self {
            ExactKind::Count => Some("sum"),
            kind => kind.function(),
        }
    }
}

/// `f(value)`, or `count()`: `f`'s call over the input's values.
fn over_values(function: &str) -> String {
    match function {
        "count" => "count()".into(),
        _ => format!("{function}(value)"),
    }
}

/// One `CREATE TABLE` per metric, in order of first Scan, over the union of
/// its Scans' label columns: in name order, the series identity last.
///
/// A metric some raw-data Scan reads keeps its rows in a MergeTree. The
/// sorting key puts the labels any Scan filters on first, then the series
/// identity, then `ts`, so a label filter and a per-series read both use the
/// primary key; a `minmax` index on `ts` skips the granules outside a time
/// range. A metric only stored chains read keeps no rows (`ENGINE = Null`):
/// its materialized views still see every insert.
fn source_tables(plan: &PlanFile) -> Vec<(SourceTable, String)> {
    struct SourceColumns<'a> {
        metric: &'a str,
        labels: Vec<&'a str>,
        filtered_labels: Vec<&'a str>,
        keeps_rows: bool,
    }
    let mut sources: Vec<SourceColumns> = Vec::new();
    for node in &plan.nodes {
        let PlanOperator::Scan {
            source: Source::TimeSeries { metric },
            predicates,
        } = &node.operator
        else {
            continue;
        };
        let at = match sources.iter().position(|s| s.metric == metric) {
            Some(at) => at,
            None => {
                sources.push(SourceColumns {
                    metric,
                    labels: Vec::new(),
                    filtered_labels: Vec::new(),
                    keeps_rows: false,
                });
                sources.len() - 1
            }
        };
        let source = &mut sources[at];
        source.keeps_rows |= node.is_raw_data_scan();
        for label in &node.label_columns {
            if !source.labels.contains(&label.as_str()) {
                source.labels.push(label);
            }
        }
        for predicate in predicates {
            if let Predicate::LabelEquals { column, .. } = predicate {
                let name = node.output_columns[*column].as_str();
                if !source.filtered_labels.contains(&name) && name != SERIES_IDENTITY {
                    source.filtered_labels.push(name);
                }
            }
        }
    }
    sources
        .into_iter()
        .map(|mut source| {
            source.labels.sort_by_key(|l| (*l == SERIES_IDENTITY, *l));
            source.filtered_labels.sort_unstable();
            let columns: String = source
                .labels
                .iter()
                .map(|label| format!(", {} {}", quoted(label), label_type(label)))
                .collect();
            let table = source_table(source.metric);
            let described = SourceTable {
                metric: source.metric.to_string(),
                table: table.clone(),
                label_columns: source.labels.iter().map(|l| l.to_string()).collect(),
            };
            if !source.keeps_rows {
                return (
                    described,
                    format!(
                        "CREATE TABLE {table} (ts DateTime64(3, 'UTC'), value Float64{columns}) \
                         ENGINE = Null"
                    ),
                );
            }
            let sorting_key: Vec<String> = source
                .filtered_labels
                .iter()
                .chain(&[SERIES_IDENTITY])
                .map(|label| quoted(label))
                .chain(["ts".to_string()])
                .collect();
            (
                described,
                format!(
                    "CREATE TABLE {table} (ts DateTime64(3, 'UTC'), value Float64{columns}, \
                     INDEX ts_minmax ts TYPE minmax GRANULARITY 1) \
                     ENGINE = MergeTree ORDER BY ({})",
                    sorting_key.join(", ")
                ),
            )
        })
        .collect()
}

/// Per metric, how long its raw rows are read back, as the planner's
/// `PhysicalASAPDAG::raw_data_retention` defines it: the longest
/// `retention_ms` of its raw-data Scans (run at query time, or kept across
/// evaluations at ingestion time). A metric some such Scan reads unbounded
/// (no `retention_ms`, or a reader outside a time range or window chain)
/// keeps every row and has no entry; so has a metric read only by
/// ingestion-time chains.
fn source_retention(plan: &PlanFile) -> Vec<TableRetention> {
    let mut lookback: Vec<(String, Option<u64>)> = Vec::new();
    for node in &plan.nodes {
        let PlanOperator::Scan {
            source: Source::TimeSeries { metric },
            ..
        } = &node.operator
        else {
            continue;
        };
        if !node.is_raw_data_scan() {
            continue;
        }
        // A reader outside a time range or a window chain reads every row.
        let read_back_bounded = plan.readers_of(node.id).iter().all(|&reader| {
            matches!(
                plan.nodes[reader].operator,
                PlanOperator::TimeRange { .. } | PlanOperator::SummaryTumblingWindowChain { .. }
            )
        });
        let retention_ms = node.retention_ms.filter(|_| read_back_bounded);
        let table = source_table(metric);
        match lookback.iter_mut().find(|(t, _)| *t == table) {
            Some((_, kept)) => *kept = kept.zip(retention_ms).map(|(a, b)| a.max(b)),
            None => lookback.push((table, retention_ms)),
        }
    }
    lookback
        .into_iter()
        .filter_map(|(table, retention_ms)| {
            Some(TableRetention {
                table,
                row_time_sql: "toDateTime(ts)".into(),
                retention_ms: retention_ms?,
            })
        })
        .collect()
}

fn labels_include_identity(labels: &[String]) -> bool {
    labels.iter().any(|label| label == SERIES_IDENTITY)
}

fn label_type(label: &str) -> &'static str {
    if label == SERIES_IDENTITY {
        "String"
    } else {
        "LowCardinality(String)"
    }
}

/// Query-time nodes two or more readers share.
fn sharing_lost(plan: &PlanFile) -> Vec<SharingLost> {
    plan.nodes
        .iter()
        .filter(|node| node.timing == Timing::QueryTime)
        .filter_map(|node| {
            let readers = plan.readers_of(node.id);
            (readers.len() >= 2).then_some(SharingLost {
                node: node.id,
                readers,
            })
        })
        .collect()
}

/// A query relation: SQL whose result has the label columns, then `value`.
struct Relation {
    sql: String,
    labels: Vec<String>,
    /// The step the evaluation time must be a multiple of (1 when the
    /// relation reads no stored windows).
    step_ms: u64,
    /// `Some(kind)` when `value` is a query-time `SummaryAgg` state not yet
    /// finalized: a partial `kind` value a `SummaryMerge` may regroup.
    unfinalized_kind: Option<ExactKind>,
    /// The source tables the relation reads, sorted, e.g. `["src_latency"]`.
    source_tables: Vec<String>,
}

/// Raw samples of a metric in `(t − range, t]`, filtered by labels.
struct RawSamples {
    table: String,
    labels: Vec<String>,
    where_sql: String,
}

/// A stored window chain: its table `v<node>` and view `v<node>_mv`.
struct StoredChain {
    node: NodeId,
    kind: ExactKind,
    /// The chain's label columns: every label for a per-series chain, the
    /// group columns for a grouped one.
    labels: Vec<String>,
    width_ms: u64,
    source_table: String,
    /// The chain's Scan predicates, as SQL conditions (empty when none).
    where_sql: String,
    retention_ms: Option<u64>,
}

/// The state columns of a stored counter chain: each stored row is one
/// time-ordered segment of one series' samples.
const COUNTER_STATE_COLUMNS: &str = "s_sample_count AggregateFunction(count), \
     s_first_ms AggregateFunction(min, Int64), \
     s_last_ms AggregateFunction(max, Int64), \
     s_first_value AggregateFunction(argMin, Float64, Int64), \
     s_last_value AggregateFunction(argMax, Float64, Int64), \
     s_positive_delta_sum AggregateFunction(deltaSumTimestamp, Float64, Int64)";

impl StoredChain {
    fn table_name(&self) -> String {
        format!("v{}", self.node)
    }

    fn table(&self) -> String {
        let label_columns: String = self
            .labels
            .iter()
            .map(|label| format!("{} {}, ", quoted(label), label_type(label)))
            .collect();
        let states = match self.kind.function() {
            None => COUNTER_STATE_COLUMNS.to_string(),
            Some("count") => "state AggregateFunction(count)".to_string(),
            Some(function) => format!("state AggregateFunction({function}, Float64)"),
        };
        let sorting_key: Vec<String> = ["window_end_ms".to_string()]
            .into_iter()
            .chain(self.labels.iter().map(|l| quoted(l)))
            .collect();
        format!(
            "CREATE TABLE {} ({label_columns}window_end_ms Int64, {states}) \
             ENGINE = AggregatingMergeTree ORDER BY ({})",
            self.table_name(),
            sorting_key.join(", ")
        )
    }

    fn view(&self) -> String {
        let ts_ms = "toUnixTimestamp64Milli(ts)";
        let width = self.width_ms;
        let states = match self.kind.function() {
            None => format!(
                "countState() AS s_sample_count, minState({ts_ms}) AS s_first_ms, \
                 maxState({ts_ms}) AS s_last_ms, argMinState(value, {ts_ms}) AS s_first_value, \
                 argMaxState(value, {ts_ms}) AS s_last_value, \
                 deltaSumTimestampState(value, {ts_ms}) AS s_positive_delta_sum"
            ),
            Some("count") => "countState() AS state".to_string(),
            Some(function) => format!("{function}State(value) AS state"),
        };
        let filter = if self.where_sql.is_empty() {
            String::new()
        } else {
            format!(" WHERE {}", self.where_sql)
        };
        let window_end = format!("(intDiv({ts_ms} - 1, {width}) + 1) * {width} AS window_end_ms");
        let group: Vec<String> = self
            .labels
            .iter()
            .map(|l| quoted(l))
            .chain(["window_end_ms".to_string()])
            .collect();
        let select: Vec<String> = self
            .labels
            .iter()
            .map(|l| quoted(l))
            .chain([window_end, states])
            .collect();
        format!(
            "CREATE MATERIALIZED VIEW {table}_mv TO {table} AS SELECT {} \
             FROM {source}{filter} GROUP BY {}",
            select.join(", "),
            group.join(", "),
            table = self.table_name(),
            source = self.source_table,
        )
    }
}

struct Translator<'a> {
    plan: &'a PlanFile,
    /// The stored chains the queries read, by node.
    chains: BTreeMap<NodeId, StoredChain>,
}

impl Translator<'_> {
    fn node(&self, id: NodeId) -> &PlanNode {
        &self.plan.nodes[id]
    }

    /// The relation node `id` computes.
    fn relation(&mut self, id: NodeId) -> Result<Relation, TranslationError> {
        let plan = self.plan;
        match &plan.nodes[id].operator {
            PlanOperator::FinalizeExactAccumulator { child } => self.finalized(id, *child),
            PlanOperator::SummaryAgg {
                child,
                family,
                reduction,
                ..
            } => self.summary_agg(id, *child, ExactKind::of_checked(family), reduction),
            PlanOperator::SummaryMerge { children, group_by } => {
                let [child] = children[..] else {
                    return Err(unsupported(id, "merge of several inputs"));
                };
                if group_by.is_some()
                    && matches!(self.node(child).operator, PlanOperator::TimeRange { .. })
                {
                    return Err(unsupported(id, "regrouping merge of stored windows"));
                }
                let input = self.relation(child)?;
                let Some(kind) = input.unfinalized_kind else {
                    return Err(unsupported(id, "merge of this operator"));
                };
                match group_by {
                    None => Ok(input),
                    Some(columns) => self.regrouped(id, child, input, kind, columns),
                }
            }
            PlanOperator::Aggregate {
                child,
                measures,
                reduction,
                ..
            } => self.aggregate(id, *child, measures, reduction),
            PlanOperator::TimeRange {
                kind: RangeKind::Instant,
                ..
            } => self.latest_samples(id),
            PlanOperator::Project { child, expressions } => self.projected(id, *child, expressions),
            PlanOperator::BinaryOp {
                lhs, rhs, operator, ..
            } => self.binary(id, *lhs, *rhs, operator),
            _ => Err(unsupported(id, "this operator as a query input")),
        }
    }

    /// The relation of `id`, which must not be an unfinalized state.
    fn value_relation(&mut self, reader: NodeId, id: NodeId) -> Result<Relation, TranslationError> {
        let relation = self.relation(id)?;
        if relation.unfinalized_kind.is_some() {
            return Err(unsupported(reader, "a summary state as an input"));
        }
        Ok(relation)
    }

    /// `range`, a range `TimeRange` over a query-time time-series Scan.
    fn raw_samples(&self, range: NodeId) -> Result<RawSamples, TranslationError> {
        let PlanOperator::TimeRange {
            child: scan,
            range_ms,
            ..
        } = self.node(range).operator
        else {
            return Err(unsupported(range, "this operator over raw samples"));
        };
        let scan_node = self.node(scan);
        let PlanOperator::Scan {
            source: Source::TimeSeries { metric },
            ..
        } = &scan_node.operator
        else {
            return Err(unsupported(range, "time range over this operator"));
        };
        // Raw samples are the source table's rows, whether the plan scans
        // them at query time or keeps them at ingestion time across
        // evaluations (both are the planner's raw-data scans).
        if !scan_node.is_raw_data_scan() {
            return Err(unsupported(range, "time range over an ingestion-time scan"));
        }
        let mut conditions = self.label_conditions(scan);
        conditions.push(format!(
            "ts > fromUnixTimestamp64Milli({{t:Int64}} - {range_ms}) \
             AND ts <= fromUnixTimestamp64Milli({{t:Int64}})"
        ));
        Ok(RawSamples {
            table: source_table(metric),
            labels: self.node(range).label_columns.clone(),
            where_sql: conditions.join(" AND "),
        })
    }

    /// Whether `id` is a range `TimeRange` (its input is raw samples).
    fn is_range(&self, id: NodeId) -> bool {
        matches!(
            self.node(id).operator,
            PlanOperator::TimeRange {
                kind: RangeKind::Range,
                ..
            }
        )
    }

    /// The label-equality predicates of Scan `scan`, as SQL conditions.
    fn label_conditions(&self, scan: NodeId) -> Vec<String> {
        let scan_node = self.node(scan);
        let PlanOperator::Scan { predicates, .. } = &scan_node.operator else {
            return Vec::new();
        };
        predicates
            .iter()
            .map(|predicate| {
                let Predicate::LabelEquals { column, value } = predicate else {
                    unreachable!("the first pass refuses every other predicate");
                };
                format!(
                    "{} = {}",
                    quoted(&scan_node.output_columns[*column]),
                    string_literal(value)
                )
            })
            .collect()
    }

    /// The names of `columns` of node `of`'s output, each one of `labels`.
    fn group_labels(
        &self,
        reader: NodeId,
        of: NodeId,
        columns: &[usize],
        labels: &[String],
    ) -> Result<Vec<String>, TranslationError> {
        let names = &self.node(of).output_columns;
        columns
            .iter()
            .map(|&column| {
                names
                    .get(column)
                    .filter(|name| labels.contains(name))
                    .cloned()
                    .ok_or_else(|| unsupported(reader, "group column"))
            })
            .collect()
    }

    /// The labels a reduction keeps of input node `of`.
    fn reduced_labels(
        &self,
        reader: NodeId,
        of: NodeId,
        reduction: &Reduction,
        input_labels: &[String],
    ) -> Result<Vec<String>, TranslationError> {
        match reduction {
            Reduction::PerEntity => Ok(input_labels.to_vec()),
            Reduction::Reduce { columns, .. } => {
                self.group_labels(reader, of, columns, input_labels)
            }
        }
    }

    /// `FinalizeExactAccumulator` over a stored chain's merged windows or over
    /// a query-time state.
    fn finalized(&mut self, id: NodeId, child: NodeId) -> Result<Relation, TranslationError> {
        if let PlanOperator::SummaryMerge {
            children,
            group_by: None,
        } = &self.node(child).operator
        {
            if let [range] = children[..] {
                if let PlanOperator::TimeRange {
                    child: chain,
                    range_ms,
                    kind: RangeKind::Range,
                } = self.node(range).operator
                {
                    if matches!(
                        self.node(chain).operator,
                        PlanOperator::SummaryTumblingWindowChain { .. }
                    ) {
                        return self.stored_read(range, chain, range_ms);
                    }
                }
            }
        }
        let mut relation = self.relation(child)?;
        if relation.unfinalized_kind.take().is_none() {
            return Err(unsupported(id, "finalize over this operator"));
        }
        Ok(relation)
    }

    /// A query-time `SummaryAgg` of `kind` over `child`: raw samples when
    /// `child` is a range `TimeRange`, otherwise another relation's values.
    fn summary_agg(
        &mut self,
        id: NodeId,
        child: NodeId,
        kind: ExactKind,
        reduction: &Reduction,
    ) -> Result<Relation, TranslationError> {
        if self.node(id).timing != Timing::QueryTime {
            return Err(unsupported(id, "summary built at ingestion"));
        }
        if self.is_range(child) {
            let PlanOperator::TimeRange { range_ms, .. } = self.node(child).operator else {
                unreachable!("a range");
            };
            let raw = self.raw_samples(child)?;
            if let Some(function) = kind.function() {
                let labels = self.reduced_labels(id, child, reduction, &raw.labels)?;
                let from = format!("{} WHERE {}", raw.table, raw.where_sql);
                return Ok(Relation {
                    sql: grouped_select(&labels, &over_values(function), &from),
                    labels,
                    step_ms: 1,
                    unfinalized_kind: Some(kind),
                    source_tables: vec![raw.table],
                });
            }
            if *reduction != Reduction::PerEntity {
                return Err(unsupported(id, "grouped counter summary"));
            }
            let group = column_list(&raw.labels);
            let inner = format!(
                "SELECT {group}, \
                 arraySort(groupArray((toUnixTimestamp64Milli(ts), value))) AS points, \
                 arrayMap(p -> p.2, points) AS vals, \
                 length(points) AS n, \
                 points[1].1 AS first_ms, \
                 points[n].1 AS last_ms, \
                 vals[1] AS v_first, \
                 vals[n] - vals[1] + arraySum(arrayMap((cur, prev) -> \
                 if(cur < prev, prev, 0), arrayPopFront(vals), arrayPopBack(vals))) \
                 AS increase \
                 FROM {} WHERE {} GROUP BY {group} HAVING n >= 2",
                raw.table, raw.where_sql
            );
            return Ok(Relation {
                sql: extrapolated(&group, &inner, range_ms, kind),
                labels: raw.labels,
                step_ms: 1,
                unfinalized_kind: Some(kind),
                source_tables: vec![raw.table],
            });
        }
        let Some(function) = kind.function() else {
            return Err(unsupported(id, "counter summary over computed values"));
        };
        let input = self.value_relation(id, child)?;
        let labels = self.reduced_labels(id, child, reduction, &input.labels)?;
        Ok(Relation {
            sql: grouped_select(
                &labels,
                &over_values(function),
                &format!("( {})", input.sql),
            ),
            labels,
            step_ms: input.step_ms,
            unfinalized_kind: Some(kind),
            source_tables: input.source_tables,
        })
    }

    /// A `SummaryMerge` that regroups the query-time states of `child` by
    /// `columns` of `child`'s output.
    fn regrouped(
        &mut self,
        id: NodeId,
        child: NodeId,
        input: Relation,
        kind: ExactKind,
        columns: &[usize],
    ) -> Result<Relation, TranslationError> {
        let Some(function) = kind.merge_function() else {
            return Err(unsupported(id, "regrouping merge of a counter state"));
        };
        let labels = self.group_labels(id, child, columns, &input.labels)?;
        Ok(Relation {
            sql: grouped_select(
                &labels,
                &format!("{function}(value)"),
                &format!("( {})", input.sql),
            ),
            labels,
            step_ms: input.step_ms,
            unfinalized_kind: Some(kind),
            source_tables: input.source_tables,
        })
    }

    /// An instant `TimeRange` over a query-time Scan: each series' latest
    /// sample in `(t − lookback, t]`.
    fn latest_samples(&self, range: NodeId) -> Result<Relation, TranslationError> {
        let raw = self.raw_samples(range)?;
        let group = column_list(&raw.labels);
        Ok(Relation {
            sql: format!(
                "SELECT {group}, argMax(value, ts) AS value FROM {} WHERE {} GROUP BY {group}",
                raw.table, raw.where_sql
            ),
            labels: raw.labels,
            step_ms: 1,
            unfinalized_kind: None,
            source_tables: vec![raw.table],
        })
    }

    /// The merged windows of stored chain `chain` in `(t − range_ms, t]`,
    /// read by `TimeRange` node `range`. `t` must be a window end.
    fn stored_read(
        &mut self,
        range: NodeId,
        chain: NodeId,
        range_ms: u64,
    ) -> Result<Relation, TranslationError> {
        let stored = self.stored_chain(chain)?;
        if !range_ms.is_multiple_of(stored.width_ms) {
            return Err(unsupported(
                range,
                "a range that is not a whole number of windows",
            ));
        }
        let width = stored.width_ms;
        let table = stored.table_name();
        let group = column_list(&stored.labels);
        let window = format!(
            "window_end_ms > {{t:Int64}} - {range_ms} AND window_end_ms <= {{t:Int64}} \
             AND (SELECT throwIf({{t:Int64}} % {width} != 0, \
             'the evaluation time is not a multiple of the window width, {width} ms')) = 0"
        );
        let sql = match stored.kind.function() {
            Some(function) => grouped_select(
                &stored.labels,
                &format!("{function}Merge(state)"),
                &format!("{table} WHERE {window}"),
            ),
            None => {
                // Each stored row is one time-ordered segment of a series:
                // one insert's samples in one window, or adjacent ones a
                // merge joined. No counter state is merged across rows here,
                // because a parallel merge of non-adjacent states counts the
                // increase between them twice. A series' segments are joined
                // in time order: the step from one segment's last value into
                // the next one's first value is added when it is an
                // increase, as `deltaSumTimestamp` does within a segment, and
                // overlapping segments (rows not inserted in time order) are
                // an error. A counter chain is per series, so `group`
                // always holds the series identity.
                let inner = format!(
                    "SELECT {group}, sum(seg_sample_count) AS n, min(seg_first_ms) AS first_ms, \
                     max(seg_last_ms) AS last_ms, argMin(seg_first_value, seg_first_ms) AS v_first, \
                     sum(seg_positive_delta_sum) + sum(step_into_segment) AS increase \
                     FROM ( SELECT {group}, seg_sample_count, seg_first_ms, seg_last_ms, \
                     seg_first_value, seg_positive_delta_sum, \
                     greatest(0, seg_first_value - lagInFrame(seg_last_value, 1, seg_first_value) \
                     OVER segments) AS step_into_segment, \
                     throwIf(lagInFrame(seg_last_ms, 1, toInt64(-1)) OVER segments >= seg_first_ms, \
                     'stored segments of a series overlap in time: insert each series in time order') \
                     AS segments_overlap \
                     FROM ( SELECT {group}, \
                     finalizeAggregation(s_sample_count) AS seg_sample_count, \
                     finalizeAggregation(s_first_ms) AS seg_first_ms, \
                     finalizeAggregation(s_last_ms) AS seg_last_ms, \
                     finalizeAggregation(s_first_value) AS seg_first_value, \
                     finalizeAggregation(s_last_value) AS seg_last_value, \
                     finalizeAggregation(s_positive_delta_sum) AS seg_positive_delta_sum \
                     FROM {table} WHERE {window}) \
                     WINDOW segments AS (PARTITION BY {group} ORDER BY seg_first_ms \
                     ROWS BETWEEN UNBOUNDED PRECEDING AND CURRENT ROW)) \
                     GROUP BY {group} HAVING n >= 2 AND max(segments_overlap) = 0"
                );
                extrapolated(&group, &inner, range_ms, stored.kind)
            }
        };
        let labels = stored.labels.clone();
        let table_source = stored.source_table.clone();
        self.chains.insert(chain, stored);
        Ok(Relation {
            sql,
            labels,
            step_ms: width,
            unfinalized_kind: None,
            source_tables: vec![table_source],
        })
    }

    /// The stored chain at `chain`, if this translator supports it:
    /// ingestion-time, kept across evaluations, over an ingestion-time
    /// time-series Scan; a counter chain per series.
    fn stored_chain(&self, chain: NodeId) -> Result<StoredChain, TranslationError> {
        let node = self.node(chain);
        let PlanOperator::SummaryTumblingWindowChain {
            child,
            family,
            reduction,
            width_ms,
            ..
        } = &node.operator
        else {
            unreachable!("called on a window chain");
        };
        let kind = ExactKind::of_checked(family);
        if kind.is_counter() && *reduction != Reduction::PerEntity {
            return Err(unsupported(chain, "grouped counter window chain"));
        }
        if *width_ms == 0 {
            return Err(unsupported(chain, "window width 0"));
        }
        if node.timing != Timing::IngestionTime || node.lifetime != Lifetime::AcrossEvaluations {
            return Err(unsupported(chain, "window chain not stored at ingestion"));
        }
        let scan = self.node(*child);
        let (
            PlanOperator::Scan {
                source: Source::TimeSeries { metric },
                ..
            },
            Timing::IngestionTime,
        ) = (&scan.operator, scan.timing)
        else {
            return Err(unsupported(chain, "window chain over this operator"));
        };
        Ok(StoredChain {
            node: chain,
            kind,
            labels: node.label_columns.clone(),
            width_ms: *width_ms,
            source_table: source_table(metric),
            where_sql: self.label_conditions(*child).join(" AND "),
            retention_ms: node.retention_ms,
        })
    }

    fn aggregate(
        &mut self,
        id: NodeId,
        child: NodeId,
        measures: &[Measure],
        reduction: &Reduction,
    ) -> Result<Relation, TranslationError> {
        let [measure] = measures else {
            return Err(unsupported(id, "several measures"));
        };
        if let Some(column) = measure.column {
            if !self.node(child).is_value_column(column) {
                return Err(unsupported(id, "measure over a label column"));
            }
        }
        let (from, input_labels, step_ms, source_tables, raw) = if self.is_range(child) {
            let raw = self.raw_samples(child)?;
            (
                format!("{} WHERE {}", raw.table, raw.where_sql),
                raw.labels,
                1,
                vec![raw.table],
                true,
            )
        } else {
            let input = self.value_relation(id, child)?;
            (
                format!("( {})", input.sql),
                input.labels,
                input.step_ms,
                input.source_tables,
                false,
            )
        };
        let group = self.reduced_labels(id, child, reduction, &input_labels)?;
        if measure.kind == "top_k" {
            if raw {
                return Err(unsupported(id, "top_k over raw samples"));
            }
            let k = whole_number_argument(id, measure, "k")?;
            let order: Vec<String> = ["value DESC".to_string()]
                .into_iter()
                .chain(input_labels.iter().map(|l| quoted(l)))
                .collect();
            let by = if group.is_empty() {
                String::new()
            } else {
                format!(" BY {}", column_list(&group))
            };
            let select: Vec<String> = input_labels
                .iter()
                .map(|l| quoted(l))
                .chain(["value".to_string()])
                .collect();
            return Ok(Relation {
                sql: format!(
                    "SELECT {} FROM {from} ORDER BY {} LIMIT {k}{by}",
                    select.join(", "),
                    order.join(", ")
                ),
                labels: input_labels,
                step_ms,
                unfinalized_kind: None,
                source_tables,
            });
        }
        let value = aggregate_value(id, measure)?;
        Ok(Relation {
            sql: grouped_select(&group, &value, &from),
            labels: group,
            step_ms,
            unfinalized_kind: None,
            source_tables,
        })
    }

    /// `Project`: each output column is a child label, kept or renamed, or
    /// the child's value, as is or cast to `Float64`.
    fn projected(
        &mut self,
        id: NodeId,
        child: NodeId,
        expressions: &[Value],
    ) -> Result<Relation, TranslationError> {
        let input = self.value_relation(id, child)?;
        let node = self.node(id);
        let child_node = self.node(child);
        let mut select = Vec::new();
        let mut labels = Vec::new();
        let mut value = None;
        for (output, expression) in node.output_columns.iter().zip(expressions) {
            let (column, cast) = match expression {
                Value::Object(map) if map.contains_key("Column") => (&map["Column"], false),
                Value::Object(map) if map.contains_key("Cast") => {
                    let cast = &map["Cast"];
                    if cast.get("to").and_then(Value::as_str) != Some("float64")
                        || cast.get("try_cast") != Some(&Value::Bool(false))
                    {
                        return Err(unsupported(id, "projected expression"));
                    }
                    let column = cast
                        .get("expr")
                        .and_then(|e| e.get("Column"))
                        .ok_or_else(|| unsupported(id, "projected expression"))?;
                    (column, true)
                }
                _ => return Err(unsupported(id, "projected expression")),
            };
            let source = column
                .as_u64()
                .and_then(|c| child_node.output_columns.get(c as usize))
                .ok_or_else(|| unsupported(id, "projected column"))?;
            if node.label_columns.contains(output) {
                if cast || !input.labels.contains(source) {
                    return Err(unsupported(id, "projected label"));
                }
                select.push(format!("{} AS {}", quoted(source), quoted(output)));
                labels.push(output.clone());
            } else {
                let is_value = column
                    .as_u64()
                    .is_some_and(|c| child_node.is_value_column(c as usize));
                if value.is_some() || !is_value {
                    return Err(unsupported(id, "projected value"));
                }
                let sql = if cast { "toFloat64(value)" } else { "value" };
                value = Some(format!("{sql} AS value"));
            }
        }
        let value = value.ok_or_else(|| unsupported(id, "projection without a value"))?;
        // Dropping a label would leave several rows per label set.
        if labels.len() != input.labels.len() {
            return Err(unsupported(id, "projection that drops a label"));
        }
        select.push(value);
        Ok(Relation {
            sql: format!("SELECT {} FROM ( {})", select.join(", "), input.sql),
            labels,
            step_ms: input.step_ms,
            unfinalized_kind: None,
            source_tables: input.source_tables,
        })
    }

    /// Arithmetic between the series of `lhs` and `rhs` that have the same
    /// labels (PromQL one-to-one matching on all labels).
    fn binary(
        &mut self,
        id: NodeId,
        lhs: NodeId,
        rhs: NodeId,
        operator: &Value,
    ) -> Result<Relation, TranslationError> {
        let symbol = arithmetic_operator(operator).expect("checked in the first pass");
        let left = self.value_relation(id, lhs)?;
        let right = self.value_relation(id, rhs)?;
        let mut left_set = left.labels.clone();
        let mut right_set = right.labels.clone();
        left_set.sort();
        right_set.sort();
        if left_set != right_set {
            return Err(unsupported(id, "binary operation on different label sets"));
        }
        // The series identity names the metric, so series of two metrics
        // never match on it, where PromQL matches them without `__name__`.
        if labels_include_identity(&left.labels) && left.source_tables != right.source_tables {
            return Err(unsupported(
                id,
                "binary operation across metrics per series",
            ));
        }
        let mut source_tables = left.source_tables.clone();
        source_tables.extend(right.source_tables.iter().cloned());
        source_tables.sort();
        source_tables.dedup();
        let labels = left.labels;
        let side = |sql: &str, name: &str, side: &str| {
            let select: Vec<String> = labels
                .iter()
                .map(|l| quoted(l))
                .chain([format!("value AS {name}")])
                .collect();
            format!("( SELECT {} FROM ( {sql})) AS {side}", select.join(", "))
        };
        // Each side has one row per label set (every relation groups by its
        // labels), so the join is one-to-one, as PromQL requires.
        let (join_kind, using) = if labels.is_empty() {
            ("CROSS JOIN", String::new())
        } else {
            ("INNER JOIN", format!(" USING ({})", column_list(&labels)))
        };
        let result = format!("toFloat64(lhs_value) {symbol} toFloat64(rhs_value)");
        let select: Vec<String> = labels
            .iter()
            .map(|l| quoted(l))
            .chain([format!("{result} AS value")])
            .collect();
        // A checked finite division (e.g. avg as sum / count) needs finite
        // operands and a finite quotient; otherwise (e.g. a sum that
        // overflowed) the planner's executor answers the original query
        // exactly, which this translation cannot, so it is an error.
        let checked = if operator.get("checked_finite_division") == Some(&Value::Bool(true)) {
            format!(
                " WHERE throwIf(NOT (isFinite(toFloat64(lhs_value)) AND isFinite(toFloat64(rhs_value)) \
                 AND isFinite({result})), 'a checked division has a value that is not finite') = 0"
            )
        } else {
            String::new()
        };
        Ok(Relation {
            sql: format!(
                "SELECT {} FROM {} {join_kind} {}{using}{checked}",
                select.join(", "),
                side(&left.sql, "lhs_value", "lhs"),
                side(&right.sql, "rhs_value", "rhs"),
            ),
            labels,
            step_ms: lcm(left.step_ms, right.step_ms),
            unfinalized_kind: None,
            source_tables,
        })
    }
}

/// `SELECT <labels>, <value> AS value FROM <from> GROUP BY <labels>`; with no
/// labels, no GROUP BY and `HAVING count() > 0`, so that an empty input
/// gives no row (PromQL gives no series), not one row of 0.
fn grouped_select(labels: &[String], value: &str, from: &str) -> String {
    if labels.is_empty() {
        format!("SELECT {value} AS value FROM {from} HAVING count() > 0")
    } else {
        let group = column_list(labels);
        format!("SELECT {group}, {value} AS value FROM {from} GROUP BY {group}")
    }
}

/// A whole-number argument of a measure, e.g. `k` of `top_k`.
fn whole_number_argument(
    id: NodeId,
    measure: &Measure,
    name: &str,
) -> Result<u64, TranslationError> {
    if measure.arguments.keys().any(|key| key != name) {
        return Err(unsupported(id, format!("measure {}", measure.kind)));
    }
    measure
        .arguments
        .get(name)
        .and_then(Value::as_u64)
        .ok_or_else(|| unsupported(id, format!("{} {name}", measure.kind)))
}

/// The SQL of one Aggregate measure over the input's `value`.
fn aggregate_value(id: NodeId, measure: &Measure) -> Result<String, TranslationError> {
    let extra_arguments = |allowed: &[&str]| {
        measure
            .arguments
            .keys()
            .any(|key| !allowed.contains(&key.as_str()))
    };
    match measure.kind.as_str() {
        "sum" | "count" | "min" | "max" | "avg" if !extra_arguments(&[]) => {
            Ok(over_values(&measure.kind))
        }
        "quantile" if !extra_arguments(&["q"]) => {
            let q = measure
                .arguments
                .get("q")
                .and_then(Value::as_f64)
                .filter(|q| (0.0..=1.0).contains(q))
                .ok_or_else(|| unsupported(id, "quantile level"))?;
            Ok(format!("quantileExactInclusive({q})(value)"))
        }
        kind => Err(unsupported(id, format!("measure {kind}"))),
    }
}

/// Prometheus `extrapolatedRate` (`promql/functions.go`, v3.15.1) over
/// `inner`, which gives per series `n`, `first_ms`, `last_ms`, `v_first` and
/// the reset-corrected `increase` of the samples in `(t − range_ms, t]`
/// (at least 2): the 1.1 × average-gap threshold first, then the zero-point
/// clamp; divided by the range in seconds for a rate.
fn extrapolated(group: &str, inner: &str, range_ms: u64, kind: ExactKind) -> String {
    let per_second = if kind == ExactKind::Rate {
        format!(" / {}", ms_as_seconds_literal(range_ms))
    } else {
        String::new()
    };
    format!(
        "SELECT {group}, increase * (sampled + least(d_start, d_zero) + d_end) / sampled{per_second} \
         AS value FROM ( SELECT {group}, increase, (last_ms - first_ms) / 1000 AS sampled, \
         sampled / (n - 1) AS avg_gap, \
         if((first_ms - ({{t:Int64}} - {range_ms})) / 1000 >= 1.1 * avg_gap, avg_gap / 2, \
         (first_ms - ({{t:Int64}} - {range_ms})) / 1000) AS d_start, \
         if(increase > 0 AND v_first >= 0, sampled * v_first / increase, d_start) AS d_zero, \
         if(({{t:Int64}} - last_ms) / 1000 >= 1.1 * avg_gap, avg_gap / 2, \
         ({{t:Int64}} - last_ms) / 1000) AS d_end \
         FROM ( {inner}))"
    )
}

/// `300000` ms as `300`; a fraction of a second keeps its decimals.
fn ms_as_seconds_literal(ms: u64) -> String {
    if ms.is_multiple_of(1000) {
        (ms / 1000).to_string()
    } else {
        format!("{}", ms as f64 / 1000.0)
    }
}

fn lcm(a: u64, b: u64) -> u64 {
    let (mut x, mut y) = (a, b);
    while y != 0 {
        (x, y) = (y, x % y);
    }
    a / x * b
}

fn source_table(metric: &str) -> String {
    let name: String = metric
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    format!("src_{name}")
}

fn column_list(labels: &[String]) -> String {
    labels
        .iter()
        .map(|l| quoted(l))
        .collect::<Vec<_>>()
        .join(", ")
}

pub(crate) fn quoted(identifier: &str) -> String {
    format!("`{}`", identifier.replace('\\', "\\\\").replace('`', "\\`"))
}

fn string_literal(value: &str) -> String {
    format!("'{}'", value.replace('\\', "\\\\").replace('\'', "\\'"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seconds_keep_fractions() {
        assert_eq!(ms_as_seconds_literal(300_000), "300");
        assert_eq!(ms_as_seconds_literal(1_500), "1.5");
    }

    #[test]
    fn identifiers_and_literals_are_escaped() {
        assert_eq!(quoted("a`b"), "`a\\`b`");
        assert_eq!(string_literal("it's"), "'it\\'s'");
    }

    #[test]
    fn lcm_of_window_widths() {
        assert_eq!(lcm(1, 60_000), 60_000);
        assert_eq!(lcm(60_000, 90_000), 180_000);
    }
}
