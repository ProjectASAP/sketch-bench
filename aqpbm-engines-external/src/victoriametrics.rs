//! Translates a plan file into VictoriaMetrics: a stream-aggregation config
//! (`-streamAggr.config`) for the stored window chains, and one MetricsQL
//! instant query per query root.
//!
//! The translator is the capability check, as for ClickHouse. A first pass,
//! in node id order, refuses every operator and field it has no rule for on
//! the node that carries it; then each root is translated from these node
//! patterns, and any other pattern is refused:
//!
//! * a stored chain of exact Sum / Count / Min / Max over an ingestion-time
//!   Scan becomes one stream-aggregation output (`sum_samples`,
//!   `count_samples`, `min`, `max`) with `enable_windows: true`, read by
//!   `TimeRange` + `SummaryMerge` + `FinalizeExactAccumulator` as
//!   `sum_over_time(<output>[W])` (Sum, Count), `min_over_time`,
//!   `max_over_time`. A per-series chain has no `by` and no `without`, so
//!   every input label is kept (one output series per input series); a
//!   grouped chain has `by: [<labels>]`; a chain grouped by no label is the
//!   per-series form plus the outer aggregate in the query, because an empty
//!   `by` would not merge series;
//! * a query-time `SummaryAgg` or `Aggregate` over a raw-sample range
//!   becomes the rollup `<f>_over_time(<selector>[W])`, `rate`, `increase`
//!   or `quantile_over_time`; an aggregate over another pattern becomes
//!   `sum` / `count` / `min` / `max` / `avg` / `quantile` / `topk`
//!   `by (<labels>)`;
//! * an instant `TimeRange` with a lookback is `last_over_time`;
//! * `Project` of columns and casts, and arithmetic `BinaryOp` between two
//!   patterns with the same labels.
//!
//! Windows are right-closed, `(t − W, t]`, like the planner's, and so are
//! MetricsQL rollup windows. A stream window `(e − w, e]` is flushed at the
//! wall-clock tick `e` plus a lag; single-node VictoriaMetrics stores the
//! output at the flush time in whole seconds, so at a time in
//! `[e − 1 s, e + lag]`. A stored read at a window end `t` therefore reads
//! `<output>[W] offset -h` with `h = (w − 1 s) / 2`: the output times in
//! `(t − W + h, t + h]`, which are exactly those of the windows ending in
//! `(t − W, t]` while the flush lag stays below `h` (the adapter checks it).
//! Example: `w` = 60 s, `W` = 300 s: `max_over_time(m:60s_max[300s] offset
//! -29500ms)`. A `sum_samples` output also writes 0 for an empty window
//! while its series is not yet stale, where the planner has no row, so a
//! Sum read keeps only the series that `count_samples` (which skips empty
//! windows) has in the range: `sum_over_time(s[W] offset -h) and
//! count_over_time(c[W] offset -h)`.
//!
//! Stream aggregation runs on wall-clock time and aligns its windows to
//! Go's zero time, which agrees with the Unix epoch only for widths that
//! divide a day: other widths are refused. See the capability README for
//! the run contract.

use std::collections::BTreeMap;

use serde_json::Value;

use crate::plan_file::{
    Lifetime, Measure, PlanFile, PlanNode, PlanOperator, Predicate, RangeKind, Reduction, Source,
    SummaryFamily, Timing,
};
use crate::{NodeId, SharingLost, Substitution, TranslationError};

/// The plan's series identity column: a label of the plan, but on
/// VictoriaMetrics a series is identified by its whole label set.
const SERIES_IDENTITY: &str = "$promql_series_identity";

/// One day: stream-aggregation windows align to Go's zero time, which is a
/// whole number of days before the Unix epoch.
const DAY_MS: u64 = 86_400_000;

/// What a plan becomes on VictoriaMetrics.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VictoriaMetricsDeployment {
    /// One stream aggregation per (selector, interval, grouping), each with
    /// the outputs the plan's stored chains need, in order of first read.
    pub stream_aggregations: Vec<StreamAggregation>,
    /// One query per query root, in root order.
    pub queries: Vec<MetricsQlQuery>,
    pub substitutions: Vec<Substitution>,
    pub sharing_lost: Vec<SharingLost>,
}

