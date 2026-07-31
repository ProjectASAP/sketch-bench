//! HyperLogLog wrappers: `oxide`, `datasketches`, `sketchlib`
//! (a.k.a. asap_sketchlib). All of them declare `CardinalityOps`,
//! which is what makes them cardinality rows.

use aqpbm_core::accuracy::CardinalityOps;
use aqpbm_core::init::{BenchImpl, BuildError, InitSketch};
use crate::wrappers::require_range;
use crate::params::HllParams;
use aqpbm_core::config::ParamSet;
use aqpbm_core::accumulator::{Accumulator, MergeUnsupported};
use aqpbm_core::memory_footprint::MemoryFootprint;
// sketch_oxide routes `.estimate()` through its `Accumulator` trait.
use sketch_oxide::Sketch as OxideSketch; // NOTE: foreign trait, not ours
// `merge` lives on sketch_oxide's `Mergeable`, not on its `Accumulator`.
use sketch_oxide::Mergeable as _;

// ---------- sketch_oxide HLL ----------
pub struct HllOxide {
    inner: sketch_oxide::cardinality::HyperLogLog,
    lg_k: u8,
}

impl InitSketch for HllOxide {
    fn init(config: &ParamSet) -> Result<Self, BuildError> {
        let p: HllParams = config.parse()?;
        let inner = sketch_oxide::cardinality::HyperLogLog::new(p.lg_k)
            .map_err(|e| BuildError(format!("oxide HLL rejected lg_k={}: {e:?}", p.lg_k)))?;
        Ok(Self {
            inner,
            lg_k: p.lg_k,
        })
    }
}

impl Accumulator for HllOxide {
    type Item = i64;
    #[inline(always)]
    fn update(&mut self, v: &i64) {
        self.inner.update(v);
    }

    /// Register-wise max. Exact for equal `lg_k`: the merged sketch is
    /// bit-identical to one fed the whole stream, so any error the merge
    /// benchmark reports beyond the single-pass figure is a real defect.
    fn merge(&mut self, other: &Self) -> Result<(), MergeUnsupported> {
        self.inner
            .merge(&other.inner)
            .expect("both operands built from one ParamSet, so lg_k matches");
        Ok(())
    }
}

impl MemoryFootprint for HllOxide {
    fn memory_bytes(&self) -> usize {
        // sketch_oxide stores registers as Vec<u8>: 1 byte/register.
        1usize << self.lg_k
    }
}

// ---------- datasketches HLL ----------
pub struct HllDatasketches {
    inner: datasketches::hll::HllSketch,
    lg_k: u8,
    hll_type: datasketches::hll::HllType,
}

/// What `datasketches::hll::HllSketch::new` accepts. It asserts rather than
/// returning an error, so the bound has to be stated on this side: the twin of
/// [`LIB_PRECISIONS`], for the library that expresses its domain as a range
/// instead of a set of types.
pub const DS_LG_K: (u8, u8) = (4, 21);

impl InitSketch for HllDatasketches {
    fn init(config: &ParamSet) -> Result<Self, BuildError> {
        let p: HllParams = config.parse()?;
        require_range("datasketches HLL", "lg_k", p.lg_k, DS_LG_K.0, DS_LG_K.1)?;
        let hll_type = datasketches::hll::HllType::Hll8;
        Ok(Self {
            inner: datasketches::hll::HllSketch::new(p.lg_k, hll_type),
            lg_k: p.lg_k,
            hll_type,
        })
    }
}

impl Accumulator for HllDatasketches {
    type Item = i64;
    #[inline(always)]
    fn update(&mut self, v: &i64) {
        self.inner.update(*v);
    }

    /// DataSketches merges HLL through a `Union` gadget, so this rebuilds `self`
    /// from its result. **Known limitation:** a fresh `HllUnion` per fold inside
    /// the timed region makes this row read lossy and ~2x slow — not comparable.
    fn merge(&mut self, other: &Self) -> Result<(), MergeUnsupported> {
        let mut union = datasketches::hll::HllUnion::new(self.lg_k);
        union.update(&self.inner);
        union.update(&other.inner);
        self.inner = union.get_result(self.hll_type);
        Ok(())
    }
}

