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

    /// Absorb another sketch of the same type and configuration.
    ///
    /// Mergeability is what makes a summary a *sketch* rather than a
    /// stopwatch: it is why one can be computed per shard, per node, or per
    /// time window and combined afterwards. So it belongs here on the base
    /// trait, not in a separate opt-in capability trait — modelling it as
    /// optional would say merging is exotic, and it is the common case.
    ///
    /// The `Result` is **not** about dimension mismatch. The benchmark always
    /// constructs both operands from one `ParamSet`, so mismatched shapes
    /// cannot arise, and a wrapper whose library returns a `Result` for that
    /// reason should `expect()` it. The error exists for the other question:
    /// *this implementation does not provide a merge at all*. Three of this
    /// repo's rows are in that position (Nitro's and UnivMon's wrappers, whose
    /// `query` is already a stub; the parallel-insert rows, which are N
    /// deliberately unmerged shards and are arguably not one sketch). The
    /// default therefore reports unsupported rather than panicking, so a
    /// merge sweep records a row saying so instead of dying — the capability
    /// matrix is itself a result worth publishing.
    fn merge(&mut self, _other: &Self) -> Result<(), MergeUnsupported> {
        Err(MergeUnsupported)
    }

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

/// Returned by [`Sketch::merge`] when an implementation provides no merge.
///
/// Deliberately a unit type with no variants for "shapes differ" or "seeds
/// differ": the benchmark builds both operands from one `ParamSet`, so those
/// cannot happen, and inventing variants for them would suggest the caller
/// has a decision to make where it does not.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MergeUnsupported;

impl std::fmt::Display for MergeUnsupported {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "this implementation provides no merge")
    }
}

impl std::error::Error for MergeUnsupported {}
