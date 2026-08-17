//! What a measurement iterates, and where it came from.
//!
//! `aqpbm-datagen` produces *columns* and writes no files. The timed loop wants
//! the opposite: one contiguous `&[Item]` at the row's own item type, replayed
//! whole by every run. This module is that adapter, and the five jobs it does
//! are one file each:
//!
//! - [`description`] — the provenance blob that lands in every JSONL record.
//! - [`numeric`] — the plain item stream, `i64` or `f64`.
//! - [`labeled`] — the multi-column stream, as `label;label` + value records.
//! - [`load`] — replaying a dataset off disk, which datagen deliberately does
//!   not do.
//! - [`spec`] — how a row says which of the above it wants, and at what item
//!   type.
//!
//! One type per *item* type, not per source: provenance is data, not a type
//! parameter, so it lives in `description` and the source picks a constructor.

pub mod description;
pub mod labeled;
pub mod load;
pub mod numeric;
pub mod spec;

pub use description::DatasetDescription;
pub use labeled::{Labeled, LabeledDataset};
pub use numeric::{F64Dataset, I64Dataset, NumericDataset};
pub use spec::{BenchItem, DatasetData, DatasetSpec};

/// The abstract contract for a dataset a measurement can consume: an ordered
/// item stream, plus the provenance that names it in the record.
///
/// [`items`](Self::items) is read once per run and iterated inside the timed
/// region, so it hands back a slice rather than an iterator.
/// [`description`](Self::description) is read once per measurement, on the way
/// out to the report.
pub trait Dataset: Sized {
    type Item: Clone;
    fn description(&self) -> DatasetDescription;
    fn items(&self) -> &[Self::Item];
}