/// One entry of the stream-aggregation config, e.g. `latency{job="j3"}` every
/// 60 s, per series, with the output `max`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StreamAggregation {
    /// The input metric, e.g. `latency`.
    pub metric: String,
    /// The `match` series selector, e.g. `latency{job="j3"}`.
    pub selector: String,
    pub interval_ms: u64,
    /// `None`: per series (no `by`, no `without`: every input label is
    /// kept). `Some(labels)`: `by: [labels]`, sorted, never empty.
    pub by: Option<Vec<String>>,
    pub outputs: Vec<StreamOutput>,
}

/// A stream-aggregation output this translator uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StreamOutput {
    SumSamples,
    CountSamples,
    Min,
    Max,
}

/// One query: a MetricsQL instant query evaluated at the evaluation time,
/// sent with `nocache=1` (see the capability README's run contract).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MetricsQlQuery {
    pub expr: String,
    /// The evaluation time must be a multiple of this, in ms: the window
    /// widths of the stream outputs it reads, or 1 when it reads none.
    pub time_step_ms: u64,
    /// `Some(metric)` when each answer series is one input series of
    /// `metric` (the plan keeps the series identity); `None` when the answer
    /// is grouped by its labels.
    pub per_series_of: Option<String>,
}

impl StreamOutput {
    /// The output's name in the config and in the output metric, e.g. `max`.
    pub fn name(self) -> &'static str {
        match self {
            StreamOutput::SumSamples => "sum_samples",
            StreamOutput::CountSamples => "count_samples",
            StreamOutput::Min => "min",
            StreamOutput::Max => "max",
        }
    }

    fn of(kind: ExactKind) -> StreamOutput {
        match kind {
            ExactKind::Sum => StreamOutput::SumSamples,
            ExactKind::Count => StreamOutput::CountSamples,
            ExactKind::Min => StreamOutput::Min,
            ExactKind::Max => StreamOutput::Max,
            ExactKind::Rate | ExactKind::Increase => unreachable!("no stored counter output"),
        }
    }

    /// The exact kind one window of this output holds.
    fn kind(self) -> ExactKind {
        match self {
            StreamOutput::SumSamples => ExactKind::Sum,
            StreamOutput::CountSamples => ExactKind::Count,
            StreamOutput::Min => ExactKind::Min,
            StreamOutput::Max => ExactKind::Max,
        }
    }

    /// The rollup that merges this output's windows: the sum of sums or of
    /// counts, the min of mins, the max of maxes.
    fn merge_rollup(self) -> &'static str {
        match self {
            StreamOutput::SumSamples | StreamOutput::CountSamples => "sum_over_time",
            StreamOutput::Min => "min_over_time",
            StreamOutput::Max => "max_over_time",
        }
    }

    /// The outputs a chain of `kind` stores: a Sum also stores its counts,
    /// which tell an empty window (no count) from a window that sums to 0.
    fn stored_for(kind: ExactKind) -> Vec<StreamOutput> {
        match kind {
            ExactKind::Sum => vec![StreamOutput::SumSamples, StreamOutput::CountSamples],
            kind => vec![StreamOutput::of(kind)],
        }
    }

    /// The aggregate that merges this output across series.
    fn merge_aggregate(self) -> &'static str {
        self.kind()
            .merge_aggregate()
            .expect("a stored output is of a plain kind")
    }
}

impl StreamAggregation {
    /// The metric name of `output`, as VictoriaMetrics names it:
    /// `<metric>:<interval>[_by_<labels>]_<output>`, e.g.
    /// `latency:60s_by_job_sum_samples`.
    pub fn output_metric(&self, output: StreamOutput) -> String {
        let by = match &self.by {
            Some(labels) => format!("_by_{}", labels.join("_")),
            None => String::new(),
        };
        format!(
            "{}:{}{by}_{}",
            self.metric,
            duration(self.interval_ms),
            output.name()
        )
    }

    /// The recording-rule expression of one window of `output`, e.g.
    /// `sum by (job) (sum_over_time(latency[60s]))`.
    fn window_expr(&self, output: StreamOutput) -> String {
        let rollup = format!(
            "{}({}[{}])",
            output.kind().rollup(),
            self.selector,
            duration(self.interval_ms)
        );
        match &self.by {
            None => rollup,
            Some(labels) => aggregate_by(output.merge_aggregate(), labels, &rollup),
        }
    }
}

