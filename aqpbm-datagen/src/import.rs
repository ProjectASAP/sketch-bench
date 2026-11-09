//! Bounded loaders for representative external trace formats.
//!
//! Loading, decompression, validation, ordering, and window selection all
//! happen here, so a consumer sees the same in-memory table shape as a
//! synthetic run.

use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use csv::StringRecord;
use flate2::read::GzDecoder;
use polars::io::ipc::IpcStreamReader;
use polars::prelude::{AnyValue, IpcReader, SerReader};
use serde::Deserialize;

use crate::error::DataGenError;
use crate::table::GeneratedTable;
use crate::value::ColumnData;

const NANOS_PER_SECOND: i64 = 1_000_000_000;

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ExternalSource {
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
pub struct ExternalTableDescription {
    pub source: ExternalSource,
    /// A path relative to the data root `load` is given, retained as the report identity.
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
    #[serde(default)]
    pub window: Option<WindowSpec>,
    #[serde(default = "default_sort_by_start_time")]
    pub sort_by_start_time: bool,
    #[serde(default = "default_min_records")]
    pub min_records: usize,
}

fn default_sort_by_start_time() -> bool {
    true
}

fn default_min_records() -> usize {
    1
}

#[derive(Debug)]
struct RawRecord {
    start_ns: i64,
    end_ns: i64,
    groups: Vec<String>,
    value: f64,
}

impl ExternalTableDescription {
    pub fn from_path(path: &Path) -> Result<Self, DataGenError> {
        let text = std::fs::read_to_string(path).map_err(|e| {
            DataGenError::Import(format!(
                "reading external table description {}: {e}",
                path.display()
            ))
        })?;
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or_default();
        match ext {
            "yaml" | "yml" => serde_norway::from_str(&text).map_err(|e| {
                DataGenError::Import(format!(
                    "parsing external table description YAML {}: {e}",
                    path.display()
                ))
            }),
            _ => serde_json::from_str(&text).map_err(|e| {
                DataGenError::Import(format!(
                    "parsing external table description JSON {}: {e}",
                    path.display()
                ))
            }),
        }
    }

    pub fn window_ns(&self) -> Result<Option<(i64, i64)>, DataGenError> {
        window_ns(self).map_err(import_error)
    }

    pub fn load(&self, data_root: &Path) -> Result<Option<GeneratedTable>, DataGenError> {
        load(self, data_root).map_err(import_error)
    }

    pub fn source_name(&self) -> &'static str {
        match self.source {
            ExternalSource::Boom => "boom",
            ExternalSource::Google => "google",
            ExternalSource::Alibaba => "alibaba",
            ExternalSource::Custom => "custom",
        }
    }

    pub fn mode_name(&self) -> &'static str {
        match self.mode {
            InputMode::Scalar => "scalar",
            InputMode::Grouped => "grouped",
            InputMode::Keyed => "keyed",
        }
    }

    pub fn group_and_value_positions(&self) -> (Vec<usize>, usize) {
        let first_group = 1 + usize::from(self.time_columns().1.is_some());
        let value = first_group + self.group_columns.len();
        ((first_group..value).collect(), value)
    }

    fn time_columns(&self) -> (String, Option<String>) {
        match &self.source {
            ExternalSource::Boom | ExternalSource::Alibaba => ("timestamp".to_string(), None),
            ExternalSource::Google => ("start_time".to_string(), Some("end_time".to_string())),
            ExternalSource::Custom => (
                self.timestamp_column.clone().unwrap_or_default(),
                self.end_timestamp_column.clone(),
            ),
        }
    }

    pub fn source_timestamp_unit(&self) -> String {
        match &self.source {
            ExternalSource::Boom => "frequency-derived".to_string(),
            ExternalSource::Google => "microseconds".to_string(),
            ExternalSource::Alibaba => self
                .timestamp_unit
                .clone()
                .unwrap_or_else(|| "milliseconds".to_string()),
            ExternalSource::Custom => self
                .timestamp_unit
                .clone()
                .unwrap_or_else(|| "nanoseconds".to_string()),
        }
    }
}

fn import_error(e: anyhow::Error) -> DataGenError {
    DataGenError::Import(format!("{e:#}"))
}

