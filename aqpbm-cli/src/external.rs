//! Bounded loaders for representative external trace formats.
//!
//! Loading, decompression, validation, ordering, and window selection all
//! happen before rows::measurements is called. The measured region therefore
//! sees the same in-memory table shape as a synthetic run.

use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use aqpbm_core::benchmark_result::schema::{ExternalWorkload, WorkloadDescription};
use aqpbm_datagen::{
    ColumnData, ColumnSpec, DataDistribution, GeneratedTable, TableDescription, UniformParameter,
    RULE_NONE,
};
use csv::StringRecord;
use flate2::read::GzDecoder;
use polars::io::ipc::IpcStreamReader;
use polars::prelude::{AnyValue, IpcReader, SerReader};
use serde::Deserialize;

const NANOS_PER_SECOND: i64 = 1_000_000_000;

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Source {
    Boom,
    Google,
    Alibaba,
    /// User-provided CSV (optionally gzip-compressed).
    Custom,
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum InputMode {
    Scalar,
    Grouped,
    Keyed,
}

#[derive(Debug, Clone, Deserialize)]
pub struct WindowSpec {
    pub start: String,
    pub end: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct WorkloadSpec {
    pub source: Source,
    /// A path relative to --data-root, retained as the report identity.
    pub dataset: String,
    #[serde(default)]
    pub table: Option<String>,
    pub mode: InputMode,
    #[serde(default)]
    pub key_columns: Vec<String>,
    #[serde(default)]
    pub group_columns: Vec<String>,
    pub value_column: String,
    /// Required for `source: custom` and interpreted using `timestamp_unit`.
    #[serde(default)]
    pub timestamp_column: Option<String>,
    /// Optional interval end column. Without it, rows are timestamped points.
    #[serde(default)]
    pub end_timestamp_column: Option<String>,
    #[serde(default)]
    pub variate: Option<usize>,
    #[serde(default)]
    pub timestamp_unit: Option<String>,
    pub window: WindowSpec,
    #[serde(default = "default_min_records")]
    pub min_records: usize,
}

fn default_min_records() -> usize {
    1
}

pub struct LoadedWorkload {
    pub table: GeneratedTable,
    /// Structural description consumed by the existing row materialisers.
    pub table_shape: TableDescription,
    pub workload: WorkloadDescription,
}

#[derive(Debug)]
struct RawRecord {
    start_ns: i64,
    end_ns: i64,
    groups: Vec<String>,
    value: f64,
    source_row: usize,
}

pub fn load_spec(path: &Path) -> Result<WorkloadSpec> {
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("reading external workload spec {}", path.display()))?;
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or_default();
    match ext {
        "yaml" | "yml" => serde_norway::from_str(&text)
            .with_context(|| format!("parsing external workload YAML {}", path.display())),
        _ => serde_json::from_str(&text)
            .with_context(|| format!("parsing external workload JSON {}", path.display())),
    }
}

pub fn load(spec: &WorkloadSpec, data_root: &Path) -> Result<Option<LoadedWorkload>> {
    if spec.dataset.is_empty() {
        bail!("external workload dataset cannot be empty");
    }
    if spec.min_records == 0 {
        bail!("--min-records must be greater than zero");
    }
    let logical = Path::new(&spec.dataset);
    if logical.is_absolute() {
        bail!(
            "external workload dataset must be relative to --data-root: {}",
            spec.dataset
        );
    }
    if spec.mode == InputMode::Keyed {
        bail!("keyed external workloads are tracked in issue #122 and are not implemented yet");
    }
    if !spec.key_columns.is_empty() {
        bail!("key_columns are only valid for keyed workloads (tracked in issue #122)");
    }
    if spec.mode == InputMode::Grouped && spec.group_columns.is_empty() {
        bail!("grouped external workloads require at least one group column");
    }
    if spec.mode == InputMode::Scalar && !spec.group_columns.is_empty() {
        bail!("scalar external workloads cannot specify group_columns");
    }
    if matches!(spec.source, Source::Custom) && spec.timestamp_column.is_none() {
        bail!("custom workloads require timestamp_column");
    }

    let window_start_ns = parse_timestamp_ns(&spec.window.start)
        .with_context(|| format!("invalid window start '{}'", spec.window.start))?;
    let window_end_ns = parse_timestamp_ns(&spec.window.end)
        .with_context(|| format!("invalid window end '{}'", spec.window.end))?;
    if window_start_ns >= window_end_ns {
        bail!("window start must be before window end");
    }
    let path = data_root.join(logical);
    let mut records = match &spec.source {
        Source::Boom => load_boom(spec, &path, window_start_ns, window_end_ns)?,
        Source::Google => load_google(spec, &path, window_start_ns, window_end_ns)?,
        Source::Alibaba => load_alibaba(spec, &path, window_start_ns, window_end_ns)?,
        Source::Custom => load_custom(spec, &path, window_start_ns, window_end_ns)?,
    };
    records.sort_by_key(|r| (r.start_ns, r.source_row));
    if records.len() < spec.min_records {
        if records.is_empty() {
            return Ok(None);
        }
        bail!(
            "external workload window contains {} records, below --min-records {}",
            records.len(),
            spec.min_records
        );
    }

    let (table, table_shape) = materialise(&records, spec);
    table
        .validate()
        .map_err(|e| anyhow::anyhow!("materialised external workload is invalid: {e}"))?;
    let workload = WorkloadDescription::External(ExternalWorkload {
        source: source_name(&spec.source).to_string(),
        dataset: spec.dataset.clone(),
        mode: mode_name(spec.mode).to_string(),
        key_columns: spec.key_columns.clone(),
        group_columns: spec.group_columns.clone(),
        variate: spec.variate,
        value_column: spec.value_column.clone(),
        window_start_ns,
        window_end_ns,
        records_loaded: records.len() as u64,
        source_timestamp_unit: source_timestamp_unit(spec),
        timestamp_unit: "nanoseconds".to_string(),
    });
    Ok(Some(LoadedWorkload {
        table,
        table_shape,
        workload,
    }))
}

fn source_name(source: &Source) -> &'static str {
    match source {
        Source::Boom => "boom",
        Source::Google => "google",
        Source::Alibaba => "alibaba",
        Source::Custom => "custom",
    }
}

