use std::cell::RefCell;
use std::collections::btree_map::Entry;
use std::collections::{BTreeMap, HashMap};
use std::rc::Rc;

use aqpbm_core::measure::{Measurement, Pass, Report, RunOutcome as MeasuredRun};
use aqpbm_core::metrics::{Metric, MetricsMask, RunMetrics, WallClock};

use asap_types::post_asap::execution_data_state::lift_plain;
use asap_types::post_asap::{PostAsapNodeId, SketchQuery};
use asap_types::pre_asap::{
    AggIntent, ColumnId, ColumnRef, DataType, Predicate, QueryExpr, QueryExprError, Reduction,
    ScalarValue, Schema, Source,
};
use asap_types::types::AccuracyTarget;

use crate::plan::lower_promql_root;
use crate::rows::{check_predicate, order, passes, variant_name};
use crate::run::{RowsFrom, RunConfig};
use crate::score;
use crate::types::{Answer, EvalError, ItemKey, Refusal, Retained, Row, Value};

pub const PRE_ASAP_ROOT: PostAsapNodeId = PostAsapNodeId(0);

#[derive(Debug, Clone)]
pub enum Data {
    Rows(Rc<Vec<Row>>),
    Scalar(f64),
}

impl Data {
    fn rows(self, node: PostAsapNodeId, position: &str) -> Result<Rc<Vec<Row>>, EvalError> {
        match self {
            Data::Rows(rows) => Ok(rows),
            Data::Scalar(value) => Err(EvalError::Refused(vec![
                Refusal::UnsupportedValueOperation {
                    node,
                    detail: format!("{position} produced the scalar {value}, not rows"),
                },
            ])),
        }
    }

    fn scalar(self, node: PostAsapNodeId, position: &str) -> Result<f64, EvalError> {
        match self {
            Data::Scalar(value) => Ok(value),
            Data::Rows(rows) => Err(EvalError::Refused(vec![
                Refusal::UnsupportedValueOperation {
                    node,
                    detail: format!("{position} produced {} rows, not a scalar", rows.len()),
                },
            ])),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Shape {
    Rows(usize),
    Scalar,
}

impl Shape {
    fn rows(self, node: PostAsapNodeId, position: &str) -> Result<usize, Refusal> {
        match self {
            Shape::Rows(columns) => Ok(columns),
            Shape::Scalar => Err(Refusal::UnsupportedValueOperation {
                node,
                detail: format!("{position} is a scalar, not rows"),
            }),
        }
    }

    fn scalar(self, node: PostAsapNodeId, position: &str) -> Result<(), Refusal> {
        match self {
            Shape::Scalar => Ok(()),
            Shape::Rows(columns) => Err(Refusal::UnsupportedValueOperation {
                node,
                detail: format!("{position} is a relation of {columns} columns, not a scalar"),
            }),
        }
    }
}

pub(crate) fn check(node: PostAsapNodeId, expr: &QueryExpr) -> Result<(), Refusal> {
    admit(node, expr).map(|_| ())
}

fn admit(node: PostAsapNodeId, expr: &QueryExpr) -> Result<Option<Leaf>, Refusal> {
    let mut admission = Admission { node, leaf: None };
    admission.shape_of(expr)?;
    Ok(admission.leaf)
}

pub(crate) fn evaluate(
    node: PostAsapNodeId,
    expr: &Rc<QueryExpr>,
    rows: &Rc<Vec<Row>>,
) -> Result<(Data, Option<usize>), EvalError> {
    check(node, expr).map_err(|refusal| EvalError::Refused(vec![refusal]))?;
    let interpreted = interpret(node, expr, rows, false)?;
    Ok((interpreted.answer, interpreted.leaf_emitted))
}

struct Interpreted {
    answer: Data,
    leaf_emitted: Option<usize>,
    node_times: Vec<NodeTime>,
}

fn interpret(
    node: PostAsapNodeId,
    expr: &Rc<QueryExpr>,
    rows: &Rc<Vec<Row>>,
    per_node_time: bool,
) -> Result<Interpreted, EvalError> {
    let mut interpreter = Interpreter {
        node,
        rows,
        memo: HashMap::new(),
        leaf_emitted: None,
        node_times: per_node_time.then(Vec::new),
        children_ns: 0,
    };
    let answer = interpreter.eval_rc(expr)?;
    Ok(Interpreted {
        answer,
        leaf_emitted: interpreter.leaf_emitted,
        node_times: interpreter
            .node_times
            .unwrap_or_default()
            .into_iter()
            .map(|held| held.time)
            .collect(),
    })
}

#[derive(Debug)]
pub struct ExactRun {
    pub root: Rc<QueryExpr>,
    pub rows_scanned: u64,
    pub rows_emitted: u64,
    pub answer: Data,
}

pub fn run_promql(
    query: &str,
    accuracy: AccuracyTarget,
    from: &RowsFrom,
) -> Result<ExactRun, EvalError> {
    let root = lower_promql_root(query, accuracy)?;
    let (rows, rows_scanned) = scan(&root, from)?;

    let (answer, leaf_emitted) = evaluate(PRE_ASAP_ROOT, &root, &rows)?;
    Ok(ExactRun {
        root,
        rows_scanned,
        rows_emitted: leaf_emitted.unwrap_or(0) as u64,
        answer,
    })
}

fn scan(root: &Rc<QueryExpr>, from: &RowsFrom) -> Result<(Rc<Vec<Row>>, u64), EvalError> {
    let leaf = admit(PRE_ASAP_ROOT, root).map_err(|refusal| EvalError::Refused(vec![refusal]))?;
    let Some(leaf) = leaf else {
        return Ok((Rc::new(Vec::new()), 0));
    };
    let unfiltered = QueryExpr::Scan {
        source: leaf.source,
        predicates: Vec::new(),
        schema: leaf.schema.clone(),
    };
    let node_schema = lift_plain(&leaf.schema);
    let mut reader = match from {
        RowsFrom::Csv(path) => crate::rows::open(&unfiltered, path, &node_schema)?,
        RowsFrom::Generated(table) => {
            crate::rows::open_generated(&unfiltered, Rc::clone(table), &node_schema)?
        }
    };
    let mut held = Vec::new();
    for row in reader.by_ref() {
        held.push(row?);
    }
    if let Some(metric) = reader.metric_absent() {
        return Err(EvalError::Refused(vec![Refusal::MetricAbsentFromRows {
            node: PRE_ASAP_ROOT,
            metric: metric.to_string(),
            detail: format!("{} names other series", reader.origin()),
        }]));
    }
    let scanned = reader.scanned();
    Ok((Rc::new(held), scanned))
}

#[derive(Debug, Clone)]
pub struct NodeTime {
    pub node: u32,
    pub operator: &'static str,
    pub elapsed_ns: u64,
}

#[derive(Debug)]
pub struct PreAsapArm {
    pub evaluate: Vec<RunMetrics>,
    pub node_times: Vec<NodeTime>,
    pub retained_bytes: usize,
    pub rows_scanned: u64,
    pub rows_emitted: u64,
    pub answer: Data,
}

pub fn time_pre_asap(root: &Rc<QueryExpr>, cfg: &RunConfig) -> Result<PreAsapArm, EvalError> {
    check(PRE_ASAP_ROOT, root).map_err(|refusal| EvalError::Refused(vec![refusal]))?;
    let (rows, rows_scanned) = scan(root, &cfg.rows)?;
    let retained_bytes = rows_bytes(&rows);

    let kept: Rc<RefCell<Vec<Interpreted>>> = Rc::new(RefCell::new(Vec::new()));
    let fault: Rc<RefCell<Option<EvalError>>> = Rc::new(RefCell::new(None));

    let (total, config) = crate::run::phase_config(Metric::Throughput, MetricsMask::empty(), cfg);
    let mut passes: Measurement = Vec::with_capacity(total);
    for _ in 0..total {
        let root = Rc::clone(root);
        let rows = Rc::clone(&rows);
        let kept = Rc::clone(&kept);
        let fault = Rc::clone(&fault);
        let per_node_time = cfg.per_node_time;
        let work = rows.len() as u64;
        let pass: Pass = Box::new(move || {
            let interpreted = interpret(PRE_ASAP_ROOT, &root, &rows, per_node_time);
            let report: Report = Box::new(move || {
                match interpreted {
                    Ok(interpreted) => kept.borrow_mut().push(interpreted),
                    Err(err) => {
                        let mut held = fault.borrow_mut();
                        if held.is_none() {
                            *held = Some(err);
                        }
                    }
                }
                MeasuredRun {
                    work,
                    memory_bytes: Some(retained_bytes as u64),
                    ..MeasuredRun::default()
                }
            });
            report
        });
        passes.push(pass);
    }

    let runs = aqpbm_core::measure::measure(&config, passes);
    if let Some(err) = fault.borrow_mut().take() {
        return Err(err);
    }
    let mut kept = std::mem::take(&mut *kept.borrow_mut());
    let node_times = mean_node_times(&kept, runs.len());
    let last = kept
        .pop()
        .ok_or_else(|| EvalError::Validation("the pre-ASAP arm ran no pass at all".to_string()))?;

    Ok(PreAsapArm {
        evaluate: runs,
        node_times,
        retained_bytes,
        rows_scanned,
        rows_emitted: last.leaf_emitted.unwrap_or(0) as u64,
        answer: last.answer,
    })
}

fn mean_node_times(kept: &[Interpreted], measured: usize) -> Vec<NodeTime> {
    let timed = &kept[kept.len().saturating_sub(measured)..];
    let Some(first) = timed.first() else {
        return Vec::new();
    };
    first
        .node_times
        .iter()
        .enumerate()
        .map(|(position, node)| {
            let total: u64 = timed
                .iter()
                .filter_map(|pass| pass.node_times.get(position))
                .map(|held| held.elapsed_ns)
                .sum();
            NodeTime {
                node: node.node,
                operator: node.operator,
                elapsed_ns: total / timed.len() as u64,
            }
        })
        .collect()
}

fn rows_bytes(rows: &[Row]) -> usize {
    rows.iter()
        .map(|row| {
            row.0.len() * std::mem::size_of::<Value>()
                + row
                    .0
                    .iter()
                    .map(|value| match value {
                        Value::Str(held) => held.len(),
                        _ => 0,
                    })
                    .sum::<usize>()
        })
        .sum()
}

struct Leaf {
    source: Source,
    schema: Schema,
    predicates: Vec<Predicate>,
}

struct Admission {
    node: PostAsapNodeId,
    leaf: Option<Leaf>,
}

impl Admission {
    fn shape_of(&mut self, expr: &QueryExpr) -> Result<Shape, Refusal> {
        match expr {
            QueryExpr::Scan {
                source,
                predicates,
                schema,
            } => {
                match source {
                    Source::Table { .. } | Source::TimeSeries { .. } => {}
                }
                self.one_leaf(source, schema, predicates)?;
                let columns = schema.columns.len();
                for predicate in predicates {
                    self.subset(&predicate.0, columns, "Scan.predicates")?;
                }
                Ok(Shape::Rows(columns))
            }
            QueryExpr::Filter { pred, child } => {
                let columns = self.shape_of(child)?.rows(self.node, "Filter.child")?;
                self.subset(&pred.0, columns, "Filter.pred")?;
                Ok(Shape::Rows(columns))
            }
            QueryExpr::Project {
                cols,
                qualifier: _,
                child,
            } => {
                let columns = self.shape_of(child)?.rows(self.node, "Project.child")?;
                crate::value::project_shape(self.node, cols)?;
                for (index, item) in cols.iter().enumerate() {
                    self.subset(&item.expr, columns, &format!("Project col {index}"))?;
                }
                Ok(Shape::Rows(cols.len()))
            }
            QueryExpr::Aggregate { child, .. } => {
                let columns = self.shape_of(child)?.rows(self.node, "Aggregate.child")?;
                let aggregation = aggregation(self.node, expr)?;
                if aggregation.child_columns != columns {
                    return Err(Refusal::UnsupportedValueOperation {
                        node: self.node,
                        detail: format!(
                            "Aggregate.child emits rows of {columns} values and its output_schema \
                             declares {} columns",
                            aggregation.child_columns
                        ),
                    });
                }
                Ok(Shape::Rows(aggregation.width))
            }
            QueryExpr::Limit { child, .. } => {
                let columns = self.shape_of(child)?.rows(self.node, "Limit.child")?;
                Ok(Shape::Rows(columns))
            }
            QueryExpr::Concat { children, .. } => {
                let mut width: Option<usize> = None;
                for (index, branch) in children.iter().enumerate() {
                    let columns = self
                        .shape_of(branch)?
                        .rows(self.node, &format!("Concat branch {index}"))?;
                    match width {
                        None => width = Some(columns),
                        Some(held) if held == columns => {}
                        Some(held) => {
                            return Err(Refusal::UnsupportedValueOperation {
                                node: self.node,
                                detail: format!(
                                    "Concat branch {index} emits rows of {columns} values and \
                                     branch 0 emits {held}: UNION ALL concatenates \
                                     union-compatible branches, and nothing in the IR checks that \
                                     they are"
                                ),
                            })
                        }
                    }
                }
                width
                    .map(Shape::Rows)
                    .ok_or(Refusal::UnsupportedValueOperation {
                        node: self.node,
                        detail: "Concat carries no branch, so there is no width to emit".to_owned(),
                    })
            }
            QueryExpr::Sort {
                keys,
                partition_by,
                child,
            } => {
                let columns = self.shape_of(child)?.rows(self.node, "Sort.child")?;
                if keys.is_empty() {
                    return Err(Refusal::UnsupportedValueOperation {
                        node: self.node,
                        detail: "Sort carries no key".to_owned(),
                    });
                }
                if !partition_by.is_empty() {
                    return Err(Refusal::UnsupportedGrouping {
                        node: self.node,
                        detail: format!(
                            "Sort partitioned by {:?}: a partitioned order only means anything \
                             together with the Limit above it — topk by (host) is k rows per \
                             host, not k overall — and the IR does not couple the two, so \
                             ordering within each partition and letting a global Limit cut the \
                             result would report a plausible wrong answer",
                            partition_by.keys()
                        ),
                    });
                }
                for (index, key) in keys.iter().enumerate() {
                    self.subset(&key.expr, columns, &format!("Sort key {index}"))?;
                }
                Ok(Shape::Rows(columns))
            }
            QueryExpr::Literal(value) => {
                numeric_literal(self.node, value)?;
                Ok(Shape::Scalar)
            }
            QueryExpr::PromqlScalarBridge(inner) => {
                self.shape_of(inner)?
                    .scalar(self.node, "PromqlScalarBridge")?;
                Ok(Shape::Scalar)
            }
            other => Err(Refusal::UnsupportedOperator {
                node: self.node,
                operator: variant_name(other).to_string(),
            }),
        }
    }

    fn one_leaf(
        &mut self,
        source: &Source,
        schema: &Schema,
        predicates: &[Predicate],
    ) -> Result<(), Refusal> {
        let Some(held) = &self.leaf else {
            self.leaf = Some(Leaf {
                source: source.clone(),
                schema: schema.clone(),
                predicates: predicates.to_vec(),
            });
            return Ok(());
        };
        if &held.source != source {
            return Err(Refusal::UnsupportedOperator {
                node: self.node,
                operator: format!(
                    "Scan over a second source ({}; the tree already scans {})",
                    source_name(source),
                    source_name(&held.source)
                ),
            });
        }
        if &held.schema != schema {
            return Err(Refusal::UnsupportedOperator {
                node: self.node,
                operator: format!(
                    "Scan over {} under a second binding schema ({}; the tree already binds {})",
                    source_name(source),
                    schema_name(schema),
                    schema_name(&held.schema)
                ),
            });
        }
        if held.predicates != predicates {
            return Err(Refusal::UnsupportedOperator {
                node: self.node,
                operator: format!(
                    "Scan over {} under a second predicate set ({} predicates; the tree already \
                     scans it under {}), so the rows the leaf admits are not one number",
                    source_name(source),
                    predicates.len(),
                    held.predicates.len()
                ),
            });
        }
        Ok(())
    }

    fn subset(&self, expr: &QueryExpr, columns: usize, position: &str) -> Result<(), Refusal> {
        check_predicate(expr, columns)
            .map_err(|fault| crate::value::predicate_refusal(self.node, position, &fault))
    }
}

struct Interpreter<'a> {
    node: PostAsapNodeId,
    rows: &'a Rc<Vec<Row>>,
    memo: HashMap<*const QueryExpr, Data>,
    leaf_emitted: Option<usize>,
    node_times: Option<Vec<NodeClock>>,
    children_ns: u64,
}

struct NodeClock {
    key: *const QueryExpr,
    time: NodeTime,
}

impl Interpreter<'_> {
    fn eval_rc(&mut self, expr: &Rc<QueryExpr>) -> Result<Data, EvalError> {
        let key = Rc::as_ptr(expr);
        if let Some(held) = self.memo.get(&key) {
            return Ok(held.clone());
        }
        let data = self.eval_node(expr)?;
        self.memo.insert(key, data.clone());
        Ok(data)
    }

    fn eval_node(&mut self, expr: &QueryExpr) -> Result<Data, EvalError> {
        if self.node_times.is_none() {
            return self.eval_body(expr);
        }
        let outer = std::mem::take(&mut self.children_ns);
        let clock = WallClock::start();
        let data = self.eval_body(expr);
        let inclusive = clock.elapsed_ns();
        let own = inclusive.saturating_sub(self.children_ns);
        self.children_ns = outer.saturating_add(inclusive);
        self.charge(expr, own);
        data
    }

    fn charge(&mut self, expr: &QueryExpr, elapsed_ns: u64) {
        let key = expr as *const QueryExpr;
        let Some(times) = self.node_times.as_mut() else {
            return;
        };
        match times.iter().position(|held| held.key == key) {
            Some(position) => times[position].time.elapsed_ns += elapsed_ns,
            None => times.push(NodeClock {
                key,
                time: NodeTime {
                    node: times.len() as u32,
                    operator: variant_name(expr),
                    elapsed_ns,
                },
            }),
        }
    }

    fn eval_body(&mut self, expr: &QueryExpr) -> Result<Data, EvalError> {
        match expr {
            QueryExpr::Scan { predicates, .. } => {
                let admitted = self.scanned(predicates)?;
                self.leaf_emitted = Some(admitted.len());
                Ok(Data::Rows(admitted))
            }
            QueryExpr::Filter { pred, child } => {
                let input = self.rows_of(child, "Filter.child")?;
                let mut kept = Vec::with_capacity(input.len());
                for row in input.iter() {
                    if passes(&pred.0, row)? {
                        kept.push(row.clone());
                    }
                }
                Ok(Data::Rows(Rc::new(kept)))
            }
            QueryExpr::Project {
                cols,
                qualifier: _,
                child,
            } => {
                let input = self.rows_of(child, "Project.child")?;
                Ok(Data::Rows(crate::value::project(self.node, cols, &input)?))
            }
            QueryExpr::Aggregate { child, .. } => {
                let aggregation = aggregation(self.node, expr)
                    .map_err(|refusal| EvalError::Refused(vec![refusal]))?;
                let input = self.rows_of(child, "Aggregate.child")?;
                Ok(Data::Rows(reduce(self.node, &aggregation, &input)?))
            }
            QueryExpr::Limit { n, offset, child } => {
                let input = self.rows_of(child, "Limit.child")?;
                Ok(Data::Rows(crate::value::limit(*n, *offset, &input)))
            }
            QueryExpr::Concat { children, .. } => {
                let mut out = Vec::new();
                for (index, branch) in children.iter().enumerate() {
                    let rows = self
                        .eval_node(branch)?
                        .rows(self.node, &format!("Concat branch {index}"))?;
                    out.extend(rows.iter().cloned());
                }
                Ok(Data::Rows(Rc::new(out)))
            }
            QueryExpr::Sort {
                keys,
                partition_by,
                child,
            } => {
                if !partition_by.is_empty() {
                    return Err(EvalError::Refused(vec![Refusal::UnsupportedGrouping {
                        node: self.node,
                        detail: format!(
                            "Sort partitioned by {:?}: a partitioned order only means anything \
                             together with the Limit above it, and the IR does not couple the two",
                            partition_by.keys()
                        ),
                    }]));
                }
                let input = self.rows_of(child, "Sort.child")?;
                Ok(Data::Rows(crate::value::sort(self.node, keys, &input)?))
            }
            QueryExpr::Literal(_) => {
                let value = crate::rows::eval(expr, &Row(Vec::new()))?;
                value.as_f64().map(Data::Scalar).ok_or_else(|| {
                    EvalError::Refused(vec![Refusal::UnsupportedValueOperation {
                        node: self.node,
                        detail: format!("Literal({value:?}) has no f64 value"),
                    }])
                })
            }
            QueryExpr::PromqlScalarBridge(inner) => {
                let value = self
                    .eval_rc(inner)?
                    .scalar(self.node, "PromqlScalarBridge")?;
                Ok(Data::Scalar(value))
            }
            other => Err(EvalError::Refused(vec![Refusal::UnsupportedOperator {
                node: self.node,
                operator: variant_name(other).to_string(),
            }])),
        }
    }

    fn scanned(&self, predicates: &[Predicate]) -> Result<Rc<Vec<Row>>, EvalError> {
        if predicates.is_empty() {
            return Ok(Rc::clone(self.rows));
        }
        let mut kept = Vec::with_capacity(self.rows.len());
        'row: for row in self.rows.iter() {
            for predicate in predicates {
                if !passes(&predicate.0, row)? {
                    continue 'row;
                }
            }
            kept.push(row.clone());
        }
        Ok(Rc::new(kept))
    }

