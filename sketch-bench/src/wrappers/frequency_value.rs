//! What a frequency (count) sketch can ingest.
//!
//! Its own module because `cms`, `cs` and `cms_heap` all need it and none owns
//! it: the constraint is "a value this benchmark can count occurrences of",
//! which is a property of the item type, not of any one algorithm. Sibling of
//! [`crate::wrappers::quantile_value`], which exists for the same reason.

use asap_sketchlib::DataInput;
use std::hash::Hash;

/// A value a frequency sketch can be keyed by.
///
/// Two routes into the wrapped libraries, because they take two shapes:
/// `sketch_oxide` and `datasketches` hash whatever implements [`Hash`], while
/// `asap_sketchlib` takes its own [`DataInput`] enum. Both are on this trait so
/// no wrapper has to match on a width.
///
/// All four widths the generator renders are here. Unlike the ordered rows,
/// nothing about counting needs an order, so `string` is in — and it is the
/// width where the fast/regular hashing paths actually diverge, since key
/// length is what a hashing insert pays for.
pub trait FrequencyValue: Clone + Sync + 'static {
    /// The borrowed form handed to a library that hashes. Borrowed so a
    /// `String` key is fed as `&str`: a clone per insert would be paid inside
    /// the region the insert throughput is measured over.
    type HashKey<'a>: Hash
    where
        Self: 'a;

    fn hash_key(&self) -> Self::HashKey<'_>;

    /// The same value in `asap_sketchlib`'s vocabulary.
    fn data_input(&self) -> DataInput<'_>;

    /// Read a key back out of a sketch that stores them — the `CMSHeap` top-k
    /// dump is the one place that happens. A variant this type did not put in
    /// is a library bug, not a caller error, so implementations panic rather
    /// than widen the return type: every row feeds one width and reads the
    /// same one back.
    fn from_data_input(input: &DataInput) -> Self;
}

impl FrequencyValue for i64 {
    type HashKey<'a> = i64;

    #[inline(always)]
    fn hash_key(&self) -> i64 {
        *self
    }

    #[inline(always)]
    fn data_input(&self) -> DataInput<'_> {
        DataInput::I64(*self)
    }

    fn from_data_input(input: &DataInput) -> Self {
        match input {
            DataInput::I64(v) => *v,
            other => panic!("an i64 row read back {other:?}"),
        }
    }
}

impl FrequencyValue for u64 {
    type HashKey<'a> = u64;

    #[inline(always)]
    fn hash_key(&self) -> u64 {
        *self
    }

    #[inline(always)]
    fn data_input(&self) -> DataInput<'_> {
        DataInput::U64(*self)
    }

    fn from_data_input(input: &DataInput) -> Self {
        match input {
            DataInput::U64(v) => *v,
            other => panic!("a u64 row read back {other:?}"),
        }
    }
}

/// `f64` is not [`Hash`], so it is keyed by its bit pattern — the same choice
/// [`crate::wrappers::hll::CardinalityValue`] makes, and the same one
/// `aqpbm_core`'s exact counter makes on the scoring side. Two floats that
/// compare equal but differ in bits (`0.0` and `-0.0`) are two keys, and `NaN`
/// is equal to itself. Both sides have to agree or the reported error is noise.
impl FrequencyValue for f64 {
    type HashKey<'a> = u64;

    #[inline(always)]
    fn hash_key(&self) -> u64 {
        self.to_bits()
    }

    #[inline(always)]
    fn data_input(&self) -> DataInput<'_> {
        DataInput::F64(*self)
    }

    fn from_data_input(input: &DataInput) -> Self {
        match input {
            DataInput::F64(v) => *v,
            other => panic!("an f64 row read back {other:?}"),
        }
    }
}

impl FrequencyValue for String {
    type HashKey<'a> = &'a str;

    #[inline(always)]
    fn hash_key(&self) -> &str {
        self.as_str()
    }

    #[inline(always)]
    fn data_input(&self) -> DataInput<'_> {
        DataInput::Str(self.as_str())
    }

    fn from_data_input(input: &DataInput) -> Self {
        match input {
            DataInput::Str(v) => (*v).to_string(),
            other => panic!("a string row read back {other:?}"),
        }
    }
}
