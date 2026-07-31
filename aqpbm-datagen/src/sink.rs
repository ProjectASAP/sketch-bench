//! The destination axis: where generated values go — the third separable concern
//! alongside structure and distribution, so a new destination never touches the
//! generators. The driver pushes values in chunks, which lets [`BinSink`] write
//! a dataset larger than RAM while [`MemorySink`] reassembles one `Vec<T>`.
//! Chunking is transport, never data: output is byte-identical at any size.

use std::fs::File;
use std::io::{BufWriter, Write};
use std::marker::PhantomData;
use std::path::Path;

use crate::error::SketchError;

use super::FixedWidth;

/// A destination for generated chunks, generic over the value type: a sink
/// cannot be handed the wrong type, and may accept only some (see [`BinSink`]).
/// Values arrive in emission order; `flush` is called once, after the last.
pub trait Sink<T> {
    /// Accept the next chunk of generated values.
    fn accept(&mut self, chunk: &[T]) -> Result<(), SketchError>;

    /// Finalise the destination. Default no-op; override for sinks
    /// holding a buffered resource.
    fn flush(&mut self) -> Result<(), SketchError> {
        Ok(())
    }
}

/// Accumulates the whole stream in memory — used by `approxbench sketchbench`, whose
/// runner replays the same item slice once per measured run.
#[derive(Debug)]
pub struct MemorySink<T> {
    values: Vec<T>,
}

impl<T> Default for MemorySink<T> {
    fn default() -> Self {
        Self { values: Vec::new() }
    }
}

impl<T> MemorySink<T> {
    pub fn new() -> Self {
        Self::default()
    }

    /// The accumulated values. Infallible: an empty run yields an empty `Vec<T>`
    /// whose type is known statically, so no chunk needs to have arrived.
    pub fn into_values(self) -> Vec<T> {
        self.values
    }
}

impl<T: Clone> Sink<T> for MemorySink<T> {
    fn accept(&mut self, chunk: &[T]) -> Result<(), SketchError> {
        self.values.extend_from_slice(chunk);
        Ok(())
    }
}

/// Streams chunks to a header-less little-endian `.bin` file, in memory bounded
/// by the chunk size rather than the dataset. Accepts only [`FixedWidth`] values
/// — with no header or delimiters, a variable-width value cannot record a length.
pub struct BinSink<T> {
    writer: BufWriter<File>,
    _item: PhantomData<T>,
}

impl<T: FixedWidth> BinSink<T> {
    pub fn create(path: &Path) -> Result<Self, SketchError> {
        Ok(Self {
            writer: BufWriter::new(File::create(path)?),
            _item: PhantomData,
        })
    }
}

impl<T: FixedWidth> Sink<T> for BinSink<T> {
    fn accept(&mut self, chunk: &[T]) -> Result<(), SketchError> {
        for v in chunk {
            v.write_le(&mut self.writer)?;
        }
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
        let mut s = MemorySink::<i64>::new();
        s.accept(&[1, 2]).unwrap();
        s.accept(&[3]).unwrap();
        assert_eq!(s.into_values(), vec![1, 2, 3]);
    }

    /// There is deliberately no "sink rejects a dtype switch mid-stream" test:
    /// `Sink<T>` cannot be handed a chunk of another type, so there is no
    /// run-time state left to check.
    #[test]
    fn a_sink_is_typed_so_an_empty_run_still_has_a_type() {
        let s = MemorySink::<f64>::new();
        assert!(s.into_values().is_empty());
    }
}
