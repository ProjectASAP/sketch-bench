use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::accuracy::{Labels, Scores};

pub const OUTPUT_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Answer {
    pub time: i64,
    pub series: Labels,
    pub value: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exact_plan: Option<Scores>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TruthCell {
    pub time: i64,
    pub series: Labels,
    pub scores: Option<Scores>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanevalOutput {
    pub schema_version: u32,
    pub use_case: String,
    pub arm: Value,
    pub planner_rev: String,
    pub samples: usize,
    pub queries: Vec<PlanevalQuery>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum PlanevalQuery {
    Ran(PlannedQuery),
    Unsupported(UnsupportedQuery),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UnsupportedQuery {
    pub id: String,
    pub promql: String,
    pub unsupported: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlannedQuery {
    pub id: String,
    pub promql: String,
    pub unit: String,
    pub threshold: Option<f64>,
    pub plan: Value,
    pub evaluations: usize,
    pub evaluation_seconds: f64,
    pub views_refreshed: usize,
    pub reference_disagreements: Option<usize>,
    pub answers: Vec<Answer>,
    pub computed_truth: Option<Vec<TruthCell>>,
    pub costs: Vec<PlanevalCost>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanevalCost {
    pub time: i64,
    pub refresh_seconds: f64,
    pub samples_ingested: usize,
    pub query_seconds: f64,
    pub compile_seconds: f64,
    pub samples_read_at_query: usize,
    pub state_bytes: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SystemevalOutput {
    pub schema_version: u32,
    pub use_case: String,
    pub system: String,
    pub samples: usize,
    pub run_seconds: f64,
    pub write: WriteSummary,
    pub queries: Vec<SystemevalQuery>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WriteSummary {
    pub samples: usize,
    pub requests: usize,
    pub compressed_bytes: usize,
    pub retries: u64,
    pub speed: Option<f64>,
    pub batch_samples: usize,
    pub send_seconds: f64,
    pub barrier_seconds: f64,
    pub barriers: usize,
    pub samples_per_second: f64,
    pub max_lag_seconds: Option<f64>,
    pub status: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SystemevalQuery {
    pub id: String,
    pub promql: String,
    pub unit: String,
    pub threshold: Option<f64>,
    pub evaluations: usize,
    pub failed_evaluations: usize,
    pub failures: Vec<Failure>,
    pub not_finite_answers: usize,
    pub costs: Vec<SystemevalCost>,
    #[serde(flatten)]
    pub outcome: SystemevalOutcome,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum SystemevalOutcome {
    Scored {
        answers: Vec<Answer>,
        computed_truth: Option<Vec<TruthCell>>,
    },
    Unsupported {
        unsupported: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Failure {
    pub time: i64,
    pub error: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SystemevalCost {
    pub time: i64,
    pub query_seconds: f64,
    pub series_returned: Option<usize>,
}
