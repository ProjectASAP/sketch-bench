//! HyperLogLog wrappers: `oxide`, `datasketches`, `sketchlib`
//! (a.k.a. asap_sketchlib). All of them are registered under
//! `Capability::Cardinality`, which is what makes them cardinality rows.

use crate::params::*;
use crate::wrappers::BuildError;
use ::polars::prelude::Column;
use asap_sketchlib::DataInput;
use sketch_oxide::Mergeable as _;
use std::hash::Hash;

pub trait CardinalityValue: Clone + Sync + 'static {
    type HashKey<'a>: Hash
    where
        Self: 'a;

    fn hash_key(&self) -> Self::HashKey<'_>;

    fn data_input(&self) -> DataInput<'_>;

    fn polars_column(values: &[Self]) -> Column;

    fn heap_bytes(&self) -> usize;
}

impl CardinalityValue for i64 {
    type HashKey<'a> = i64;

    #[inline(always)]
    fn hash_key(&self) -> i64 {
        *self
    }

    #[inline(always)]
    fn data_input(&self) -> DataInput<'_> {
        DataInput::I64(*self)
    }

    fn polars_column(values: &[Self]) -> Column {
        Column::new("v".into(), values)
    }

    #[inline(always)]
    fn heap_bytes(&self) -> usize {
        0
    }
}

impl CardinalityValue for u64 {
    type HashKey<'a> = u64;

    #[inline(always)]
    fn hash_key(&self) -> u64 {
        *self
    }

    #[inline(always)]
    fn data_input(&self) -> DataInput<'_> {
        DataInput::U64(*self)
    }

    fn polars_column(values: &[Self]) -> Column {
        Column::new("v".into(), values)
    }

    #[inline(always)]
    fn heap_bytes(&self) -> usize {
        0
    }
}

impl CardinalityValue for f64 {
    type HashKey<'a> = u64;

    #[inline(always)]
    fn hash_key(&self) -> u64 {
        self.to_bits()
    }

    #[inline(always)]
    fn data_input(&self) -> DataInput<'_> {
        DataInput::F64(*self)
    }

    fn polars_column(values: &[Self]) -> Column {
        Column::new("v".into(), values)
    }

    #[inline(always)]
    fn heap_bytes(&self) -> usize {
        0
    }
}

impl CardinalityValue for String {
    type HashKey<'a> = &'a str;

    #[inline(always)]
    fn hash_key(&self) -> &str {
        self.as_str()
    }

    #[inline(always)]
    fn data_input(&self) -> DataInput<'_> {
        DataInput::Str(self.as_str())
    }

    fn polars_column(values: &[Self]) -> Column {
        Column::new("v".into(), values)
    }

    #[inline(always)]
    fn heap_bytes(&self) -> usize {
        self.capacity()
    }
}

pub mod datasketches;
pub mod oxide;
pub mod polars;
pub mod sketchlib;

/// so the wrapper's check and the registry's dispatch cannot disagree about it.
pub const LIB_PRECISIONS: [u8; 3] = [12, 14, 16];

/// The `lg_k` values this build compiled an HLL storage for, as a refusal.
/// `asap_sketchlib` puts the register count in the storage *type*, so anything
/// else is refused rather than built at a neighbouring precision.
pub fn unsupported_precision(lg_k: u8) -> BuildError {
    BuildError(format!(
        "asap HLL is compiled in at lg_k {LIB_PRECISIONS:?}; {lg_k} is not one of them"
    ))
}

#[cfg(test)]
mod tests {
    use super::datasketches::*;
    use super::oxide::*;
    use super::sketchlib::*;
    use super::*;
    use crate::params::ParamSet;

    use asap_sketchlib::{HllBucketListP12, HllBucketListP14, HllBucketListP16};

    fn params(lg_k: u8) -> ParamSet {
        ParamSet::of(&HllParams { lg_k })
    }

    /// Feed a sketch directly, the way its row's insert closure does.
    fn fed<R: asap_sketchlib::HllRegisterStorage>(
        sketch: &mut super::sketchlib::HllLib<R>,
        n: i64,
    ) {
        for v in 0..n {
            sketch.inner.insert(&asap_sketchlib::DataInput::I64(v));
        }
    }