fn source_timestamp_unit(spec: &WorkloadSpec) -> String {
    match &spec.source {
        Source::Boom => "frequency-derived".to_string(),
        Source::Google => "microseconds".to_string(),
        Source::Alibaba => spec
            .timestamp_unit
            .clone()
            .unwrap_or_else(|| "milliseconds".to_string()),
        Source::Custom => spec
            .timestamp_unit
            .clone()
            .unwrap_or_else(|| "nanoseconds".to_string()),
    }
}

fn mode_name(mode: InputMode) -> &'static str {
    match mode {
        InputMode::Scalar => "scalar",
        InputMode::Grouped => "grouped",
        InputMode::Keyed => "keyed",
    }
}

fn materialise(records: &[RawRecord], spec: &WorkloadSpec) -> (GeneratedTable, TableDescription) {
    let mut data = Vec::new();
    let mut labels = Vec::new();
    let mut specs = Vec::new();
    for column in &spec.group_columns {
        labels.push(column.clone());
        specs.push(shape_column("string"));
        let values = records
            .iter()
            .map(|r| r.groups[labels.len() - 1].clone())
            .collect();
        data.push(ColumnData::String(values));
    }
    labels.push(spec.value_column.clone());
    specs.push(shape_column("f64"));
    data.push(ColumnData::Float64(
        records.iter().map(|r| r.value).collect(),
    ));
    let row_num = records.len() as u64;
    let table = GeneratedTable {
        column_num: data.len() as u32,
        column_title: labels.clone(),
        data,
        row_num,
    };
    let shape = TableDescription {
        column_num: specs.len() as u32,
        column_label: labels,
        column_spec: specs,
        column_connected: Vec::new(),
        row_num,
    };
    (table, shape)
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
    }
}

