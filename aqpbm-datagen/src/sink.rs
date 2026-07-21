//! The destination axis: where generated values go.
//!
//! Generation has three separable concerns — *what* the values mean
//! ([`super::shape::Shape`]), *how* they are spread
//! ([`super::dist::Distribution`]), and *where* they end up. The first
//! two were already orthogonal; this module makes the third one an
//! interface too, so a new destination never touches the generators and
//! a new distribution never touches the destinations.
//!
//! The driver ([`super::GenSpec::generate_into`]) pushes the column to
//! the sink in chunks. That is what lets [`FileSink`] write a dataset
//! larger than RAM in constant memory; [`MemorySink`] concatenates the
//! chunks back into one [`Column`] for callers (the benchmark) that
//! need the whole stream resident.
//!
//! Chunking is an implementation detail of the *transport*, never of
//! the *data*: a fixed `(spec, seed, size)` yields byte-identical
//! output at every chunk size. `chunk_size_does_not_change_output`
//! pins that down.

use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::Path;

use crate::error::SketchError;

use super::Column;

/// A destination for generated chunks.
///
/// Implementors see the column in emission order and must not reorder
/// it. `flush` is called exactly once, after the last chunk.
pub trait Sink {
    /// Accept the next chunk of generated values.
    fn accept(&mut self, chunk: &Column) -> Result<(), SketchError>;

    /// Finalise the destination. Default no-op; override for sinks
    /// holding a buffered resource.
    fn flush(&mut self) -> Result<(), SketchError> {
        Ok(())
    }
}

/// Accumulates the whole stream in memory as one [`Column`].
///
/// Used by `sketchlib bench`, whose runner replays the same item slice
/// once per measured run and therefore needs it resident.
#[derive(Debug, Default)]
pub struct MemorySink {
    column: Option<Column>,
}

impl MemorySink {
    pub fn new() -> Self {
        Self { column: None }
    }

    /// The accumulated column. Empty generation yields an empty column
    /// of the generator's dtype only if at least one chunk arrived;
    /// with zero chunks there is nothing to infer a dtype from, so this
    /// returns `None`.
    pub fn into_column(self) -> Option<Column> {
        self.column
    }
}

impl Sink for MemorySink {
    fn accept(&mut self, chunk: &Column) -> Result<(), SketchError> {
        match (&mut self.column, chunk) {
            (None, c) => self.column = Some(c.clone()),
            (Some(Column::I64(dst)), Column::I64(src)) => dst.extend_from_slice(src),
            (Some(Column::U64(dst)), Column::U64(src)) => dst.extend_from_slice(src),
            (Some(Column::F64(dst)), Column::F64(src)) => dst.extend_from_slice(src),
            (Some(dst), src) => {
                // A generator switching dtype mid-stream is a bug in the
                // generator, not bad user input — but silently producing
                // a column of mixed provenance would be worse.
                return Err(SketchError::BadParam(format!(
                    "sink: chunk dtype {} does not match stream dtype {}",
                    src.dtype().as_str(),
                    dst.dtype().as_str(),
                )));
            }
        }
        Ok(())
    }
}

/// Streams chunks to a header-less little-endian `.bin` file.
///
/// Memory is bounded by the chunk size, not the dataset size, so this
/// can write a file far larger than RAM. The provenance sidecar is not
/// written here — it is the caller's choice (`--no-meta`), and the
/// caller gets the [`super::GenMeta`] back from the driver.
pub struct FileSink {
    writer: BufWriter<File>,
}

impl FileSink {
    pub fn create(path: &Path) -> Result<Self, SketchError> {
        Ok(Self {
            writer: BufWriter::new(File::create(path)?),
        })
    }
}

impl Sink for FileSink {
    fn accept(&mut self, chunk: &Column) -> Result<(), SketchError> {
        chunk.write_le(&mut self.writer)?;
        Ok(())
    }

    fn flush(&mut self) -> Result<(), SketchError> {
        self.writer.flush()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn memory_sink_concatenates_in_order() {
        let mut s = MemorySink::new();
        s.accept(&Column::I64(vec![1, 2])).unwrap();
        s.accept(&Column::I64(vec![3])).unwrap();
        assert_eq!(s.into_column(), Some(Column::I64(vec![1, 2, 3])));
    }

    #[test]
    fn memory_sink_rejects_dtype_switch_mid_stream() {
        let mut s = MemorySink::new();
        s.accept(&Column::I64(vec![1])).unwrap();
        let err = s.accept(&Column::F64(vec![1.0])).unwrap_err();
        assert!(err.to_string().contains("dtype"));
    }
}
