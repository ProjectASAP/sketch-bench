use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use asap_types::pre_asap::agg_intent::{AggIntent, MathFunc, TimeFunc};
use asap_types::pre_asap::expr_ir::{ArithmeticOpKind, CompareOpKind, ScalarValue};
use asap_types::pre_asap::query_expr::{
    BinaryOpKind, GroupKeys, InfoMatcher, JoinKind, Predicate, ProjectItem, Reduction,
    RelationalSetOpKind, SampleKind, SortKey, Source, TimeShift, VectorMatch, VectorMatchKind,
    WindowFuncKind,
};
use asap_types::pre_asap::schema::{Column, DataType, Schema};
use asap_types::pre_asap::QueryExpr;
use asap_types::types::AccuracyTarget;
use datafusion::arrow::datatypes::{
    DataType as ArrowDataType, Field, Schema as ArrowSchema, TimeUnit,
};
use datafusion::common::DFSchema;
use datafusion::logical_expr::{LogicalPlan, LogicalPlanBuilder};

use crate::df::scalar::ColumnScope;

pub const TABLE: &str = "t";

pub const TS: usize = 0;
pub const SERVICE: usize = 1;
pub const LATENCY: usize = 2;
pub const BYTES: usize = 3;

pub const ACCURACY: AccuracyTarget = AccuracyTarget::Epsilon(0.01);

pub fn leaf_schema() -> Schema {
    Schema::new(vec![
        Column::new("ts", DataType::Timestamp, false),
        Column::new("service", DataType::Utf8, false),
        Column::new("latency", DataType::Float64, false),
        Column::new("bytes", DataType::Int64, false),
    ])
}

pub fn leaf_arrow_schema() -> ArrowSchema {
    ArrowSchema::new(vec![
        Field::new(
            "ts",
            ArrowDataType::Timestamp(TimeUnit::Millisecond, None),
            false,
        ),
        Field::new("service", ArrowDataType::Utf8, false),
        Field::new("latency", ArrowDataType::Float64, false),
        Field::new("bytes", ArrowDataType::Int64, false),
    ])
}

pub fn leaf_scope() -> ColumnScope {
    let schema = Arc::new(DFSchema::try_from(leaf_arrow_schema()).expect("a flat Arrow schema"));
    ColumnScope::of_schema(&schema)
}

pub fn leaf_plan() -> LogicalPlan {
    LogicalPlanBuilder::empty(true)
        .build()
        .expect("an empty relation builds")
}

pub fn scan() -> Rc<QueryExpr> {
    Rc::new(QueryExpr::Scan {
        source: Source::Table {
            table_ref: TABLE.to_owned(),
        },
        predicates: Vec::new(),
        schema: leaf_schema(),
    })
}

pub fn qualified_scan(qualifier: &str) -> Rc<QueryExpr> {
    Rc::new(QueryExpr::Project {
        cols: (0..leaf_schema().columns.len())
            .map(|id| ProjectItem {
                alias: None,
                expr: QueryExpr::Column(id),
            })
            .collect(),
        qualifier: Some(qualifier.to_owned()),
        child: scan(),
    })
}

pub fn column(id: usize) -> QueryExpr {
    QueryExpr::Column(id)
}

pub fn literal(value: f64) -> QueryExpr {
    QueryExpr::Literal(ScalarValue::Float64(value))
}

pub fn predicate() -> Predicate {
    Predicate(Rc::new(QueryExpr::Compare {
        left: Rc::new(column(LATENCY)),
        op: CompareOpKind::Gt,
        right: Rc::new(literal(1.0)),
    }))
}