fn window_ns(spec: &ExternalTableDescription) -> Result<Option<(i64, i64)>> {
    let Some(window) = &spec.window else {
        return Ok(None);
    };
    let window_start_ns = parse_timestamp_ns(&window.start)
        .with_context(|| format!("invalid window start '{}'", window.start))?;
    let window_end_ns = parse_timestamp_ns(&window.end)
        .with_context(|| format!("invalid window end '{}'", window.end))?;
    if window_start_ns >= window_end_ns {
        bail!("window start must be before window end");
    }
    Ok(Some((window_start_ns, window_end_ns)))
}

fn load(spec: &ExternalTableDescription, data_root: &Path) -> Result<Option<GeneratedTable>> {
    if spec.dataset.is_empty() {
        bail!("external table dataset cannot be empty");
    }
    if spec.min_records == 0 {
        bail!("min_records must be greater than zero");
    }
    let logical = Path::new(&spec.dataset);
    if logical.is_absolute() {
        bail!(
            "external table dataset must be relative to the data root: {}",
            spec.dataset
        );
    }
    if spec.mode == InputMode::Keyed {
        bail!("keyed external tables are tracked in issue #122 and are not implemented yet");
    }
    if !spec.key_columns.is_empty() {
        bail!("key_columns are only valid for keyed workloads (tracked in issue #122)");
    }
    if spec.mode == InputMode::Grouped && spec.group_columns.is_empty() {
        bail!("grouped external tables require at least one group column");
    }
    if spec.mode == InputMode::Scalar && !spec.group_columns.is_empty() {
        bail!("scalar external tables cannot specify group_columns");
    }
    if matches!(spec.source, ExternalSource::Custom) && spec.timestamp_column.is_none() {
        bail!("custom workloads require timestamp_column");
    }

    let window = window_ns(spec)?;
    let path = data_root.join(logical);
    let mut records = match &spec.source {
        ExternalSource::Boom => load_boom(spec, &path, window)?,
        ExternalSource::Google => load_google(spec, &path, window)?,
        ExternalSource::Alibaba => load_alibaba(spec, &path, window)?,
        ExternalSource::Custom => load_custom(spec, &path, window)?,
    };
    if spec.sort_by_start_time {
        records.sort_by_key(|r| r.start_ns);
    }
    if records.len() < spec.min_records {
        if records.is_empty() {
            return Ok(None);
        }
        bail!(
            "external table holds {} records, below min_records {}",
            records.len(),
            spec.min_records
        );
    }

    materialise(&records, spec).map(Some)
}

fn materialise(records: &[RawRecord], spec: &ExternalTableDescription) -> Result<GeneratedTable> {
    let (start, end) = spec.time_columns();
    let mut labels = vec![start];
    let mut data = vec![ColumnData::Int64(
        records.iter().map(|r| r.start_ns).collect(),
    )];
    if let Some(end) = end {
        labels.push(end);
        data.push(ColumnData::Int64(
            records.iter().map(|r| r.end_ns).collect(),
        ));
    }
    for (i, column) in spec.group_columns.iter().enumerate() {
        labels.push(column.clone());
        data.push(ColumnData::String(
            records.iter().map(|r| r.groups[i].clone()).collect(),
        ));
    }
    labels.push(spec.value_column.clone());
    data.push(ColumnData::Float64(
        records.iter().map(|r| r.value).collect(),
    ));
    GeneratedTable::new(labels, data)
        .map_err(|e| anyhow::anyhow!("materialised external table is invalid: {e}"))
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

fn in_window(record_start: i64, record_end: i64, window: Option<(i64, i64)>) -> bool {
    window.is_none_or(|(start, end)| contained(record_start, record_end, start, end))
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
    spec: &ExternalTableDescription,
    path: &Path,
    window: Option<(i64, i64)>,
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
        if !in_window(start_ns, start_ns, window) {
            continue;
        }
        let value = parse_value(field(&record, value, row)?)
            .with_context(|| format!("Alibaba row {} value", row + 2))?;
        out.push(RawRecord {
            start_ns,
            end_ns: start_ns,
            groups: groups
                .iter()
                .map(|i| field(&record, *i, row).map(str::to_string))
                .collect::<Result<_>>()?,
            value,
        });
    }
    Ok(out)
}

fn load_custom(
    spec: &ExternalTableDescription,
    path: &Path,
    window: Option<(i64, i64)>,
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
        if !in_window(start_ns, end_ns, window) {
            continue;
        }
        out.push(RawRecord {
            start_ns,
            end_ns,
            groups: groups
                .iter()
                .map(|column| field(&record, *column, row).map(str::to_string))
                .collect::<Result<_>>()?,
            value,
        });
    }
    Ok(out)
}

