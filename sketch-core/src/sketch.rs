//! The [`Sketch`] trait — the one abstraction every downstream
//! crate in the sketchlib-tool graph depends on.
//!
//! Deliberately narrow: just enough to drive a generic
//! `BenchRunner` and a `Probe` decorator. Each wrapped impl
//! defines its own `Item`/`Query`/`Answer` types so we can cover
//! the zoo of sketch APIs (HLL's `()→f64` cardinality, KLL's
//! `f64→f64` quantile, CMS's `K→u64` point-lookup, ...) without
//! one trait per family.
//!
//! See `docs/DESIGN.md` §4.1.

/// Minimal contract every benchable sketch satisfies.
///
/// Owning-input shape: `update` takes `&Self::Item` so wrappers
/// can carry strings / byte-slices without forcing a clone on
/// the hot path. The workload owns the items and hands refs to
/// the probe.
pub trait Sketch {
    /// The item type ingested by `update`. Can be any owned
    /// type (primitive, `String`, `Vec<u8>`, custom struct).
    type Item;

    /// The query argument type. `()` for sketches that answer a
    /// single global question (HLL cardinality, UnivMon
    /// entropy), concrete values for point-lookups
    /// (`Item` for CMS), `f64` quantile for KLL, etc.
    type Query;

    /// The answer type returned by `query`. `f64` is idiomatic
    /// for most sketches — count estimates, quantiles,
    /// cardinality all float back. Wrappers may pick a richer
    /// type where needed (`Vec<(Item, u64)>` for top-k).
    type Answer;

    /// Ingest a single item.
    fn update(&mut self, v: &Self::Item);

    /// Ingest many items. Default falls back to one-by-one
    /// `update`. Override for sketches with a cheaper batch
    /// path (e.g. Nitro sampling).
    fn bulk_update(&mut self, vs: &[Self::Item]) {
        for v in vs {
            self.update(v);
        }
    }

    /// Answer a query.
    fn query(&self, q: Self::Query) -> Self::Answer;

    /// Best-effort memory footprint in bytes. Implementations
    /// that can't compute this cheaply should return a tight
    /// upper bound; returning `0` is acceptable for families
    /// where memory is fixed at construction and uninteresting
    /// to the bench (the operator can log the constant instead).
    fn memory_bytes(&self) -> usize;

    /// One-shot transition from "ingesting" to "queryable".
    /// Called once between the last `update` and the first
    /// `query`. Default no-op.
    ///
    /// Use this for any maintenance work that some sketches do
    /// inside `update` (KLL/DD continuously maintain a queryable
    /// structure) but other sketches defer (the exact baseline
    /// can buffer raw values and sort once at the end). Doing the
    /// work here, billed to the insert phase, keeps query-side
    /// throughput numbers comparable across families: every
    /// sketch's `query` is measured starting from the same
    /// "ready-to-answer" state.
    fn finalize_for_query(&mut self) {}
}