    fn rows_of(
        &mut self,
        child: &Rc<QueryExpr>,
        position: &str,
    ) -> Result<Rc<Vec<Row>>, EvalError> {
        self.eval_rc(child)?.rows(self.node, position)
    }
}

struct Aggregation {
    group: Vec<ColumnId>,
    measures: Vec<Measure>,
    child_columns: usize,
    width: usize,
}

#[derive(Debug, Clone, PartialEq)]
enum Measure {
    Count,
    Sum { column: ColumnId, integral: bool },
    Min(ColumnId),
    Max(ColumnId),
    Avg(ColumnId),
    Quantile { column: ColumnId, q: f64 },
    Cardinality(ColumnId),
}

enum Fold {
    Rows(Retained),
    Numbers(Retained),
    Whole { total: i128, contributed: bool },
    Extreme(Option<Value>),
    Keys(Retained),
}

#[derive(Debug, Clone)]
struct GroupOrder(Vec<Value>);

impl PartialEq for GroupOrder {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other).is_eq()
    }
}

impl Eq for GroupOrder {}

impl PartialOrd for GroupOrder {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for GroupOrder {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.0
            .iter()
            .zip(&other.0)
            .map(|(held, seen)| ranked_value(held, seen))
            .find(|ordering| ordering.is_ne())
            .unwrap_or_else(|| self.0.len().cmp(&other.0.len()))
    }
}

fn ranked_value(left: &Value, right: &Value) -> std::cmp::Ordering {
    value_class(left)
        .cmp(&value_class(right))
        .then_with(|| order(left, right))
}

fn value_class(value: &Value) -> u8 {
    match value {
        Value::Null => 0,
        Value::Int(_) | Value::Float(_) | Value::Timestamp(_) => 1,
        Value::Str(_) => 2,
    }
}

fn aggregation(node: PostAsapNodeId, expr: &QueryExpr) -> Result<Aggregation, Refusal> {
    let QueryExpr::Aggregate {
        reduction,
        measures,
        having,
        child,
        ..
    } = expr
    else {
        return Err(Refusal::UnsupportedOperator {
            node,
            operator: variant_name(expr).to_string(),
        });
    };

    if having.is_some() {
        return Err(Refusal::UnsupportedGrouping {
            node,
            detail: "Aggregate.having is Some, and this arm applies no post-reduction filter: \
                     admitting it would report an unfiltered aggregate as the answer"
                .to_owned(),
        });
    }

    let by = match reduction {
        Reduction::PerEntity => {
            return Err(Refusal::UnsupportedGrouping {
                node,
                detail: "Reduction::PerEntity has no entity concept over CSV rows".to_owned(),
            })
        }
        Reduction::Reduce(by) if by.is_without() => {
            return Err(Refusal::UnsupportedGrouping {
                node,
                detail: "GroupKeys::without keeps every label except these, and a PromQL leaf's \
                         schema is usage-derived and open, so the complement is not enumerable \
                         from the tree"
                    .to_owned(),
            })
        }
        Reduction::Reduce(by) => by,
    };

    let in_schema = schema_of(node, child, "Aggregate.child")?;
    let child_columns = in_schema.columns.len();

    let mut group = Vec::with_capacity(by.keys().len());
    for key in by.keys() {
        if *key >= child_columns {
            return Err(Refusal::UnresolvableColumn {
                node,
                column: key.to_string(),
                detail: format!(
                    "Aggregate by column {key} is past the child's {child_columns}-column output \
                     schema"
                ),
            });
        }
        group.push(*key);
    }

    let probe = in_schema.column_id("value");

    let mut resolved = Vec::with_capacity(measures.len());
    for intent in measures {
        resolved.push(measure(node, intent, &in_schema, probe)?);
    }

    let declared = schema_of(node, expr, "Aggregate")?;
    let width = group.len() + resolved.len();
    if declared.columns.len() != width {
        return Err(Refusal::UnsupportedValueOperation {
            node,
            detail: format!(
                "Aggregate would emit {} group values and {} measures, and its output_schema \
                 declares {} columns",
                group.len(),
                resolved.len(),
                declared.columns.len()
            ),
        });
    }
    for (position, held) in resolved.iter().enumerate() {
        let produces = produced_type(held, &in_schema);
        let says = &declared.columns[group.len() + position].dtype;
        if !same_class(&produces, says) {
            return Err(Refusal::UnsupportedValueOperation {
                node,
                detail: format!(
                    "the {} measure reads a {} column and the output_schema declares the answer \
                     {}, so the two arms would not be answering one question",
                    measure_name(held),
                    crate::value::debug_variant(&produces),
                    crate::value::debug_variant(says)
                ),
            });
        }
    }

    Ok(Aggregation {
        group,
        measures: resolved,
        child_columns,
        width,
    })
}