fn parse_timestamp_ns(value: &str) -> Result<i64> {
    if let Ok(ns) = value.parse::<i64>() {
        return Ok(ns);
    }
    if let Ok(timestamp) = chrono::DateTime::parse_from_rfc3339(value) {
        return timestamp
            .timestamp_nanos_opt()
            .ok_or_else(|| anyhow::anyhow!("timestamp is outside nanosecond range"));
    }
    for format in ["%Y-%m-%dT%H:%M:%S%.f", "%Y-%m-%d %H:%M:%S%.f"] {
        if let Ok(naive) = chrono::NaiveDateTime::parse_from_str(value, format) {
            return naive
                .and_utc()
                .timestamp_nanos_opt()
                .ok_or_else(|| anyhow::anyhow!("timestamp is outside nanosecond range"));
        }
    }
    bail!("expected Unix nanoseconds or RFC3339/UTC timestamp, got '{value}'")
}

fn number_to_ns(value: &str, unit: &str) -> Result<i64> {
    let value: i128 = value
        .parse()
        .with_context(|| format!("timestamp '{value}' is not an integer in {unit}"))?;
    let multiplier: i128 = match unit {
        "seconds" | "s" => NANOS_PER_SECOND as i128,
        "milliseconds" | "ms" => 1_000_000,
        "microseconds" | "us" => 1_000,
        "nanoseconds" | "ns" => 1,
        other => bail!("unknown timestamp unit '{other}'"),
    };
    let ns = value
        .checked_mul(multiplier)
        .ok_or_else(|| anyhow::anyhow!("timestamp overflows nanoseconds"))?;
    i64::try_from(ns).context("timestamp is outside nanosecond range")
}

fn include(record: RawRecord, start_ns: i64, end_ns: i64, out: &mut Vec<RawRecord>) {
    if contained(record.start_ns, record.end_ns, start_ns, end_ns) {
        out.push(record);
    }
}

fn contained(record_start: i64, record_end: i64, window_start: i64, window_end: i64) -> bool {
    if record_start == record_end {
        record_start >= window_start && record_start < window_end
    } else {
        record_start >= window_start && record_end <= window_end
    }
}

fn open_reader(path: &Path) -> Result<Box<dyn Read>> {
    let file = File::open(path).with_context(|| format!("opening dataset {}", path.display()))?;
    if path.extension().and_then(|e| e.to_str()) == Some("gz") {
        Ok(Box::new(GzDecoder::new(file)))
    } else {
        Ok(Box::new(file))
    }
}

fn load_alibaba(
    spec: &WorkloadSpec,
    path: &Path,
    window_start_ns: i64,
    window_end_ns: i64,
) -> Result<Vec<RawRecord>> {
    let mut reader = csv::ReaderBuilder::new()
        .flexible(false)
        .from_reader(open_reader(path)?);
    let headers = reader
        .headers()
        .context("reading Alibaba CSV header")?
        .clone();
    let timestamp = index(&headers, "timestamp")?;
    let value = index(&headers, &spec.value_column)?;
    let groups: Vec<usize> = spec
        .group_columns
        .iter()
        .map(|name| index(&headers, name))
        .collect::<Result<_>>()?;
    let unit = spec.timestamp_unit.as_deref().unwrap_or("milliseconds");
    let mut out = Vec::new();
    for (row, result) in reader.records().enumerate() {
        let record = result.with_context(|| format!("malformed Alibaba CSV row {}", row + 2))?;
        let start_ns = number_to_ns(field(&record, timestamp, row)?, unit)
            .with_context(|| format!("Alibaba row {} timestamp", row + 2))?;
        if !contained(start_ns, start_ns, window_start_ns, window_end_ns) {
            continue;
        }
        let value = parse_value(field(&record, value, row)?)
            .with_context(|| format!("Alibaba row {} value", row + 2))?;
        include(
            RawRecord {
                start_ns,
                end_ns: start_ns,
                groups: groups
                    .iter()
                    .map(|i| field(&record, *i, row).map(str::to_string))
                    .collect::<Result<_>>()?,
                value,
                source_row: row,
            },
            window_start_ns,
            window_end_ns,
            &mut out,
        );
    }
    Ok(out)
}

