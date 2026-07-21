//! The destination axis: where generated values go.
//!
//! Generation has three separable concerns — *what* the values mean
//! ([`super::shape::Shape`]), *how* they are spread
//! ([`super::dist::Distribution`]), and *where* they end up. The first
//! two were already orthogonal; this module makes the third one an
//! interface too, so a new destination never touches the generators and
//! a new distribution never touches the destinations.
//!
//! The driver ([`super::GenSpec::generate_into`]) pushes values to the sink
//! in chunks. That is what lets [`BinSink`] write a dataset larger than RAM
//! in constant memory; [`MemorySink`] concatenates the chunks back into one
//! `Vec<T>` for callers (the benchmark) that need the whole stream
//! resident.
//!
//! Chunking is an implementation detail of the *transport*, never of
//! the *data*: a fixed `(spec, seed, size)` yields byte-identical
//! output at every chunk size. `chunk_size_does_not_change_output`
//! pins that down.

use std::fs::File;
use std::io::{BufWriter, Write};
use std::marker::PhantomData;
use std::path::Path;

use crate::error::SketchError;

use super::FixedWidth;

/// A destination for generated chunks.
///
/// Generic over the value type rather than taking a run-time-tagged column.
/// Two things fall out of that. A sink can no longer be handed a chunk of
/// the wrong type, so the "generator switched dtype mid-stream" error this
/// module used to carry is gone — not handled, *unrepresentable*. And a
/// sink is free to accept only some types: see [`BinSink`].
///
/// Implementors see values in emission order and must not reorder them.
/// `flush` is called exactly once, after the last chunk.
pub trait Sink<T> {
    /// Accept the next chunk of generated values.
    fn accept(&mut self, chunk: &[T]) -> Result<(), SketchError>;

    /// Finalise the destination. Default no-op; override for sinks
    /// holding a buffered resource.
    fn flush(&mut self) -> Result<(), SketchError> {
        Ok(())
    }
}

/// Accumulates the whole stream in memory.
///
/// Used by `sketchlib bench`, whose runner replays the same item slice
/// once per measured run and therefore needs it resident.
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

    /// The accumulated values.
    ///
    /// Infallible, unlike the `Option<Column>` this replaced: an empty run
    /// yields an empty `Vec<T>` whose type is known statically, so there is
    /// no longer a case where the dtype cannot be inferred because no chunk
    /// arrived.
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

/// Streams chunks to a header-less little-endian `.bin` file.
///
/// Memory is bounded by the chunk size, not the dataset size, so this can
/// write a file far larger than RAM. The provenance sidecar is not written
/// here — it is the caller's choice (`--no-meta`), and the caller gets the
/// [`super::GenMeta`] back from the driver.
///
/// Accepts only [`FixedWidth`] values. The `.bin` layout is a bare sequence
/// of equal-width little-endian values with no header, offsets, or
/// delimiters, so a variable-width value has nowhere to record its length.
/// That is a property of this format, not of the generator: a string
/// workload is fine in memory or in CSV, and `BinSink::<String>` simply
/// does not compile.
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

    /// The old `memory_sink_rejects_dtype_switch_mid_stream` has no successor
    /// on purpose: `Sink<T>` cannot be handed a chunk of another type, so
    /// there is no run-time state left to check.
    #[test]
    fn a_sink_is_typed_so_an_empty_run_still_has_a_type() {
        let s = MemorySink::<f64>::new();
        assert!(s.into_values().is_empty());
    }
}
