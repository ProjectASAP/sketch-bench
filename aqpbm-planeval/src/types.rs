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

impl ItemKey {
    pub fn total_cmp(&self, other: &ItemKey) -> std::cmp::Ordering {
        fn rank(key: &ItemKey) -> u8 {
            match key {
                ItemKey::Str(_) => 0,
                ItemKey::Int(_) => 1,
                ItemKey::Float(_) => 2,
            }
        }
        match (self, other) {
            (ItemKey::Str(a), ItemKey::Str(b)) => a.cmp(b),
            (ItemKey::Int(a), ItemKey::Int(b)) => a.cmp(b),
            (ItemKey::Float(a), ItemKey::Float(b)) => a.total_cmp(b),
            _ => rank(self).cmp(&rank(other)),
        }
    }

    pub fn is_literal(&self, value: &str) -> bool {
        match self {
            ItemKey::Str(held) => held == value,
            ItemKey::Int(held) => value.parse::<i64>().map(|v| v == *held).unwrap_or(false),
            ItemKey::Float(held) => value
                .parse::<f64>()
                .map(|v| v.total_cmp(held).is_eq())
                .unwrap_or(false),
        }
    }

    pub fn rendered(&self) -> String {
        match self {
            ItemKey::Str(held) => held.clone(),
            ItemKey::Int(held) => held.to_string(),
            ItemKey::Float(held) => held.to_string(),
        }
    }

    pub fn heap_bytes(&self) -> usize {
        match self {
            ItemKey::Str(s) => s.capacity(),
            ItemKey::Int(_) | ItemKey::Float(_) => 0,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Retained {
    pub weights: Vec<f64>,
    pub keyed: Vec<(ItemKey, f64)>,
}

impl Retained {
    pub fn push(&mut self, item: Option<&ItemKey>, weight: f64) {
        self.weights.push(weight);
        if let Some(item) = item {
            self.keyed.push((item.clone(), weight));
        }
    }

    pub fn len(&self) -> usize {
        self.weights.len()
    }

    pub fn is_empty(&self) -> bool {
        self.weights.is_empty()
    }

    pub fn bytes(&self) -> usize {
        let keys: usize = self.keyed.iter().map(|(key, _)| key.heap_bytes()).sum();
        self.weights.capacity() * std::mem::size_of::<f64>()
            + self.keyed.capacity() * std::mem::size_of::<(ItemKey, f64)>()
            + keys
    }
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
    #[error("node {node:?}: the rows name their series and none of them is {metric:?}: {detail}")]
    MetricAbsentFromRows {
        node: PostAsapNodeId,
        metric: String,
        detail: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlanningStage {
    Catalog,
    Lower,
    Search,
    Compile,
    Validate,
}

impl PlanningStage {
    pub fn tag(self) -> &'static str {
        match self {
            PlanningStage::Catalog => "catalog",
            PlanningStage::Lower => "lower",
            PlanningStage::Search => "search",
            PlanningStage::Compile => "compile",
            PlanningStage::Validate => "validate",
        }
    }
}

impl std::fmt::Display for PlanningStage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.tag())
    }
}

#[derive(Debug, thiserror::Error)]
pub enum EvalError {
    #[error("plan was refused: {0:?}")]
    Refused(Vec<Refusal>),
    #[error("plan does not translate: {0:?}")]
    Untranslated(Vec<crate::df::Refusal>),
    #[error("dag validation failed: {0}")]
    Validation(String),
    #[error("planning failed at the {stage} stage: {detail}")]
    Planning {
        stage: PlanningStage,
        detail: String,
    },
    #[error("row source: {0}")]
    RowSource(String),
    #[error("summary handle: {0}")]
    Handle(String),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}