fn load_custom(
    spec: &WorkloadSpec,
    path: &Path,
    window_start_ns: i64,
    window_end_ns: i64,
) -> Result<Vec<RawRecord>> {
    let mut reader = csv::ReaderBuilder::new()
        .flexible(false)
        .from_reader(open_reader(path)?);
    let headers = reader
        .headers()
        .context("reading custom CSV header")?
        .clone();
    let timestamp_name = spec
        .timestamp_column
        .as_deref()
        .ok_or_else(|| anyhow::anyhow!("custom workloads require timestamp_column"))?;
    let timestamp = index(&headers, timestamp_name)?;
    let end_timestamp = spec
        .end_timestamp_column
        .as_deref()
        .map(|name| index(&headers, name))
        .transpose()?;
    let value = index(&headers, &spec.value_column)?;
    let groups: Vec<usize> = spec
        .group_columns
        .iter()
        .map(|name| index(&headers, name))
        .collect::<Result<_>>()?;
    let unit = spec.timestamp_unit.as_deref().unwrap_or("nanoseconds");
    let mut out = Vec::new();
    for (row, result) in reader.records().enumerate() {
        let record = result.with_context(|| format!("malformed custom CSV row {}", row + 2))?;
        let start_ns = number_to_ns(field(&record, timestamp, row)?, unit)
            .with_context(|| format!("custom row {} timestamp", row + 2))?;
        let end_ns = end_timestamp
            .map(|column| number_to_ns(field(&record, column, row)?, unit))
            .transpose()
            .with_context(|| format!("custom row {} end timestamp", row + 2))?
            .unwrap_or(start_ns);
        if end_ns < start_ns {
            bail!("custom row {} ends before it starts", row + 2);
        }
        let value = parse_value(field(&record, value, row)?)
            .with_context(|| format!("custom row {} value", row + 2))?;
        include(
            RawRecord {
                start_ns,
                end_ns,
                groups: groups
                    .iter()
                    .map(|column| field(&record, *column, row).map(str::to_string))
                    .collect::<Result<_>>()?,
                value,
                source_row: row,
            },
            window_start_ns,
            window_end_ns,
            &mut out,
        );
    }
    Ok(out)
}

fn load_google(
    spec: &WorkloadSpec,
    path: &Path,
    window_start_ns: i64,
    window_end_ns: i64,
) -> Result<Vec<RawRecord>> {
    let table = spec.table.as_deref().unwrap_or("task_usage");
    if table != "task_usage" {
        bail!("Google adapter currently supports table task_usage, got '{table}'");
    }
    let mut reader = csv::ReaderBuilder::new()
        .has_headers(false)
        .flexible(false)
        .from_reader(open_reader(path)?);
    let value = google_column(&spec.value_column)?;
    let groups: Vec<usize> = spec
        .group_columns
        .iter()
        .map(|name| google_column(name))
        .collect::<Result<_>>()?;
    let mut out = Vec::new();
    for (row, result) in reader.records().enumerate() {
        let record = result.with_context(|| format!("malformed Google CSV row {}", row + 1))?;
        if record.len() < 20 {
            bail!(
                "Google task_usage row {} has {}, expected 20 fields",
                row + 1,
                record.len()
            );
        }
        let start_ns = number_to_ns(field(&record, 0, row)?, "microseconds")?;
        let end_ns = number_to_ns(field(&record, 1, row)?, "microseconds")?;
        if end_ns < start_ns {
            bail!("Google task_usage row {} ends before it starts", row + 1);
        }
        if !contained(start_ns, end_ns, window_start_ns, window_end_ns) {
            continue;
        }
        let value = parse_value(field(&record, value, row)?)
            .with_context(|| format!("Google row {} value", row + 1))?;
        include(
            RawRecord {
                start_ns,
                end_ns,
                groups: groups
                    .iter()
                    .map(|i| field(&record, *i, row).map(str::to_string))
                    .collect::<Result<_>>()?,
                value,
                source_row: row,
            },
            window_start_ns,
            window_end_ns,
            &mut out,
        );
    }
    Ok(out)
}

