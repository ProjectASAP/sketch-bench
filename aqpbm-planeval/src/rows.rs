//! The row source: the one place rows enter the evaluator.
//!
//! A `Fallback` node is the row source of every plan — it is the only leaf a
//! compiled post-ASAP DAG can have — and the only `Fallback` payload this
//! evaluator accepts is a bare `QueryExpr::Scan` (PLAN.md §1.6). A `Scan` says
//! where rows come from and which of them survive; anything else in that
//! position is a program, and running programs is not what this crate does.
//!
//! Three things happen here and nowhere else:
//!
//! 1. **The derived schema is checked against the node's `output_schema`.** The
//!    document carries every schema three times (`payload.expression.schema`,
//!    `node.output_schema`, `edge.intermediate_schema`) and that redundancy is
//!    its only source of checkability (PLAN.md §1.9). An off-by-one between the
//!    scan schema and the node schema does not crash — it silently produces the
//!    wrong number, the worst failure mode there is — so it is asserted.
//! 2. **`Scan.predicates` are evaluated per row** by a deliberately minimal
//!    scalar interpreter. The supported subset is `Column`, `Literal`,
//!    `Compare`, `BoolAnd`, `BoolOr`, `Not`, `InList`; every other variant is
//!    refused **by name**. Note the conjunction node is
//!    `QueryExpr::BoolAnd(Vec<_>)`, not a `BinaryOp` carrying a logical kind —
//!    `BinaryOpKind` only has `Arithmetic | Compare`.
//! 3. **Column references are resolved** against a `SummarySchema`.
//!    `ColumnRef::SampleValue` resolves to the field named `value`: it is
//!    PromQL's canonical reference to the sample-value column (the metric name
//!    rides on `Scan.source`), not a wildcard. Refusing it would refuse every
//!    PromQL-produced plan (PLAN.md §1.3).

use std::fs::File;
use std::path::{Path, PathBuf};

use asap_types::post_asap::{SummaryFamilyType, SummarySchema};
use asap_types::pre_asap::{
    ColumnRef, CompareOpKind, DataType, Predicate, QueryExpr, ScalarValue, Schema, Source,
};

use crate::types::{EvalError, Row, Value};

// ── Row source ───────────────────────────────────────────────────────────────

/// Rows read from one CSV file, decoded against a `Scan`'s schema and filtered
/// by that `Scan`'s predicates.
///
/// Field order is the schema's field order, not the CSV's column order: the
/// header is matched by name, so a CSV may carry its columns in any order and
/// may carry extra columns the plan never references.
pub struct RowSource {
    reader: csv::Reader<File>,
    record: csv::StringRecord,
    /// Which CSV column feeds each schema field, in schema field order.
    projection: Vec<usize>,
    /// Decoded type per schema field, in schema field order.
    types: Vec<DataType>,
    nullable: Vec<bool>,
    predicates: Vec<Predicate>,
    path: PathBuf,
    /// Rows read from the file, whether or not they passed the predicates.
    scanned: u64,
    /// Rows handed to the caller.
    emitted: u64,
    finished: bool,
}

impl std::fmt::Debug for RowSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RowSource")
            .field("path", &self.path)
            .field("projection", &self.projection)
            .field("predicates", &self.predicates.len())
            .field("scanned", &self.scanned)
            .field("emitted", &self.emitted)
            .finish()
    }
}

impl RowSource {
    /// Rows read so far, before predicate filtering.
    pub fn scanned(&self) -> u64 {
        self.scanned
    }

    /// Rows emitted so far — those that passed every predicate.
    pub fn emitted(&self) -> u64 {
        self.emitted
    }

    /// The file these rows come from.
    pub fn path(&self) -> &Path {
        &self.path
    }
}

/// Open the row source described by a `Fallback` node's payload expression.
///
/// `scan` must be a bare `QueryExpr::Scan`; `schema` is the owning node's
/// `output_schema`, which the scan's own schema is checked against. Both
/// `Source::Table { table_ref }` (SQL) and `Source::TimeSeries { metric }`
/// (PromQL) are accepted — the source only carries the leaf's identity, and
/// which bytes that identity names is the run manifest's business, which is
/// why the path is a separate argument.
pub fn open(
    scan: &QueryExpr,
    csv_path: &Path,
    schema: &SummarySchema,
) -> Result<RowSource, EvalError> {
    let (scan_schema, predicates) = match scan {
        QueryExpr::Scan {
            source,
            predicates,
            schema: scan_schema,
        } => {
            // Exhaustive rather than ignored: a third source variant must not
            // reach a CSV reader by default.
            match source {
                Source::Table { .. } | Source::TimeSeries { .. } => {}
            }
            (scan_schema, predicates)
        }
        other => {
            return Err(EvalError::RowSource(format!(
                "a row source must be a bare Scan, found {}",
                variant_name(other)
            )))
        }
    };

    check_scan_schema(scan_schema, schema)?;

    for predicate in predicates {
        check_predicate(&predicate.0, scan_schema.columns.len()).map_err(|fault| {
            EvalError::RowSource(format!("Scan.predicates: {}", fault.detail()))
        })?;
    }

    let file = File::open(csv_path)?;
    let mut reader = csv::ReaderBuilder::new()
        .has_headers(true)
        .from_reader(file);
    let header = reader.headers().map_err(|err| {
        EvalError::RowSource(format!("{}: reading the header: {err}", csv_path.display()))
    })?;

    let mut projection = Vec::with_capacity(scan_schema.columns.len());
    for column in &scan_schema.columns {
        let found = header.iter().position(|name| name == column.name);
        match found {
            Some(index) => projection.push(index),
            None => {
                return Err(EvalError::RowSource(format!(
                    "{}: no CSV column named {:?} (header: {:?})",
                    csv_path.display(),
                    column.name,
                    header.iter().collect::<Vec<_>>()
                )))
            }
        }
    }

    Ok(RowSource {
        reader,
        record: csv::StringRecord::new(),
        projection,
        types: scan_schema
            .columns
            .iter()
            .map(|column| column.dtype.clone())
            .collect(),
        nullable: scan_schema
            .columns
            .iter()
            .map(|column| column.nullable)
            .collect(),
        predicates: predicates.clone(),
        path: csv_path.to_path_buf(),
        scanned: 0,
        emitted: 0,
        finished: false,
    })
}

