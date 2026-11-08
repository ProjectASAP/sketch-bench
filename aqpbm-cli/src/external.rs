use std::path::Path;

use anyhow::Result;
use aqpbm_core::benchmark_result::schema::{ExternalWorkload, WorkloadDescription};
use aqpbm_datagen::{ExternalTableDescription, GeneratedTable};
use sketch_bench::request::ColumnRoles;

pub struct LoadedWorkload {
    pub table: GeneratedTable,
    pub columns: ColumnRoles,
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
    let columns = ColumnRoles {
        labels: spec
            .group_columns
            .iter()
            .map(|title| table.column_index(title))
            .collect::<Result<_, _>>()?,
        value: table.column_index(&spec.value_column)?,
        value_key_offset: None,
    };
    Ok(Some(LoadedWorkload {
        table,
        columns,
        workload,
    }))
}