impl MemoryFootprint for HllDatasketches {
    fn memory_bytes(&self) -> usize {
        // Apache datasketches packs registers per HllType:
        // Hll4 → 0.5 B, Hll6 → 0.75 B, Hll8 → 1 B.
        let m = 1usize << self.lg_k;
        match self.hll_type {
            datasketches::hll::HllType::Hll4 => m / 2,
            datasketches::hll::HllType::Hll6 => (m * 6).div_ceil(8),
            datasketches::hll::HllType::Hll8 => m,
        }
    }
}

// ---------- asap_sketchlib HLL ----------
//
// The register count is a *type* in this library: `HyperLogLogImpl<Variant, R>`
// picks it through `R`, and `HllBucketListP12 / P14 / P16` are the three it
// ships. So `lg_k` selects a monomorphisation, which is why these two rows are
// generic and the catalog dispatches on the requested value. An enum here would
// have put a branch in `update`, on the row whose whole purpose is to price
// that insert.
//
// Three precisions, not the 4..=18 range `oxide` offers. A row reports the
// bound its own library has: refusing `lg_k = 13` by name is the whole point,
// since the alternative is what this code used to do, which was to run at 14
// and label the record 13.

/// The `lg_k` values `asap_sketchlib` ships a register storage for. Named once
/// so the wrapper's check and the catalog's dispatch cannot disagree about it.
pub const LIB_PRECISIONS: [u8; 3] = [12, 14, 16];

/// The error both `lib` rows give for an `lg_k` this library has no storage
/// type for. Shared with the catalog's dispatch, so the row that cannot be
/// selected and the row that cannot be built refuse in the same words.
pub(crate) fn unsupported_precision(lg_k: u8) -> BuildError {
    BuildError(format!(
        "asap HLL: lg_k={lg_k} has no register storage in this library; it ships {LIB_PRECISIONS:?}"
    ))
}

/// `HyperLogLog<Classic>`, the classic estimator (Flajolet et al., 2007):
/// insert bumps registers, `estimate()` scans all `2^lg_k` of them.
pub struct HllLib<R: asap_sketchlib::HllRegisterStorage = asap_sketchlib::HllBucketListP14> {
    inner: asap_sketchlib::hll::HyperLogLogImpl<asap_sketchlib::Classic, R>,
}

impl<R: asap_sketchlib::HllRegisterStorage> InitSketch for HllLib<R> {
    fn init(config: &ParamSet) -> Result<Self, BuildError> {
        let p: HllParams = config.parse()?;
        // The catalog picked `R` off this same `lg_k`, so this only fires for a
        // direct caller. It fires rather than silently building at `R`, because
        // building at a precision other than the one requested is the defect
        // this row is being fixed for.
        if p.lg_k as usize != R::PRECISION {
            return Err(unsupported_precision(p.lg_k));
        }
        Ok(Self {
            inner: asap_sketchlib::hll::HyperLogLogImpl::<asap_sketchlib::Classic, R>::new(),
        })
    }
}

impl<R: asap_sketchlib::HllRegisterStorage> Accumulator for HllLib<R> {
    type Item = i64;
    #[inline(always)]
    fn update(&mut self, v: &i64) {
        self.inner.insert(&asap_sketchlib::DataInput::I64(*v));
    }

    fn merge(&mut self, other: &Self) -> Result<(), MergeUnsupported> {
        self.inner.merge(&other.inner);
        Ok(())
    }
}

impl<R: asap_sketchlib::HllRegisterStorage> MemoryFootprint for HllLib<R> {
    fn memory_bytes(&self) -> usize {
        // Off the storage type, so it tracks whichever precision was built.
        // A written-in `1 << 14` was what let three different `lg_k` values
        // report three footprints for one sketch.
        R::NUM_REGISTERS
    }
}

