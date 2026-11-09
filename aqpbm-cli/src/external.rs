use std::path::Path;

use anyhow::{bail, Result};
use aqpbm_core::benchmark_result::schema::{ExternalWorkload, WorkloadDescription};
use aqpbm_datagen::{ExternalTableDescription, GeneratedTable};
use sketch_bench::request::ColumnRoles;

pub struct LoadedWorkload {
    pub table: GeneratedTable,
    pub columns: ColumnRoles,
    pub workload: WorkloadDescription,
}

pub fn load(spec: &ExternalTableDescription, data_root: &Path) -> Result<Option<LoadedWorkload>> {
    let Some((window_start_ns, window_end_ns)) = spec.window_ns()? else {
        bail!("sketchbench runs one window per sketch: give the workload a window, or pass --window-start and --window-end");
    };
    if !spec.sort_by_start_time {
        bail!("sketchbench reports do not record arrival order yet, so sort_by_start_time has to stay true");
    }
    let Some(table) = spec.load(data_root)? else {
        return Ok(None);
    };
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
    let value = table.column_num as usize - 1;
    let columns = ColumnRoles {
        labels: (value - spec.group_columns.len()..value).collect(),
        value,
        value_key_offset: None,
    };
    Ok(Some(LoadedWorkload {
        table,
        columns,
        workload,
    }))
}