fn load_google(
    spec: &ExternalTableDescription,
    path: &Path,
    window: Option<(i64, i64)>,
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
        if !in_window(start_ns, end_ns, window) {
            continue;
        }
        let value = parse_value(field(&record, value, row)?)
            .with_context(|| format!("Google row {} value", row + 1))?;
        out.push(RawRecord {
            start_ns,
            end_ns,
            groups: groups
                .iter()
                .map(|i| field(&record, *i, row).map(str::to_string))
                .collect::<Result<_>>()?,
            value,
        });
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
    spec: &ExternalTableDescription,
    path: &Path,
    window: Option<(i64, i64)>,
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
        if !in_window(timestamp, timestamp, window) {
            continue;
        }
        out.push(RawRecord {
            start_ns: timestamp,
            end_ns: timestamp,
            groups: Vec::new(),
            value,
        });
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
        assert!(!in_window(10, 10, Some((0, 10))));
    }

    #[test]
    fn intervals_must_be_completely_contained() {
        assert!(!in_window(5, 15, Some((0, 10))));
    }

    #[test]
    fn timezone_less_timestamps_are_utc() {
        assert_eq!(
            parse_timestamp_ns("1970-01-01T00:00:01").unwrap(),
            NANOS_PER_SECOND
        );
    }

    fn custom_table(name: &str, options: &str) -> (ExternalTableDescription, GeneratedTable) {
        let root = std::env::temp_dir().join(format!("aqpbm-import-{}-{name}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(
            root.join("events.csv"),
            "ts,end_ts,service,latency\n30,40,b,2.0\n10,10,a,1.0\n90,95,a,3.0\n",
        )
        .unwrap();
        let spec: ExternalTableDescription = serde_norway::from_str(&format!(
            r#"
source: custom
dataset: events.csv
mode: grouped
timestamp_column: ts
timestamp_unit: nanoseconds
group_columns: [service]
value_column: latency
{options}
"#
        ))
        .unwrap();
        let table = spec.load(&root).unwrap().unwrap();
        std::fs::remove_dir_all(&root).unwrap();
        (spec, table)
    }

    fn position_titles(
        spec: &ExternalTableDescription,
        table: &GeneratedTable,
    ) -> (Vec<String>, String) {
        let (groups, value) = spec.group_and_value_positions();
        (
            groups
                .iter()
                .map(|&i| table.column_title[i].clone())
                .collect(),
            table.column_title[value].clone(),
        )
    }

    fn start_times(table: &GeneratedTable) -> &ColumnData {
        table.column(0).unwrap()
    }

    const WINDOW: &str = r#"window: {start: "0", end: "50"}"#;

    #[test]
    fn point_records_are_sorted_by_start_time_by_default() {
        let (spec, table) = custom_table("sorted", WINDOW);
        assert_eq!(table.column_title, ["ts", "service", "latency"]);
        assert_eq!(
            position_titles(&spec, &table),
            (vec!["service".to_string()], "latency".to_string())
        );
        assert_eq!(start_times(&table), &ColumnData::Int64(vec![10, 30]));
    }

    #[test]
    fn file_order_is_kept_when_sorting_is_off() {
        let (_, table) = custom_table(
            "file-order",
            &format!("{WINDOW}\nsort_by_start_time: false"),
        );
        assert_eq!(start_times(&table), &ColumnData::Int64(vec![30, 10]));
    }

    #[test]
    fn without_a_window_every_record_is_kept() {
        let (_, table) = custom_table("no-window", "sort_by_start_time: false");
        assert_eq!(start_times(&table), &ColumnData::Int64(vec![30, 10, 90]));
    }

    #[test]
    fn interval_records_keep_both_ends() {
        let (spec, table) = custom_table(
            "intervals",
            &format!("{WINDOW}\nend_timestamp_column: end_ts"),
        );
        assert_eq!(table.column_title, ["ts", "end_ts", "service", "latency"]);
        assert_eq!(
            position_titles(&spec, &table),
            (vec!["service".to_string()], "latency".to_string())
        );
        assert_eq!(table.column(1).unwrap(), &ColumnData::Int64(vec![10, 40]));
    }
}
