use thiserror::Error;

/// Why a description cannot be honoured. Named for this crate's job — data
/// generation — because nothing here knows what a sketch is.
#[derive(Debug, Error)]
pub enum DataGenError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    /// A description that contradicts itself, or one this crate cannot serve.
    /// Every check that fires does so before the first draw.
    #[error("bad parameter: {0}")]
    BadParam(String),
    /// A column asked for as one type and peeled as another.
    #[error("column holds {held}, but {wanted} was asked for")]
    TypeMismatch {
        held: &'static str,
        wanted: &'static str,
    },
}