/// `HyperLogLogHIP` maintains the estimate incrementally on the insert path —
/// every register upgrade pays a few fp ops — so query is O(1) rather than
/// Classic's O(m) scan. That trade is what this algorithm exists to measure,
/// and it is a different estimator, so it is its own algorithm and not an impl
/// of `hll`.
pub struct HllLibHip<R: asap_sketchlib::HllRegisterStorage = asap_sketchlib::HllBucketListP14> {
    inner: asap_sketchlib::hll::HyperLogLogHIPImpl<R>,
}

impl<R: asap_sketchlib::HllRegisterStorage> InitSketch for HllLibHip<R> {
    fn init(config: &ParamSet) -> Result<Self, BuildError> {
        let p: HllParams = config.parse()?;
        if p.lg_k as usize != R::PRECISION {
            return Err(unsupported_precision(p.lg_k));
        }
        Ok(Self {
            inner: asap_sketchlib::hll::HyperLogLogHIPImpl::<R>::new(),
        })
    }
}

impl<R: asap_sketchlib::HllRegisterStorage> Accumulator for HllLibHip<R> {
    type Item = i64;
    #[inline(always)]
    fn update(&mut self, v: &i64) {
        self.inner.insert(&asap_sketchlib::DataInput::I64(*v));
    }
}

impl<R: asap_sketchlib::HllRegisterStorage> MemoryFootprint for HllLibHip<R> {
    fn memory_bytes(&self) -> usize {
        R::NUM_REGISTERS
    }
}


// ---------- statistic membership ----------
// The parallel-HLL row ingests `i64` like these but answers nothing, so it is
// absent here — which is what keeps it out of accuracy scoring.

impl CardinalityOps for HllOxide {
    fn estimate_distinct(&self) -> f64 {
        self.inner.estimate()
    }
}

impl CardinalityOps for HllDatasketches {
    fn estimate_distinct(&self) -> f64 {
        self.inner.estimate()
    }
}

impl<R: asap_sketchlib::HllRegisterStorage> CardinalityOps for HllLib<R> {
    fn estimate_distinct(&self) -> f64 {
        self.inner.estimate() as f64
    }
}

impl<R: asap_sketchlib::HllRegisterStorage> CardinalityOps for HllLibHip<R> {
    fn estimate_distinct(&self) -> f64 {
        self.inner.estimate() as f64
    }
}

// ---------- catalog identity ----------
// `IMPL` is the library and nothing else. The HIP estimator answers the same
// question by different arithmetic and gets different numbers, so it is on the
// algorithm axis; every precision of one estimator is one algorithm, because
// `lg_k` is the knob that selects it.

impl BenchImpl for HllOxide { type Params = HllParams; const IMPL: &'static str = "oxide"; }
impl BenchImpl for HllDatasketches { type Params = HllParams; const IMPL: &'static str = "datasketches"; }

impl<R: asap_sketchlib::HllRegisterStorage> BenchImpl for HllLib<R> {
    type Params = HllParams;
    const IMPL: &'static str = "lib";
}

impl<R: asap_sketchlib::HllRegisterStorage> BenchImpl for HllLibHip<R> {
    type Params = HllParams;
    const ALGORITHM: &'static str = "hll-hip";
    const IMPL: &'static str = "lib";
}

#[cfg(test)]
mod tests {
    use super::*;
    use asap_sketchlib::{HllBucketListP12, HllBucketListP14, HllBucketListP16};

    fn params(lg_k: u8) -> ParamSet {
        ParamSet::of(&HllParams { lg_k })
    }

    fn fed<S>(sketch: &mut S, n: i64)
    where
        S: Accumulator<Item = i64>,
    {
        for v in 0..n {
            sketch.update(&v);
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
        fed(&mut p12, 50_000);
        fed(&mut p14, 50_000);
        fed(&mut p16, 50_000);
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
