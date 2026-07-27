//! On-disk output for generated workloads: a raw little-endian value stream
//! (`.bin`) plus a JSON provenance sidecar. The `.bin` layout is deliberately
//! header-less, so all provenance — dtype included — lives in the sidecar,
//! which the readers ignore.

use std::path::{Path, PathBuf};

use crate::error::SketchError;

use super::{FixedWidth, GenMeta};

/// Sidecar path for a `.bin` output: `foo.bin` -> `foo.bin.meta.json`. The full
/// filename is preserved so the sidecar is unambiguous for any output name.
pub fn sidecar_path(bin_path: &Path) -> PathBuf {
    let mut name = bin_path.as_os_str().to_os_string();
    name.push(".meta.json");
    PathBuf::from(name)
}

/// Write already-materialised values as a raw little-endian stream, no header.
/// The one-shot convenience for callers holding the whole column; streaming
/// generation goes through [`super::BinSink`].
pub fn write_bin<T: FixedWidth>(path: &Path, values: &[T]) -> Result<(), SketchError> {
    use super::Sink;
    let mut sink = super::BinSink::<T>::create(path)?;
    sink.accept(values)?;
    sink.flush()
}

/// Write the provenance sidecar next to `bin_path`. Returns its path.
pub fn write_meta(bin_path: &Path, meta: &GenMeta) -> Result<PathBuf, SketchError> {
    let path = sidecar_path(bin_path);
    let json = serde_json::to_string_pretty(meta)
        .map_err(|e| SketchError::BadParam(format!("meta serialize: {e}")))?;
    std::fs::write(&path, json)?;
    Ok(path)
}

/// Read the provenance sidecar for `bin_path`, if one exists.
pub fn read_meta(bin_path: &Path) -> Result<Option<GenMeta>, SketchError> {
    let path = sidecar_path(bin_path);
    if !path.exists() {
        return Ok(None);
    }
    let text = std::fs::read_to_string(&path)?;
    let meta = serde_json::from_str(&text)
        .map_err(|e| SketchError::BadParam(format!("meta parse: {e}")))?;
    Ok(Some(meta))
}
