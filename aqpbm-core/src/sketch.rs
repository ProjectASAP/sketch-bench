//! The [`Sketch`] trait — the one abstraction every downstream
//! crate in the sketchlib-tool graph depends on.
//!
//! Deliberately narrow: the **insert** side only, just enough to drive a
//! generic `BenchRunner` and a `Probe` decorator. The zoo of query shapes
//! (HLL's `()→f64`, KLL's `f64→f64`, CMS's `K→u64`, …) belongs to the
//! per-statistic capability traits in `sketch-bench` — `CardinalityOps`,
//! `FrequencyOps`, `QuantileOps`, `TopKOps` — which an impl opts into by name.
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

    /// Best-effort memory footprint in bytes. Implementations
    /// that can't compute this cheaply should return a tight
    /// upper bound; returning `0` is acceptable for families
    /// where memory is fixed at construction and uninteresting
    /// to the bench (the operator can log the constant instead).
    fn memory_bytes(&self) -> usize;

    /// Absorb another sketch of the same type and configuration.
    ///
    /// Mergeability is what makes a summary a *sketch* rather than a
    /// stopwatch — it is why one can be computed per shard, per node, or per
    /// time window and combined afterwards — so it belongs on the base trait.
    /// Modelling it as an opt-in capability would say merging is exotic, and
    /// it is the common case.
    ///
    /// The `Result` is **not** about dimension mismatch: both operands are
    /// always built from one `ParamSet`, so a wrapper whose library returns a
    /// `Result` for that reason should `expect()` it. The error means *this
    /// implementation provides no merge at all* — Nitro and UnivMon (which
    /// declare no query capability either), the parallel-insert rows (N
    /// deliberately unmerged shards, arguably not one sketch), and others.
    /// Reporting unsupported rather than panicking lets a merge sweep record a
    /// row saying so instead of dying; the capability matrix is itself a
    /// result worth publishing.
    fn merge(&mut self, _other: &Self) -> Result<(), MergeUnsupported> {
        Err(MergeUnsupported)
    }

    /// One-shot transition from "ingesting" to "queryable". Called once
    /// between the last `update` and the first query. Default no-op.
    ///
    /// For the maintenance work some sketches do inside `update` (KLL/DD keep
    /// a queryable structure continuously) and others defer (an exact baseline
    /// can buffer raw values and sort at the end). Billing it to the insert
    /// phase is what keeps query-side throughput comparable across families:
    /// every sketch's query path is measured from the same "ready-to-answer"
    /// state.
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