pub fn every_query_expr() -> Vec<(&'static str, QueryExpr)> {
    vec![
        ("Scan", (*scan()).clone()),
        (
            "PromqlScalarBridge",
            QueryExpr::PromqlScalarBridge(Rc::new(literal(3.0))),
        ),
        ("EvalTimestamp", QueryExpr::EvalTimestamp),
        ("CurrentTimestamp", QueryExpr::CurrentTimestamp),
        (
            "PromqlVectorFromScalar",
            QueryExpr::PromqlVectorFromScalar(Rc::new(literal(0.0))),
        ),
        (
            "PromqlScalarFromVector",
            QueryExpr::PromqlScalarFromVector(scan()),
        ),
        (
            "PromqlRelabel",
            QueryExpr::PromqlRelabel {
                dst: "host".to_owned(),
                value: Rc::new(literal(0.0)),
                child: scan(),
            },
        ),
        (
            "PromqlInfoEnrich",
            QueryExpr::PromqlInfoEnrich {
                selector: vec![InfoMatcher {
                    label: "__name__".to_owned(),
                    op: CompareOpKind::Eq,
                    value: "target_info".to_owned(),
                }],
                child: scan(),
            },
        ),
        (
            "PromqlSeriesSample",
            QueryExpr::PromqlSeriesSample {
                by: GroupKeys::by(vec![SERVICE]),
                kind: SampleKind::LimitK(3),
                child: scan(),
            },
        ),
        (
            "Filter",
            QueryExpr::Filter {
                pred: predicate(),
                child: scan(),
            },
        ),
        (
            "Project",
            QueryExpr::Project {
                cols: vec![ProjectItem {
                    alias: None,
                    expr: column(SERVICE),
                }],
                qualifier: None,
                child: scan(),
            },
        ),
        (
            "Aggregate",
            QueryExpr::Aggregate {
                reduction: Reduction::by(vec![SERVICE]),
                measures: vec![AggIntent::Sum { col: Some(BYTES) }],
                output_names: vec!["total".to_owned()],
                having: None,
                child: scan(),
            },
        ),
        (
            "Dedup",
            QueryExpr::Dedup {
                cols: vec![SERVICE],
                child: scan(),
            },
        ),
        (
            "Concat",
            QueryExpr::Concat {
                children: vec![(*scan()).clone(), (*scan()).clone()],
                discriminator_unique_key: None,
            },
        ),
        (
            "Join",
            QueryExpr::Join {
                kind: JoinKind::Inner,
                pred: Predicate(Rc::new(QueryExpr::Compare {
                    left: Rc::new(column(SERVICE)),
                    op: CompareOpKind::Eq,
                    right: Rc::new(column(SERVICE + leaf_schema().columns.len())),
                })),
                left: scan(),
                right: qualified_scan("u"),
            },
        ),
        (
            "SetOp",
            QueryExpr::SetOp {
                kind: RelationalSetOpKind::Union,
                all: true,
                left: scan(),
                right: scan(),
            },
        ),
        (
            "Sort",
            QueryExpr::Sort {
                keys: vec![SortKey {
                    expr: column(LATENCY),
                    ascending: true,
                    nulls_first: false,
                }],
                partition_by: GroupKeys::none(),
                child: scan(),
            },
        ),
        (
            "Limit",
            QueryExpr::Limit {
                n: 5,
                offset: 0,
                child: scan(),
            },
        ),
        (
            "PromqlSubquery",
            QueryExpr::PromqlSubquery {
                range: Duration::from_secs(300),
                resolution: None,
                child: scan(),
            },
        ),
        (
            "TimeRange",
            QueryExpr::TimeRange {
                range: Duration::from_secs(300),
                child: scan(),
            },
        ),
        (
            "TimeShift",
            QueryExpr::TimeShift {
                shift: TimeShift {
                    offset_ms: 1_000,
                    at: None,
                },
                child: scan(),
            },
        ),
        (
            "SQLWindowFunc",
            QueryExpr::SQLWindowFunc {
                func: WindowFuncKind::RowNumber,
                args: Vec::new(),
                partition_by: GroupKeys::by(vec![SERVICE]),
                order_by: vec![SortKey {
                    expr: column(LATENCY),
                    ascending: false,
                    nulls_first: false,
                }],
                frame: None,
                output_name: "rn".to_owned(),
                child: scan(),
            },
        ),
        (
            "BinaryOp",
            QueryExpr::BinaryOp {
                op: BinaryOpKind::Arithmetic(ArithmeticOpKind::Add),
                lhs: scan(),
                rhs: scan(),
                vector_match: None,
            },
        ),
        ("Column", column(LATENCY)),
        ("Literal", literal(2.0)),
        (
            "Compare",
            QueryExpr::Compare {
                left: Rc::new(column(LATENCY)),
                op: CompareOpKind::Ge,
                right: Rc::new(literal(0.0)),
            },
        ),
        (
            "BoolAnd",
            QueryExpr::BoolAnd(vec![(*predicate().0).clone()]),
        ),
        ("BoolOr", QueryExpr::BoolOr(vec![(*predicate().0).clone()])),
        ("Not", QueryExpr::Not(predicate().0)),
        ("IsNull", QueryExpr::IsNull(Rc::new(column(LATENCY)))),
        ("IsNotNull", QueryExpr::IsNotNull(Rc::new(column(LATENCY)))),
        (
            "Cast",
            QueryExpr::Cast {
                expr: Rc::new(column(BYTES)),
                to: DataType::Float64,
                try_cast: false,
            },
        ),
        (
            "InList",
            QueryExpr::InList {
                expr: Rc::new(column(SERVICE)),
                list: vec![QueryExpr::Literal(ScalarValue::Utf8("a".to_owned()))],
                negated: false,
            },
        ),
        (
            "FunctionCall",
            QueryExpr::FunctionCall {
                name: "abs".to_owned(),
                args: vec![column(LATENCY)],
            },
        ),
        (
            "Arithmetic",
            QueryExpr::Arithmetic {
                op: ArithmeticOpKind::Mul,
                left: Rc::new(column(LATENCY)),
                right: Rc::new(literal(2.0)),
            },
        ),
        (
            "Case",
            QueryExpr::Case {
                operand: None,
                branches: vec![((*predicate().0).clone(), literal(1.0))],
                else_expr: Some(Rc::new(literal(0.0))),
            },
        ),
    ]
}

