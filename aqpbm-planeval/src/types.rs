use asap_types::post_asap::{PostAsapNodeId, SketchQuery, SummaryFamilyType};

pub type PlanId = [u8; 32];
pub type GroupKey = String;

#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Null,
    Int(i64),
    Float(f64),
    Str(String),
    Timestamp(i64),
}

impl Value {
    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Value::Float(v) => Some(*v),
            Value::Int(v) => Some(*v as f64),
            Value::Timestamp(v) => Some(*v as f64),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Row(pub Vec<Value>);

/// An item a keyed summary counts occurrences of. `Float` is here because
/// `SummaryInputExpr::Column(SampleValue)` is a keyed input the planner really
/// emits — `count(cpu_cores)` — over a `float64` column, and `DataInput::F64`
/// is a key `asap_sketchlib` hashes natively (`common/hash.rs:141`). Carrying
/// it means `Eq`/`Hash` cannot be derived; nothing keys a map by an `ItemKey`.
#[derive(Debug, Clone, PartialEq)]
pub enum ItemKey {
    Str(String),
    Int(i64),
    Float(f64),
}

#[derive(Debug, Clone, PartialEq)]
pub enum Answer {
    Scalar(f64),
    Ranked(Vec<(ItemKey, u64)>),
}

/// Every reason this evaluator declines a node, as a typed value so a corpus
/// sweep produces a machine-checkable table rather than prose.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum Refusal {
    #[error("node {node:?}: Fallback payload is a program, not a row source: {variant}")]
    FallbackIsAProgram {
        node: PostAsapNodeId,
        variant: String,
    },
    #[error("node {node:?}: operator {operator} is not implemented")]
    UnsupportedOperator {
        node: PostAsapNodeId,
        operator: String,
    },
    #[error("node {node:?}: no inverse operation exists for {operator}")]
    NoInverseOperation {
        node: PostAsapNodeId,
        operator: String,
    },
    #[error("node {node:?}: summary family {family:?} has no binding: {reason}")]
    UnboundFamily {
        node: PostAsapNodeId,
        family: Box<SummaryFamilyType>,
        reason: String,
    },
    #[error("node {node:?}: parameter out of bounds: {detail}")]
    ParameterOutOfBounds {
        node: PostAsapNodeId,
        detail: String,
    },
    #[error("node {node:?}: SketchKind is internally inconsistent: {detail}")]
    InconsistentSketchKind {
        node: PostAsapNodeId,
        detail: String,
    },
    #[error("node {node:?}: readout {query:?} is not implemented")]
    UnsupportedReadout {
        node: PostAsapNodeId,
        query: Box<SketchQuery>,
    },
    #[error(
        "node {node:?}: summary family {family:?} does not answer readout {query:?}; the exact \
         arm would score a different question"
    )]
    FamilyDoesNotAnswerReadout {
        node: PostAsapNodeId,
        family: Box<SummaryFamilyType>,
        query: Box<SketchQuery>,
    },
    #[error("node {node:?}: cannot resolve column {column}: {detail}")]
    UnresolvableColumn {
        node: PostAsapNodeId,
        column: String,
        detail: String,
    },
    #[error("node {node:?}: unsupported grouping: {detail}")]
    UnsupportedGrouping {
        node: PostAsapNodeId,
        detail: String,
    },
    #[error("node {node:?}: unsupported update shape: {detail}")]
    UnsupportedUpdate {
        node: PostAsapNodeId,
        detail: String,
    },
    #[error("node {node:?}: unmet window obligation on edge from {producer:?}")]
    UnmetWindowObligation {
        node: PostAsapNodeId,
        producer: PostAsapNodeId,
    },
    #[error("node {node:?}: extension escape hatch {name} has no registry entry")]
    UnregisteredExtension { node: PostAsapNodeId, name: String },
    #[error("node {node:?}: unsupported value operation: {detail}")]
    UnsupportedValueOperation {
        node: PostAsapNodeId,
        detail: String,
    },
}

#[derive(Debug, thiserror::Error)]
pub enum EvalError {
    #[error("plan was refused: {0:?}")]
    Refused(Vec<Refusal>),
    #[error("dag validation failed: {0}")]
    Validation(String),
    #[error("planning failed: {0}")]
    Planning(String),
    #[error("row source: {0}")]
    RowSource(String),
    #[error("summary handle: {0}")]
    Handle(String),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}
