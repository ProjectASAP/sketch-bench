//! [`Accumulator`] — feed it items, it accumulates state. That is all this
//! trait claims; an exact `HashMap` counter satisfies it as fully as an HLL
//! does. Answering a statistic, reporting a footprint — those are capabilities
//! declared by name elsewhere (`accuracy::CardinalityOps` and friends,
//! `memory_footprint::MemoryFootprint`). See `docs/DESIGN.md` §4.1.

/// Feed it items; it accumulates. `update` takes `&Item` so a wrapper can
/// carry strings and byte-slices without cloning on the hot path.
pub trait Accumulator {
    /// What `update` ingests. Any owned type: primitive, `String`,
    /// `Vec<u8>`, a custom struct.
    type Item;

    /// Ingest one item.
    fn update(&mut self, v: &Self::Item);

    /// Absorb another built from the same `ParamSet`. `Err` means this
    /// implementation has no merge at all — shapes cannot mismatch here, so a
    /// library returning `Result` for that reason should `expect()` it.
    fn merge(&mut self, _other: &Self) -> Result<(), MergeUnsupported> {
        Err(MergeUnsupported)
    }

    /// Optional: no more items are coming. Called once after the last
    /// `update`. Timed separately, so an implementation that defers its work
    /// is not credited with a fast insert loop.
    fn prepare(&mut self) {}
}

/// Returned by [`Accumulator::merge`] when an implementation provides none.
/// A unit type on purpose: the caller has no decision to make.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MergeUnsupported;

impl std::fmt::Display for MergeUnsupported {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "this implementation provides no merge")
    }
}

impl std::error::Error for MergeUnsupported {}
