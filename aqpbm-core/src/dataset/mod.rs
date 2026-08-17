//! What a measurement iterates, and where it came from: the adapter from
//! `aqpbm-datagen`'s columns to the one contiguous `&[Item]` the timed loop
//! replays. One type per *item* type, not per source; provenance is data.

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
/// [`items`](Self::items) hands back a slice because it is read inside the clock.
pub trait Dataset: Sized {
    type Item: Clone;
    fn description(&self) -> DatasetDescription;
    fn items(&self) -> &[Self::Item];
}