pub fn vector_matched_binary_op() -> QueryExpr {
    QueryExpr::BinaryOp {
        op: BinaryOpKind::Arithmetic(ArithmeticOpKind::Add),
        lhs: scan(),
        rhs: scan(),
        vector_match: Some(VectorMatch {
            kind: VectorMatchKind::On,
            labels: vec!["service".to_owned()],
            grouping: None,
        }),
    }
}

pub fn every_agg_intent() -> Vec<(&'static str, AggIntent)> {
    vec![
        ("AggIntent::Count", AggIntent::Count { accuracy: ACCURACY }),
        ("AggIntent::Sum", AggIntent::Sum { col: Some(BYTES) }),
        ("AggIntent::Min", AggIntent::Min { col: Some(LATENCY) }),
        ("AggIntent::Max", AggIntent::Max { col: Some(LATENCY) }),
        ("AggIntent::Avg", AggIntent::Avg { col: Some(LATENCY) }),
        (
            "AggIntent::StdDev",
            AggIntent::StdDev {
                col: Some(LATENCY),
                population: false,
            },
        ),
        (
            "AggIntent::Variance",
            AggIntent::Variance {
                col: Some(LATENCY),
                population: true,
            },
        ),
        (
            "AggIntent::PearsonCorr",
            AggIntent::PearsonCorr {
                left: LATENCY,
                right: BYTES,
            },
        ),
        (
            "AggIntent::Quantile",
            AggIntent::Quantile {
                col: Some(LATENCY),
                q: 0.99,
                accuracy: ACCURACY,
            },
        ),
        (
            "AggIntent::TopK",
            AggIntent::TopK {
                k: 5,
                accuracy: ACCURACY,
            },
        ),
        (
            "AggIntent::Cardinality",
            AggIntent::Cardinality {
                col: Some(SERVICE),
                accuracy: ACCURACY,
            },
        ),
        (
            "AggIntent::FrequencyL2",
            AggIntent::FrequencyL2 {
                col: Some(SERVICE),
                accuracy: ACCURACY,
            },
        ),
        (
            "AggIntent::FrequencyEntropy",
            AggIntent::FrequencyEntropy {
                col: Some(SERVICE),
                accuracy: ACCURACY,
            },
        ),
        ("AggIntent::Rate", AggIntent::Rate),
        ("AggIntent::IRate", AggIntent::IRate),
        ("AggIntent::Increase", AggIntent::Increase),
        ("AggIntent::Changes", AggIntent::Changes),
        ("AggIntent::Delta", AggIntent::Delta),
        ("AggIntent::IDelta", AggIntent::IDelta),
        ("AggIntent::Deriv", AggIntent::Deriv),
        ("AggIntent::Resets", AggIntent::Resets),
        (
            "AggIntent::PredictLinear",
            AggIntent::PredictLinear { seconds: 60.0 },
        ),
        (
            "AggIntent::DoubleExpSmoothing",
            AggIntent::DoubleExpSmoothing {
                smoothing: 0.5,
                trend: 0.5,
            },
        ),
        ("AggIntent::HistogramCount", AggIntent::HistogramCount),
        ("AggIntent::HistogramSum", AggIntent::HistogramSum),
        ("AggIntent::HistogramAvg", AggIntent::HistogramAvg),
        ("AggIntent::HistogramStdDev", AggIntent::HistogramStdDev),
        ("AggIntent::HistogramStdVar", AggIntent::HistogramStdVar),
        (
            "AggIntent::HistogramFraction",
            AggIntent::HistogramFraction {
                lower: 0.0,
                upper: 1.0,
            },
        ),
        (
            "AggIntent::HistogramQuantile",
            AggIntent::HistogramQuantile { q: 0.99 },
        ),
        ("AggIntent::Math", AggIntent::Math(MathFunc::Abs)),
        ("AggIntent::Absent", AggIntent::Absent),
        ("AggIntent::AbsentOverTime", AggIntent::AbsentOverTime),
        ("AggIntent::PresentOverTime", AggIntent::PresentOverTime),
        ("AggIntent::TimeFn", AggIntent::TimeFn(TimeFunc::Hour)),
        ("AggIntent::Group", AggIntent::Group),
        (
            "AggIntent::CountValues",
            AggIntent::CountValues {
                label: "l".to_owned(),
            },
        ),
        ("AggIntent::LastOverTime", AggIntent::LastOverTime),
        ("AggIntent::FirstOverTime", AggIntent::FirstOverTime),
        ("AggIntent::MadOverTime", AggIntent::MadOverTime),
        ("AggIntent::TsOfMinOverTime", AggIntent::TsOfMinOverTime),
        ("AggIntent::TsOfMaxOverTime", AggIntent::TsOfMaxOverTime),
        ("AggIntent::TsOfFirstOverTime", AggIntent::TsOfFirstOverTime),
        ("AggIntent::TsOfLastOverTime", AggIntent::TsOfLastOverTime),
        (
            "AggIntent::Extension",
            AggIntent::Extension {
                ext_kind: "argMax".to_owned(),
                payload: serde_json::Value::Null,
            },
        ),
    ]
}