impl Iterator for RowSource {
    type Item = Result<Row, EvalError>;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            if self.finished {
                return None;
            }
            match self.reader.read_record(&mut self.record) {
                Ok(false) => {
                    self.finished = true;
                    return None;
                }
                Err(err) => {
                    self.finished = true;
                    return Some(Err(EvalError::RowSource(format!(
                        "{}: row {}: {err}",
                        self.path.display(),
                        self.scanned + 1
                    ))));
                }
                Ok(true) => {}
            }
            self.scanned += 1;

            let row = match self.decode() {
                Ok(row) => row,
                Err(err) => {
                    self.finished = true;
                    return Some(Err(err));
                }
            };

            match self.keeps(&row) {
                Ok(true) => {
                    self.emitted += 1;
                    return Some(Ok(row));
                }
                Ok(false) => continue,
                Err(err) => {
                    self.finished = true;
                    return Some(Err(err));
                }
            }
        }
    }
}

impl RowSource {
    fn decode(&self) -> Result<Row, EvalError> {
        let mut values = Vec::with_capacity(self.projection.len());
        for (field, &csv_index) in self.projection.iter().enumerate() {
            let raw = self.record.get(csv_index).ok_or_else(|| {
                EvalError::RowSource(format!(
                    "{}: row {} has {} fields, needs at least {}",
                    self.path.display(),
                    self.scanned,
                    self.record.len(),
                    csv_index + 1
                ))
            })?;
            values.push(self.decode_one(field, raw)?);
        }
        Ok(Row(values))
    }

    fn decode_one(&self, field: usize, raw: &str) -> Result<Value, EvalError> {
        let dtype = &self.types[field];
        if raw.is_empty() && !matches!(dtype, DataType::Utf8) {
            if self.nullable[field] || matches!(dtype, DataType::Null) {
                return Ok(Value::Null);
            }
            return Err(EvalError::RowSource(format!(
                "{}: row {}: empty value in non-nullable {dtype:?} column",
                self.path.display(),
                self.scanned
            )));
        }
        let parse_failure = |wanted: &str| {
            EvalError::RowSource(format!(
                "{}: row {}: {raw:?} is not {wanted}",
                self.path.display(),
                self.scanned
            ))
        };
        match dtype {
            DataType::Null => Ok(Value::Null),
            DataType::Int64 => raw
                .trim()
                .parse::<i64>()
                .map(Value::Int)
                .map_err(|_| parse_failure("an int64")),
            DataType::Float64 => raw
                .trim()
                .parse::<f64>()
                .map(Value::Float)
                .map_err(|_| parse_failure("a float64")),
            DataType::Utf8 => Ok(Value::Str(raw.to_string())),
            DataType::Bool => match raw.trim() {
                "true" | "TRUE" | "True" | "1" => Ok(Value::Int(1)),
                "false" | "FALSE" | "False" | "0" => Ok(Value::Int(0)),
                _ => Err(parse_failure("a bool")),
            },
            // Unix epoch integers. A timestamp column whose CSV text is an
            // RFC-3339 string is refused rather than guessed at: the IR says
            // nothing about the encoding, so a guess would silently shift every
            // row's time axis.
            DataType::Timestamp => raw
                .trim()
                .parse::<i64>()
                .map(Value::Timestamp)
                .map_err(|_| parse_failure("a unix-epoch timestamp")),
            DataType::List { .. } | DataType::Struct { .. } | DataType::Map { .. } => {
                Err(EvalError::RowSource(format!(
                    "{}: column {} has nested type {}, which a CSV row source cannot decode",
                    self.path.display(),
                    field,
                    data_type_name(dtype)
                )))
            }
        }
    }

    fn keeps(&self, row: &Row) -> Result<bool, EvalError> {
        for predicate in &self.predicates {
            if !truthy(&eval(&predicate.0, row)?) {
                return Ok(false);
            }
        }
        Ok(true)
    }
}

// ── Schema checks ────────────────────────────────────────────────────────────

