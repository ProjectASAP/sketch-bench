//! What a measurement iterates, and where it came from: the adapter from
//! `aqpbm-datagen`'s columns to the one contiguous `&[Item]` the timed loop
//! replays. One type per *item* type, not per source; provenance is data.

pub mod description;
pub mod labeled;
pub mod load;
pub mod numeric;
pub mod spec;

pub use description::InputDataSetDescription;
pub use labeled::Labeled;
pub use spec::{BenchItem, InputDataSetData, InputDataSetSpec};

/// A materialised dataset: the ordered item stream a measurement replays, and
/// the provenance that names it in the record. A pair rather than a type,
/// because nothing here outlives the row that peels it — the items go straight
/// into the closures and the description goes straight into the record.
pub type Materialised<T> = (InputDataSetDescription, Vec<T>);
