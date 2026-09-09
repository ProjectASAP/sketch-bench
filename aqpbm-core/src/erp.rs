//! Error–Resource Profile (ERP) export for workload-conditioned sketch tuning.
//!
//! ERP deliberately preserves the benchmark workload descriptor instead of
//! collapsing measurements into a configuration-only cost table.  Consumers
//! may therefore use a row only when its distribution context applies.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::{reduce_one, MergedRecord, SkipReason, WorkloadDescription};

pub const ERP_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ErpResourceProfile {
    pub memory_bytes: f64,
    pub update_cpu_seconds: f64,
    pub merge_cpu_seconds: f64,
    pub query_cpu_seconds: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ErpRecord {
    pub id: String,
    /// Structural implementation identity, e.g. `cms-fastpath-vector2d`.
    pub sketch: String,
    pub implementation: String,
    pub parameters: serde_json::Value,
    /// Generator parameters or external trace identity. This is an
    /// applicability key, not decorative provenance.
    pub distribution: WorkloadDescription,
    /// Number of benchmark runs backing the exported error/resource point.
    pub trials: u32,
    pub error_metrics: BTreeMap<String, f64>,
    pub resources: ErpResourceProfile,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ErpArtifact {
    pub schema_version: u32,
    pub producer_version: String,
    pub records: Vec<ErpRecord>,
}

/// Convert a complete flat benchmark row into a distribution-conditioned ERP
/// row. The caller supplies an artifact-local ID and producer revision so an
/// export remains auditable.
pub fn erp_record(id: impl Into<String>, record: &MergedRecord) -> Result<ErpRecord, SkipReason> {
    let atomic = reduce_one(record)?;
    Ok(ErpRecord {
        id: id.into(),
        sketch: atomic.sketch,
        implementation: record.library.clone(),
        parameters: atomic.sketch_config,
        distribution: record.input_dataset.clone(),
        trials: u32::try_from(record.runs).unwrap_or(u32::MAX),
        error_metrics: atomic.query_accuracy,
        resources: ErpResourceProfile {
            memory_bytes: atomic.mem_bytes_per_instance,
            update_cpu_seconds: atomic.insert_cpu_secs,
            merge_cpu_seconds: atomic.merge_cpu_secs,
            query_cpu_seconds: atomic.query_cpu_secs,
        },
    })
}

pub fn erp_artifact(
    records: &[MergedRecord],
    producer_version: impl Into<String>,
) -> (ErpArtifact, Vec<(usize, SkipReason)>) {
    let mut output = Vec::new();
    let mut skipped = Vec::new();
    for (index, record) in records.iter().enumerate() {
        match erp_record(format!("erp-{index}"), record) {
            Ok(row) => output.push(row),
            Err(reason) => skipped.push((index, reason)),
        }
    }
    (
        ErpArtifact {
            schema_version: ERP_SCHEMA_VERSION,
            producer_version: producer_version.into(),
            records: output,
        },
        skipped,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn artifact_version_is_explicit() {
        let (artifact, skipped) = erp_artifact(&[], "bench-rev-1");
        assert_eq!(artifact.schema_version, ERP_SCHEMA_VERSION);
        assert_eq!(artifact.producer_version, "bench-rev-1");
        assert!(artifact.records.is_empty());
        assert!(skipped.is_empty());
    }
}