/// Assert the scan's own (pre-ASAP) schema against the node's `output_schema`.
///
/// `Fallback` output is always plain rows, so every node field must be
/// `SummaryFamilyType::Plain(dtype)` and must agree with the scan column at the
/// same position in name and type. Field *count* is checked first and named
/// explicitly, because an off-by-one there is the failure that produces wrong
/// numbers instead of a crash.
fn check_scan_schema(scan: &Schema, node: &SummarySchema) -> Result<(), EvalError> {
    if scan.columns.len() != node.fields.len() {
        return Err(EvalError::RowSource(format!(
            "Scan.schema derives {} columns but the node's output_schema has {} fields",
            scan.columns.len(),
            node.fields.len()
        )));
    }
    for (index, (column, field)) in scan.columns.iter().zip(&node.fields).enumerate() {
        if column.name != field.name {
            return Err(EvalError::RowSource(format!(
                "field {index}: Scan.schema names it {:?}, output_schema names it {:?}",
                column.name, field.name
            )));
        }
        match &field.dtype {
            SummaryFamilyType::Plain(dtype) if *dtype == column.dtype => {}
            SummaryFamilyType::Plain(dtype) => {
                return Err(EvalError::RowSource(format!(
                    "field {index} ({:?}): Scan.schema types it {}, output_schema types it {}",
                    field.name,
                    data_type_name(&column.dtype),
                    data_type_name(dtype)
                )))
            }
            other => {
                return Err(EvalError::RowSource(format!(
                    "field {index} ({:?}): a Fallback produces rows, but output_schema types it \
                     as summary state ({})",
                    field.name,
                    family_name(other)
                )))
            }
        }
    }
    if scan.time_index != node.time_index {
        return Err(EvalError::RowSource(format!(
            "Scan.schema puts the time axis at {:?}, output_schema puts it at {:?}",
            scan.time_index, node.time_index
        )));
    }
    Ok(())
}

/// Resolve a `SummaryUpdate`'s column reference against a node's output schema.
///
/// `Named(name)` and `Qualified { _, name }` both match on `name`: a
/// `SummarySchema` field for `metrics.latency` is literally named `latency`,
/// so the qualifier is not part of the field name to match (PLAN.md §1.3).
///
/// `SampleValue` resolves to the field named `value`. That is not a wildcard —
/// it is PromQL's canonical reference to the sample-value column of a
/// usage-derived `{ts, value}` row schema, and it is the weight every
/// PromQL-produced `SummaryAgg` carries.
///
/// `Wildcard` never resolves.
pub fn resolve_column(col: &ColumnRef, schema: &SummarySchema) -> Option<usize> {
    let wanted = match col {
        ColumnRef::Named(name) => name.as_str(),
        ColumnRef::Qualified { name, .. } => name.as_str(),
        ColumnRef::SampleValue => "value",
        ColumnRef::Wildcard => return None,
    };
    schema.fields.iter().position(|field| field.name == wanted)
}

// ── The predicate subset ─────────────────────────────────────────────────────

/// Why a scalar expression is outside the interpreter's subset.
///
/// Separate from `Refusal` so `admit` can map the same fault onto the refusal
/// that fits its own position (a `Scan` predicate and a read-time `Filter`
/// predicate are the same subset but different refusals), and so this module
/// does not have to know which node it is being asked about.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum PredicateFault {
    /// A `QueryExpr` variant the interpreter does not implement, by name.
    Unsupported(String),
    /// A positional column reference past the end of the schema it indexes.
    ColumnOutOfRange { column: usize, columns: usize },
}

impl PredicateFault {
    pub(crate) fn detail(&self) -> String {
        match self {
            Self::Unsupported(name) => format!("{name} is not in the predicate subset"),
            Self::ColumnOutOfRange { column, columns } => {
                format!("column {column} is out of range (the schema has {columns} columns)")
            }
        }
    }
}

/// Walk a predicate and report the first reason it is outside the subset.
///
/// Run before any data is read, so a plan that would fail on row one is refused
/// at admission instead. `columns` is the width of the schema the positional
/// `ColumnId`s index into.
pub(crate) fn check_predicate(expr: &QueryExpr, columns: usize) -> Result<(), PredicateFault> {
    match expr {
        QueryExpr::Column(column) => {
            if *column >= columns {
                return Err(PredicateFault::ColumnOutOfRange {
                    column: *column,
                    columns,
                });
            }
            Ok(())
        }
        QueryExpr::Literal(_) => Ok(()),
        QueryExpr::Compare { left, op, right } => {
            match op {
                CompareOpKind::Eq
                | CompareOpKind::Ne
                | CompareOpKind::Lt
                | CompareOpKind::Le
                | CompareOpKind::Gt
                | CompareOpKind::Ge => {}
                // Pattern and regex matching would need a matcher whose
                // semantics (RE2 vs SQL glob) the IR does not pin down.
                CompareOpKind::Like
                | CompareOpKind::NotLike
                | CompareOpKind::ILike
                | CompareOpKind::NotILike
                | CompareOpKind::Regex
                | CompareOpKind::NotRegex => {
                    return Err(PredicateFault::Unsupported(format!("Compare({op})")))
                }
            }
            check_predicate(left, columns)?;
            check_predicate(right, columns)
        }
        QueryExpr::BoolAnd(terms) | QueryExpr::BoolOr(terms) => {
            for term in terms {
                check_predicate(term, columns)?;
            }
            Ok(())
        }
        QueryExpr::Not(inner) => check_predicate(inner, columns),
        QueryExpr::InList {
            expr,
            list,
            negated: _,
        } => {
            check_predicate(expr, columns)?;
            for item in list {
                check_predicate(item, columns)?;
            }
            Ok(())
        }
        other => Err(PredicateFault::Unsupported(variant_name(other).to_string())),
    }
}

