//! HyperLogLog wrappers: `oxide`, `datasketches`, `sketchlib`
//! (a.k.a. asap_sketchlib). All of them declare `CardinalityOps`,
//! which is what makes them cardinality rows.

use aqpbm_core::init::BuildError;
use crate::params::*;
use sketch_oxide::Mergeable as _;

pub mod datasketches;
pub mod oxide;
pub mod polars;
pub mod sketchlib;

/// so the wrapper's check and the catalog's dispatch cannot disagree about it.
pub const LIB_PRECISIONS: [u8; 3] = [12, 14, 16];

#[cfg(test)]
mod tests {
    use aqpbm_core::init::InitSketch;
    use aqpbm_core::memory_footprint::MemoryFootprint;
    use aqpbm_core::config::ParamSet;
    use super::datasketches::{HllDatasketches, DS_LG_K};
    use super::oxide::HllOxide;
    use super::sketchlib::{insert_hll_lib, HllLib, HllLibHip};
    use super::*;
    use asap_sketchlib::{HllBucketListP12, HllBucketListP14, HllBucketListP16};

    fn params(lg_k: u8) -> ParamSet {
        ParamSet::of(&HllParams { lg_k })
    }

    /// Feed a sketch through its own insert function — the same one its row
    /// uses, rather than a trait method every sketch had to share.
    fn fed<S>(sketch: &mut S, insert: fn(&mut S, &i64), n: i64) {
        for v in 0..n {
            insert(sketch, &v);
        }
    }

    /// The bug these rows had: `lg_k` was parsed and dropped, so every value
    /// built the same P14 sketch, reported the same 16384-byte footprint and
    /// scored the same error, under three different labels.
    ///
    /// Three precisions must now give three register counts and three
    /// estimates.
    #[test]
    fn lib_honours_lg_k() {
        let mut p12 = HllLib::<HllBucketListP12>::init(&params(12)).expect("P12 builds");
        let mut p14 = HllLib::<HllBucketListP14>::init(&params(14)).expect("P14 builds");
        let mut p16 = HllLib::<HllBucketListP16>::init(&params(16)).expect("P16 builds");
        assert_eq!(
            (p12.memory_bytes(), p14.memory_bytes(), p16.memory_bytes()),
            (1 << 12, 1 << 14, 1 << 16)
        );
        // Three types, so three calls: the whole point of this row is that the
        // precision is a type and not a field.
        fed(&mut p12, insert_hll_lib, 50_000);
        fed(&mut p14, insert_hll_lib, 50_000);
        fed(&mut p16, insert_hll_lib, 50_000);
        // A coarser register array is a worse estimate of the same stream. The
        // assertion is that the three differ at all: pinning an ordering would
        // pin the estimator's luck on one draw.
        let est = [
            p12.estimate_distinct(),
            p14.estimate_distinct(),
            p16.estimate_distinct(),
        ];
        assert!(
            est[0] != est[1] && est[1] != est[2],
            "three precisions answered {est:?}, so lg_k is not reaching the sketch"
        );
    }

    /// The HIP rows carry the same knob, and the same defect if it were dropped.
    #[test]
    fn lib_hip_honours_lg_k() {
        let p12 = HllLibHip::<HllBucketListP12>::init(&params(12)).expect("P12 builds");
        let p16 = HllLibHip::<HllBucketListP16>::init(&params(16)).expect("P16 builds");
        assert_eq!(p12.memory_bytes(), 1 << 12);
        assert_eq!(p16.memory_bytes(), 1 << 16);
    }

    /// An `lg_k` with no storage type is refused by name rather than built at
    /// whichever precision the caller happened to select.
    #[test]
    fn lib_refuses_a_precision_it_does_not_ship() {
        for lg_k in [4u8, 10, 13, 18] {
            let Err(err) = HllLib::<HllBucketListP14>::init(&params(lg_k)) else {
                panic!("lg_k={lg_k} has no storage type, so it must be refused");
            };
            assert!(
                err.to_string().contains(&lg_k.to_string()),
                "error should name the lg_k: {err}"
            );
        }
    }

    /// The precisions the catalog dispatches over are the ones the wrapper
    /// accepts. Two lists that could drift silently: a value in one and not the
    /// other is either an unreachable row or a panic-free dead branch.
    #[test]
    fn the_dispatch_set_matches_what_the_rows_accept() {
        assert_eq!(LIB_PRECISIONS, [12, 14, 16]);
        assert!(HllLib::<HllBucketListP12>::init(&params(LIB_PRECISIONS[0])).is_ok());
        assert!(HllLib::<HllBucketListP14>::init(&params(LIB_PRECISIONS[1])).is_ok());
        assert!(HllLib::<HllBucketListP16>::init(&params(LIB_PRECISIONS[2])).is_ok());
    }

    /// The datasketches row asserts inside its own constructor, so the bound is
    /// stated here. `lg_k = 3` and `lg_k = 22` both aborted the process before.
    #[test]
    fn datasketches_refuses_a_precision_outside_its_range() {
        for lg_k in [0u8, 3, 22, 255] {
            let err = HllDatasketches::init(&params(lg_k))
                .err()
                .unwrap_or_else(|| panic!("lg_k={lg_k} is outside [4, 21] and must be refused"));
            assert!(err.to_string().contains(&lg_k.to_string()), "{err}");
        }
        for lg_k in [DS_LG_K.0, 14, DS_LG_K.1] {
            assert!(HllDatasketches::init(&params(lg_k)).is_ok(), "lg_k={lg_k} is legal");
        }
    }

    /// The three HLL libraries have three different domains, and each states
    /// its own. What they must not do is disagree about a value *all* of them
    /// reject, which is what a caller sweeping the axis will hit first.
    #[test]
    fn every_hll_row_refuses_a_precision_no_library_has() {
        for lg_k in [0u8, 3, 30] {
            assert!(HllOxide::init(&params(lg_k)).is_err(), "oxide lg_k={lg_k}");
            assert!(HllDatasketches::init(&params(lg_k)).is_err(), "datasketches lg_k={lg_k}");
            assert!(
                HllLib::<HllBucketListP14>::init(&params(lg_k)).is_err(),
                "lib lg_k={lg_k}"
            );
        }
    }
}


/// The `lg_k` values this build compiled an HLL storage for, as a refusal.
///
/// `asap_sketchlib` puts the register count in the storage *type*, so a row
/// exists only at the precisions some build instantiated. Anything else is
/// refused by name rather than silently built at a neighbouring precision —
/// which would report one `lg_k` and measure another.
pub fn unsupported_precision(lg_k: u8) -> BuildError {
    BuildError(format!(
        "asap HLL is compiled in at lg_k {LIB_PRECISIONS:?}; {lg_k} is not one of them"
    ))
}
