//! "Octo" parallel-insert wrappers. Each worker builds its own `FastPath`
//! sketch on a disjoint partition and the shards are *not* merged, so these
//! rows declare no query capability. `update` only buffers — read them off
//! `build_throughput_items_per_sec`, since the parallel section runs in
//! `prepare` and the timed region spans partition, spawn, barrier, insert, join.

use std::sync::Barrier;

use aqpbm_core::init::{BenchImpl, BuildError};
use crate::params::{CmsParams, CountSketchParams, HllParams};
use aqpbm_core::config::ParamSet;
use aqpbm_core::accumulator::Accumulator;
use aqpbm_core::memory_footprint::MemoryFootprint;
use asap_sketchlib::{
    impl_fixed_matrix, Count, CountMin, DataInput, ErtlMLE, FastPath, HyperLogLog,
};

impl_fixed_matrix!(M5x32K, i32, 5, 32768);

/// CMS, parallel-insert FastPath.
pub struct ParallelCmsFastPath {
    buf: Vec<i64>,
    workers: usize,
}

impl aqpbm_core::cell::ParallelInit for ParallelCmsFastPath {
    /// Not an `InitSketch`: it needs the worker count, a run knob (`--workers`)
    /// rather than a sketch parameter. Parses the config to reject a malformed
    /// one, then ignores its values — the shape is fixed internally.
    fn build(config: &ParamSet, workers: usize) -> Result<Self, BuildError> {
        let _p: CmsParams = config.parse()?;
        Ok(Self {
            buf: Vec::new(),
            workers: workers.max(1),
        })
    }
}

impl Accumulator for ParallelCmsFastPath {
    type Item = i64;

    #[inline(always)]
    fn update(&mut self, v: &i64) {
        self.buf.push(*v);
    }

    fn prepare(&mut self) {
        run_parallel_cms(&self.buf, self.workers);
    }

}

impl MemoryFootprint for ParallelCmsFastPath {
    fn memory_bytes(&self) -> usize {
        self.workers * (5 * 32768 * std::mem::size_of::<i32>())
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
        let _p: CountSketchParams = config.parse()?;
        Ok(Self {
            buf: Vec::new(),
            workers: workers.max(1),
        })
    }
}

impl Accumulator for ParallelCsFastPath {
    type Item = i64;

    #[inline(always)]
    fn update(&mut self, v: &i64) {
        self.buf.push(*v);
    }

    fn prepare(&mut self) {
        run_parallel_cs(&self.buf, self.workers);
    }

}

impl MemoryFootprint for ParallelCsFastPath {
    fn memory_bytes(&self) -> usize {
        self.workers * (5 * 32768 * std::mem::size_of::<i32>())
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
        let _p: HllParams = config.parse()?;
        Ok(Self {
            buf: Vec::new(),
            workers: workers.max(1),
        })
    }
}

impl Accumulator for ParallelHllFastPath {
    type Item = i64;

    #[inline(always)]
    fn update(&mut self, v: &i64) {
        self.buf.push(*v);
    }

    fn prepare(&mut self) {
        run_parallel_hll(&self.buf, self.workers);
    }

}

impl MemoryFootprint for ParallelHllFastPath {
    fn memory_bytes(&self) -> usize {
        // Each worker holds an HLL with ErtlMLE registers — leave
        // it at a coarse upper bound (P14 default for the
        // sketchlib HLL ≈ 16k regs × 1 byte).
        self.workers * (1 << 14) + self.buf.capacity() * std::mem::size_of::<i64>()
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
}
impl BenchImpl for ParallelCsFastPath {
    type Params = CountSketchParams;
    const ALGORITHM: &'static str = "countsketch-fastpath-fixedmatrix-32k-parallel";
    const IMPL: &'static str = "lib";
}
impl BenchImpl for ParallelHllFastPath {
    type Params = HllParams;
    const ALGORITHM: &'static str = "hll-fastpath-parallel";
    const IMPL: &'static str = "lib";
}