/// Evaluate one scalar expression against one row.
///
/// Only reachable for expressions [`check_predicate`] has already accepted; the
/// same subset is matched here so a divergence between the two is a visible
/// error rather than a wrong answer.
fn eval(expr: &QueryExpr, row: &Row) -> Result<Value, EvalError> {
    match expr {
        QueryExpr::Column(column) => row.0.get(*column).cloned().ok_or_else(|| {
            EvalError::RowSource(format!(
                "column {column} is out of range (the row has {} values)",
                row.0.len()
            ))
        }),
        QueryExpr::Literal(value) => Ok(literal(value)),
        QueryExpr::Compare { left, op, right } => {
            let left = eval(left, row)?;
            let right = eval(right, row)?;
            Ok(bool_value(compare(&left, op, &right)?))
        }
        QueryExpr::BoolAnd(terms) => {
            for term in terms {
                if !truthy(&eval(term, row)?) {
                    return Ok(bool_value(false));
                }
            }
            Ok(bool_value(true))
        }
        QueryExpr::BoolOr(terms) => {
            for term in terms {
                if truthy(&eval(term, row)?) {
                    return Ok(bool_value(true));
                }
            }
            Ok(bool_value(false))
        }
        QueryExpr::Not(inner) => Ok(bool_value(!truthy(&eval(inner, row)?))),
        QueryExpr::InList {
            expr,
            list,
            negated,
        } => {
            let probe = eval(expr, row)?;
            let mut found = false;
            for item in list {
                let candidate = eval(item, row)?;
                if compare(&probe, &CompareOpKind::Eq, &candidate)? {
                    found = true;
                    break;
                }
            }
            Ok(bool_value(found != *negated))
        }
        other => Err(EvalError::RowSource(format!(
            "{} is not in the predicate subset",
            variant_name(other)
        ))),
    }
}

fn literal(value: &ScalarValue) -> Value {
    match value {
        ScalarValue::Int64(v) => Value::Int(*v),
        ScalarValue::Float64(v) => Value::Float(*v),
        ScalarValue::Utf8(v) => Value::Str(v.clone()),
        ScalarValue::Boolean(v) => bool_value(*v),
        ScalarValue::Null => Value::Null,
    }
}

/// `Bool` has no `Value` variant of its own; `Int(0)`/`Int(1)` is the encoding
/// used for both decoded `Bool` columns and predicate results, so the two are
/// comparable without a special case.
fn bool_value(value: bool) -> Value {
    Value::Int(i64::from(value))
}

fn truthy(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Int(v) => *v != 0,
        Value::Float(v) => *v != 0.0,
        Value::Timestamp(v) => *v != 0,
        Value::Str(v) => !v.is_empty(),
    }
}

/// Compare two decoded values.
///
/// Numbers compare numerically (an `Int` column against a `Float64` literal is
/// the ordinary case, and a `Timestamp` is a number); strings compare as
/// strings. A `Null` on either side yields `false` for every operator including
/// `Ne` — SQL's unknown, collapsed to "this row does not pass", which is what a
/// filter does with it anyway. Mixing a string and a number is an error, not a
/// silent `false`: it means the schema and the predicate disagree.
fn compare(left: &Value, op: &CompareOpKind, right: &Value) -> Result<bool, EvalError> {
    if matches!(left, Value::Null) || matches!(right, Value::Null) {
        return Ok(false);
    }
    let ordering = match (left.as_f64(), right.as_f64()) {
        (Some(left), Some(right)) => left.partial_cmp(&right),
        (None, None) => match (left, right) {
            (Value::Str(left), Value::Str(right)) => Some(left.cmp(right)),
            _ => None,
        },
        _ => {
            return Err(EvalError::RowSource(format!(
                "cannot compare {left:?} with {right:?}"
            )))
        }
    };
    let Some(ordering) = ordering else {
        // NaN: unordered, so every comparison but `Ne` is false.
        return Ok(matches!(op, CompareOpKind::Ne));
    };
    Ok(match op {
        CompareOpKind::Eq => ordering.is_eq(),
        CompareOpKind::Ne => ordering.is_ne(),
        CompareOpKind::Lt => ordering.is_lt(),
        CompareOpKind::Le => ordering.is_le(),
        CompareOpKind::Gt => ordering.is_gt(),
        CompareOpKind::Ge => ordering.is_ge(),
        CompareOpKind::Like
        | CompareOpKind::NotLike
        | CompareOpKind::ILike
        | CompareOpKind::NotILike
        | CompareOpKind::Regex
        | CompareOpKind::NotRegex => {
            return Err(EvalError::RowSource(format!(
                "Compare({op}) is not in the predicate subset"
            )))
        }
    })
}

// ── Naming ───────────────────────────────────────────────────────────────────

