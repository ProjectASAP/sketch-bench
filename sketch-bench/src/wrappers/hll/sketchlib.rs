//! The `asap_sketchlib` implementations.
//!
//! Grouped under `wrappers/hll/` with the other implementations of this
//! algorithm; how each is driven lives beside it.

use super::*;
use asap_sketchlib::{HllBucketListP12, HllBucketListP14, HllBucketListP16};
use crate::wrappers::parallel_shared::*;
use aqpbm_core::accuracy::cardinality::CardinalityGT;
use aqpbm_core::cell::{RunError, WorkloadData};
use aqpbm_core::config::ParamSet;
use aqpbm_core::init::{BenchImpl, BuildError, InitSketch};
use aqpbm_core::memory_footprint::MemoryFootprint;
use aqpbm_core::ops::SketchOps;
use aqpbm_core::request::Numeric;
use aqpbm_core::runner::{BenchConfig, BenchReport};
use asap_sketchlib::{
    DataInput, ErtlMLE, HyperLogLog};
use std::sync::Barrier;

// `merge` lives on sketch_oxide's `Mergeable`, not on its `Accumulator`.


/// `HyperLogLog<Classic>`, the classic estimator (Flajolet et al., 2007):
/// insert bumps registers, `estimate()` scans all `2^lg_k` of them.
pub struct HllLib<R: asap_sketchlib::HllRegisterStorage = asap_sketchlib::HllBucketListP14> {
    inner: asap_sketchlib::hll::HyperLogLogImpl<asap_sketchlib::Classic, R>}

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
            inner: asap_sketchlib::hll::HyperLogLogImpl::<asap_sketchlib::Classic, R>::new()})
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
    inner: asap_sketchlib::hll::HyperLogLogHIPImpl<R>}

impl<R: asap_sketchlib::HllRegisterStorage> InitSketch for HllLibHip<R> {
    fn init(config: &ParamSet) -> Result<Self, BuildError> {
        let p: HllParams = config.parse()?;
        if p.lg_k as usize != R::PRECISION {
            return Err(unsupported_precision(p.lg_k));
        }
        Ok(Self {
            inner: asap_sketchlib::hll::HyperLogLogHIPImpl::<R>::new()})
    }
}