fn measure(
    node: PostAsapNodeId,
    intent: &AggIntent,
    in_schema: &Schema,
    probe: Option<ColumnId>,
) -> Result<Measure, Refusal> {
    match intent {
        AggIntent::Count { .. } => Ok(Measure::Count),
        AggIntent::Sum { .. } => {
            let column = reduced_column(node, intent, in_schema, probe, arithmetic)?;
            Ok(Measure::Sum {
                column,
                integral: matches!(in_schema.columns[column].dtype, DataType::Int64),
            })
        }
        AggIntent::Min { .. } => Ok(Measure::Min(reduced_column(
            node, intent, in_schema, probe, comparable,
        )?)),
        AggIntent::Max { .. } => Ok(Measure::Max(reduced_column(
            node, intent, in_schema, probe, comparable,
        )?)),
        AggIntent::Avg { .. } => Ok(Measure::Avg(reduced_column(
            node, intent, in_schema, probe, arithmetic,
        )?)),
        AggIntent::Quantile { q, .. } => {
            if !(0.0..=1.0).contains(q) {
                return Err(Refusal::ParameterOutOfBounds {
                    node,
                    detail: format!("AggIntent::Quantile q = {q} is outside [0, 1]"),
                });
            }
            Ok(Measure::Quantile {
                column: reduced_column(node, intent, in_schema, probe, arithmetic)?,
                q: *q,
            })
        }
        AggIntent::Cardinality { .. } => Ok(Measure::Cardinality(reduced_column(
            node, intent, in_schema, probe, comparable,
        )?)),
        other => Err(Refusal::UnsupportedOperator {
            node,
            operator: format!("AggIntent::{}", intent_name(other)),
        }),
    }
}

fn produced_type(held: &Measure, in_schema: &Schema) -> DataType {
    match held {
        Measure::Count | Measure::Cardinality(_) => DataType::Int64,
        Measure::Sum { integral: true, .. } => DataType::Int64,
        Measure::Sum { .. } | Measure::Avg(_) | Measure::Quantile { .. } => DataType::Float64,
        Measure::Min(column) | Measure::Max(column) => in_schema.columns[*column].dtype.clone(),
    }
}

fn measure_name(held: &Measure) -> &'static str {
    match held {
        Measure::Count => "Count",
        Measure::Sum { .. } => "Sum",
        Measure::Min(_) => "Min",
        Measure::Max(_) => "Max",
        Measure::Avg(_) => "Avg",
        Measure::Quantile { .. } => "Quantile",
        Measure::Cardinality(_) => "Cardinality",
    }
}

fn same_class(produced: &DataType, declared: &DataType) -> bool {
    let numeric = |dtype: &DataType| {
        matches!(
            dtype,
            DataType::Int64 | DataType::Float64 | DataType::Timestamp
        )
    };
    produced == declared || (numeric(produced) && numeric(declared))
}

fn arithmetic(dtype: &DataType) -> bool {
    matches!(dtype, DataType::Int64 | DataType::Float64)
}

fn comparable(dtype: &DataType) -> bool {
    matches!(
        dtype,
        DataType::Int64 | DataType::Float64 | DataType::Utf8 | DataType::Bool | DataType::Timestamp
    )
}

fn reduced_column(
    node: PostAsapNodeId,
    intent: &AggIntent,
    in_schema: &Schema,
    probe: Option<ColumnId>,
    accepts: fn(&DataType) -> bool,
) -> Result<ColumnId, Refusal> {
    let columns = in_schema.columns.len();
    let column = match intent.input_col() {
        Some(column) if column >= columns => {
            return Err(Refusal::UnresolvableColumn {
                node,
                column: column.to_string(),
                detail: format!(
                    "AggIntent::{} reduces column {column}, past the child's {columns}-column \
                     output schema",
                    intent_name(intent)
                ),
            })
        }
        Some(column) => column,
        None => probe.ok_or_else(|| Refusal::UnresolvableColumn {
            node,
            column: "value".to_owned(),
            detail: format!(
                "AggIntent::{} reduces the sample value, and the child's output schema offers no \
                 column to read it from",
                intent_name(intent)
            ),
        })?,
    };

    let dtype = &in_schema.columns[column].dtype;
    if !accepts(dtype) {
        return Err(Refusal::UnsupportedValueOperation {
            node,
            detail: format!(
                "AggIntent::{} reduces column {column}, which the child's output schema declares \
                 {}",
                intent_name(intent),
                crate::value::debug_variant(dtype)
            ),
        });
    }
    Ok(column)
}

fn intent_name(intent: &AggIntent) -> String {
    crate::value::debug_variant(intent)
}

fn schema_of(node: PostAsapNodeId, expr: &QueryExpr, position: &str) -> Result<Schema, Refusal> {
    expr.output_schema()
        .map_err(|error: QueryExprError| Refusal::UnsupportedValueOperation {
            node,
            detail: format!("{position} has no output schema: {error}"),
        })
}

fn reduce(
    node: PostAsapNodeId,
    aggregation: &Aggregation,
    rows: &Rc<Vec<Row>>,
) -> Result<Rc<Vec<Row>>, EvalError> {
    let mut groups: BTreeMap<GroupOrder, Vec<Fold>> = BTreeMap::new();

    for row in rows.iter() {
        let key = GroupOrder(group_values(row, &aggregation.group)?);
        let held = match groups.entry(key) {
            Entry::Occupied(occupied) => occupied.into_mut(),
            Entry::Vacant(slot) => slot.insert(folds(&aggregation.measures)),
        };
        for (fold, measure) in held.iter_mut().zip(&aggregation.measures) {
            accumulate(fold, measure, row)?;
        }
    }

    if groups.is_empty() && aggregation.group.is_empty() {
        groups.insert(GroupOrder(Vec::new()), folds(&aggregation.measures));
    }

    let mut out = Vec::with_capacity(groups.len());
    for (key, mut held) in groups {
        let mut emitted = key.0;
        for (fold, measure) in held.iter_mut().zip(&aggregation.measures) {
            emitted.push(finish(node, fold, measure)?);
        }
        if emitted.len() != aggregation.width {
            return Err(EvalError::Refused(vec![
                Refusal::UnsupportedValueOperation {
                    node,
                    detail: format!(
                        "Aggregate emitted a row of {} values where its output schema has {}",
                        emitted.len(),
                        aggregation.width
                    ),
                },
            ]));
        }
        out.push(Row(emitted));
    }
    Ok(Rc::new(out))
}

fn folds(measures: &[Measure]) -> Vec<Fold> {
    measures
        .iter()
        .map(|measure| match measure {
            Measure::Count => Fold::Rows(Retained::default()),
            Measure::Sum { integral: true, .. } => Fold::Whole {
                total: 0,
                contributed: false,
            },
            Measure::Sum { .. } | Measure::Avg(_) | Measure::Quantile { .. } => {
                Fold::Numbers(Retained::default())
            }
            Measure::Min(_) | Measure::Max(_) => Fold::Extreme(None),
            Measure::Cardinality(_) => Fold::Keys(Retained::default()),
        })
        .collect()
}

fn accumulate(fold: &mut Fold, measure: &Measure, row: &Row) -> Result<(), EvalError> {
    match (fold, measure) {
        (Fold::Rows(retained), Measure::Count) => {
            retained.push(None, 1.0);
            Ok(())
        }
        (
            Fold::Numbers(retained),
            Measure::Sum { column, .. } | Measure::Avg(column) | Measure::Quantile { column, .. },
        ) => {
            let value = cell(row, *column)?;
            if matches!(value, Value::Null) {
                return Ok(());
            }
            retained.push(None, number(value, *column)?);
            Ok(())
        }
        (Fold::Whole { total, contributed }, Measure::Sum { column, .. }) => {
            let value = cell(row, *column)?;
            match value {
                Value::Null => Ok(()),
                Value::Int(held) => {
                    *total += i128::from(*held);
                    *contributed = true;
                    Ok(())
                }
                other => Err(EvalError::RowSource(format!(
                    "column {column} is declared Int64 and holds {other:?}"
                ))),
            }
        }
        (Fold::Extreme(held), Measure::Min(column) | Measure::Max(column)) => {
            let value = cell(row, *column)?;
            if matches!(value, Value::Null) {
                return Ok(());
            }
            let wanted = if matches!(measure, Measure::Min(_)) {
                std::cmp::Ordering::Less
            } else {
                std::cmp::Ordering::Greater
            };
            match held {
                Some(seen) if order(value, seen) != wanted => {}
                _ => *held = Some(value.clone()),
            }
            Ok(())
        }
        (Fold::Keys(retained), Measure::Cardinality(column)) => {
            let value = cell(row, *column)?;
            match item_key(value) {
                Some(key) => retained.push(Some(&key), 1.0),
                None => return Ok(()),
            }
            Ok(())
        }
        (_, measure) => Err(EvalError::RowSource(format!(
            "{measure:?} was handed an accumulator of another measure"
        ))),
    }
}

fn finish(node: PostAsapNodeId, fold: &mut Fold, measure: &Measure) -> Result<Value, EvalError> {
    match (fold, measure) {
        (Fold::Rows(retained), Measure::Count) => {
            Ok(Value::Int(scalar_answer(node, retained, &counted())? as i64))
        }
        (Fold::Numbers(retained), Measure::Sum { .. }) => {
            if retained.is_empty() {
                return Ok(Value::Null);
            }
            Ok(Value::Float(scalar_answer(node, retained, &summed())?))
        }
        (Fold::Whole { total, contributed }, Measure::Sum { .. }) => {
            if !*contributed {
                return Ok(Value::Null);
            }
            i64::try_from(*total).map(Value::Int).map_err(|_| {
                EvalError::RowSource(format!("the exact integer sum {total} is past an i64"))
            })
        }
        (Fold::Numbers(retained), Measure::Avg(_)) => {
            if retained.is_empty() {
                return Ok(Value::Null);
            }
            let sum = scalar_answer(node, retained, &summed())?;
            let count = scalar_answer(node, retained, &counted())?;
            Ok(Value::Float(sum / count))
        }
        (Fold::Numbers(retained), Measure::Quantile { q, .. }) => {
            if retained.is_empty() {
                return Ok(Value::Null);
            }
            let query = SketchQuery::Quantile { q: *q };
            if score::needs_a_sorted_column(&query) {
                retained.weights.sort_by(f64::total_cmp);
            }
            Ok(Value::Float(scalar_answer(node, retained, &query)?))
        }
        (Fold::Extreme(held), Measure::Min(_) | Measure::Max(_)) => {
            Ok(held.clone().unwrap_or(Value::Null))
        }
        (Fold::Keys(retained), Measure::Cardinality(_)) => {
            Ok(Value::Int(
                scalar_answer(node, retained, &SketchQuery::Cardinality)? as i64,
            ))
        }
        (_, measure) => Err(EvalError::RowSource(format!(
            "{measure:?} was handed an accumulator of another measure"
        ))),
    }
}

fn counted() -> SketchQuery {
    SketchQuery::PointCount {
        key: ColumnRef::Wildcard,
        value: None,
    }
}

fn summed() -> SketchQuery {
    SketchQuery::PointCount {
        key: ColumnRef::SampleValue,
        value: None,
    }
}

fn scalar_answer(
    node: PostAsapNodeId,
    retained: &Retained,
    query: &SketchQuery,
) -> Result<f64, EvalError> {
    match score::exact_answer(retained, query)? {
        Answer::Scalar(value) => Ok(value),
        Answer::Ranked(_) => Err(EvalError::Refused(vec![Refusal::UnsupportedReadout {
            node,
            query: Box::new(query.clone()),
        }])),
    }
}

fn group_values(row: &Row, group: &[ColumnId]) -> Result<Vec<Value>, EvalError> {
    group
        .iter()
        .map(|column| cell(row, *column).cloned())
        .collect()
}

fn cell(row: &Row, column: ColumnId) -> Result<&Value, EvalError> {
    row.0.get(column).ok_or_else(|| {
        EvalError::RowSource(format!(
            "column {column} is out of range (the row has {} values)",
            row.0.len()
        ))
    })
}

fn number(value: &Value, column: ColumnId) -> Result<f64, EvalError> {
    value.as_f64().ok_or_else(|| {
        EvalError::RowSource(format!(
            "column {column} holds {value:?}, which carries no f64 to reduce"
        ))
    })
}

fn item_key(value: &Value) -> Option<ItemKey> {
    match value {
        Value::Null => None,
        Value::Str(held) => Some(ItemKey::Str(held.clone())),
        Value::Int(held) | Value::Timestamp(held) => Some(ItemKey::Int(*held)),
        Value::Float(held) => Some(ItemKey::Float(*held)),
    }
}

fn numeric_literal(node: PostAsapNodeId, value: &ScalarValue) -> Result<(), Refusal> {
    match value {
        ScalarValue::Int64(_) | ScalarValue::Float64(_) => Ok(()),
        ScalarValue::Utf8(_) | ScalarValue::Boolean(_) | ScalarValue::Null => {
            Err(Refusal::UnsupportedValueOperation {
                node,
                detail: format!(
                    "Literal({}) carries no f64, and a value position holds one",
                    literal_name(value)
                ),
            })
        }
        ScalarValue::Interval { .. } => Err(Refusal::UnsupportedValueOperation {
            node,
            detail: "Literal(Interval) is not in the predicate subset".to_owned(),
        }),
    }
}