/// The variant name of a `QueryExpr`, for refusals that must point at something
/// a plan author can find.
///
/// Deliberately exhaustive with no wildcard arm: `QueryExpr` is not
/// `#[non_exhaustive]`, so a new variant upstream is a compile error here
/// rather than a refusal that says nothing.
pub(crate) fn variant_name(expr: &QueryExpr) -> &'static str {
    match expr {
        QueryExpr::Scan { .. } => "Scan",
        QueryExpr::PromqlScalarBridge(_) => "PromqlScalarBridge",
        QueryExpr::EvalTimestamp => "EvalTimestamp",
        QueryExpr::CurrentTimestamp => "CurrentTimestamp",
        QueryExpr::PromqlVectorFromScalar(_) => "PromqlVectorFromScalar",
        QueryExpr::PromqlScalarFromVector(_) => "PromqlScalarFromVector",
        QueryExpr::PromqlRelabel { .. } => "PromqlRelabel",
        QueryExpr::PromqlInfoEnrich { .. } => "PromqlInfoEnrich",
        QueryExpr::PromqlSeriesSample { .. } => "PromqlSeriesSample",
        QueryExpr::Filter { .. } => "Filter",
        QueryExpr::Project { .. } => "Project",
        QueryExpr::Aggregate { .. } => "Aggregate",
        QueryExpr::Dedup { .. } => "Dedup",
        QueryExpr::Concat { .. } => "Concat",
        QueryExpr::Join { .. } => "Join",
        QueryExpr::SetOp { .. } => "SetOp",
        QueryExpr::Sort { .. } => "Sort",
        QueryExpr::Limit { .. } => "Limit",
        QueryExpr::PromqlSubquery { .. } => "PromqlSubquery",
        QueryExpr::TimeRange { .. } => "TimeRange",
        QueryExpr::TimeShift { .. } => "TimeShift",
        QueryExpr::SQLWindowFunc { .. } => "SQLWindowFunc",
        QueryExpr::BinaryOp { .. } => "BinaryOp",
        QueryExpr::Column(_) => "Column",
        QueryExpr::Literal(_) => "Literal",
        QueryExpr::Compare { .. } => "Compare",
        QueryExpr::BoolAnd(_) => "BoolAnd",
        QueryExpr::BoolOr(_) => "BoolOr",
        QueryExpr::Not(_) => "Not",
        QueryExpr::IsNull(_) => "IsNull",
        QueryExpr::IsNotNull(_) => "IsNotNull",
        QueryExpr::Cast { .. } => "Cast",
        QueryExpr::InList { .. } => "InList",
        QueryExpr::FunctionCall { .. } => "FunctionCall",
        QueryExpr::Arithmetic { .. } => "Arithmetic",
        QueryExpr::Case { .. } => "Case",
    }
}

fn data_type_name(dtype: &DataType) -> &'static str {
    match dtype {
        DataType::Null => "null",
        DataType::Int64 => "int64",
        DataType::Float64 => "float64",
        DataType::Utf8 => "utf8",
        DataType::Bool => "bool",
        DataType::Timestamp => "timestamp",
        DataType::List { .. } => "list",
        DataType::Struct { .. } => "struct",
        DataType::Map { .. } => "map",
    }
}

