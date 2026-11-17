//! Runs ASAPPlanner plan files on external engines.
//!
//! [`plan_file`] reads an ASAPPlanner plan file (`InstallablePlan`, wire
//! version 15). [`clickhouse`] translates it into ClickHouse tables,
//! materialized views and one SQL query per query root. A translator is the
//! capability check: a node it has no rule for is refused with
//! [`TranslationError::UnsupportedShape`], and a rule that changes the
//! planner's guarantee is recorded as a [`Substitution`].
//!
//! The deployment-capabilities files under `capabilities/` tell the planner
//! what each engine can run; their README gives the evidence per entry.

pub mod clickhouse;
pub mod clickhouse_server;
pub mod plan_file;

/// A node's position in the plan file's DAG.
pub type NodeId = usize;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum TranslationError {
    /// The file is of another plan-file wire version, or has none.
    #[error("plan file wire version {found:?}; this reader supports {supported}")]
    UnsupportedWireVersion { found: Option<u32>, supported: u32 },
    /// A standalone scalar query (no node), which no translator runs.
    #[error("query {query_index} is a standalone scalar query")]
    UnsupportedScalarQuery { query_index: usize },
    /// A node the translator has no rule for.
    #[error("node {node}: no translation for {reason}")]
    UnsupportedShape { node: NodeId, reason: String },
    /// The file is not a plan of the supported wire version.
    #[error("malformed plan file: {reason}")]
    MalformedPlan { reason: String },
}

/// A translation rule whose result differs from the planner's semantics.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Substitution {
    pub node: NodeId,
    pub rule: String,
    pub guarantee_difference: String,
}

/// A query-time node two or more readers share in the plan: the engine
/// recomputes it for each reader.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SharingLost {
    pub node: NodeId,
    pub readers: Vec<NodeId>,
}
