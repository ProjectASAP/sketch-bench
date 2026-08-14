//! The on-disk workload format: a raw little-endian value stream (`.bin`) plus a
//! JSON provenance sidecar.
//!
//! It lives here, beside the readers in [`crate::workload`], rather than in
//! `aqpbm-datagen` — the generator produces values in memory and nothing else,
//! so a file format is this crate's concern. The `.bin` layout is deliberately
//! header-less, which is exactly why the sidecar has to carry the dtype: an
//! `f64` file is byte-indistinguishable from an `i64` one.

use std::io::Write;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use aqpbm_datagen::{BasicStats, ColumnData, DataGenError, TableDescription};

/// Schema version of the `.meta.json` sidecar. Version 3 carries a
/// [`TableDescription`]; versions 1 and 2 carried the retired `Shape` axis and
/// do not deserialize.
pub const BIN_META_SCHEMA_VERSION: u32 = 3;

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
            stats: column.basic_stats(),
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