impl VictoriaMetricsDeployment {
    /// The `-streamAggr.config` YAML: one list entry per stream aggregation;
    /// `[]` when there is none.
    pub fn stream_aggregation_config(&self) -> String {
        if self.stream_aggregations.is_empty() {
            return "[]\n".into();
        }
        let mut yaml = String::new();
        for aggregation in &self.stream_aggregations {
            yaml.push_str(&format!(
                "- match: {}\n  interval: {}\n",
                yaml_quoted(&aggregation.selector),
                duration(aggregation.interval_ms)
            ));
            if let Some(labels) = &aggregation.by {
                let labels: Vec<String> = labels.iter().map(|l| yaml_quoted(l)).collect();
                yaml.push_str(&format!("  by: [{}]\n", labels.join(", ")));
            }
            let outputs: Vec<&str> = aggregation.outputs.iter().map(|o| o.name()).collect();
            yaml.push_str(&format!(
                "  outputs: [{}]\n  enable_windows: true\n  ignore_first_intervals: 0\n  \
                 flush_on_shutdown: false\n",
                outputs.join(", ")
            ));
        }
        yaml
    }

    /// The same outputs as vmalert recording rules: one group per interval,
    /// evaluated at aligned times, each rule recording one output's window
    /// under the output's name. This is the cross-check form: it needs no
    /// real time.
    pub fn recording_rules(&self) -> String {
        let mut by_interval: BTreeMap<u64, Vec<&StreamAggregation>> = BTreeMap::new();
        for aggregation in &self.stream_aggregations {
            by_interval
                .entry(aggregation.interval_ms)
                .or_default()
                .push(aggregation);
        }
        if by_interval.is_empty() {
            return "groups: []\n".into();
        }
        let mut yaml = String::from("groups:\n");
        for (interval_ms, aggregations) in by_interval {
            let interval = duration(interval_ms);
            yaml.push_str(&format!(
                "  - name: stream-aggregation-{interval}\n    interval: {interval}\n    \
                 eval_alignment: true\n    eval_delay: 0s\n    rules:\n"
            ));
            for aggregation in aggregations {
                for &output in &aggregation.outputs {
                    yaml.push_str(&format!(
                        "      - record: {}\n        expr: {}\n",
                        aggregation.output_metric(output),
                        yaml_quoted(&aggregation.window_expr(output))
                    ));
                }
            }
        }
        yaml
    }
}

