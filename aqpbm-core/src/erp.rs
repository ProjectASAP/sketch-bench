//! Error–Resource Profile (ERP) export for workload-conditioned sketch tuning.
//!
//! ERP deliberately preserves the benchmark workload descriptor instead of
//! collapsing measurements into a configuration-only cost table.  Consumers
//! may therefore use a row only when its distribution context applies.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use aqpbm_datagen::DataDistribution;

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
    pub distribution: serde_json::Value,
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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ErpDataShape {
    pub cardinality: u64,
    /// Absent means uniform; present means the Zipf exponent.
    pub zipf_exponent: Option<f64>,
    pub benchmark_events: u64,
}

fn distribution_descriptor(workload: &WorkloadDescription) -> serde_json::Value {
    let workload_value = serde_json::to_value(workload).expect("workload description serializes");
    let shape = match workload {
        WorkloadDescription::Synthetic { description, .. } => {
            description.column_spec.first().and_then(|column| {
                let cardinality = column.distribution.domain()?.size;
                let zipf_exponent = match &column.distribution {
                    DataDistribution::Zipf(parameters) => Some(parameters.skewness),
                    DataDistribution::Uniform(_) => None,
                    DataDistribution::Normal(_) => return None,
                };
                Some(ErpDataShape {
                    cardinality,
                    zipf_exponent,
                    benchmark_events: description.row_num,
                })
            })
        }
        WorkloadDescription::External(_) => None,
    };
    let mut output = serde_json::Map::from_iter([("workload".into(), workload_value)]);
    if let Some(shape) = shape {
        output.insert(
            "erp_shape".into(),
            serde_json::to_value(shape).expect("ERP shape serializes"),
        );
    }
    serde_json::Value::Object(output)
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
        distribution: distribution_descriptor(&record.input_dataset),
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

    #[test]
    fn synthetic_profile_exports_matchable_shape() {
        let workload = WorkloadDescription::Synthetic {
            description: aqpbm_datagen::TableDescription::single(
                "key",
                aqpbm_datagen::ColumnSpec {
                    distribution: DataDistribution::Zipf(aqpbm_datagen::ZipfParameter {
                        skewness: 1.2,
                        population_size: 1_000,
                        seed: 7,
                    }),
                    shift: None,
                    cardinality: Some(1_000),
                    special_rule: 0,
                    data_type: "u64".into(),
                    string: None,
                },
                100_000,
            ),
            burst: None,
        };
        let descriptor = distribution_descriptor(&workload);
        assert_eq!(descriptor["erp_shape"]["cardinality"], 1_000);
        assert_eq!(descriptor["erp_shape"]["zipf_exponent"], 1.2);
        assert_eq!(descriptor["erp_shape"]["benchmark_events"], 100_000);
        assert_eq!(
            descriptor["workload"]["synthetic"]["description"]["row_num"],
            100_000
        );
    }
}
