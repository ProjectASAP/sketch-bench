use thiserror::Error;

#[derive(Debug, Error)]
pub enum SketchCoreError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("bad parameter: {0}")]
    BadParam(String),
    #[error("schema version mismatch: file={file}, expected={expected}")]
    SchemaVersion { file: u32, expected: u32 },
}