fn google_column(name: &str) -> Result<usize> {
    match name {
        "start_time" => Ok(0),
        "end_time" => Ok(1),
        "job_id" => Ok(2),
        "task_index" => Ok(3),
        "machine_id" => Ok(4),
        "cpu_rate" => Ok(5),
        "canonical_memory_usage" => Ok(6),
        "assigned_memory_usage" => Ok(7),
        "unmapped_page_cache" => Ok(8),
        "total_page_cache" => Ok(9),
        "max_memory_usage" => Ok(10),
        "disk_io_time" => Ok(11),
        "local_disk_space_usage" => Ok(12),
        "max_cpu_rate" => Ok(13),
        "max_disk_io_time" => Ok(14),
        "cycles_per_instruction" => Ok(15),
        "memory_accesses_per_instruction" => Ok(16),
        "sample_portion" => Ok(17),
        "aggregation_type" => Ok(18),
        "sampled_cpu_usage" => Ok(19),
        other => bail!("unknown Google task_usage column '{other}'"),
    }
}

fn load_boom(
    spec: &WorkloadSpec,
    path: &Path,
    window_start_ns: i64,
    window_end_ns: i64,
) -> Result<Vec<RawRecord>> {
    let arrow_path: PathBuf = if path.is_dir() {
        path.join("data-00000-of-00001.arrow")
    } else {
        path.to_path_buf()
    };
    let file = File::open(&arrow_path)
        .with_context(|| format!("opening BOOM Arrow file {}", arrow_path.display()))?;
    let frame = match IpcReader::new(file).finish() {
        Ok(frame) => frame,
        Err(file_error) => {
            let stream = File::open(&arrow_path)
                .with_context(|| format!("reopening BOOM Arrow stream {}", arrow_path.display()))?;
            IpcStreamReader::new(stream)
                .finish()
                .with_context(|| format!("reading BOOM Arrow file (file error: {file_error})"))?
        }
    };
    if frame.height() == 0 {
        bail!("BOOM dataset {} has no rows", spec.dataset);
    }
    if frame.height() != 1 {
        bail!(
            "BOOM adapter expects one selected series, but {} contains {} rows",
            spec.dataset,
            frame.height()
        );
    }
    let start = frame
        .column("start")
        .context("BOOM dataset is missing start")?
        .str()
        .context("BOOM start is not string data")?
        .get(0)
        .context("BOOM start is null")?;
    let start_ns = parse_timestamp_ns(start)?;
    let freq = frame
        .column("freq")
        .context("BOOM dataset is missing freq")?
        .str()
        .context("BOOM freq is not string data")?
        .get(0)
        .context("BOOM freq is null")?;
    let period_ns = frequency_ns(freq)?;
    let target = frame
        .column("target")
        .context("BOOM dataset is missing target")?
        .get(0)
        .context("BOOM target is null")?;
    let values = match target {
        AnyValue::List(values) => values,
        other => bail!("BOOM target must be a list, got {other:?}"),
    };
    let values = if let Some(variate) = spec.variate {
        let nested = values
            .iter()
            .map(|v| match v {
                AnyValue::List(series) => Ok(series),
                other => bail!("BOOM multivariate target contains {other:?}, not a list"),
            })
            .collect::<Result<Vec<_>>>()?;
        let series = nested
            .get(variate)
            .ok_or_else(|| anyhow::anyhow!("BOOM variate {variate} is out of range"))?;
        series.clone()
    } else {
        values
    };
    let mut out = Vec::new();
    for (row, value) in values.iter().enumerate() {
        let value = any_value_f64(&value).with_context(|| format!("BOOM target value {row}"))?;
        let timestamp = start_ns
            .checked_add(
                period_ns
                    .checked_mul(row as i64)
                    .context("BOOM timestamp overflow")?,
            )
            .context("BOOM timestamp overflow")?;
        include(
            RawRecord {
                start_ns: timestamp,
                end_ns: timestamp,
                groups: Vec::new(),
                value,
                source_row: row,
            },
            window_start_ns,
            window_end_ns,
            &mut out,
        );
    }
    Ok(out)
}

