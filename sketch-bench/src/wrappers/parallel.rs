//! "Octo" parallel-insert wrappers. Each worker builds its own `FastPath`
//! sketch on a disjoint partition and the shards are *not* merged, so these
//! rows declare no query capability. `update` only buffers — read them off
//! `build_throughput_items_per_sec`, since the parallel section runs in
//! `prepare` and the timed region spans partition, spawn, barrier, insert, join.

use std::sync::Barrier;

use crate::params::{CmsParams, CountSketchParams, HllParams};
use crate::wrappers::require_shape;
use aqpbm_core::config::ParamSet;
use aqpbm_core::init::{BenchImpl, BuildError};
use aqpbm_core::memory_footprint::MemoryFootprint;
use asap_sketchlib::{
    impl_fixed_matrix, Count, CountMin, DataInput, ErtlMLE, FastPath, HyperLogLog,
};

impl_fixed_matrix!(M5x32K, i32, 5, 32768);

/// The shape every worker's matrix is baked at, and the `lg_k` its HLL is fixed
/// at. Written here because these rows *check* the request against them: the
/// per-worker sketch is a compile-time type, so any other config is unbuildable
/// and is refused instead of being accepted and ignored.
pub const PARALLEL_ROWS: usize = 5;
pub const PARALLEL_COLS: usize = 32768;
pub const PARALLEL_HLL_LG_K: u8 = 14;

/// CMS, parallel-insert FastPath.
pub struct ParallelCmsFastPath {
    buf: Vec<i64>,
    workers: usize,
}

impl aqpbm_core::cell::ParallelInit for ParallelCmsFastPath {
    /// Not an `InitSketch`: it needs the worker count, a run knob (`--workers`)
    /// rather than a sketch parameter. The construction config is checked
    /// against the baked shape and refused if it differs, exactly as the
    /// single-threaded fixed-matrix rows do.
    fn build(config: &ParamSet, workers: usize) -> Result<Self, BuildError> {
        let p: CmsParams = config.parse()?;
        require_shape(p.rows, p.cols, PARALLEL_ROWS, PARALLEL_COLS)?;
        Ok(Self {
            buf: Vec::new(),
            workers: workers.max(1),
        })
    }
}


impl MemoryFootprint for ParallelCmsFastPath {
    fn memory_bytes(&self) -> usize {
        self.workers * (PARALLEL_ROWS * PARALLEL_COLS * std::mem::size_of::<i32>())
            + self.buf.capacity() * std::mem::size_of::<i64>()
    }
}

/// CountSketch, parallel-insert FastPath.
pub struct ParallelCsFastPath {
    buf: Vec<i64>,
    workers: usize,
}

impl aqpbm_core::cell::ParallelInit for ParallelCsFastPath {
    fn build(config: &ParamSet, workers: usize) -> Result<Self, BuildError> {
        let p: CountSketchParams = config.parse()?;
        require_shape(p.rows, p.cols, PARALLEL_ROWS, PARALLEL_COLS)?;
        Ok(Self {
            buf: Vec::new(),
            workers: workers.max(1),
        })
    }
}


impl MemoryFootprint for ParallelCsFastPath {
    fn memory_bytes(&self) -> usize {
        self.workers * (PARALLEL_ROWS * PARALLEL_COLS * std::mem::size_of::<i32>())
            + self.buf.capacity() * std::mem::size_of::<i64>()
    }
}

/// HLL, parallel-insert ErtlMLE FastPath.
pub struct ParallelHllFastPath {
    buf: Vec<i64>,
    workers: usize,
}

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
            workers: workers.max(1),
        })
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

// ---------- shared parallel-insert kernels ----------

fn partition(items: &[i64], n: usize) -> Vec<&[i64]> {
    let n = n.max(1);
    let chunk = (items.len() + n - 1) / n;
    if chunk == 0 {
        return vec![items];
    }
    items.chunks(chunk).collect()
}

