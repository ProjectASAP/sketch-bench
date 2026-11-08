use std::path::Path;

use anyhow::Result;
use aqpbm_core::benchmark_result::schema::{ExternalWorkload, WorkloadDescription};
use aqpbm_datagen::{
    ColumnSpec, DataDistribution, ExternalTableDescription, GeneratedTable, TableDescription,
    UniformParameter, RULE_NONE,
};

pub struct LoadedWorkload {
    pub table: GeneratedTable,
    /// Structural description consumed by the existing row materialisers.
    pub table_shape: TableDescription,
    pub workload: WorkloadDescription,
}

pub fn load(spec: &ExternalTableDescription, data_root: &Path) -> Result<Option<LoadedWorkload>> {
    let Some(table) = spec.load(data_root)? else {
        return Ok(None);
    };
    let (window_start_ns, window_end_ns) = spec.window_ns()?;
    let workload = WorkloadDescription::External(ExternalWorkload {
        source: spec.source_name().to_string(),
        dataset: spec.dataset.clone(),
        mode: spec.mode_name().to_string(),
        key_columns: spec.key_columns.clone(),
        group_columns: spec.group_columns.clone(),
        variate: spec.variate,
        value_column: spec.value_column.clone(),
        window_start_ns,
        window_end_ns,
        records_loaded: table.row_num,
        source_timestamp_unit: spec.source_timestamp_unit(),
        timestamp_unit: "nanoseconds".to_string(),
    });
    let table_shape = TableDescription {
        column_num: table.column_num,
        column_label: table.column_title.clone(),
        column_spec: table.data.iter().map(|c| shape_column(c.kind())).collect(),
        column_connected: Vec::new(),
        row_num: table.row_num,
    };
    Ok(Some(LoadedWorkload {
        table,
        table_shape,
        workload,
    }))
}

fn shape_column(data_type: &str) -> ColumnSpec {
    ColumnSpec {
        distribution: DataDistribution::Uniform(UniformParameter {
            lower_bound: 0.0,
            upper_bound: 1.0,
            seed: 0,
        }),
        shift: None,
        cardinality: None,
        special_rule: RULE_NONE,
        data_type: data_type.to_string(),
        string: None,
        child_of: None,
        fan_out: None,
        scale_by: None,
        scale_range: None,
        key_offset: None,
    }
}