fn literal_name(value: &ScalarValue) -> &'static str {
    match value {
        ScalarValue::Int64(_) => "Int64",
        ScalarValue::Float64(_) => "Float64",
        ScalarValue::Utf8(_) => "Utf8",
        ScalarValue::Boolean(_) => "Boolean",
        ScalarValue::Null => "Null",
        ScalarValue::Interval { .. } => "Interval",
    }
}

fn source_name(source: &Source) -> String {
    match source {
        Source::TimeSeries { metric } => format!("TimeSeries({metric})"),
        Source::Table { table_ref } => format!("Table({table_ref})"),
    }
}

fn schema_name(schema: &Schema) -> String {
    let columns: Vec<&str> = schema
        .columns
        .iter()
        .map(|column| column.name.as_str())
        .collect();
    format!("[{}]", columns.join(", "))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    use asap_types::post_asap::ValueOperation;
    use asap_types::pre_asap::{
        ArithmeticOpKind, Column, CompareOpKind, DataType, GroupKeys, JoinKind, ProjectItem,
        Reduction, RelationalSetOpKind, SampleKind, SortKey, TimeShift, WindowFuncKind,
    };
    use asap_types::types::AccuracyTarget;

    use asap_frontend_promql::lower_promql;

    use crate::types::Value;

    const ACCURACY: AccuracyTarget = AccuracyTarget::Epsilon(0.01);

    fn node() -> PostAsapNodeId {
        PostAsapNodeId(3)
    }

    fn schema() -> Schema {
        Schema {
            columns: vec![
                Column::new("ts", DataType::Timestamp, false),
                Column::new("value", DataType::Float64, false),
                Column::new("cluster", DataType::Utf8, false),
            ],
            time_index: Some(0),
            unique_keys: Vec::new(),
            closed: false,
        }
    }

    fn fixture() -> Rc<Vec<Row>> {
        Rc::new(
            [(1i64, 10.0, "a"), (2, 20.0, "b"), (3, 30.0, "a")]
                .into_iter()
                .map(|(ts, value, cluster)| {
                    Row(vec![
                        Value::Timestamp(ts),
                        Value::Float(value),
                        Value::Str(cluster.to_owned()),
                    ])
                })
                .collect(),
        )
    }

    fn values(rows: &Rc<Vec<Row>>) -> Vec<f64> {
        rows.iter()
            .map(|row| match row.0[1] {
                Value::Float(value) => value,
                ref other => panic!("{other:?}"),
            })
            .collect()
    }

    fn compare(column: usize, op: CompareOpKind, literal: ScalarValue) -> Rc<QueryExpr> {
        Rc::new(QueryExpr::Compare {
            left: Rc::new(QueryExpr::Column(column)),
            op,
            right: Rc::new(QueryExpr::Literal(literal)),
        })
    }

    fn scan(predicates: Vec<Predicate>) -> Rc<QueryExpr> {
        Rc::new(QueryExpr::Scan {
            source: Source::TimeSeries {
                metric: "cpu_cores".into(),
            },
            predicates,
            schema: schema(),
        })
    }

    fn scan_of(metric: &str) -> Rc<QueryExpr> {
        Rc::new(QueryExpr::Scan {
            source: Source::TimeSeries {
                metric: metric.into(),
            },
            predicates: Vec::new(),
            schema: schema(),
        })
    }

    fn data_of(
        node: PostAsapNodeId,
        expr: &Rc<QueryExpr>,
        rows: &Rc<Vec<Row>>,
    ) -> Result<Data, EvalError> {
        evaluate(node, expr, rows).map(|(data, _)| data)
    }

    fn rows_from(data: Data) -> Rc<Vec<Row>> {
        match data {
            Data::Rows(rows) => rows,
            Data::Scalar(value) => panic!("expected rows, got the scalar {value}"),
        }
    }

    #[test]
    fn a_scan_applies_its_own_predicates_to_the_supplied_rows() {
        let tree = scan(vec![Predicate(compare(
            1,
            CompareOpKind::Gt,
            ScalarValue::Float64(15.0),
        ))]);
        let produced = rows_from(data_of(node(), &tree, &fixture()).expect("evaluates"));
        assert_eq!(values(&produced), vec![20.0, 30.0]);
    }

    #[test]
    fn a_scan_with_no_predicate_hands_back_the_rows_it_was_given() {
        let supplied = fixture();
        let produced = rows_from(data_of(node(), &scan(Vec::new()), &supplied).expect("evaluates"));
        assert!(Rc::ptr_eq(&supplied, &produced));
    }

    #[test]
    fn a_filter_over_a_scan_narrows_what_the_scan_emitted() {
        let tree = Rc::new(QueryExpr::Filter {
            pred: Predicate(compare(2, CompareOpKind::Eq, ScalarValue::Utf8("a".into()))),
            child: scan(vec![Predicate(compare(
                1,
                CompareOpKind::Gt,
                ScalarValue::Float64(15.0),
            ))]),
        });
        let produced = rows_from(data_of(node(), &tree, &fixture()).expect("evaluates"));
        assert_eq!(values(&produced), vec![30.0]);
    }

    #[test]
    fn a_project_agrees_with_the_post_asap_value_operation_on_the_same_rows() {
        let cols = vec![
            ProjectItem {
                alias: None,
                expr: QueryExpr::Column(2),
            },
            ProjectItem {
                alias: None,
                expr: QueryExpr::Column(1),
            },
        ];
        let tree = Rc::new(QueryExpr::Project {
            cols: cols.clone(),
            qualifier: None,
            child: scan(Vec::new()),
        });

        let through_exact = rows_from(data_of(node(), &tree, &fixture()).expect("evaluates"));
        let through_value = crate::value::apply(
            node(),
            &ValueOperation::Project {
                cols,
                qualifier: None,
            },
            &fixture(),
        )
        .expect("applies");

        assert_eq!(through_exact, through_value);
        assert_eq!(through_exact[0].0.len(), 2);
    }

    #[test]
    fn a_shared_rc_subtree_is_evaluated_once_and_every_reader_sees_the_same_rows() {
        let shared = Rc::new(QueryExpr::Filter {
            pred: Predicate(compare(1, CompareOpKind::Gt, ScalarValue::Float64(15.0))),
            child: scan(Vec::new()),
        });
        let supplied = fixture();
        let mut interpreter = Interpreter {
            node: node(),
            rows: &supplied,
            memo: HashMap::new(),
            leaf_emitted: None,
            node_times: None,
            children_ns: 0,
        };

        let first = rows_from(interpreter.eval_rc(&shared).expect("evaluates"));
        let memoized = interpreter.memo.len();
        let second = rows_from(interpreter.eval_rc(&shared).expect("evaluates"));

        assert!(Rc::ptr_eq(&first, &second));
        assert_eq!(interpreter.memo.len(), memoized);
        assert_eq!(values(&first), vec![20.0, 30.0]);
    }

    #[test]
    fn no_admitted_tree_can_hold_two_scans_so_the_leaf_guard_is_forward_defense() {
        let two_leaves = Rc::new(QueryExpr::Join {
            kind: JoinKind::Inner,
            pred: Predicate(Rc::new(QueryExpr::Literal(ScalarValue::Boolean(true)))),
            left: scan_of("cpu_cores"),
            right: scan_of("other_metric"),
        });
        let refusal = check(node(), &two_leaves).expect_err("refuses");
        assert!(refusal.to_string().contains("Join"), "{refusal}");

        let mut admission = Admission {
            node: node(),
            leaf: None,
        };
        admission
            .shape_of(&two_leaves)
            .expect_err("refuses before either leaf");
        assert!(admission.leaf.is_none());
    }

    #[test]
    fn the_leaf_guard_records_the_first_binding_and_conflicts_with_any_other() {
        let mut admission = Admission {
            node: node(),
            leaf: None,
        };
        admission
            .shape_of(&scan_of("cpu_cores"))
            .expect("admits the first leaf");
        admission
            .shape_of(&scan_of("cpu_cores"))
            .expect("admits the same leaf again");

        let refusal = admission
            .shape_of(&scan_of("other_metric"))
            .expect_err("refuses a second source");
        let rendered = refusal.to_string();
        assert!(rendered.contains("TimeSeries(other_metric)"), "{rendered}");
        assert!(rendered.contains("TimeSeries(cpu_cores)"), "{rendered}");
    }

    #[test]
    fn one_source_under_two_binding_schemas_is_refused_because_the_positions_differ() {
        let narrowed = Rc::new(QueryExpr::Scan {
            source: Source::TimeSeries {
                metric: "cpu_cores".into(),
            },
            predicates: Vec::new(),
            schema: Schema {
                columns: vec![
                    Column::new("ts", DataType::Timestamp, false),
                    Column::new("cluster", DataType::Utf8, false),
                ],
                time_index: Some(0),
                unique_keys: Vec::new(),
                closed: false,
            },
        });

        let mut admission = Admission {
            node: node(),
            leaf: None,
        };
        admission
            .shape_of(&scan_of("cpu_cores"))
            .expect("admits the first leaf");
        let refusal = admission
            .shape_of(&narrowed)
            .expect_err("refuses the rebound leaf");
        let rendered = refusal.to_string();
        assert!(rendered.contains("[ts, cluster]"), "{rendered}");
        assert!(rendered.contains("[ts, value, cluster]"), "{rendered}");
    }

    #[test]
    fn a_scalar_literal_and_the_promql_scalar_bridge_both_evaluate_to_a_scalar() {
        for (tree, expected) in [
            (Rc::new(QueryExpr::Literal(ScalarValue::Float64(2.5))), 2.5),
            (Rc::new(QueryExpr::promql_scalar(7.0)), 7.0),
            (Rc::new(QueryExpr::Literal(ScalarValue::Int64(4))), 4.0),
        ] {
            match data_of(node(), &tree, &fixture()).expect("evaluates") {
                Data::Scalar(value) => assert_eq!(value, expected),
                other => panic!("{other:?}"),
            }
        }
    }

    #[test]
    fn a_literal_with_no_f64_value_is_refused_rather_than_coerced() {
        for literal in [
            ScalarValue::Utf8("a".into()),
            ScalarValue::Boolean(true),
            ScalarValue::Null,
        ] {
            let tree = Rc::new(QueryExpr::Literal(literal.clone()));
            let refusal = check(node(), &tree).expect_err("refuses");
            assert!(
                refusal.to_string().contains(literal_name(&literal)),
                "{literal:?}: {refusal}"
            );
        }
    }

    fn deferred_variants() -> Vec<(&'static str, Rc<QueryExpr>)> {
        vec![
            (
                "TimeRange",
                Rc::new(QueryExpr::TimeRange {
                    range: Duration::from_secs(300),
                    child: scan(Vec::new()),
                }),
            ),
            (
                "PromqlSubquery",
                Rc::new(QueryExpr::PromqlSubquery {
                    range: Duration::from_secs(300),
                    resolution: None,
                    child: scan(Vec::new()),
                }),
            ),
            (
                "Join",
                Rc::new(QueryExpr::Join {
                    kind: JoinKind::Inner,
                    pred: Predicate(compare(1, CompareOpKind::Eq, ScalarValue::Float64(1.0))),
                    left: scan(Vec::new()),
                    right: scan(Vec::new()),
                }),
            ),
            (
                "SetOp",
                Rc::new(QueryExpr::SetOp {
                    kind: RelationalSetOpKind::Union,
                    all: true,
                    left: scan(Vec::new()),
                    right: scan(Vec::new()),
                }),
            ),
            (
                "Dedup",
                Rc::new(QueryExpr::Dedup {
                    cols: vec![0],
                    child: scan(Vec::new()),
                }),
            ),
            (
                "PromqlVectorFromScalar",
                Rc::new(QueryExpr::PromqlVectorFromScalar(Rc::new(
                    QueryExpr::promql_scalar(1.0),
                ))),
            ),
            (
                "PromqlScalarFromVector",
                Rc::new(QueryExpr::PromqlScalarFromVector(scan(Vec::new()))),
            ),
            (
                "PromqlRelabel",
                Rc::new(QueryExpr::PromqlRelabel {
                    dst: "cluster".into(),
                    value: Rc::new(QueryExpr::Literal(ScalarValue::Utf8("a".into()))),
                    child: scan(Vec::new()),
                }),
            ),
            (
                "PromqlInfoEnrich",
                Rc::new(QueryExpr::PromqlInfoEnrich {
                    selector: Vec::new(),
                    child: scan(Vec::new()),
                }),
            ),
            (
                "PromqlSeriesSample",
                Rc::new(QueryExpr::PromqlSeriesSample {
                    by: GroupKeys::default(),
                    kind: SampleKind::LimitK(2),
                    child: scan(Vec::new()),
                }),
            ),
            (
                "TimeShift",
                Rc::new(QueryExpr::TimeShift {
                    shift: TimeShift {
                        offset_ms: 300_000,
                        at: None,
                    },
                    child: scan(Vec::new()),
                }),
            ),
            (
                "SQLWindowFunc",
                Rc::new(QueryExpr::SQLWindowFunc {
                    func: WindowFuncKind::RowNumber,
                    args: Vec::new(),
                    partition_by: GroupKeys::default(),
                    order_by: vec![SortKey {
                        expr: QueryExpr::Column(1),
                        ascending: true,
                        nulls_first: false,
                    }],
                    frame: None,
                    output_name: "row_number".into(),
                    child: scan(Vec::new()),
                }),
            ),
            ("EvalTimestamp", Rc::new(QueryExpr::EvalTimestamp)),
            ("CurrentTimestamp", Rc::new(QueryExpr::CurrentTimestamp)),
            (
                "BinaryOp",
                Rc::new(QueryExpr::BinaryOp {
                    op: asap_types::pre_asap::BinaryOpKind::Compare(CompareOpKind::Gt),
                    lhs: scan(Vec::new()),
                    rhs: Rc::new(QueryExpr::promql_scalar(1.0)),
                    vector_match: None,
                }),
            ),
            ("Column", Rc::new(QueryExpr::Column(1))),
            (
                "Compare",
                compare(1, CompareOpKind::Gt, ScalarValue::Float64(1.0)),
            ),
            ("BoolAnd", Rc::new(QueryExpr::BoolAnd(Vec::new()))),
            ("BoolOr", Rc::new(QueryExpr::BoolOr(Vec::new()))),
            (
                "Not",
                Rc::new(QueryExpr::Not(Rc::new(QueryExpr::Literal(
                    ScalarValue::Boolean(true),
                )))),
            ),
            (
                "IsNull",
                Rc::new(QueryExpr::IsNull(Rc::new(QueryExpr::Column(1)))),
            ),
            (
                "IsNotNull",
                Rc::new(QueryExpr::IsNotNull(Rc::new(QueryExpr::Column(1)))),
            ),
            (
                "Cast",
                Rc::new(QueryExpr::Cast {
                    expr: Rc::new(QueryExpr::Column(1)),
                    to: DataType::Float64,
                    try_cast: false,
                }),
            ),
            (
                "InList",
                Rc::new(QueryExpr::InList {
                    expr: Rc::new(QueryExpr::Column(1)),
                    list: vec![QueryExpr::Literal(ScalarValue::Float64(1.0))],
                    negated: false,
                }),
            ),
            (
                "FunctionCall",
                Rc::new(QueryExpr::FunctionCall {
                    name: "lower".into(),
                    args: vec![QueryExpr::Column(2)],
                }),
            ),
            (
                "Arithmetic",
                Rc::new(QueryExpr::Arithmetic {
                    op: ArithmeticOpKind::Add,
                    left: Rc::new(QueryExpr::Column(1)),
                    right: Rc::new(QueryExpr::Literal(ScalarValue::Float64(1.0))),
                }),
            ),
            (
                "Case",
                Rc::new(QueryExpr::Case {
                    operand: None,
                    branches: vec![(
                        QueryExpr::Literal(ScalarValue::Boolean(true)),
                        QueryExpr::Literal(ScalarValue::Float64(1.0)),
                    )],
                    else_expr: None,
                }),
            ),
        ]
    }

    #[test]
    fn every_variant_outside_this_step_is_refused_by_name_before_a_row_is_read() {
        for (name, tree) in deferred_variants() {
            let refusal = check(node(), &tree).expect_err("refuses");
            assert!(refusal.to_string().contains(name), "{name}: {refusal}");

            match data_of(node(), &tree, &fixture()).expect_err("refuses") {
                EvalError::Refused(refusals) => assert!(
                    refusals
                        .iter()
                        .any(|refusal| refusal.to_string().contains(name)),
                    "{name}: {refusals:?}"
                ),
                other => panic!("{name}: {other:?}"),
            }
        }
    }

    fn admitted_variants() -> Vec<(&'static str, Rc<QueryExpr>)> {
        vec![
            ("Scan", scan(Vec::new())),
            (
                "Scan",
                scan(vec![Predicate(compare(
                    1,
                    CompareOpKind::Ge,
                    ScalarValue::Float64(0.0),
                ))]),
            ),
            (
                "Filter",
                Rc::new(QueryExpr::Filter {
                    pred: Predicate(compare(1, CompareOpKind::Lt, ScalarValue::Float64(25.0))),
                    child: scan(Vec::new()),
                }),
            ),
            (
                "Project",
                Rc::new(QueryExpr::Project {
                    cols: vec![ProjectItem {
                        alias: None,
                        expr: QueryExpr::Column(0),
                    }],
                    qualifier: None,
                    child: scan(Vec::new()),
                }),
            ),
            (
                "Aggregate",
                aggregate(
                    Reduction::by(Vec::new()),
                    vec![AggIntent::Count { accuracy: ACCURACY }],
                    scan(Vec::new()),
                ),
            ),
            (
                "Aggregate",
                aggregate(
                    Reduction::by(vec![2]),
                    vec![
                        AggIntent::Count { accuracy: ACCURACY },
                        AggIntent::Sum { col: None },
                        AggIntent::Min { col: None },
                        AggIntent::Max { col: None },
                        AggIntent::Avg { col: None },
                        quantile(0.5),
                        cardinality(None),
                    ],
                    scan(Vec::new()),
                ),
            ),
            (
                "Aggregate",
                aggregate(
                    Reduction::by(vec![2]),
                    vec![AggIntent::Sum { col: None }],
                    Rc::new(QueryExpr::Filter {
                        pred: Predicate(compare(1, CompareOpKind::Gt, ScalarValue::Float64(0.0))),
                        child: scan(Vec::new()),
                    }),
                ),
            ),
            (
                "Literal",
                Rc::new(QueryExpr::Literal(ScalarValue::Float64(1.0))),
            ),
            ("PromqlScalarBridge", Rc::new(QueryExpr::promql_scalar(1.0))),
            ("Limit", limit(2, 1, scan(Vec::new()))),
            (
                "Concat",
                Rc::new(QueryExpr::concat(vec![
                    (*scan(Vec::new())).clone(),
                    (*scan(Vec::new())).clone(),
                ])),
            ),
            ("Sort", sorted(vec![key(1, true, false)], scan(Vec::new()))),
        ]
    }

    fn key(column: ColumnId, ascending: bool, nulls_first: bool) -> SortKey {
        SortKey {
            expr: QueryExpr::Column(column),
            ascending,
            nulls_first,
        }
    }

    fn sorted(keys: Vec<SortKey>, child: Rc<QueryExpr>) -> Rc<QueryExpr> {
        Rc::new(QueryExpr::Sort {
            keys,
            partition_by: GroupKeys::default(),
            child,
        })
    }

    fn limit(n: usize, offset: usize, child: Rc<QueryExpr>) -> Rc<QueryExpr> {
        Rc::new(QueryExpr::Limit { n, offset, child })
    }

    #[test]
    fn the_deferred_table_names_each_variant_once_and_none_this_step_runs() {
        let mut seen: Vec<&'static str> =
            deferred_variants().iter().map(|(name, _)| *name).collect();
        let before = seen.len();
        seen.sort_unstable();
        seen.dedup();
        assert_eq!(seen.len(), before);

        let mut admitted: Vec<&'static str> =
            admitted_variants().iter().map(|(name, _)| *name).collect();
        admitted.sort_unstable();
        admitted.dedup();
        assert_eq!(
            admitted,
            vec![
                "Aggregate",
                "Concat",
                "Filter",
                "Limit",
                "Literal",
                "Project",
                "PromqlScalarBridge",
                "Scan",
                "Sort"
            ]
        );

        for name in admitted {
            assert!(!seen.contains(&name), "{name}");
        }
    }

    #[test]
    fn every_name_in_the_two_tables_is_the_one_variant_name_gives_that_tree() {
        for (name, tree) in deferred_variants().into_iter().chain(admitted_variants()) {
            assert_eq!(variant_name(&tree), name);
        }
    }

    #[test]
    fn a_relational_variant_in_a_scalar_position_is_refused_by_name() {
        let tree = Rc::new(QueryExpr::Filter {
            pred: Predicate(Rc::new(QueryExpr::Compare {
                left: scan(Vec::new()),
                op: CompareOpKind::Gt,
                right: Rc::new(QueryExpr::Literal(ScalarValue::Float64(1.0))),
            })),
            child: scan(Vec::new()),
        });
        let refusal = check(node(), &tree).expect_err("refuses");
        let rendered = refusal.to_string();
        assert!(rendered.contains("Filter.pred"), "{rendered}");
        assert!(rendered.contains("Scan"), "{rendered}");
    }

    #[test]
    fn a_scalar_variant_outside_the_predicate_subset_is_refused_by_name() {
        let tree = Rc::new(QueryExpr::Filter {
            pred: Predicate(Rc::new(QueryExpr::FunctionCall {
                name: "lower".into(),
                args: vec![QueryExpr::Column(2)],
            })),
            child: scan(Vec::new()),
        });
        let refusal = check(node(), &tree).expect_err("refuses");
        assert!(refusal.to_string().contains("FunctionCall"), "{refusal}");
    }

    #[test]
    fn a_projection_past_the_end_of_its_input_is_refused_as_an_unresolvable_column() {
        let tree = Rc::new(QueryExpr::Project {
            cols: vec![ProjectItem {
                alias: None,
                expr: QueryExpr::Column(9),
            }],
            qualifier: None,
            child: scan(Vec::new()),
        });
        let refusal = check(node(), &tree).expect_err("refuses");
        assert!(
            matches!(refusal, Refusal::UnresolvableColumn { .. }),
            "{refusal:?}"
        );
    }

    #[test]
    fn an_out_of_range_column_is_the_same_typed_refusal_from_either_arm() {
        let pred = Predicate(compare(9, CompareOpKind::Gt, ScalarValue::Float64(0.0)));

        let pre_asap = check(
            node(),
            &QueryExpr::Filter {
                pred: pred.clone(),
                child: scan(Vec::new()),
            },
        )
        .expect_err("refuses");
        let post_asap =
            crate::value::check(node(), &ValueOperation::Filter { pred }, 3).expect_err("refuses");

        assert_eq!(
            std::mem::discriminant(&pre_asap),
            std::mem::discriminant(&post_asap)
        );
        assert!(
            matches!(pre_asap, Refusal::UnresolvableColumn { .. }),
            "{pre_asap:?}"
        );
        assert!(
            matches!(post_asap, Refusal::UnresolvableColumn { .. }),
            "{post_asap:?}"
        );
    }

    #[test]
    fn check_admits_nothing_the_evaluator_falls_back_on() {
        for (name, tree) in admitted_variants() {
            check(node(), &tree).unwrap_or_else(|refusal| panic!("{name}: {refusal}"));
            data_of(node(), &tree, &fixture())
                .unwrap_or_else(|err| panic!("{name} was admitted but not run: {err:?}"));
        }

        for (name, tree) in deferred_variants() {
            assert!(check(node(), &tree).is_err(), "{name} was admitted");
        }
    }

    #[test]
    fn a_lowered_promql_selector_is_a_scan_this_step_runs() {
        let lowered = Rc::new(lower_promql("cpu_cores", ACCURACY).expect("lowers"));
        assert_eq!(variant_name(&lowered), "Scan");
        let produced = rows_from(data_of(node(), &lowered, &fixture()).expect("evaluates"));
        assert_eq!(produced.len(), fixture().len());
    }

    #[test]
    fn a_lowered_promql_aggregation_is_an_aggregate_this_step_runs() {
        let lowered = Rc::new(lower_promql("sum(cpu_cores)", ACCURACY).expect("lowers"));
        assert_eq!(variant_name(&lowered), "Aggregate");
        assert_eq!(
            rows_of(&lowered, &fixture()),
            vec![Row(vec![Value::Float(60.0)])]
        );

        let grouped =
            Rc::new(lower_promql("sum by (cluster) (cpu_cores)", ACCURACY).expect("lowers"));
        assert_eq!(
            rows_of(&grouped, &fixture()),
            vec![
                Row(vec![Value::Str("a".to_owned()), Value::Float(40.0)]),
                Row(vec![Value::Str("b".to_owned()), Value::Float(20.0)]),
            ]
        );
    }

    #[test]
    fn a_reduction_over_a_label_column_is_refused_rather_than_answered() {
        let lowered =
            Rc::new(lower_promql("max(sum by (cluster) (cpu_cores))", ACCURACY).expect("lowers"));

        let refusal = check(node(), &lowered).expect_err("a label is not a maximum");
        let spelled = format!("{refusal}");
        assert!(
            spelled.contains("Max") || spelled.contains("value"),
            "the refusal must name what it could not read: {spelled}"
        );

        let err = data_of(node(), &lowered, &fixture()).expect_err("and it does not evaluate");
        assert!(matches!(err, EvalError::Refused(_)), "{err:?}");
    }

    fn aggregate(
        reduction: Reduction,
        measures: Vec<AggIntent>,
        child: Rc<QueryExpr>,
    ) -> Rc<QueryExpr> {
        Rc::new(QueryExpr::Aggregate {
            reduction,
            measures,
            output_names: Vec::new(),
            having: None,
            child,
        })
    }

    fn global(measures: Vec<AggIntent>) -> Rc<QueryExpr> {
        aggregate(Reduction::by(Vec::new()), measures, scan(Vec::new()))
    }

    fn quantile(q: f64) -> AggIntent {
        AggIntent::Quantile {
            col: None,
            q,
            accuracy: ACCURACY,
        }
    }

    fn cardinality(col: Option<ColumnId>) -> AggIntent {
        AggIntent::Cardinality {
            col,
            accuracy: ACCURACY,
        }
    }

    fn rows_of(tree: &Rc<QueryExpr>, rows: &Rc<Vec<Row>>) -> Vec<Row> {
        rows_from(data_of(node(), tree, rows).expect("evaluates"))
            .as_ref()
            .clone()
    }

    #[test]
    fn a_global_aggregate_collapses_every_row_into_exactly_one_row() {
        let tree = global(vec![
            AggIntent::Count { accuracy: ACCURACY },
            AggIntent::Sum { col: None },
        ]);
        assert_eq!(
            rows_of(&tree, &fixture()),
            vec![Row(vec![Value::Int(3), Value::Float(60.0)])]
        );
    }

    #[test]
    fn a_grouped_aggregate_emits_the_group_values_before_the_measures() {
        let tree = aggregate(
            Reduction::by(vec![2]),
            vec![AggIntent::Sum { col: None }, AggIntent::Max { col: None }],
            scan(Vec::new()),
        );
        assert_eq!(
            rows_of(&tree, &fixture()),
            vec![
                Row(vec![
                    Value::Str("a".to_owned()),
                    Value::Float(40.0),
                    Value::Float(30.0)
                ]),
                Row(vec![
                    Value::Str("b".to_owned()),
                    Value::Float(20.0),
                    Value::Float(20.0)
                ]),
            ]
        );
    }

    #[test]
    fn several_measures_on_one_node_each_land_in_their_own_position() {
        let tree = aggregate(
            Reduction::by(vec![2]),
            vec![
                AggIntent::Count { accuracy: ACCURACY },
                AggIntent::Sum { col: None },
                AggIntent::Min { col: None },
                AggIntent::Max { col: None },
                AggIntent::Avg { col: None },
                quantile(0.5),
                cardinality(None),
            ],
            scan(Vec::new()),
        );
        let produced = rows_of(&tree, &fixture());
        assert_eq!(
            produced[0],
            Row(vec![
                Value::Str("a".to_owned()),
                Value::Int(2),
                Value::Float(40.0),
                Value::Float(10.0),
                Value::Float(30.0),
                Value::Float(20.0),
                Value::Float(30.0),
                Value::Int(2),
            ])
        );
        assert_eq!(
            produced[1],
            Row(vec![
                Value::Str("b".to_owned()),
                Value::Int(1),
                Value::Float(20.0),
                Value::Float(20.0),
                Value::Float(20.0),
                Value::Float(20.0),
                Value::Float(20.0),
                Value::Int(1),
            ])
        );
    }

    #[test]
    fn each_supported_intent_reduces_the_column_its_col_names() {
        for (intent, expected) in [
            (AggIntent::Count { accuracy: ACCURACY }, Value::Int(3)),
            (AggIntent::Sum { col: None }, Value::Float(60.0)),
            (AggIntent::Min { col: None }, Value::Float(10.0)),
            (AggIntent::Max { col: None }, Value::Float(30.0)),
            (AggIntent::Avg { col: None }, Value::Float(20.0)),
            (quantile(0.0), Value::Float(10.0)),
            (quantile(0.5), Value::Float(20.0)),
            (quantile(1.0), Value::Float(30.0)),
            (cardinality(None), Value::Int(3)),
            (cardinality(Some(2)), Value::Int(2)),
        ] {
            let produced = rows_of(&global(vec![intent.clone()]), &fixture());
            assert_eq!(produced, vec![Row(vec![expected])], "{intent:?}");
        }
    }

    #[test]
    fn the_statistics_with_a_sketch_query_counterpart_agree_with_score_exact_answer() {
        let rows = fixture();
        let mut numbers = Retained::default();
        let mut keys = Retained::default();
        for row in rows.iter() {
            numbers.push(None, row.0[1].as_f64().expect("a numeric sample value"));
            keys.push(item_key(&row.0[2]).as_ref(), 1.0);
        }
        numbers.weights.sort_by(f64::total_cmp);

        for (intent, query, retained) in [
            (quantile(0.5), SketchQuery::Quantile { q: 0.5 }, &numbers),
            (AggIntent::Sum { col: None }, summed(), &numbers),
            (AggIntent::Count { accuracy: ACCURACY }, counted(), &numbers),
            (cardinality(Some(2)), SketchQuery::Cardinality, &keys),
        ] {
            let produced = rows_of(&global(vec![intent.clone()]), &rows);
            let truth = match score::exact_answer(retained, &query).expect("answers") {
                Answer::Scalar(value) => value,
                other => panic!("{intent:?}: {other:?}"),
            };
            assert_eq!(
                produced[0].0[0].as_f64().expect("a numeric measure"),
                truth,
                "{intent:?} disagrees with {query:?}"
            );
        }
    }

    #[test]
    fn the_emitted_row_width_is_the_one_output_schema_declares() {
        for tree in [
            global(vec![AggIntent::Count { accuracy: ACCURACY }]),
            global(vec![AggIntent::Sum { col: None }, quantile(0.9)]),
            aggregate(
                Reduction::by(vec![2]),
                vec![AggIntent::Avg { col: None }],
                scan(Vec::new()),
            ),
            aggregate(
                Reduction::by(vec![2, 0]),
                vec![
                    AggIntent::Min { col: None },
                    AggIntent::Max { col: None },
                    cardinality(None),
                ],
                scan(Vec::new()),
            ),
        ] {
            let declared = tree.output_schema().expect("has a schema").columns.len();
            let produced = rows_of(&tree, &fixture());
            assert!(!produced.is_empty());
            for row in &produced {
                assert_eq!(row.0.len(), declared, "{tree:?}");
            }
        }
    }

    #[test]
    fn output_names_rename_a_column_without_moving_or_adding_one() {
        let measures = vec![
            AggIntent::Sum { col: None },
            AggIntent::Count { accuracy: ACCURACY },
        ];
        let bare = QueryExpr::Aggregate {
            reduction: Reduction::by(vec![2]),
            measures: measures.clone(),
            output_names: Vec::new(),
            having: None,
            child: scan(Vec::new()),
        };
        let named = QueryExpr::Aggregate {
            reduction: Reduction::by(vec![2]),
            measures,
            output_names: vec!["sum(metrics.bytes)".to_owned(), "count(*)".to_owned()],
            having: None,
            child: scan(Vec::new()),
        };

        let bare_schema = bare.output_schema().expect("has a schema");
        let named_schema = named.output_schema().expect("has a schema");
        assert_eq!(bare_schema.columns.len(), named_schema.columns.len());
        assert_eq!(bare_schema.columns[1].name, "sum");
        assert_eq!(named_schema.columns[1].name, "sum(metrics.bytes)");
        assert_eq!(
            rows_of(&Rc::new(bare), &fixture()),
            rows_of(&Rc::new(named), &fixture())
        );
    }

    #[test]
    fn a_null_is_skipped_by_every_measure_and_counted_only_by_count() {
        let rows = Rc::new(vec![
            Row(vec![
                Value::Timestamp(1),
                Value::Float(10.0),
                Value::Str("a".to_owned()),
            ]),
            Row(vec![
                Value::Timestamp(2),
                Value::Null,
                Value::Str("a".to_owned()),
            ]),
            Row(vec![
                Value::Timestamp(3),
                Value::Float(30.0),
                Value::Str("a".to_owned()),
            ]),
        ]);
        let tree = global(vec![
            AggIntent::Count { accuracy: ACCURACY },
            AggIntent::Sum { col: None },
            AggIntent::Avg { col: None },
            AggIntent::Min { col: None },
            AggIntent::Max { col: None },
            quantile(0.5),
            cardinality(None),
        ]);
        assert_eq!(
            rows_of(&tree, &rows),
            vec![Row(vec![
                Value::Int(3),
                Value::Float(40.0),
                Value::Float(20.0),
                Value::Float(10.0),
                Value::Float(30.0),
                Value::Float(30.0),
                Value::Int(2),
            ])]
        );
    }

    #[test]
    fn a_group_whose_measured_column_is_entirely_null_reads_back_null_not_zero() {
        let rows = Rc::new(vec![Row(vec![
            Value::Timestamp(1),
            Value::Null,
            Value::Str("a".to_owned()),
        ])]);
        let tree = aggregate(
            Reduction::by(vec![2]),
            vec![
                AggIntent::Count { accuracy: ACCURACY },
                AggIntent::Sum { col: None },
                AggIntent::Avg { col: None },
                AggIntent::Min { col: None },
                quantile(0.5),
                cardinality(None),
            ],
            scan(Vec::new()),
        );
        assert_eq!(
            rows_of(&tree, &rows),
            vec![Row(vec![
                Value::Str("a".to_owned()),
                Value::Int(1),
                Value::Null,
                Value::Null,
                Value::Null,
                Value::Null,
                Value::Int(0),
            ])]
        );
    }

    #[test]
    fn an_empty_input_is_one_row_when_by_is_empty_and_no_rows_when_it_is_not() {
        let empty: Rc<Vec<Row>> = Rc::new(Vec::new());
        let measures = vec![
            AggIntent::Count { accuracy: ACCURACY },
            AggIntent::Sum { col: None },
            quantile(0.5),
            cardinality(None),
        ];

        assert_eq!(
            rows_of(&global(measures.clone()), &empty),
            vec![Row(vec![
                Value::Int(0),
                Value::Null,
                Value::Null,
                Value::Int(0),
            ])]
        );

        let grouped = aggregate(Reduction::by(vec![2]), measures, scan(Vec::new()));
        assert_eq!(rows_of(&grouped, &empty), Vec::new());
    }

    #[test]
    fn a_nan_propagates_through_sum_and_orders_last_under_total_cmp() {
        let rows = Rc::new(
            [10.0f64, f64::NAN, 30.0]
                .into_iter()
                .enumerate()
                .map(|(i, value)| {
                    Row(vec![
                        Value::Timestamp(i as i64),
                        Value::Float(value),
                        Value::Str("a".to_owned()),
                    ])
                })
                .collect::<Vec<Row>>(),
        );
        let tree = global(vec![
            AggIntent::Sum { col: None },
            AggIntent::Min { col: None },
            AggIntent::Max { col: None },
            quantile(0.5),
        ]);
        let produced = rows_of(&tree, &rows);
        let row = &produced[0].0;
        assert!(matches!(row[0], Value::Float(value) if value.is_nan()));
        assert_eq!(row[1], Value::Float(10.0));
        assert!(matches!(row[2], Value::Float(value) if value.is_nan()));
        assert_eq!(row[3], Value::Float(30.0));
    }

    fn wide_scan() -> Rc<QueryExpr> {
        Rc::new(QueryExpr::Scan {
            source: Source::TimeSeries {
                metric: "cpu_cores".into(),
            },
            predicates: Vec::new(),
            schema: Schema {
                columns: vec![
                    Column::new("ts", DataType::Timestamp, false),
                    Column::new("value", DataType::Float64, false),
                    Column::new("shard", DataType::Int64, false),
                    Column::new("bytes", DataType::Int64, false),
                    Column::new("cluster", DataType::Utf8, false),
                    Column::new("zone", DataType::Utf8, false),
                ],
                time_index: Some(0),
                unique_keys: Vec::new(),
                closed: false,
            },
        })
    }

    fn wide_rows(held: &[(i64, f64, i64, i64, &str, &str)]) -> Rc<Vec<Row>> {
        Rc::new(
            held.iter()
                .map(|(ts, value, shard, bytes, cluster, zone)| {
                    Row(vec![
                        Value::Timestamp(*ts),
                        Value::Float(*value),
                        Value::Int(*shard),
                        Value::Int(*bytes),
                        Value::Str((*cluster).to_owned()),
                        Value::Str((*zone).to_owned()),
                    ])
                })
                .collect(),
        )
    }

    #[test]
    fn two_groups_a_joined_string_key_would_merge_stay_distinct() {
        let rows = wide_rows(&[
            (1, 10.0, 0, 0, "a;b", "c"),
            (2, 20.0, 0, 0, "a", "b;c"),
            (3, 5.0, 0, 0, "a;b", "c"),
        ]);
        let tree = aggregate(
            Reduction::by(vec![4, 5]),
            vec![AggIntent::Sum { col: None }],
            wide_scan(),
        );
        assert_eq!(
            rows_of(&tree, &rows),
            vec![
                Row(vec![
                    Value::Str("a".to_owned()),
                    Value::Str("b;c".to_owned()),
                    Value::Float(20.0),
                ]),
                Row(vec![
                    Value::Str("a;b".to_owned()),
                    Value::Str("c".to_owned()),
                    Value::Float(15.0),
                ]),
            ]
        );
    }

    #[test]
    fn the_groups_come_out_in_value_order_not_in_the_text_order_of_a_joined_key() {
        let rows = wide_rows(&[
            (1, 1.0, 9, 0, "", ""),
            (2, 1.0, 10, 0, "", ""),
            (3, 1.0, 2, 0, "", ""),
            (4, 1.0, 100, 0, "", ""),
        ]);
        let tree = aggregate(
            Reduction::by(vec![2]),
            vec![AggIntent::Count { accuracy: ACCURACY }],
            wide_scan(),
        );
        let emitted: Vec<i64> = rows_of(&tree, &rows)
            .iter()
            .map(|row| match row.0[0] {
                Value::Int(held) => held,
                ref other => panic!("{other:?}"),
            })
            .collect();
        assert_eq!(emitted, vec![2, 9, 10, 100]);
    }

    #[test]
    fn the_groups_come_out_in_the_same_order_every_run() {
        let mut rows = Vec::new();
        for shard in 0..12i64 {
            for step in 0..5i64 {
                rows.push(Row(vec![
                    Value::Timestamp(step),
                    Value::Float((shard * 10 + step) as f64),
                    Value::Int((7 * shard + 3) % 12 * 9),
                    Value::Int(0),
                    Value::Str(String::new()),
                    Value::Str(String::new()),
                ]));
            }
        }
        let rows = Rc::new(rows);
        let tree = aggregate(
            Reduction::by(vec![2]),
            vec![AggIntent::Sum { col: None }],
            wide_scan(),
        );

        let groups = || -> Vec<i64> {
            rows_of(&tree, &rows)
                .iter()
                .map(|row| match row.0[0] {
                    Value::Int(held) => held,
                    ref other => panic!("{other:?}"),
                })
                .collect()
        };

        let first = groups();
        assert_eq!(first.len(), 12, "one row per shard");
        let mut sorted = first.clone();
        sorted.sort_unstable();
        assert_eq!(first, sorted, "a HashMap's iteration order is not an order");
        for _ in 0..5 {
            assert_eq!(groups(), first, "group order moved between runs");
        }
    }

    #[test]
    fn a_measured_column_whose_declared_type_carries_no_number_is_refused_unread() {
        let rows = wide_rows(&[(1, 10.0, 1, 7, "a", "b")]);
        for (name, intent) in [
            ("Sum", AggIntent::Sum { col: Some(4) }),
            ("Sum", AggIntent::Sum { col: Some(0) }),
            ("Avg", AggIntent::Avg { col: Some(4) }),
            (
                "Quantile",
                AggIntent::Quantile {
                    col: Some(4),
                    q: 0.5,
                    accuracy: ACCURACY,
                },
            ),
        ] {
            let tree = aggregate(Reduction::by(vec![2]), vec![intent.clone()], wide_scan());
            let refusal = check(node(), &tree).expect_err("refuses");
            assert!(
                matches!(refusal, Refusal::UnsupportedValueOperation { .. }),
                "{intent:?}: {refusal:?}"
            );
            assert!(refusal.to_string().contains(name), "{refusal}");
            assert!(
                matches!(
                    data_of(node(), &tree, &rows).expect_err("refuses"),
                    EvalError::Refused(_)
                ),
                "{intent:?} reached a row"
            );
        }
    }

    #[test]
    fn min_max_and_cardinality_still_reduce_a_text_column_they_can_order() {
        let rows = wide_rows(&[
            (1, 1.0, 0, 0, "b", ""),
            (2, 1.0, 0, 0, "a", ""),
            (3, 1.0, 0, 0, "b", ""),
        ]);
        let tree = aggregate(
            Reduction::by(Vec::new()),
            vec![
                AggIntent::Min { col: Some(4) },
                AggIntent::Max { col: Some(4) },
                cardinality(Some(4)),
            ],
            wide_scan(),
        );
        assert_eq!(
            rows_of(&tree, &rows),
            vec![Row(vec![
                Value::Str("a".to_owned()),
                Value::Str("b".to_owned()),
                Value::Int(2),
            ])]
        );
    }

    fn integer_sum(held: &[i64]) -> Value {
        let rows = Rc::new(
            held.iter()
                .enumerate()
                .map(|(i, bytes)| {
                    Row(vec![
                        Value::Timestamp(i as i64),
                        Value::Float(0.0),
                        Value::Int(0),
                        Value::Int(*bytes),
                        Value::Str(String::new()),
                        Value::Str(String::new()),
                    ])
                })
                .collect::<Vec<Row>>(),
        );
        let tree = aggregate(
            Reduction::by(Vec::new()),
            vec![AggIntent::Sum { col: Some(3) }],
            wide_scan(),
        );
        rows_of(&tree, &rows)[0].0[0].clone()
    }

    #[test]
    fn an_integer_sum_agrees_with_score_exact_answer_on_inputs_an_f64_holds_exactly() {
        for held in [
            vec![3i64, 4],
            vec![-5, 5, 0],
            vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 10],
            vec![1 << 40, 1 << 41],
            vec![(1i64 << 53) - 1, 0],
        ] {
            let mut retained = Retained::default();
            for value in &held {
                retained.push(None, *value as f64);
            }
            let truth = match score::exact_answer(&retained, &summed()).expect("answers") {
                Answer::Scalar(value) => value,
                other => panic!("{other:?}"),
            };
            assert_eq!(
                integer_sum(&held),
                Value::Int(truth as i64),
                "{held:?} disagrees with the f64 readout it is exactly representable in"
            );
        }
    }

    #[test]
    fn an_integer_sum_keeps_the_precision_an_f64_sum_would_lose() {
        let held = vec![(1i64 << 53) + 1, 0];
        let mut retained = Retained::default();
        for value in &held {
            retained.push(None, *value as f64);
        }
        let through_f64 = match score::exact_answer(&retained, &summed()).expect("answers") {
            Answer::Scalar(value) => value,
            other => panic!("{other:?}"),
        };
        assert_eq!(through_f64 as i64, (1i64 << 53));
        assert_eq!(integer_sum(&held), Value::Int((1i64 << 53) + 1));
    }

    #[test]
    fn an_integer_sum_declares_the_int64_its_output_schema_promises() {
        let tree = aggregate(
            Reduction::by(Vec::new()),
            vec![
                AggIntent::Sum { col: Some(3) },
                AggIntent::Sum { col: None },
            ],
            wide_scan(),
        );
        let declared = tree.output_schema().expect("has a schema");
        assert_eq!(declared.columns[0].dtype, DataType::Int64);
        assert_eq!(declared.columns[1].dtype, DataType::Float64);

        let rows = wide_rows(&[(1, 1.5, 0, 3, "", ""), (2, 2.5, 0, 4, "", "")]);
        assert_eq!(
            rows_of(&tree, &rows),
            vec![Row(vec![Value::Int(7), Value::Float(4.0)])]
        );
    }

    #[test]
    fn a_without_grouping_is_refused_because_an_open_schema_has_no_complement() {
        let tree = aggregate(
            Reduction::Reduce(GroupKeys::without(vec![2])),
            vec![AggIntent::Sum { col: None }],
            scan(Vec::new()),
        );
        let refusal = check(node(), &tree).expect_err("refuses");
        let rendered = refusal.to_string();
        assert!(rendered.contains("GroupKeys::without"), "{rendered}");
        assert!(rendered.contains("open"), "{rendered}");

        let lowered =
            Rc::new(lower_promql("sum without (cluster) (cpu_cores)", ACCURACY).expect("lowers"));
        assert!(check(node(), &lowered)
            .expect_err("refuses")
            .to_string()
            .contains("GroupKeys::without"));
    }

    #[test]
    fn a_per_entity_reduction_is_refused_in_the_post_asap_arms_own_words() {
        let tree = aggregate(
            Reduction::PerEntity,
            vec![AggIntent::Sum { col: None }],
            scan(Vec::new()),
        );
        let refusal = check(node(), &tree).expect_err("refuses");
        assert!(
            refusal
                .to_string()
                .contains("Reduction::PerEntity has no entity concept over CSV rows"),
            "{refusal}"
        );
    }

    #[test]
    fn a_having_predicate_is_refused_rather_than_admitted_and_dropped() {
        let tree = Rc::new(QueryExpr::Aggregate {
            reduction: Reduction::by(vec![2]),
            measures: vec![AggIntent::Sum { col: None }],
            output_names: Vec::new(),
            having: Some(Predicate(compare(
                1,
                CompareOpKind::Gt,
                ScalarValue::Float64(0.0),
            ))),
            child: scan(Vec::new()),
        });
        let refusal = check(node(), &tree).expect_err("refuses");
        assert!(
            refusal.to_string().contains("Aggregate.having"),
            "{refusal}"
        );
    }

    #[test]
    fn an_intent_outside_the_supported_seven_is_refused_by_name() {
        for (name, intent) in [
            (
                "StdDev",
                AggIntent::StdDev {
                    col: None,
                    population: false,
                },
            ),
            (
                "Variance",
                AggIntent::Variance {
                    col: None,
                    population: true,
                },
            ),
            (
                "TopK",
                AggIntent::TopK {
                    k: 3,
                    accuracy: ACCURACY,
                },
            ),
            ("Rate", AggIntent::Rate),
            ("Group", AggIntent::Group),
            (
                "CountValues",
                AggIntent::CountValues {
                    label: "l".to_owned(),
                },
            ),
            (
                "FrequencyL2",
                AggIntent::FrequencyL2 {
                    col: None,
                    accuracy: ACCURACY,
                },
            ),
            ("HistogramQuantile", AggIntent::HistogramQuantile { q: 0.9 }),
            (
                "Extension",
                AggIntent::Extension {
                    ext_kind: "arg_max".to_owned(),
                    payload: serde_json::Value::Null,
                },
            ),
        ] {
            let tree = global(vec![intent.clone()]);
            let refusal = check(node(), &tree).expect_err("refuses");
            assert!(refusal.to_string().contains(name), "{intent:?}: {refusal}");

            match data_of(node(), &tree, &fixture()).expect_err("refuses") {
                EvalError::Refused(refusals) => assert!(
                    refusals
                        .iter()
                        .any(|refusal| refusal.to_string().contains(name)),
                    "{intent:?}: {refusals:?}"
                ),
                other => panic!("{intent:?}: {other:?}"),
            }
        }
    }

    #[test]
    fn a_measure_or_group_column_past_the_childs_output_schema_is_refused() {
        for tree in [
            global(vec![AggIntent::Sum { col: Some(9) }]),
            aggregate(
                Reduction::by(vec![9]),
                vec![AggIntent::Sum { col: None }],
                scan(Vec::new()),
            ),
        ] {
            let refusal = check(node(), &tree).expect_err("refuses");
            assert!(
                matches!(refusal, Refusal::UnresolvableColumn { .. }),
                "{refusal:?}"
            );
        }
    }

    #[test]
    fn a_quantile_outside_zero_to_one_is_refused_before_a_row_is_read() {
        for q in [-0.5, 1.5, f64::NAN] {
            let refusal = check(node(), &global(vec![quantile(q)])).expect_err("refuses");
            assert!(
                matches!(refusal, Refusal::ParameterOutOfBounds { .. }),
                "{q}: {refusal:?}"
            );
        }
    }

    #[test]
    fn an_aggregate_resolves_its_columns_against_the_child_not_the_scan_binding() {
        let narrowed = Rc::new(QueryExpr::Project {
            cols: vec![ProjectItem {
                alias: None,
                expr: QueryExpr::Column(2),
            }],
            qualifier: None,
            child: scan(Vec::new()),
        });

        let tree = aggregate(
            Reduction::by(Vec::new()),
            vec![cardinality(Some(0))],
            Rc::clone(&narrowed),
        );
        assert_eq!(rows_of(&tree, &fixture()), vec![Row(vec![Value::Int(2)])]);

        let past_the_end = aggregate(
            Reduction::by(Vec::new()),
            vec![cardinality(Some(1))],
            narrowed,
        );
        assert!(
            matches!(
                check(node(), &past_the_end).expect_err("refuses"),
                Refusal::UnresolvableColumn { .. }
            ),
            "the scan binds 3 columns, and the Project hands the Aggregate 1"
        );
    }

    #[test]
    fn an_aggregate_under_a_filter_reduces_only_the_rows_the_filter_kept() {
        let tree = aggregate(
            Reduction::by(vec![2]),
            vec![AggIntent::Sum { col: None }],
            Rc::new(QueryExpr::Filter {
                pred: Predicate(compare(1, CompareOpKind::Gt, ScalarValue::Float64(15.0))),
                child: scan(Vec::new()),
            }),
        );
        assert_eq!(
            rows_of(&tree, &fixture()),
            vec![
                Row(vec![Value::Str("a".to_owned()), Value::Float(30.0)]),
                Row(vec![Value::Str("b".to_owned()), Value::Float(20.0)]),
            ]
        );
    }

    fn ordered_rows(held: &[(i64, f64, &str)]) -> Rc<Vec<Row>> {
        Rc::new(
            held.iter()
                .map(|(ts, value, cluster)| {
                    Row(vec![
                        Value::Timestamp(*ts),
                        Value::Float(*value),
                        Value::Str((*cluster).to_owned()),
                    ])
                })
                .collect(),
        )
    }

    fn stamps(rows: &[Row]) -> Vec<i64> {
        rows.iter()
            .map(|row| match row.0[0] {
                Value::Timestamp(held) => held,
                ref other => panic!("{other:?}"),
            })
            .collect()
    }

    #[test]
    fn a_limit_takes_n_rows_starting_at_its_offset_and_stops_at_the_end_of_the_input() {
        for (n, offset, expected) in [
            (1usize, 0usize, vec![10.0]),
            (2, 1, vec![20.0, 30.0]),
            (9, 1, vec![20.0, 30.0]),
            (2, 9, Vec::new()),
            (0, 0, Vec::new()),
        ] {
            let produced = rows_from(
                data_of(node(), &limit(n, offset, scan(Vec::new())), &fixture())
                    .expect("evaluates"),
            );
            assert_eq!(values(&produced), expected, "n={n} offset={offset}");
        }
    }

    #[test]
    fn a_limit_over_a_sort_is_the_shape_promql_topk_lowers_to() {
        let tree = limit(2, 0, sorted(vec![key(1, false, false)], scan(Vec::new())));
        let produced = rows_from(data_of(node(), &tree, &fixture()).expect("evaluates"));
        assert_eq!(values(&produced), vec![30.0, 20.0]);
    }

    #[test]
    fn a_concat_emits_every_branch_in_order_and_never_deduplicates_a_repeated_row() {
        let branch = (*scan(Vec::new())).clone();
        let tree = Rc::new(QueryExpr::concat(vec![branch.clone(), branch]));
        let produced = rows_from(data_of(node(), &tree, &fixture()).expect("evaluates"));
        assert_eq!(values(&produced), vec![10.0, 20.0, 30.0, 10.0, 20.0, 30.0]);
    }

    #[test]
    fn a_concat_whose_branches_disagree_on_width_is_refused_because_nothing_else_checks_it() {
        let narrowed = QueryExpr::Project {
            cols: vec![ProjectItem {
                alias: None,
                expr: QueryExpr::Column(1),
            }],
            qualifier: None,
            child: scan(Vec::new()),
        };
        let tree = Rc::new(QueryExpr::concat(vec![
            (*scan(Vec::new())).clone(),
            narrowed,
        ]));
        let refusal = check(node(), &tree).expect_err("refuses");
        let rendered = refusal.to_string();
        assert!(rendered.contains("Concat branch 1"), "{rendered}");
        assert!(rendered.contains("union-compatible"), "{rendered}");
        assert!(
            matches!(
                data_of(node(), &tree, &fixture()).expect_err("refuses"),
                EvalError::Refused(_)
            ),
            "a branch of the wrong width reached a row"
        );
    }

    #[test]
    fn a_concat_branch_is_not_memoized_because_the_ir_holds_it_by_value_not_by_rc() {
        let shared = Rc::new(QueryExpr::Filter {
            pred: Predicate(compare(1, CompareOpKind::Gt, ScalarValue::Float64(15.0))),
            child: scan(Vec::new()),
        });
        let tree = Rc::new(QueryExpr::concat(vec![
            (*shared).clone(),
            (*shared).clone(),
        ]));
        let supplied = fixture();
        let mut interpreter = Interpreter {
            node: node(),
            rows: &supplied,
            memo: HashMap::new(),
            leaf_emitted: None,
            node_times: None,
            children_ns: 0,
        };
        let produced = rows_from(interpreter.eval_rc(&tree).expect("evaluates"));

        assert_eq!(values(&produced), vec![20.0, 30.0, 20.0, 30.0]);
        assert_eq!(
            interpreter.memo.len(),
            2,
            "only the Concat itself and the Rc-held Scan beneath its branches are keyed"
        );
    }

    #[test]
    fn a_global_sort_orders_by_every_key_in_turn_and_reverses_a_descending_one() {
        let rows = ordered_rows(&[(1, 20.0, "b"), (2, 10.0, "a"), (3, 20.0, "a")]);

        let ascending = sorted(
            vec![key(1, true, false), key(2, true, false)],
            scan(Vec::new()),
        );
        assert_eq!(
            stamps(&rows_of(&ascending, &rows)),
            vec![2, 3, 1],
            "10 first, then the two 20s ordered by cluster"
        );

        let descending = sorted(vec![key(1, false, false)], scan(Vec::new()));
        assert_eq!(stamps(&rows_of(&descending, &rows)), vec![1, 3, 2]);
    }

    #[test]
    fn a_sort_is_stable_so_rows_the_keys_cannot_separate_keep_the_order_they_arrived_in() {
        let held: Vec<(i64, f64, &str)> = (0..60i64)
            .map(|ts| (ts, ((ts % 3) * 10) as f64, "a"))
            .collect();
        let rows = ordered_rows(&held);
        let tree = sorted(vec![key(1, true, false)], scan(Vec::new()));

        let expected: Vec<i64> = (0..3i64)
            .flat_map(|residue| (0..60i64).filter(move |ts| ts % 3 == residue))
            .collect();
        assert_eq!(
            stamps(&rows_of(&tree, &rows)),
            expected,
            "60 rows over 3 tied keys is past the 20-element insertion-sort window an unstable \
             sort keeps ordered by accident"
        );
    }

    #[test]
    fn a_sort_produces_the_same_order_on_every_run_over_the_same_rows() {
        let mut held = Vec::new();
        for i in 0..64i64 {
            held.push((i, ((7 * i + 3) % 11) as f64, "a"));
        }
        let rows = ordered_rows(
            &held
                .iter()
                .map(|(ts, value, cluster)| (*ts, *value, *cluster))
                .collect::<Vec<_>>(),
        );
        let tree = sorted(vec![key(1, true, false)], scan(Vec::new()));

        let first = stamps(&rows_of(&tree, &rows));
        assert_eq!(first.len(), 64);
        for _ in 0..5 {
            assert_eq!(
                stamps(&rows_of(&tree, &rows)),
                first,
                "the order moved between runs"
            );
        }
    }

    #[test]
    fn a_sort_key_puts_nulls_where_nulls_first_says_rather_than_ignoring_the_field() {
        let rows = Rc::new(vec![
            Row(vec![
                Value::Timestamp(1),
                Value::Float(20.0),
                Value::Str("a".to_owned()),
            ]),
            Row(vec![
                Value::Timestamp(2),
                Value::Null,
                Value::Str("a".to_owned()),
            ]),
            Row(vec![
                Value::Timestamp(3),
                Value::Float(10.0),
                Value::Str("a".to_owned()),
            ]),
        ]);

        let last = sorted(vec![key(1, true, false)], scan(Vec::new()));
        assert_eq!(stamps(&rows_of(&last, &rows)), vec![3, 1, 2]);

        let first = sorted(vec![key(1, true, true)], scan(Vec::new()));
        assert_eq!(stamps(&rows_of(&first, &rows)), vec![2, 3, 1]);
    }

    #[test]
    fn a_sort_carrying_no_key_is_refused_rather_than_passed_through_unordered() {
        let tree = sorted(Vec::new(), scan(Vec::new()));
        let refusal = check(node(), &tree).expect_err("refuses");
        assert!(
            refusal.to_string().contains("Sort carries no key"),
            "{refusal}"
        );
    }

    #[test]
    fn a_partitioned_sort_is_refused_by_name_because_the_limit_above_it_would_cut_globally() {
        let tree = Rc::new(QueryExpr::Sort {
            keys: vec![key(1, false, false)],
            partition_by: GroupKeys::by(vec![2]),
            child: scan(Vec::new()),
        });

        let refusal = check(node(), &tree).expect_err("refuses");
        let rendered = refusal.to_string();
        assert!(rendered.contains("Sort partitioned by"), "{rendered}");
        assert!(
            rendered.contains("k rows per host, not k overall"),
            "{rendered}"
        );

        match data_of(node(), &tree, &fixture()).expect_err("refuses") {
            EvalError::Refused(refusals) => assert!(
                refusals
                    .iter()
                    .any(|refusal| refusal.to_string().contains("Sort partitioned by")),
                "{refusals:?}"
            ),
            other => panic!("{other:?}"),
        }

        let lowered =
            Rc::new(lower_promql("topk by (cluster) (3, cpu_cores)", ACCURACY).expect("lowers"));
        assert!(
            check(node(), &lowered)
                .expect_err("refuses")
                .to_string()
                .contains("Sort partitioned by"),
            "a PromQL per-group topk must reach the same refusal"
        );
    }

    #[test]
    fn a_lowered_topk_is_a_limit_over_a_sort_and_this_step_runs_it() {
        let lowered = Rc::new(lower_promql("topk(2, cpu_cores)", ACCURACY).expect("lowers"));
        assert_eq!(variant_name(&lowered), "Limit");

        let rows = Rc::new(
            [(1i64, 10.0), (2, 30.0), (3, 20.0)]
                .into_iter()
                .map(|(ts, value)| Row(vec![Value::Timestamp(ts), Value::Float(value)]))
                .collect::<Vec<Row>>(),
        );
        assert_eq!(
            rows_of(&lowered, &rows),
            vec![
                Row(vec![Value::Timestamp(2), Value::Float(30.0)]),
                Row(vec![Value::Timestamp(3), Value::Float(20.0)]),
            ]
        );
    }

    fn labelled_csv() -> tempfile::NamedTempFile {
        use std::io::Write;
        let mut file = tempfile::NamedTempFile::new().expect("temp file");
        writeln!(file, "ts,value,cluster").expect("header");
        for (i, (value, cluster)) in [
            (10.0, "a"),
            (20.0, "b"),
            (30.0, "a"),
            (40.0, "b"),
            (50.0, "a"),
            (60.0, "b"),
        ]
        .into_iter()
        .enumerate()
        {
            writeln!(file, "{},{value},{cluster}", 1_700_000_000 + i as i64).expect("row");
        }
        file.flush().expect("flush");
        file
    }

    fn from_csv(file: &tempfile::NamedTempFile) -> RowsFrom {
        RowsFrom::Csv(file.path().to_path_buf())
    }

    fn answered(run: &ExactRun) -> Vec<Row> {
        match &run.answer {
            Data::Rows(rows) => rows.as_ref().clone(),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn a_promql_string_becomes_an_answer_over_rows_with_no_post_asap_plan_in_between() {
        let csv = labelled_csv();
        let run = run_promql("sum by (cluster) (cpu_cores)", ACCURACY, &from_csv(&csv))
            .expect("lowers and evaluates");

        assert_eq!((run.rows_scanned, run.rows_emitted), (6, 6));
        assert_eq!(
            answered(&run),
            vec![
                Row(vec![Value::Str("a".to_owned()), Value::Float(90.0)]),
                Row(vec![Value::Str("b".to_owned()), Value::Float(120.0)]),
            ]
        );
    }

    #[test]
    fn a_scan_predicate_is_applied_once_by_the_interpreter_and_never_by_the_row_source() {
        let csv = labelled_csv();
        let run = run_promql(
            "sum by (cluster) (cpu_cores{cluster=\"a\"})",
            ACCURACY,
            &from_csv(&csv),
        )
        .expect("lowers and evaluates");

        let QueryExpr::Aggregate { child, .. } = run.root.as_ref() else {
            panic!("{:?}", run.root);
        };
        let QueryExpr::Scan { predicates, .. } = child.as_ref() else {
            panic!("{child:?}");
        };
        assert_eq!(predicates.len(), 1, "the label matcher is a Scan predicate");

        assert_eq!(
            (run.rows_scanned, run.rows_emitted),
            (6, 3),
            "the row source read all 6 and the leaf predicate admitted the 3 that reached the \
             aggregate"
        );
        assert_eq!(
            answered(&run),
            vec![Row(vec![Value::Str("a".to_owned()), Value::Float(90.0)])],
            "the interpreter applied the predicate, and applied it once"
        );

        let unfiltered = run_promql("sum by (cluster) (cpu_cores)", ACCURACY, &from_csv(&csv))
            .expect("lowers and evaluates");
        assert_eq!(
            (unfiltered.rows_scanned, unfiltered.rows_emitted),
            (6, 6),
            "with no leaf predicate the two counts are the same, so the filtered case above is \
             the predicate and not a constant"
        );
    }

    #[test]
    fn the_pre_asap_answer_equals_the_post_asap_arms_exact_side_on_the_same_rows() {
        let csv = labelled_csv();
        for (query, groups) in [
            ("quantile by (cluster) (0.5, cpu_cores)", 2usize),
            ("quantile by (cluster) (0.5, cpu_cores{cluster=\"a\"})", 1),
        ] {
            let pre_asap = answered(
                &run_promql(query, ACCURACY, &from_csv(&csv)).expect("lowers and evaluates"),
            );
            assert_eq!(pre_asap.len(), groups, "{query}");

            let plan = crate::plan::plan_promql(query, ACCURACY).expect("plans");
            let outcome =
                crate::run::run(&plan, &crate::run::RunConfig::new(from_csv(&csv), 0, true))
                    .expect("runs");

            assert_eq!(
                outcome.readouts.len(),
                pre_asap.len(),
                "{query}: one readout per group"
            );
            for (readout, row) in outcome.readouts.iter().zip(&pre_asap) {
                assert_eq!(
                    readout.group,
                    match &row.0[0] {
                        Value::Str(held) => held.clone(),
                        other => panic!("{other:?}"),
                    }
                );
                assert_eq!(
                    readout.exact,
                    Some(Answer::Scalar(
                        row.0[1].as_f64().expect("a numeric measure")
                    )),
                    "{query}: the two arms disagree on {}",
                    readout.group
                );
            }
        }
    }

    #[test]
    fn a_tree_that_reads_no_rows_still_answers_without_opening_a_row_source() {
        let csv = labelled_csv();
        let run = run_promql("2.5", ACCURACY, &from_csv(&csv)).expect("lowers and evaluates");
        assert_eq!((run.rows_scanned, run.rows_emitted), (0, 0));
        match run.answer {
            Data::Scalar(value) => assert_eq!(value, 2.5),
            ref other => panic!("{other:?}"),
        }
    }
}