/// The barrier is load-bearing: a worker inserting while its peers are still
/// being spawned is not measuring a parallel insert. It costs one rendezvous
/// inside the runner's timed region, which is the honest place for it.
fn run_parallel_cms(items: &[i64], workers: usize) {
    let parts = partition(items, workers);
    let barrier = Barrier::new(parts.len());
    std::thread::scope(|s| {
        for part in &parts {
            let barrier = &barrier;
            s.spawn(move || {
                let mut sketch = CountMin::<M5x32K, FastPath>::from_storage(M5x32K::default());
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

fn run_parallel_cs(items: &[i64], workers: usize) {
    let parts = partition(items, workers);
    let barrier = Barrier::new(parts.len());
    std::thread::scope(|s| {
        for part in &parts {
            let barrier = &barrier;
            s.spawn(move || {
                let mut sketch = Count::<M5x32K, FastPath>::from_storage(M5x32K::default());
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

// ---------- catalog identity ----------
// These build through `ParallelInit`, not `InitSketch` — identity is declared
// the same way regardless. Parallel insert is a different structure, not a
// different library, so it names the algorithm; the two matrix rows name the
// shape they are baked at, because that shape is not something a caller can
// move and the name should not suggest otherwise.

impl BenchImpl for ParallelCmsFastPath {
    type Params = CmsParams;
    const ALGORITHM: &'static str = "cms-fastpath-fixedmatrix-32k-parallel";
    const IMPL: &'static str = "lib";
    const SUPPORTS_PREPARE: bool = true;
}
impl BenchImpl for ParallelCsFastPath {
    type Params = CountSketchParams;
    const ALGORITHM: &'static str = "countsketch-fastpath-fixedmatrix-32k-parallel";
    const IMPL: &'static str = "lib";
    const SUPPORTS_PREPARE: bool = true;
}
impl BenchImpl for ParallelHllFastPath {
    type Params = HllParams;
    const ALGORITHM: &'static str = "hll-fastpath-parallel";
    const IMPL: &'static str = "lib";
    const SUPPORTS_PREPARE: bool = true;
}

// ---------- how this sketch is driven ----------
//
// One function per operation, per sketch. These used to be an
// `impl Accumulator for X` block, which fixed one signature for every
// implementation in the repo. As free functions each states its own
// terms, and `catalog` names them in the row's `SketchOps`.
    #[inline(always)]
pub fn insert_parallel_cms_fast_path(sketch: &mut ParallelCmsFastPath, v: &i64)
{
        sketch.buf.push(*v);
}

pub fn prepare_parallel_cms_fast_path(sketch: &mut ParallelCmsFastPath)
{
        run_parallel_cms(&sketch.buf, sketch.workers);
}
    #[inline(always)]
pub fn insert_parallel_cs_fast_path(sketch: &mut ParallelCsFastPath, v: &i64)
{
        sketch.buf.push(*v);
}

pub fn prepare_parallel_cs_fast_path(sketch: &mut ParallelCsFastPath)
{
        run_parallel_cs(&sketch.buf, sketch.workers);
}
    #[inline(always)]
pub fn insert_parallel_hll_fast_path(sketch: &mut ParallelHllFastPath, v: &i64)
{
        sketch.buf.push(*v);
}

pub fn prepare_parallel_hll_fast_path(sketch: &mut ParallelHllFastPath)
{
        run_parallel_hll(&sketch.buf, sketch.workers);
}

// ---------- the rows this file provides ----------
//
// Nothing scores a parallel row, so its `ask` is never called: `run_parallel`
// passes no ground truth. It is still stated, because the type says every row
// has one — and stating a no-op is more honest than a special case.

use aqpbm_core::cell::{RunError, WorkloadData};
use aqpbm_core::ops::SketchOps;
use aqpbm_core::request::Numeric;
use aqpbm_core::runner::{BenchConfig, BenchReport};

pub const CMS_OPS: SketchOps<ParallelCmsFastPath, i64, (), ()> = SketchOps {
    merge: None,
    prepare: Some(prepare_parallel_cms_fast_path),
    ask: |_, _| (),
        _item: std::marker::PhantomData,
};
pub const CS_OPS: SketchOps<ParallelCsFastPath, i64, (), ()> = SketchOps {
    merge: None,
    prepare: Some(prepare_parallel_cs_fast_path),
    ask: |_, _| (),
        _item: std::marker::PhantomData,
};
pub const HLL_OPS: SketchOps<ParallelHllFastPath, i64, (), ()> = SketchOps {
    merge: None,
    prepare: Some(prepare_parallel_hll_fast_path),
    ask: |_, _| (),
        _item: std::marker::PhantomData,
};

pub fn run_cms(
    cfg: &BenchConfig,
    data: WorkloadData,
    params: &ParamSet,
    width: Numeric,
) -> Result<Vec<BenchReport>, RunError> {
    crate::catalog::run_parallel::<ParallelCmsFastPath, i64, _>(cfg, data, params, width, insert_parallel_cms_fast_path, &CMS_OPS)
}
pub fn run_cs(
    cfg: &BenchConfig,
    data: WorkloadData,
    params: &ParamSet,
    width: Numeric,
) -> Result<Vec<BenchReport>, RunError> {
    crate::catalog::run_parallel::<ParallelCsFastPath, i64, _>(cfg, data, params, width, insert_parallel_cs_fast_path, &CS_OPS)
}
pub fn run_hll(
    cfg: &BenchConfig,
    data: WorkloadData,
    params: &ParamSet,
    width: Numeric,
) -> Result<Vec<BenchReport>, RunError> {
    crate::catalog::run_parallel::<ParallelHllFastPath, i64, _>(cfg, data, params, width, insert_parallel_hll_fast_path, &HLL_OPS)
}
