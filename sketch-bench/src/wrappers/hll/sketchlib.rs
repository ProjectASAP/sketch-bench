//! The `asap_sketchlib` implementations.
//!
//! Grouped under `wrappers/hll/` with the other implementations of this
//! algorithm; how each is driven lives beside it.

use super::*;
use crate::params::ParamSet;
use crate::wrappers::partition;
use aqpbm_core::RunError;
use asap_sketchlib::{DataInput, ErtlMLE, HyperLogLog};
use std::sync::Barrier;

// `merge` lives on sketch_oxide's `Mergeable`, not on its `Accumulator`.

/// `HyperLogLog<Classic>`, the classic estimator (Flajolet et al., 2007):
/// insert bumps registers, `estimate()` scans all `2^lg_k` of them.
pub struct HllLib<R: asap_sketchlib::HllRegisterStorage = asap_sketchlib::HllBucketListP14> {
    inner: asap_sketchlib::hll::HyperLogLogImpl<asap_sketchlib::Classic, R>,
}

pub fn build_hll_lib<R: asap_sketchlib::HllRegisterStorage>(
    config: &ParamSet,
) -> Result<HllLib<R>, RunError> {
    let p: HllParams = config.parse()?;
    // The registry picked `R` off this same `lg_k`, so this only fires for a
    // direct caller. It fires rather than silently building at `R`, because
    // building at a precision other than the one requested is the defect
    // this row is being fixed for.
    if p.lg_k as usize != R::PRECISION {
        return Err(unsupported_precision(p.lg_k));
    }
    Ok(HllLib {
        inner: asap_sketchlib::hll::HyperLogLogImpl::<asap_sketchlib::Classic, R>::new(),
    })
}

pub fn memory_hll_lib<R: asap_sketchlib::HllRegisterStorage>(_sketch: &HllLib<R>) -> usize {
    // Off the storage type, so it tracks whichever precision was built.
    // A written-in `1 << 14` was what let three different `lg_k` values
    // report three footprints for one sketch.
    R::NUM_REGISTERS
}

/// `HyperLogLogHIP` maintains the estimate incrementally on the insert path —
/// every register upgrade pays a few fp ops — so query is O(1) rather than
/// Classic's O(m) scan. That trade is what this algorithm exists to measure,
/// and it is a different estimator, so it is its own algorithm and not an impl
/// of `hll`.
pub struct HllLibHip<R: asap_sketchlib::HllRegisterStorage = asap_sketchlib::HllBucketListP14> {
    inner: asap_sketchlib::hll::HyperLogLogHIPImpl<R>,
}

pub fn build_hll_lib_hip<R: asap_sketchlib::HllRegisterStorage>(
    config: &ParamSet,
) -> Result<HllLibHip<R>, RunError> {
    let p: HllParams = config.parse()?;
    if p.lg_k as usize != R::PRECISION {
        return Err(unsupported_precision(p.lg_k));
    }
    Ok(HllLibHip {
        inner: asap_sketchlib::hll::HyperLogLogHIPImpl::<R>::new(),
    })
}

pub fn memory_hll_lib_hip<R: asap_sketchlib::HllRegisterStorage>(_sketch: &HllLibHip<R>) -> usize {
    R::NUM_REGISTERS
}

impl<R: asap_sketchlib::HllRegisterStorage> HllLib<R> {
    pub fn estimate_distinct(&self) -> f64 {
        self.inner.estimate() as f64
    }
}

impl<R: asap_sketchlib::HllRegisterStorage> HllLibHip<R> {
    pub fn estimate_distinct(&self) -> f64 {
        self.inner.estimate() as f64
    }
}

pub fn insert_hll_lib<R: asap_sketchlib::HllRegisterStorage>(sketch: &mut HllLib<R>, v: &i64) {
    sketch.inner.insert(&asap_sketchlib::DataInput::I64(*v));
}

pub fn merge_hll_lib<R: asap_sketchlib::HllRegisterStorage>(
    into: &mut HllLib<R>,
    from: &HllLib<R>,
) {
    into.inner.merge(&from.inner);
}

pub fn insert_hll_lib_hip<R: asap_sketchlib::HllRegisterStorage>(
    sketch: &mut HllLibHip<R>,
    v: &i64,
) {
    sketch.inner.insert(&asap_sketchlib::DataInput::I64(*v));
}

pub fn query_hll_lib<R: asap_sketchlib::HllRegisterStorage>(s: &mut HllLib<R>, _: &()) -> f64 {
    s.estimate_distinct()
}

pub fn query_hll_lib_hip<R: asap_sketchlib::HllRegisterStorage>(
    s: &mut HllLibHip<R>,
    _: &(),
) -> f64 {
    s.estimate_distinct()
}

/// The `lg_k` this row is fixed at. Its per-worker sketch is a compile-time
/// type — `HyperLogLog<ErtlMLE>` is the P14 alias — so any other value is
/// unbuildable and is refused rather than run at 14 under its name.
const PARALLEL_HLL_LG_K: u8 = 14;

/// HLL, parallel-insert ErtlMLE FastPath.
pub struct ParallelHllFastPath {
    workers: usize,
}

pub fn build_parallel_hll_fast_path(
    config: &ParamSet,
    workers: usize,
) -> Result<ParallelHllFastPath, RunError> {
    let p: HllParams = config.parse()?;
    // `HyperLogLog<ErtlMLE>` is the P14 alias, so this row exists at one
    // precision. Refuse the others instead of running at 14 under their name.
    if p.lg_k != PARALLEL_HLL_LG_K {
        return Err(RunError::Sketch(format!(
            "parallel HLL: fixed at lg_k={PARALLEL_HLL_LG_K}, requested lg_k={}",
            p.lg_k
        )));
    }
    Ok(ParallelHllFastPath {
        workers: workers.max(1),
    })
}

pub fn memory_parallel_hll_fast_path(sketch: &ParallelHllFastPath) -> usize {
    // Each worker holds an HLL with ErtlMLE registers, one byte each.
    // `build` has already refused any lg_k other than the one baked in, so
    // this is the precision that ran and not a coarse upper bound.
    sketch.workers * (1usize << PARALLEL_HLL_LG_K)
}

fn run_parallel_hll(items: &[i64], workers: usize) {
    let parts = partition(items, workers);
    let barrier = Barrier::new(parts.len());
    std::thread::scope(|s| {
        for part in &parts {
            let barrier = &barrier;
            s.spawn(move || {
                let mut sketch = HyperLogLog::<ErtlMLE>::default();
                barrier.wait();
                for &v in *part {
                    sketch.insert_emit_delta(&DataInput::I64(v), &mut |d| {
                        std::hint::black_box(&d);
                    });
                }
                std::hint::black_box(&sketch);
            });
        }
    });
}

/// The whole stream in one call: this row's ingest *is* the parallel fan-out,
/// so there is no per-item step to buffer and none to time.
pub fn insert_parallel_hll_fast_path(sketch: &mut ParallelHllFastPath, items: &[i64]) {
    run_parallel_hll(items, sketch.workers);
}