    /// Three precisions must give three register counts and three estimates —
    /// a dropped `lg_k` would build one sketch under three different labels.
    #[test]
    fn lib_honours_lg_k() {
        let mut p12 = build_hll_lib::<HllBucketListP12>(&params(12)).expect("P12 builds");
        let mut p14 = build_hll_lib::<HllBucketListP14>(&params(14)).expect("P14 builds");
        let mut p16 = build_hll_lib::<HllBucketListP16>(&params(16)).expect("P16 builds");
        assert_eq!(
            (
                memory_hll_lib(&p12),
                memory_hll_lib(&p14),
                memory_hll_lib(&p16)
            ),
            (1 << 12, 1 << 14, 1 << 16)
        );
        // Three types, so three calls: the whole point of this row is that the
        // precision is a type and not a field.
        fed(&mut p12, 50_000);
        fed(&mut p14, 50_000);
        fed(&mut p16, 50_000);
        // A coarser register array is a worse estimate of the same stream. The
        // assertion is that the three differ at all: pinning an ordering would
        // pin the estimator's luck on one draw.
        let est = [
            p12.inner.estimate(),
            p14.inner.estimate(),
            p16.inner.estimate(),
        ];
        assert!(
            est[0] != est[1] && est[1] != est[2],
            "three precisions answered {est:?}, so lg_k is not reaching the sketch"
        );
    }

    /// The HIP rows carry the same knob, and the same defect if it were dropped.
    #[test]
    fn lib_hip_honours_lg_k() {
        let p12 = build_hll_lib_hip::<HllBucketListP12>(&params(12)).expect("P12 builds");
        let p16 = build_hll_lib_hip::<HllBucketListP16>(&params(16)).expect("P16 builds");
        assert_eq!(memory_hll_lib_hip(&p12), 1 << 12);
        assert_eq!(memory_hll_lib_hip(&p16), 1 << 16);
    }

    /// An `lg_k` with no storage type is refused by name rather than built at
    /// whichever precision the caller happened to select.
    #[test]
    fn lib_refuses_a_precision_it_does_not_ship() {
        for lg_k in [4u8, 10, 13, 18] {
            let Err(err) = build_hll_lib::<HllBucketListP14>(&params(lg_k)) else {
                panic!("lg_k={lg_k} has no storage type, so it must be refused");
            };
            assert!(
                err.to_string().contains(&lg_k.to_string()),
                "error should name the lg_k: {err}"
            );
        }
    }

    /// The precisions the registry dispatches over are the ones the wrapper
    /// accepts. Two lists that could drift silently: a value in one and not the
    /// other is either an unreachable row or a panic-free dead branch.
    #[test]
    fn the_dispatch_set_matches_what_the_rows_accept() {
        assert_eq!(LIB_PRECISIONS, [12, 14, 16]);
        assert!(build_hll_lib::<HllBucketListP12>(&params(LIB_PRECISIONS[0])).is_ok());
        assert!(build_hll_lib::<HllBucketListP14>(&params(LIB_PRECISIONS[1])).is_ok());
        assert!(build_hll_lib::<HllBucketListP16>(&params(LIB_PRECISIONS[2])).is_ok());
    }

    /// The datasketches row asserts inside its own constructor, so the bound is
    /// stated here. `lg_k = 3` and `lg_k = 22` both aborted the process before.
    #[test]
    fn datasketches_refuses_a_precision_outside_its_range() {
        for lg_k in [0u8, 3, 22, 255] {
            let err = build_hll_datasketches(&params(lg_k))
                .err()
                .unwrap_or_else(|| panic!("lg_k={lg_k} is outside [4, 21] and must be refused"));
            assert!(err.to_string().contains(&lg_k.to_string()), "{err}");
        }
        for lg_k in [DS_LG_K.0, 14, DS_LG_K.1] {
            assert!(
                build_hll_datasketches(&params(lg_k)).is_ok(),
                "lg_k={lg_k} is legal"
            );
        }
    }

    /// The three HLL libraries have three different domains, and each states
    /// its own. What they must not do is disagree about a value *all* of them
    /// reject, which is what a caller sweeping the axis will hit first.
    #[test]
    fn every_hll_row_refuses_a_precision_no_library_has() {
        for lg_k in [0u8, 3, 30] {
            assert!(build_hll_oxide(&params(lg_k)).is_err(), "oxide lg_k={lg_k}");
            assert!(
                build_hll_datasketches(&params(lg_k)).is_err(),
                "datasketches lg_k={lg_k}"
            );
            assert!(
                build_hll_lib::<HllBucketListP14>(&params(lg_k)).is_err(),
                "lib lg_k={lg_k}"
            );
        }
    }
}