impl<R: asap_sketchlib::HllRegisterStorage> MemoryFootprint for HllLibHip<R> {
    fn memory_bytes(&self) -> usize {
        R::NUM_REGISTERS
    }
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

impl<R: asap_sketchlib::HllRegisterStorage> BenchImpl for HllLib<R> {
    type Params = HllParams;
    const IMPL: &'static str = "lib";
    const SUPPORTS_MERGE: bool = true;
}

impl<R: asap_sketchlib::HllRegisterStorage> BenchImpl for HllLibHip<R> {
    type Params = HllParams;
    const ALGORITHM: &'static str = "hll-hip";
    const IMPL: &'static str = "lib";
}

pub fn insert_hll_lib<R: asap_sketchlib::HllRegisterStorage>(sketch: &mut HllLib<R>, v: &i64)
{
        sketch.inner.insert(&asap_sketchlib::DataInput::I64(*v));
}

pub fn merge_hll_lib<R: asap_sketchlib::HllRegisterStorage>(into: &mut HllLib<R>, from: &HllLib<R>)
{
        into.inner.merge(&from.inner);
}

pub fn insert_hll_lib_hip<R: asap_sketchlib::HllRegisterStorage>(sketch: &mut HllLibHip<R>, v: &i64)
{
        sketch.inner.insert(&asap_sketchlib::DataInput::I64(*v));
}

/// One body for all three precisions: the register count is in the storage
/// type, so `lg_k` selects `R` and this instantiates at each.
pub const fn lib_ops<R: asap_sketchlib::HllRegisterStorage>() -> SketchOps<HllLib<R>, i64, (), f64> {
    SketchOps {
        merge: Some(merge_hll_lib),
        prepare: None,
        ask: ask_hll_lib,
        _item: std::marker::PhantomData}
}

pub fn ask_hll_lib<R: asap_sketchlib::HllRegisterStorage>(s: &mut HllLib<R>, _: &()) -> f64 {
    s.estimate_distinct()
}

/// The HIP variant maintains its estimate on the insert path, and provides no
/// merge — the `None` below is the whole declaration.
pub const fn lib_hip_ops<R: asap_sketchlib::HllRegisterStorage>(
) -> SketchOps<HllLibHip<R>, i64, (), f64> {
    SketchOps {
        merge: None,
        prepare: None,
        ask: ask_hll_lib_hip,
        _item: std::marker::PhantomData}
}

pub fn ask_hll_lib_hip<R: asap_sketchlib::HllRegisterStorage>(s: &mut HllLibHip<R>, _: &()) -> f64 {
    s.estimate_distinct()
}

pub fn run_lib(
    cfg: &BenchConfig,
    data: WorkloadData,
    params: &ParamSet,
    width: Numeric,
) -> Result<Vec<BenchReport>, RunError> {
    use asap_sketchlib::{HllBucketListP12, HllBucketListP14, HllBucketListP16};
    crate::catalog::run_lib_hll::<
        HllLib<HllBucketListP12>,
        HllLib<HllBucketListP14>,
        HllLib<HllBucketListP16>,
        CardinalityGT,
        _,
        _,
        _,
    >(
        cfg,
        data,
        params,
        width,
        insert_hll_lib,
        &lib_ops::<HllBucketListP12>(),
        insert_hll_lib,
        &lib_ops::<HllBucketListP14>(),
        insert_hll_lib,
        &lib_ops::<HllBucketListP16>(),
    )
}

pub fn run_lib_hip(
    cfg: &BenchConfig,
    data: WorkloadData,
    params: &ParamSet,
    width: Numeric,
) -> Result<Vec<BenchReport>, RunError> {
    crate::catalog::run_lib_hll::<
        HllLibHip<HllBucketListP12>,
        HllLibHip<HllBucketListP14>,
        HllLibHip<HllBucketListP16>,
        CardinalityGT,
        _,
        _,
        _,
    >(
        cfg,
        data,
        params,
        width,
        insert_hll_lib_hip,
        &lib_hip_ops::<HllBucketListP12>(),
        insert_hll_lib_hip,
        &lib_hip_ops::<HllBucketListP14>(),
        insert_hll_lib_hip,
        &lib_hip_ops::<HllBucketListP16>(),
    )
}

/// HLL, parallel-insert ErtlMLE FastPath.
pub struct ParallelHllFastPath {
    buf: Vec<i64>,
    workers: usize}

impl aqpbm_core::cell::ParallelInit for ParallelHllFastPath {
    fn build(config: &ParamSet, workers: usize) -> Result<Self, BuildError> {
        let p: HllParams = config.parse()?;
        // `HyperLogLog<ErtlMLE>` is the P14 alias, so this row exists at one
        // precision. Refuse the others instead of running at 14 under their name.
        if p.lg_k != PARALLEL_HLL_LG_K {
            return Err(BuildError(format!(
                "parallel HLL: fixed at lg_k={PARALLEL_HLL_LG_K}, requested lg_k={}",
                p.lg_k
            )));
        }
        Ok(Self {
            buf: Vec::new(),
            workers: workers.max(1)})
    }
}

impl MemoryFootprint for ParallelHllFastPath {
    fn memory_bytes(&self) -> usize {
        // Each worker holds an HLL with ErtlMLE registers, one byte each.
        // `build` has already refused any lg_k other than the one baked in, so
        // this is the precision that ran and not a coarse upper bound.
        self.workers * (1usize << PARALLEL_HLL_LG_K)
            + self.buf.capacity() * std::mem::size_of::<i64>()
    }
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

impl BenchImpl for ParallelHllFastPath {
    type Params = HllParams;
    const ALGORITHM: &'static str = "hll-fastpath-parallel";
    const IMPL: &'static str = "lib";
    const SUPPORTS_PREPARE: bool = true;
}

pub fn insert_parallel_hll_fast_path(sketch: &mut ParallelHllFastPath, v: &i64)
{
        sketch.buf.push(*v);
}

pub fn prepare_parallel_hll_fast_path(sketch: &mut ParallelHllFastPath)
{
        run_parallel_hll(&sketch.buf, sketch.workers);
}

pub const HLL_OPS: SketchOps<ParallelHllFastPath, i64, (), ()> = SketchOps {
    merge: None,
    prepare: Some(prepare_parallel_hll_fast_path),
    ask: |_, _| (),
        _item: std::marker::PhantomData};

pub fn run_hll(
    cfg: &BenchConfig,
    data: WorkloadData,
    params: &ParamSet,
    width: Numeric,
) -> Result<Vec<BenchReport>, RunError> {
    crate::catalog::run_parallel::<ParallelHllFastPath, i64, _>(cfg, data, params, width, insert_parallel_hll_fast_path, &HLL_OPS)
}