fn family_name(family: &SummaryFamilyType) -> &'static str {
    match family {
        SummaryFamilyType::Plain(_) => "Plain",
        SummaryFamilyType::ExactAggregate(_, _) => "ExactAggregate",
        SummaryFamilyType::Sketch(_, _) => "Sketch",
        SummaryFamilyType::Sample(_, _) => "Sample",
        SummaryFamilyType::Wavelet(_, _) => "Wavelet",
        SummaryFamilyType::StatModel(_, _) => "StatModel",
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::rc::Rc;

    use asap_types::post_asap::{
        compile_executable_dag, ExecutableDagNode, ExecutableOperatorPayload, PostAsapDagDocument,
        SummaryField,
    };
    use asap_types::pre_asap::Column;
    use asap_types::types::AccuracyTarget;

    use asap_aware_mapping::{search_workload, DefaultCostModel};
    use asap_frontend_promql::lower_promql;

    const ACCURACY: AccuracyTarget = AccuracyTarget::Epsilon(0.01);

    // ── Fixtures ─────────────────────────────────────────────────────────────

    /// The in-memory chain `ASAPPlanner/crates/devtools/examples/minimal_dags.rs`
    /// uses. Nothing is written to a wire; `compile_executable_dag` is still
    /// called because the `ExecutionDataState` assignment and `validate()` only
    /// exist on the compiled dag.
    pub(crate) fn plan(query: &str) -> asap_types::post_asap::ExecutableDag {
        let expr = lower_promql(query, ACCURACY).expect("lowers");
        let space = search_workload(vec![(query.to_string(), Rc::new(expr))]);
        let selection = space.global_selection(&DefaultCostModel);
        let (_, root) = space.roots.first().expect("one root");
        let materialized = selection
            .materialize(root)
            .expect("materializes")
            .expect("is discovered");
        let dag = compile_executable_dag(&materialized).expect("compiles");
        let document = PostAsapDagDocument::new(dag);
        document.validate().expect("validates");
        document.dag
    }

    pub(crate) fn node_of(
        dag: &asap_types::post_asap::ExecutableDag,
        operator: asap_types::post_asap::ExecutableOperator,
    ) -> &ExecutableDagNode {
        dag.nodes
            .iter()
            .find(|node| node.operator == operator)
            .expect("the plan has this operator")
    }

    fn fallback_expression(dag: &asap_types::post_asap::ExecutableDag) -> &QueryExpr {
        match &node_of(dag, asap_types::post_asap::ExecutableOperator::Fallback).payload {
            ExecutableOperatorPayload::Fallback { expression } => expression,
            other => panic!("the Fallback node carries {other:?}"),
        }
    }

    struct TempCsv(PathBuf);

    impl TempCsv {
        fn new(name: &str, body: &str) -> Self {
            let mut path = std::env::temp_dir();
            path.push(format!(
                "aqpbm-planeval-rows-{}-{name}.csv",
                std::process::id()
            ));
            std::fs::write(&path, body).expect("writes the fixture");
            Self(path)
        }
        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TempCsv {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }

    /// `{ts, value}` — the usage-derived PromQL row schema, plus one label.
    fn promql_schema(labels: &[&str]) -> (Schema, SummarySchema) {
        let mut columns = vec![
            Column {
                name: "ts".into(),
                dtype: DataType::Timestamp,
                nullable: false,
                table: None,
            },
            Column {
                name: "value".into(),
                dtype: DataType::Float64,
                nullable: false,
                table: None,
            },
        ];
        for label in labels {
            columns.push(Column {
                name: (*label).to_string(),
                dtype: DataType::Utf8,
                nullable: false,
                table: None,
            });
        }
        let fields = columns
            .iter()
            .map(|column| SummaryField {
                name: column.name.clone(),
                dtype: SummaryFamilyType::Plain(column.dtype.clone()),
                nullable: column.nullable,
            })
            .collect();
        (
            Schema {
                columns,
                time_index: Some(0),
                unique_keys: Vec::new(),
                closed: false,
            },
            SummarySchema {
                fields,
                time_index: Some(0),
            },
        )
    }

    fn scan_with(predicates: Vec<Predicate>, schema: Schema) -> QueryExpr {
        QueryExpr::Scan {
            source: Source::TimeSeries {
                metric: "cpu_cores".into(),
            },
            predicates,
            schema,
        }
    }

    fn eq_literal(column: usize, value: ScalarValue) -> Predicate {
        Predicate(Rc::new(QueryExpr::Compare {
            left: Rc::new(QueryExpr::Column(column)),
            op: CompareOpKind::Eq,
            right: Rc::new(QueryExpr::Literal(value)),
        }))
    }

    // ── The planner's own Fallback ───────────────────────────────────────────

    #[test]
    fn the_planners_fallback_node_opens_as_a_row_source() {
        let dag = plan("quantile(0.5, cpu_cores)");
        let node = node_of(&dag, asap_types::post_asap::ExecutableOperator::Fallback);
        let csv = TempCsv::new("planner", "ts,value\n1,10\n2,20\n3,30\n");

        let rows: Vec<Row> = open(fallback_expression(&dag), csv.path(), &node.output_schema)
            .expect("opens")
            .map(|row| row.expect("decodes"))
            .collect();

        assert_eq!(
            rows,
            vec![
                Row(vec![Value::Timestamp(1), Value::Float(10.0)]),
                Row(vec![Value::Timestamp(2), Value::Float(20.0)]),
                Row(vec![Value::Timestamp(3), Value::Float(30.0)]),
            ]
        );
    }

    #[test]
    fn a_sample_value_weight_resolves_to_the_value_column() {
        let dag = plan("quantile(0.5, cpu_cores)");
        let fallback = node_of(&dag, asap_types::post_asap::ExecutableOperator::Fallback);
        let agg = node_of(&dag, asap_types::post_asap::ExecutableOperator::SummaryAgg);

        let weight = match &agg.payload {
            ExecutableOperatorPayload::SummaryAgg { input, .. } => input.weight.clone(),
            other => panic!("the SummaryAgg node carries {other:?}"),
        };
        let column = match &weight {
            asap_types::post_asap::SummaryInputExpr::Column(column) => column.clone(),
            other => panic!("a PromQL SummaryAgg weights by a column, found {other:?}"),
        };

        // This is the shape 24 of the 33 corpus plans carry.
        assert_eq!(column, ColumnRef::SampleValue);
        let resolved = resolve_column(&column, &fallback.output_schema).expect("resolves");
        assert_eq!(fallback.output_schema.fields[resolved].name, "value");
    }

    // ── resolve_column ───────────────────────────────────────────────────────

    #[test]
    fn column_refs_resolve_by_name_and_wildcard_never_does() {
        let (_, schema) = promql_schema(&["cluster"]);

        assert_eq!(
            resolve_column(&ColumnRef::Named("cluster".into()), &schema),
            Some(2)
        );
        // A field for `metrics.latency` is literally named `latency`, so a
        // qualified reference matches on the name alone.
        assert_eq!(
            resolve_column(
                &ColumnRef::Qualified {
                    table: "metrics".into(),
                    name: "cluster".into()
                },
                &schema
            ),
            Some(2)
        );
        assert_eq!(resolve_column(&ColumnRef::SampleValue, &schema), Some(1));
        assert_eq!(resolve_column(&ColumnRef::Wildcard, &schema), None);
        assert_eq!(
            resolve_column(&ColumnRef::Named("nope".into()), &schema),
            None
        );
    }

    #[test]
    fn sample_value_does_not_resolve_without_a_value_field() {
        let schema = SummarySchema {
            fields: vec![SummaryField {
                name: "latency".into(),
                dtype: SummaryFamilyType::Plain(DataType::Float64),
                nullable: false,
            }],
            time_index: None,
        };
        assert_eq!(resolve_column(&ColumnRef::SampleValue, &schema), None);
    }

    // ── Only a bare Scan ─────────────────────────────────────────────────────

    #[test]
    fn a_program_in_the_row_source_position_is_refused_by_name() {
        let (scan_schema, node_schema) = promql_schema(&[]);
        let csv = TempCsv::new("program", "ts,value\n1,10\n");
        let program = QueryExpr::Limit {
            n: 1,
            offset: 0,
            child: Rc::new(scan_with(Vec::new(), scan_schema)),
        };

        let err = open(&program, csv.path(), &node_schema).expect_err("is refused");
        let message = err.to_string();
        assert!(message.contains("bare Scan"), "{message}");
        assert!(message.contains("Limit"), "{message}");
    }

    // ── Predicates ───────────────────────────────────────────────────────────

    #[test]
    fn a_label_matcher_filters_rows() {
        // `cpu_cores{cluster="N001"}`.
        let (scan_schema, node_schema) = promql_schema(&["cluster"]);
        let csv = TempCsv::new(
            "matcher",
            "ts,value,cluster\n1,10,N001\n2,20,N002\n3,30,N001\n4,40,N003\n",
        );
        let scan = scan_with(
            vec![eq_literal(2, ScalarValue::Utf8("N001".into()))],
            scan_schema,
        );

        let mut source = open(&scan, csv.path(), &node_schema).expect("opens");
        let rows: Vec<Row> = source.by_ref().map(|row| row.expect("decodes")).collect();

        assert_eq!(
            rows,
            vec![
                Row(vec![
                    Value::Timestamp(1),
                    Value::Float(10.0),
                    Value::Str("N001".into())
                ]),
                Row(vec![
                    Value::Timestamp(3),
                    Value::Float(30.0),
                    Value::Str("N001".into())
                ]),
            ]
        );
        assert_eq!(source.scanned(), 4);
        assert_eq!(source.emitted(), 2);
    }

    #[test]
    fn conjunction_is_bool_and_not_a_binary_op() {
        let (scan_schema, node_schema) = promql_schema(&["cluster"]);
        let csv = TempCsv::new("and", "ts,value,cluster\n1,10,N001\n2,90,N001\n3,90,N002\n");
        let both = Predicate(Rc::new(QueryExpr::BoolAnd(vec![
            QueryExpr::Compare {
                left: Rc::new(QueryExpr::Column(2)),
                op: CompareOpKind::Eq,
                right: Rc::new(QueryExpr::Literal(ScalarValue::Utf8("N001".into()))),
            },
            QueryExpr::Compare {
                left: Rc::new(QueryExpr::Column(1)),
                op: CompareOpKind::Gt,
                right: Rc::new(QueryExpr::Literal(ScalarValue::Float64(50.0))),
            },
        ])));

        let rows: Vec<Row> = open(
            &scan_with(vec![both], scan_schema),
            csv.path(),
            &node_schema,
        )
        .expect("opens")
        .map(|row| row.expect("decodes"))
        .collect();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].0[1], Value::Float(90.0));
    }

    #[test]
    fn bool_or_not_and_in_list_all_evaluate() {
        let (scan_schema, node_schema) = promql_schema(&["cluster"]);
        let csv = TempCsv::new(
            "subset",
            "ts,value,cluster\n1,10,N001\n2,20,N002\n3,30,N003\n4,40,N004\n",
        );
        // NOT (cluster IN ("N002", "N004")) OR value >= 40
        let pred = Predicate(Rc::new(QueryExpr::BoolOr(vec![
            QueryExpr::Not(Rc::new(QueryExpr::InList {
                expr: Rc::new(QueryExpr::Column(2)),
                list: vec![
                    QueryExpr::Literal(ScalarValue::Utf8("N002".into())),
                    QueryExpr::Literal(ScalarValue::Utf8("N004".into())),
                ],
                negated: false,
            })),
            QueryExpr::Compare {
                left: Rc::new(QueryExpr::Column(1)),
                op: CompareOpKind::Ge,
                right: Rc::new(QueryExpr::Literal(ScalarValue::Float64(40.0))),
            },
        ])));

        let rows: Vec<Row> = open(
            &scan_with(vec![pred], scan_schema),
            csv.path(),
            &node_schema,
        )
        .expect("opens")
        .map(|row| row.expect("decodes"))
        .collect();
        let clusters: Vec<&Value> = rows.iter().map(|row| &row.0[2]).collect();
        assert_eq!(
            clusters,
            vec![
                &Value::Str("N001".into()),
                &Value::Str("N003".into()),
                &Value::Str("N004".into()),
            ]
        );
    }

    #[test]
    fn a_negated_in_list_is_the_complement() {
        let (scan_schema, node_schema) = promql_schema(&["cluster"]);
        let csv = TempCsv::new("notin", "ts,value,cluster\n1,10,N001\n2,20,N002\n");
        let pred = Predicate(Rc::new(QueryExpr::InList {
            expr: Rc::new(QueryExpr::Column(2)),
            list: vec![QueryExpr::Literal(ScalarValue::Utf8("N001".into()))],
            negated: true,
        }));

        let rows: Vec<Row> = open(
            &scan_with(vec![pred], scan_schema),
            csv.path(),
            &node_schema,
        )
        .expect("opens")
        .map(|row| row.expect("decodes"))
        .collect();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].0[2], Value::Str("N002".into()));
    }

    #[test]
    fn several_predicates_are_a_conjunction() {
        let (scan_schema, node_schema) = promql_schema(&["cluster"]);
        let csv = TempCsv::new(
            "conj",
            "ts,value,cluster\n1,10,N001\n2,90,N001\n3,90,N002\n",
        );
        let scan = scan_with(
            vec![
                eq_literal(2, ScalarValue::Utf8("N001".into())),
                Predicate(Rc::new(QueryExpr::Compare {
                    left: Rc::new(QueryExpr::Column(1)),
                    op: CompareOpKind::Gt,
                    right: Rc::new(QueryExpr::Literal(ScalarValue::Float64(50.0))),
                })),
            ],
            scan_schema,
        );

        let rows: Vec<Row> = open(&scan, csv.path(), &node_schema)
            .expect("opens")
            .map(|row| row.expect("decodes"))
            .collect();
        assert_eq!(rows.len(), 1);
    }

    #[test]
    fn a_predicate_outside_the_subset_is_refused_by_name_before_any_row_is_read() {
        let (scan_schema, node_schema) = promql_schema(&[]);
        let csv = TempCsv::new("unsupported", "ts,value\n1,10\n");
        let pred = Predicate(Rc::new(QueryExpr::Compare {
            left: Rc::new(QueryExpr::FunctionCall {
                name: "lower".into(),
                args: vec![QueryExpr::Column(1)],
            }),
            op: CompareOpKind::Eq,
            right: Rc::new(QueryExpr::Literal(ScalarValue::Float64(1.0))),
        }));

        let err = open(
            &scan_with(vec![pred], scan_schema),
            csv.path(),
            &node_schema,
        )
        .expect_err("is refused");
        assert!(err.to_string().contains("FunctionCall"), "{err}");
    }

    #[test]
    fn a_regex_matcher_is_refused_rather_than_guessed_at() {
        let (scan_schema, node_schema) = promql_schema(&["cluster"]);
        let csv = TempCsv::new("regex", "ts,value,cluster\n1,10,N001\n");
        let pred = Predicate(Rc::new(QueryExpr::Compare {
            left: Rc::new(QueryExpr::Column(2)),
            op: CompareOpKind::Regex,
            right: Rc::new(QueryExpr::Literal(ScalarValue::Utf8("N00.*".into()))),
        }));

        let err = open(
            &scan_with(vec![pred], scan_schema),
            csv.path(),
            &node_schema,
        )
        .expect_err("is refused");
        assert!(err.to_string().contains("=~"), "{err}");
    }

    #[test]
    fn a_predicate_column_past_the_schema_is_refused() {
        let (scan_schema, node_schema) = promql_schema(&[]);
        let csv = TempCsv::new("oob", "ts,value\n1,10\n");
        let scan = scan_with(vec![eq_literal(7, ScalarValue::Int64(1))], scan_schema);

        let err = open(&scan, csv.path(), &node_schema).expect_err("is refused");
        assert!(err.to_string().contains("out of range"), "{err}");
    }

    // ── Schema assertions ────────────────────────────────────────────────────

    #[test]
    fn a_field_count_mismatch_is_named_not_tolerated() {
        let (scan_schema, mut node_schema) = promql_schema(&["cluster"]);
        node_schema.fields.pop();
        let csv = TempCsv::new("count", "ts,value,cluster\n1,10,N001\n");

        let err = open(
            &scan_with(Vec::new(), scan_schema),
            csv.path(),
            &node_schema,
        )
        .expect_err("is refused");
        let message = err.to_string();
        assert!(message.contains("3 columns"), "{message}");
        assert!(message.contains("2 fields"), "{message}");
    }

    #[test]
    fn a_field_type_mismatch_is_named() {
        let (scan_schema, mut node_schema) = promql_schema(&[]);
        node_schema.fields[1].dtype = SummaryFamilyType::Plain(DataType::Int64);
        let csv = TempCsv::new("dtype", "ts,value\n1,10\n");

        let err = open(
            &scan_with(Vec::new(), scan_schema),
            csv.path(),
            &node_schema,
        )
        .expect_err("is refused");
        let message = err.to_string();
        assert!(message.contains("float64"), "{message}");
        assert!(message.contains("int64"), "{message}");
    }

    #[test]
    fn a_summary_typed_output_field_on_a_fallback_is_refused() {
        let dag = plan("quantile(0.5, cpu_cores)");
        let agg = node_of(&dag, asap_types::post_asap::ExecutableOperator::SummaryAgg);
        let (scan_schema, mut node_schema) = promql_schema(&[]);
        node_schema.fields[1].dtype = agg.output_schema.fields[0].dtype.clone();
        let csv = TempCsv::new("sketchy", "ts,value\n1,10\n");

        let err = open(
            &scan_with(Vec::new(), scan_schema),
            csv.path(),
            &node_schema,
        )
        .expect_err("is refused");
        assert!(err.to_string().contains("Sketch"), "{err}");
    }

    // ── CSV decoding ─────────────────────────────────────────────────────────

    #[test]
    fn csv_columns_are_matched_by_name_not_position() {
        let (scan_schema, node_schema) = promql_schema(&["cluster"]);
        // Reordered, plus a column the plan never asked for.
        let csv = TempCsv::new("byname", "cluster,extra,value,ts\nN001,x,10,1\n");

        let rows: Vec<Row> = open(
            &scan_with(Vec::new(), scan_schema),
            csv.path(),
            &node_schema,
        )
        .expect("opens")
        .map(|row| row.expect("decodes"))
        .collect();
        assert_eq!(
            rows,
            vec![Row(vec![
                Value::Timestamp(1),
                Value::Float(10.0),
                Value::Str("N001".into())
            ])]
        );
    }

    #[test]
    fn a_missing_csv_column_names_itself() {
        let (scan_schema, node_schema) = promql_schema(&["cluster"]);
        let csv = TempCsv::new("missing", "ts,value\n1,10\n");

        let err = open(
            &scan_with(Vec::new(), scan_schema),
            csv.path(),
            &node_schema,
        )
        .expect_err("is refused");
        assert!(err.to_string().contains("cluster"), "{err}");
    }

    #[test]
    fn an_undecodable_cell_stops_the_iterator_with_an_error() {
        let (scan_schema, node_schema) = promql_schema(&[]);
        let csv = TempCsv::new("badcell", "ts,value\n1,10\n2,not-a-number\n3,30\n");

        let mut source = open(
            &scan_with(Vec::new(), scan_schema),
            csv.path(),
            &node_schema,
        )
        .expect("opens");
        assert!(source.next().expect("a first row").is_ok());
        let err = source.next().expect("a second item").expect_err("fails");
        assert!(err.to_string().contains("float64"), "{err}");
        assert!(
            source.next().is_none(),
            "the iterator is done after an error"
        );
    }
}
