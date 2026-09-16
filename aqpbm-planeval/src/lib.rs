//! Reads a post-ASAP DAG, runs it over rows, and scores every readout against
//! the exact answer computed from the same rows.
//!
//! The plan is obtained in main memory (`plan::plan_promql`) rather than from a
//! file: everything the wire document drops — the candidates that lost, the
//! typed accuracy rejections, the share/recompute decision — is still reachable
//! there, and that is the counterfactual an advantage measurement needs.
//! `plan::to_json` exists so a run can still leave a fixture behind.

pub mod admit;
pub mod handle;
pub mod plan;
pub mod record;
pub mod rows;
pub mod run;
pub mod score;
pub mod types;
pub(crate) mod value;

pub use types::{Answer, EvalError, GroupKey, ItemKey, PlanId, Refusal, Row, Value};