fn frequency_ns(freq: &str) -> Result<i64> {
    match freq {
        "S" => Ok(1_000_000_000),
        "T" | "min" => Ok(60_000_000_000),
        "H" => Ok(3_600_000_000_000),
        "D" => Ok(86_400_000_000_000),
        other => {
            let split = other
                .find(|c: char| !c.is_ascii_digit())
                .ok_or_else(|| anyhow::anyhow!("unsupported BOOM frequency '{other}'"))?;
            let multiplier: i64 = other[..split]
                .parse()
                .with_context(|| format!("invalid BOOM frequency '{other}'"))?;
            let base = frequency_ns(&other[split..])?;
            base.checked_mul(multiplier)
                .context("BOOM frequency overflows nanoseconds")
        }
    }
}

fn any_value_f64(value: &AnyValue<'_>) -> Result<f64> {
    let value = match value {
        AnyValue::Float64(v) => *v,
        AnyValue::Float32(v) => *v as f64,
        AnyValue::Int64(v) => *v as f64,
        AnyValue::Int32(v) => *v as f64,
        other => bail!("expected numeric BOOM value, got {other:?}"),
    };
    if !value.is_finite() {
        bail!("BOOM value is NaN or infinite");
    }
    Ok(value)
}

fn parse_value(value: &str) -> Result<f64> {
    let value: f64 = value
        .parse()
        .with_context(|| format!("'{value}' is not numeric"))?;
    if !value.is_finite() {
        bail!("value is NaN or infinite");
    }
    Ok(value)
}

fn index(headers: &StringRecord, name: &str) -> Result<usize> {
    headers
        .iter()
        .position(|header| header == name)
        .ok_or_else(|| anyhow::anyhow!("dataset is missing required column '{name}'"))
}

fn field(record: &StringRecord, index: usize, row: usize) -> Result<&str> {
    let value = record
        .get(index)
        .ok_or_else(|| anyhow::anyhow!("row {} is missing field {}", row + 1, index))?;
    if value.is_empty() {
        bail!("row {} field {} is empty", row + 1, index);
    }
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn point_windows_are_half_open() {
        let mut out = Vec::new();
        include(
            RawRecord {
                start_ns: 10,
                end_ns: 10,
                groups: Vec::new(),
                value: 1.0,
                source_row: 0,
            },
            0,
            10,
            &mut out,
        );
        assert!(out.is_empty());
    }

    #[test]
    fn intervals_must_be_completely_contained() {
        let mut out = Vec::new();
        include(
            RawRecord {
                start_ns: 5,
                end_ns: 15,
                groups: Vec::new(),
                value: 1.0,
                source_row: 0,
            },
            0,
            10,
            &mut out,
        );
        assert!(out.is_empty());
    }

    #[test]
    fn timezone_less_timestamps_are_utc() {
        assert_eq!(
            parse_timestamp_ns("1970-01-01T00:00:01").unwrap(),
            NANOS_PER_SECOND
        );
    }

    #[test]
    fn user_csv_spec_preserves_generic_column_mapping() {
        let spec: WorkloadSpec = serde_norway::from_str(
            r#"
source: custom
dataset: tenant/events.csv.gz
mode: grouped
timestamp_column: ts
end_timestamp_column: end_ts
timestamp_unit: milliseconds
group_columns: [service]
value_column: key
window: {start: "0", end: "60000"}
min_records: 100
"#,
        )
        .unwrap();
        assert!(matches!(spec.source, Source::Custom));
        assert_eq!(spec.timestamp_column.as_deref(), Some("ts"));
        assert_eq!(spec.end_timestamp_column.as_deref(), Some("end_ts"));
        assert_eq!(spec.dataset, "tenant/events.csv.gz");
    }
}