/// Translates `plan`; the first node with no rule is refused.
pub fn translate(plan: &PlanFile) -> Result<VictoriaMetricsDeployment, TranslationError> {
    for node in &plan.nodes {
        refuse_unsupported_fields(node)?;
    }
    let mut translator = Translator {
        plan,
        stream_aggregations: Vec::new(),
        substitutions: BTreeMap::new(),
    };
    let mut queries = Vec::new();
    for &root in &plan.roots {
        let relation = translator.relation(root)?;
        if relation.unfinalized_kind.is_some() {
            return Err(unsupported(root, "a summary state as a query result"));
        }
        queries.push(MetricsQlQuery {
            expr: relation.expr,
            time_step_ms: relation.step_ms,
            per_series_of: relation.per_series_of,
        });
    }
    Ok(VictoriaMetricsDeployment {
        stream_aggregations: translator.stream_aggregations,
        queries,
        substitutions: translator.substitutions.into_values().collect(),
        sharing_lost: SharingLost::of_plan(plan),
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
            let Source::TimeSeries { metric } = source else {
                return refuse("table source");
            };
            if !is_metric_name(metric) {
                return refuse(&format!("metric name {metric}"));
            }
            for predicate in predicates {
                let Predicate::LabelEquals { column, .. } = predicate else {
                    return refuse("predicate");
                };
                let Some(name) = node
                    .output_columns
                    .get(*column)
                    .filter(|name| node.label_columns.contains(name))
                else {
                    return refuse("predicate on a non-label column");
                };
                if name == SERIES_IDENTITY {
                    return refuse("predicate on the series identity");
                }
                if !is_label_name(name) {
                    return refuse(&format!("label name {name}"));
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
            if !operator.get("vector_match").is_none_or(Value::is_null) {
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

/// The MetricsQL operator of an arithmetic `BinaryOperator`:
/// `{"kind": {"Arithmetic": "Div"}, ...}` is `/`.
fn arithmetic_operator(operator: &Value) -> Option<&'static str> {
    match operator.get("kind")?.get("Arithmetic")?.as_str()? {
        "Add" => Some("+"),
        "Sub" => Some("-"),
        "Mul" => Some("*"),
        "Div" => Some("/"),
        _ => None,
    }
}

/// The exact kinds this translator reads.
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

    /// The rollup over one series' raw samples in a range.
    fn rollup(self) -> &'static str {
        match self {
            ExactKind::Sum => "sum_over_time",
            ExactKind::Count => "count_over_time",
            ExactKind::Min => "min_over_time",
            ExactKind::Max => "max_over_time",
            ExactKind::Rate => "rate",
            ExactKind::Increase => "increase",
        }
    }

    /// The aggregate over raw values of a plain kind (`None` for a counter).
    fn aggregate(self) -> Option<&'static str> {
        match self {
            ExactKind::Sum => Some("sum"),
            ExactKind::Count => Some("count"),
            ExactKind::Min => Some("min"),
            ExactKind::Max => Some("max"),
            ExactKind::Rate | ExactKind::Increase => None,
        }
    }

    /// The aggregate that merges partial values of a plain kind: a count of
    /// counts is their sum.
    fn merge_aggregate(self) -> Option<&'static str> {
        match self {
            ExactKind::Count => Some("sum"),
            kind => kind.aggregate(),
        }
    }
}

/// A query relation: a MetricsQL expression whose series carry `labels`.
struct Relation {
    expr: String,
    /// The plan's label columns of the result, e.g. `["job"]`, or every
    /// label plus the series identity for a per-series result.
    labels: Vec<String>,
    /// The step the evaluation time must be a multiple of (1 when the
    /// relation reads no stream output).
    step_ms: u64,
    /// `Some(kind)` when the result is a query-time `SummaryAgg` state not
    /// yet finalized: a partial `kind` value a `SummaryMerge` may regroup.
    unfinalized_kind: Option<ExactKind>,
    /// `Some(metric)` when each result series is one input series of `metric`.
    per_series_of: Option<String>,
}

/// Raw samples of a metric in a range, filtered by labels.
struct RawSamples {
    metric: String,
    /// `metric{label="value",...}`.
    selector: String,
    labels: Vec<String>,
    range_ms: u64,
}

/// A stored window chain as one stream output.
struct StoredChain {
    kind: ExactKind,
    metric: String,
    selector: String,
    width_ms: u64,
    grouping: ChainGrouping,
    /// The chain's label columns (every label plus the identity per series).
    labels: Vec<String>,
}

enum ChainGrouping {
    PerSeries,
    /// Non-empty, sorted.
    By(Vec<String>),
    /// Grouped by no label: the per-series output plus the outer aggregate.
    AllSeries,
}

struct Translator<'a> {
    plan: &'a PlanFile,
    stream_aggregations: Vec<StreamAggregation>,
    /// By node, so a node several roots read is recorded once.
    substitutions: BTreeMap<NodeId, Substitution>,
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

    /// `range`, a `TimeRange` over a raw-data time-series Scan.
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
        // The raw series, kept by `-streamAggr.keepInput`, whether the plan
        // scans them at query time or keeps them at ingestion time.
        if !scan_node.is_raw_data_scan() {
            return Err(unsupported(range, "time range over an ingestion-time scan"));
        }
        Ok(RawSamples {
            metric: metric.clone(),
            selector: self.selector(scan),
            labels: self.node(range).label_columns.clone(),
            range_ms,
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

    /// The series selector of Scan `scan`: its metric and label matchers,
    /// e.g. `latency{job="j3"}`.
    fn selector(&self, scan: NodeId) -> String {
        let scan_node = self.node(scan);
        let PlanOperator::Scan {
            source: Source::TimeSeries { metric },
            predicates,
        } = &scan_node.operator
        else {
            unreachable!("called on a time-series Scan");
        };
        if predicates.is_empty() {
            return metric.clone();
        }
        let mut matchers: Vec<String> = predicates
            .iter()
            .map(|predicate| {
                let Predicate::LabelEquals { column, value } = predicate else {
                    unreachable!("the first pass refuses every other predicate");
                };
                format!(
                    "{}={}",
                    scan_node.output_columns[*column],
                    string_literal(value)
                )
            })
            .collect();
        matchers.sort();
        format!("{metric}{{{}}}", matchers.join(","))
    }

    /// The names of `columns` of node `of`'s output, each one of `labels`
    /// and never the series identity.
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
                let name = names
                    .get(column)
                    .filter(|name| labels.contains(name))
                    .ok_or_else(|| unsupported(reader, "group column"))?;
                if name == SERIES_IDENTITY {
                    return Err(unsupported(reader, "group by the series identity"));
                }
                if !is_label_name(name) {
                    return Err(unsupported(reader, format!("label name {name}")));
                }
                Ok(name.clone())
            })
            .collect()
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
            let raw = self.raw_samples(child)?;
            let rollup = format!(
                "{}({}[{}])",
                kind.rollup(),
                raw.selector,
                duration(raw.range_ms)
            );
            if kind.is_counter() {
                self.substitutions.insert(
                    id,
                    Substitution {
                        node: id,
                        rule: "victoriametrics.rate_increase.no_extrapolation".into(),
                        guarantee_difference: "no extrapolation to the range's ends: the \
                                               increase runs from the last sample before the \
                                               range to the last sample in it, and `rate` \
                                               divides it by the time between those two \
                                               samples; with no sample before the range, \
                                               `increase` counts from 0 and `rate` uses the \
                                               first sample in it (no answer from one sample); \
                                               an earlier sample and none in the range gives \
                                               0; a drop smaller than prev/8 adds only \
                                               prev - v as a reset"
                            .into(),
                    },
                );
            }
            return match reduction {
                Reduction::PerEntity => Ok(Relation {
                    expr: rollup,
                    labels: raw.labels,
                    step_ms: 1,
                    unfinalized_kind: Some(kind),
                    per_series_of: Some(raw.metric),
                }),
                Reduction::Reduce { columns, .. } => {
                    let Some(merge) = kind.merge_aggregate() else {
                        return Err(unsupported(id, "grouped counter summary"));
                    };
                    let labels = self.group_labels(id, child, columns, &raw.labels)?;
                    Ok(Relation {
                        expr: aggregate_by(merge, &labels, &rollup),
                        labels,
                        step_ms: 1,
                        unfinalized_kind: Some(kind),
                        per_series_of: None,
                    })
                }
            };
        }
        let Some(function) = kind.aggregate() else {
            return Err(unsupported(id, "counter summary over computed values"));
        };
        let Reduction::Reduce { columns, .. } = reduction else {
            return Err(unsupported(id, "per-series summary over computed values"));
        };
        let input = self.value_relation(id, child)?;
        let labels = self.group_labels(id, child, columns, &input.labels)?;
        Ok(Relation {
            expr: aggregate_by(function, &labels, &input.expr),
            labels,
            step_ms: input.step_ms,
            unfinalized_kind: Some(kind),
            per_series_of: None,
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
        let Some(merge) = kind.merge_aggregate() else {
            return Err(unsupported(id, "regrouping merge of a counter state"));
        };
        let labels = self.group_labels(id, child, columns, &input.labels)?;
        Ok(Relation {
            expr: aggregate_by(merge, &labels, &input.expr),
            labels,
            step_ms: input.step_ms,
            unfinalized_kind: Some(kind),
            per_series_of: None,
        })
    }

    /// An instant `TimeRange` over a raw-data Scan: each series' latest
    /// sample in `(t − lookback, t]`.
    fn latest_samples(&self, range: NodeId) -> Result<Relation, TranslationError> {
        let raw = self.raw_samples(range)?;
        if raw.range_ms == 0 {
            return Err(unsupported(range, "instant selector without a lookback"));
        }
        Ok(Relation {
            expr: format!(
                "last_over_time({}[{}])",
                raw.selector,
                duration(raw.range_ms)
            ),
            labels: raw.labels,
            step_ms: 1,
            unfinalized_kind: None,
            per_series_of: Some(raw.metric),
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
        let by = match &stored.grouping {
            ChainGrouping::By(labels) => Some(labels.clone()),
            ChainGrouping::PerSeries | ChainGrouping::AllSeries => None,
        };
        let mut output_metrics = Vec::new();
        for output in StreamOutput::stored_for(stored.kind) {
            output_metrics.push(self.add_stream_output(chain, &stored, output, by.clone())?);
        }
        // Each output's times are in `[e − 1 s, e + lag]` for a window ending
        // at `e`; see the module docs.
        let shift = duration((stored.width_ms - 1000) / 2);
        let range = duration(range_ms);
        let read =
            |rollup: &str, metric: &str| format!("{rollup}({metric}[{range}] offset -{shift})");
        let rollup = match &output_metrics[..] {
            [sums, counts] => format!(
                "{} and {}",
                read("sum_over_time", sums),
                read("count_over_time", counts)
            ),
            [single] => read(StreamOutput::of(stored.kind).merge_rollup(), single),
            _ => unreachable!("one or two outputs"),
        };
        let merge = StreamOutput::of(stored.kind).merge_aggregate();
        let (expr, labels, per_series_of) = match stored.grouping {
            ChainGrouping::PerSeries => (rollup, stored.labels, Some(stored.metric)),
            ChainGrouping::By(labels) => (rollup, labels, None),
            ChainGrouping::AllSeries => (format!("{merge}({rollup})"), Vec::new(), None),
        };
        Ok(Relation {
            expr,
            labels,
            step_ms: stored.width_ms,
            unfinalized_kind: None,
            per_series_of,
        })
    }

    /// Adds `output` of `stored` to its stream aggregation (one per
    /// selector, interval and grouping) and returns the output metric. An
    /// output whose name another aggregation already writes is refused:
    /// both would write one series, e.g. `by [a_b]` and `by [a, b]`.
    fn add_stream_output(
        &mut self,
        chain: NodeId,
        stored: &StoredChain,
        output: StreamOutput,
        by: Option<Vec<String>>,
    ) -> Result<String, TranslationError> {
        let candidate = StreamAggregation {
            metric: stored.metric.clone(),
            selector: stored.selector.clone(),
            interval_ms: stored.width_ms,
            by,
            outputs: Vec::new(),
        };
        let output_metric = candidate.output_metric(output);
        let same_entry = |a: &StreamAggregation| {
            a.metric == candidate.metric
                && a.selector == candidate.selector
                && a.interval_ms == candidate.interval_ms
                && a.by == candidate.by
        };
        if self.stream_aggregations.iter().any(|a| {
            !same_entry(a)
                && a.outputs
                    .iter()
                    .any(|&o| a.output_metric(o) == output_metric)
        }) {
            return Err(unsupported(
                chain,
                format!("two stored chains with the output {output_metric}"),
            ));
        }
        let at = match self.stream_aggregations.iter().position(same_entry) {
            Some(at) => at,
            None => {
                self.stream_aggregations.push(candidate);
                self.stream_aggregations.len() - 1
            }
        };
        let outputs = &mut self.stream_aggregations[at].outputs;
        if !outputs.contains(&output) {
            outputs.push(output);
        }
        Ok(output_metric)
    }

    /// The stored chain at `chain`, if this translator supports it:
    /// ingestion-time, kept across evaluations, over an ingestion-time
    /// time-series Scan; exact Sum, Count, Min or Max; a width of whole
    /// seconds, at least 2 s, that divides a day.
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
        if kind.is_counter() {
            return Err(unsupported(chain, "stored counter window chain"));
        }
        let width_ms = *width_ms;
        if !width_ms.is_multiple_of(1000) {
            return Err(unsupported(
                chain,
                format!("window width {width_ms} ms is not whole seconds"),
            ));
        }
        if width_ms < 2000 {
            return Err(unsupported(
                chain,
                format!("window width {width_ms} ms is below 2 s"),
            ));
        }
        if !DAY_MS.is_multiple_of(width_ms) {
            return Err(unsupported(
                chain,
                format!("window width {width_ms} ms does not divide 86,400 s"),
            ));
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
        let grouping = match reduction {
            Reduction::PerEntity => ChainGrouping::PerSeries,
            Reduction::Reduce { columns, .. } if columns.is_empty() => ChainGrouping::AllSeries,
            Reduction::Reduce { columns, .. } => {
                let mut labels = self.group_labels(chain, *child, columns, &scan.label_columns)?;
                labels.sort();
                labels.dedup();
                ChainGrouping::By(labels)
            }
        };
        Ok(StoredChain {
            kind,
            metric: metric.clone(),
            selector: self.selector(*child),
            width_ms,
            grouping,
            labels: node.label_columns.clone(),
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
        if self.is_range(child) {
            let raw = self.raw_samples(child)?;
            if *reduction != Reduction::PerEntity {
                return Err(unsupported(id, "grouped aggregate over raw samples"));
            }
            let window = format!("{}[{}]", raw.selector, duration(raw.range_ms));
            let expr = match measure.kind.as_str() {
                "sum" | "count" | "min" | "max" | "avg" if no_arguments(measure, &[]) => {
                    format!("{}_over_time({window})", measure.kind)
                }
                "quantile" if no_arguments(measure, &["q"]) => {
                    format!(
                        "quantile_over_time({}, {window})",
                        quantile_level(id, measure)?
                    )
                }
                kind => return Err(unsupported(id, format!("measure {kind}"))),
            };
            return Ok(Relation {
                expr,
                labels: raw.labels,
                step_ms: 1,
                unfinalized_kind: None,
                per_series_of: Some(raw.metric),
            });
        }
        let Reduction::Reduce { columns, .. } = reduction else {
            return Err(unsupported(id, "per-series aggregate over computed values"));
        };
        let input = self.value_relation(id, child)?;
        let group = self.group_labels(id, child, columns, &input.labels)?;
        if measure.kind == "top_k" {
            let k = whole_number_argument(id, measure, "k")?;
            return Ok(Relation {
                expr: aggregate_by("topk", &group, &format!("{k}, {}", input.expr)),
                labels: input.labels,
                step_ms: input.step_ms,
                unfinalized_kind: None,
                per_series_of: input.per_series_of,
            });
        }
        let expr = match measure.kind.as_str() {
            "sum" | "count" | "min" | "max" | "avg" if no_arguments(measure, &[]) => {
                aggregate_by(&measure.kind, &group, &input.expr)
            }
            "quantile" if no_arguments(measure, &["q"]) => aggregate_by(
                "quantile",
                &group,
                &format!("{}, {}", quantile_level(id, measure)?, input.expr),
            ),
            kind => return Err(unsupported(id, format!("measure {kind}"))),
        };
        Ok(Relation {
            expr,
            labels: group,
            step_ms: input.step_ms,
            unfinalized_kind: None,
            per_series_of: None,
        })
    }

    /// `Project`: each output column is a child label under its own name, or
    /// the child's value, as is or cast to Float64 (every MetricsQL value is
    /// one).
    fn projected(
        &mut self,
        id: NodeId,
        child: NodeId,
        expressions: &[Value],
    ) -> Result<Relation, TranslationError> {
        let input = self.value_relation(id, child)?;
        let node = self.node(id);
        let child_node = self.node(child);
        let mut labels = Vec::new();
        let mut has_value = false;
        for (output, expression) in node.output_columns.iter().zip(expressions) {
            let column = match expression {
                Value::Object(map) if map.contains_key("Column") => &map["Column"],
                Value::Object(map) if map.contains_key("Cast") => {
                    let cast = &map["Cast"];
                    if cast.get("to").and_then(Value::as_str) != Some("float64")
                        || cast.get("try_cast") != Some(&Value::Bool(false))
                    {
                        return Err(unsupported(id, "projected expression"));
                    }
                    cast.get("expr")
                        .and_then(|e| e.get("Column"))
                        .ok_or_else(|| unsupported(id, "projected expression"))?
                }
                _ => return Err(unsupported(id, "projected expression")),
            };
            let index = column
                .as_u64()
                .map(|c| c as usize)
                .ok_or_else(|| unsupported(id, "projected column"))?;
            let source = child_node
                .output_columns
                .get(index)
                .ok_or_else(|| unsupported(id, "projected column"))?;
            if node.label_columns.contains(output) {
                if !input.labels.contains(source) {
                    return Err(unsupported(id, "projected label"));
                }
                if source != output {
                    return Err(unsupported(id, "projected label rename"));
                }
                labels.push(output.clone());
            } else {
                if has_value || !child_node.is_value_column(index) {
                    return Err(unsupported(id, "projected value"));
                }
                has_value = true;
            }
        }
        if !has_value {
            return Err(unsupported(id, "projection without a value"));
        }
        // Dropping a label would leave several series per label set.
        if labels.len() != input.labels.len() {
            return Err(unsupported(id, "projection that drops a label"));
        }
        Ok(Relation { labels, ..input })
    }

    /// Arithmetic between the series of `lhs` and `rhs` that have the same
    /// labels (one-to-one matching on all labels, `__name__` ignored).
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
        // The plan's series identity names the metric, so per-series values
        // of two metrics never match in the plan, where MetricsQL would
        // match them on their other labels.
        if left.per_series_of != right.per_series_of {
            return Err(unsupported(
                id,
                "binary operation across metrics per series",
            ));
        }
        if symbol == "/" {
            let checked = operator.get("checked_finite_division") == Some(&Value::Bool(true));
            self.substitutions.insert(
                id,
                Substitution {
                    node: id,
                    rule: "victoriametrics.division.nan_dropped".into(),
                    guarantee_difference: if checked {
                        "VictoriaMetrics drops a series whose quotient is NaN (e.g. 0/0), where \
                         PromQL keeps it; and a checked division (avg as sum / count) whose \
                         operand or result is not finite is answered as VictoriaMetrics divides, \
                         where the planner's executor answers the original query exactly"
                    } else {
                        "VictoriaMetrics drops a series whose quotient is NaN (e.g. 0/0), where \
                         PromQL keeps it"
                    }
                    .into(),
                },
            );
        }
        Ok(Relation {
            expr: format!("({}) {symbol} ({})", left.expr, right.expr),
            labels: left.labels,
            step_ms: lcm(left.step_ms, right.step_ms),
            unfinalized_kind: None,
            per_series_of: left.per_series_of,
        })
    }
}

/// `f by (a, b) (arguments)`, or `f(arguments)` with no label.
fn aggregate_by(function: &str, labels: &[String], arguments: &str) -> String {
    if labels.is_empty() {
        format!("{function}({arguments})")
    } else {
        format!("{function} by ({}) ({arguments})", labels.join(", "))
    }
}

/// Whether `measure` has no argument other than `allowed`.
fn no_arguments(measure: &Measure, allowed: &[&str]) -> bool {
    measure
        .arguments
        .keys()
        .all(|key| allowed.contains(&key.as_str()))
}

/// The `q` of a quantile measure, in [0, 1].
fn quantile_level(id: NodeId, measure: &Measure) -> Result<f64, TranslationError> {
    measure
        .arguments
        .get("q")
        .and_then(Value::as_f64)
        .filter(|q| (0.0..=1.0).contains(q))
        .ok_or_else(|| unsupported(id, "quantile level"))
}

/// A whole-number argument of a measure, e.g. `k` of `top_k`.
fn whole_number_argument(
    id: NodeId,
    measure: &Measure,
    name: &str,
) -> Result<u64, TranslationError> {
    if !no_arguments(measure, &[name]) {
        return Err(unsupported(id, format!("measure {}", measure.kind)));
    }
    measure
        .arguments
        .get(name)
        .and_then(Value::as_u64)
        .ok_or_else(|| unsupported(id, format!("{} {name}", measure.kind)))
}

/// A duration as MetricsQL and the config write it: `300000` ms is `300s`,
/// `1500` ms is `1500ms`.
fn duration(ms: u64) -> String {
    if ms.is_multiple_of(1000) {
        format!("{}s", ms / 1000)
    } else {
        format!("{ms}ms")
    }
}

/// A MetricsQL string literal: `j"3` is `"j\"3"`.
fn string_literal(value: &str) -> String {
    let mut out = String::from("\"");
    for c in value.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// A YAML single-quoted scalar: a quote is doubled.
fn yaml_quoted(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

/// A label name MetricsQL can write unquoted: `[a-zA-Z_][a-zA-Z0-9_]*`.
fn is_label_name(name: &str) -> bool {
    let mut chars = name.chars();
    chars
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// A metric name MetricsQL can write unquoted: `[a-zA-Z_:][a-zA-Z0-9_:]*`.
fn is_metric_name(name: &str) -> bool {
    let mut chars = name.chars();
    chars
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_' || c == ':')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_' || c == ':')
}

fn lcm(a: u64, b: u64) -> u64 {
    let (mut x, mut y) = (a, b);
    while y != 0 {
        (x, y) = (y, x % y);
    }
    a / x * b
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn durations_and_literals() {
        assert_eq!(duration(300_000), "300s");
        assert_eq!(duration(1_500), "1500ms");
        assert_eq!(string_literal("j\"3\\"), r#""j\"3\\""#);
        assert_eq!(yaml_quoted("a'b"), "'a''b'");
        assert!(is_label_name("job_1") && !is_label_name("a-b") && !is_label_name("1a"));
        assert!(is_metric_name("a:b") && !is_metric_name("a-b"));
    }
}
