//! [`MemoryFootprint`] — how much space the accumulated state takes.
//!
//! Its own trait rather than a method on `Accumulator`, because a footprint is
//! a *reported* number, not something needed to run. The schema already calls
//! it optional (`Record::memory_bytes: Option<u64>`); the trait should agree.

/// Reports the size of what it has accumulated.
pub trait MemoryFootprint {
    /// Best-effort bytes; a tight upper bound is fine. Implement it only
    /// where the number means something.
    fn memory_bytes(&self) -> usize;
}
