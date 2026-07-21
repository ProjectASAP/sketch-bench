//! On-disk output for generated workloads: a raw little-endian value
//! stream (`.bin`) plus a JSON provenance sidecar.
//!
//! The `.bin` layout is deliberately header-less so `i64` output stays
//! byte-compatible with the pre-existing `input/benchmark_data_*.bin`
//! files and both readers (`crate::workload::FileI64` and the C++
//! `cpp-bench` loader). All provenance — dtype included — lives in the
//! sidecar, which those readers ignore.

use std::path::{Path, PathBuf};

use crate::error::SketchCoreError;

use super::{Column, GenMeta};

/// Sidecar path for a `.bin` output: `foo.bin` -> `foo.bin.meta.json`.
///
/// The full filename (extension included) is preserved so the sidecar
/// is unambiguous even for non-`.bin` output names.
pub fn sidecar_path(bin_path: &Path) -> PathBuf {
    let mut name = bin_path.as_os_str().to_os_string();
    name.push(".meta.json");
    PathBuf::from(name)
}

/// Write an already-materialised column as a raw little-endian stream
/// (no header). Streaming generation should go through
/// [`super::FileSink`] instead — this is the one-shot convenience for
/// callers that already hold the whole column.
pub fn write_bin(path: &Path, col: &Column) -> Result<(), SketchCoreError> {
    use super::Sink;
    let mut sink = super::FileSink::create(path)?;
    sink.accept(col)?;
    sink.flush()
}

/// Write the provenance sidecar next to `bin_path`. Returns its path.
pub fn write_meta(bin_path: &Path, meta: &GenMeta) -> Result<PathBuf, SketchCoreError> {
    let path = sidecar_path(bin_path);
    let json = serde_json::to_string_pretty(meta)
        .map_err(|e| SketchCoreError::BadParam(format!("meta serialize: {e}")))?;
    std::fs::write(&path, json)?;
    Ok(path)
}

/// Read the provenance sidecar for `bin_path`, if one exists.
pub fn read_meta(bin_path: &Path) -> Result<Option<GenMeta>, SketchCoreError> {
    let path = sidecar_path(bin_path);
    if !path.exists() {
        return Ok(None);
    }
    let text = std::fs::read_to_string(&path)?;
    let meta = serde_json::from_str(&text)
        .map_err(|e| SketchCoreError::BadParam(format!("meta parse: {e}")))?;
    Ok(Some(meta))
}
