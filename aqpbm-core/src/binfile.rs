//! The on-disk dataset format: a raw little-endian value stream (`.bin`) plus a
//! JSON provenance sidecar. The `.bin` layout is header-less, which is why the
//! sidecar has to carry the dtype: an `f64` file looks exactly like an `i64` one.

use std::io::Write;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use aqpbm_datagen::{ColumnData, DataGenError, TableDescription};

/// Schema version of the `.meta.json` sidecar. Version 3 carries a
/// [`TableDescription`]; versions 1 and 2 do not deserialize.
pub const BIN_META_SCHEMA_VERSION: u32 = 3;

/// A human-facing summary of a stored column, for `dataset describe` to print.
/// `min`/`max`/`first`/`last` are `f64`, so beyond `2^53` they are approximate —
/// the `.bin` stream holds the exact values.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BasicStats {
    pub count: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub first: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last: Option<f64>,
}

impl BasicStats {
    /// Summarise one column. A string column reports only `count`: a string's
    /// length under a field named `min` would be a lie, and a `.bin` cannot hold
    /// one anyway.
    pub fn of(column: &ColumnData) -> Self {
        fn over<T>(values: &[T], to_f64: impl Fn(&T) -> f64) -> BasicStats {
            let mut stats = BasicStats {
                count: values.len(),
                min: None,
                max: None,
                first: None,
                last: None,
            };
            for v in values {
                let x = to_f64(v);
                stats.min = Some(stats.min.map_or(x, |m: f64| m.min(x)));
                stats.max = Some(stats.max.map_or(x, |m: f64| m.max(x)));
                stats.first.get_or_insert(x);
                stats.last = Some(x);
            }
            stats
        }
        match column {
            ColumnData::Int64(v) => over(v, |x| *x as f64),
            ColumnData::Unsigned64(v) => over(v, |x| *x as f64),
            ColumnData::Float64(v) => over(v, |x| *x),
            ColumnData::String(v) => BasicStats {
                count: v.len(),
                min: None,
                max: None,
                first: None,
                last: None,
            },
        }
    }
}

/// Provenance written beside a `.bin`. Carries the full description, so a stored
/// column is reproducible from the file alone.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BinMeta {
    pub schema_version: u32,
    pub generator_version: String,
    pub dtype: String,
    pub count: usize,
    pub description: TableDescription,
    pub stats: BasicStats,
}

impl BinMeta {
    pub fn new(description: &TableDescription, column: &ColumnData) -> Self {
        Self {
            schema_version: BIN_META_SCHEMA_VERSION,
            generator_version: env!("CARGO_PKG_VERSION").to_string(),
            dtype: column.kind().to_string(),
            count: column.len(),
            description: description.clone(),
            stats: BasicStats::of(column),
        }
    }
}

/// Sidecar path for a `.bin` output: `foo.bin` -> `foo.bin.meta.json`. The full
/// filename is preserved so the sidecar is unambiguous for any output name.
pub fn sidecar_path(bin_path: &Path) -> PathBuf {
    let mut name = bin_path.as_os_str().to_os_string();
    name.push(".meta.json");
    PathBuf::from(name)
}

/// Write one column as a raw little-endian stream, no header.
///
/// Strings are refused rather than encoded: the format is equal-width values
/// with nowhere to record a length, so a `.bin` of them could not be read back.
pub fn write_bin(path: &Path, column: &ColumnData) -> Result<(), DataGenError> {
    let file = std::fs::File::create(path)?;
    let mut w = std::io::BufWriter::new(file);
    match column {
        ColumnData::Int64(v) => {
            for x in v {
                w.write_all(&x.to_le_bytes())?;
            }
        }
        ColumnData::Unsigned64(v) => {
            for x in v {
                w.write_all(&x.to_le_bytes())?;
            }
        }
        ColumnData::Float64(v) => {
            for x in v {
                w.write_all(&x.to_le_bytes())?;
            }
        }
        ColumnData::String(_) => {
            return Err(DataGenError::BadParam(
                "a string column cannot be written to a .bin file: the format has no \
                 length field. Strings generate fine in-process; the file format is \
                 what is missing"
                    .into(),
            ))
        }
    }
    w.flush()?;
    Ok(())
}

/// Write the provenance sidecar next to `bin_path`. Returns its path.
pub fn write_meta(bin_path: &Path, meta: &BinMeta) -> Result<PathBuf, DataGenError> {
    let path = sidecar_path(bin_path);
    let json = serde_json::to_string_pretty(meta)
        .map_err(|e| DataGenError::BadParam(format!("meta serialize: {e}")))?;
    std::fs::write(&path, json)?;
    Ok(path)
}

/// Read the provenance sidecar for `bin_path`, if one exists.
pub fn read_meta(bin_path: &Path) -> Result<Option<BinMeta>, DataGenError> {
    let path = sidecar_path(bin_path);
    if !path.exists() {
        return Ok(None);
    }
    let text = std::fs::read_to_string(&path)?;
    let meta = serde_json::from_str(&text)
        .map_err(|e| DataGenError::BadParam(format!("meta parse: {e}")))?;
    Ok(Some(meta))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_numeric_summary_reports_extremes_and_ends() {
        let s = BasicStats::of(&ColumnData::Int64(vec![3, 1, 4, 1, 5]));
        assert_eq!(s.count, 5);
        assert_eq!((s.min, s.max), (Some(1.0), Some(5.0)));
        assert_eq!((s.first, s.last), (Some(3.0), Some(5.0)));
    }

    /// A string's length is not its value, so the fields that would describe a
    /// value stay absent rather than describing something else.
    #[test]
    fn a_string_summary_reports_only_its_count() {
        let s = BasicStats::of(&ColumnData::String(vec!["ab".into(), "cde".into()]));
        assert_eq!(s.count, 2);
        assert!(s.min.is_none() && s.max.is_none() && s.first.is_none() && s.last.is_none());
    }

    #[test]
    fn an_empty_column_has_no_extremes() {
        let s = BasicStats::of(&ColumnData::Float64(vec![]));
        assert_eq!(s.count, 0);
        assert!(s.min.is_none() && s.max.is_none());
    }

    /// The `.bin` payload is a bare little-endian stream with no framing, so a
    /// variable-width item has nowhere to record where one value ends.
    #[test]
    fn a_string_column_cannot_be_written_to_a_bin() {
        use crate::test_support::zipf_column;
        use aqpbm_datagen::TableDescription;

        let spec = TableDescription::single("key", zipf_column(64, 1.0, 1, "string"), 8);
        let column = spec.generate().unwrap().into_column(0).unwrap();
        let path = std::env::temp_dir().join("sketchlib_string_bin.bin");
        let err = write_bin(&path, &column).unwrap_err().to_string();
        assert!(err.contains("length field"), "{err}");
        std::fs::remove_file(&path).ok();
    }
}
