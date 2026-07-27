//! "Octo" parallel-insert wrappers: `cms/lib-fastpath-parallel`,
//! `countsketch/lib-fastpath-parallel`, `hll/lib-fastpath-parallel`.
//!
//! Mirrors the legacy `throughput/octo/` binary: each worker thread builds its
//! own `FastPath` sketch on a disjoint partition of the input, deltas emitted
//! via `insert_emit_delta` are passed through `black_box`, and the partition
//! sketches are *not* merged — so these impls declare no query capability and
//! are not scored.
//!
//! `update` only buffers; the parallel section runs in `finalize_for_query`,
//! which the runner times into `RunMetrics::finalize_wall_time_ns`. So read
//! these rows off `build_throughput_items_per_sec` — their
//! `throughput_items_per_sec` is the `Vec::push` that buffers the partition
//! and says nothing about parallel insert.
//!
//! The timed region is the whole parallel section: partition, spawn, barrier,
//! insert, join. Legacy octo's headline instead took the max worker's own
//! elapsed time, which excludes spawn and join — each kernel here used to
//! compute that number and then drop it on the floor, unreadable by anything,
//! costing a per-thread `Instant` pair for nothing. The end-to-end reading is
//! the one a caller can act on (you pay for the threads whether or not the
//! workers were the slow part), so the kernels now return `()` and the clock
//! that matters is the runner's.
//!
//! Worker count is plumbed via `BenchConfig.threads` (the `--workers N` CLI
//! flag). The family's `ParamSet` knobs are ignored: CMS and CountSketch are
//! fixed to the legacy `M5x32K` (5 × 32768) octo matrix, the HLL row to
//! sketchlib's P14 `ErtlMLE` default.

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
    /// Not an `InitSketch`: it needs the worker count, which is a run knob
    /// (`--workers`), not a sketch parameter. Parses the config to reject a
    /// malformed one, then ignores its values — this impl's shape is fixed
    /// internally, so any well-formed config builds it.
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

    fn finalize_for_query(&mut self) {
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

    fn finalize_for_query(&mut self) {
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

    fn finalize_for_query(&mut self) {
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

/// The barrier is load-bearing and stays: a worker that started inserting
/// while its peers were still being spawned would not be measuring a parallel
/// insert at all. It costs one rendezvous inside the runner's timed region,
/// which is the honest place for it.
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
//
// These build through `ParallelInit`, not `InitSketch` — identity is declared
// the same way regardless.

impl BenchImpl for ParallelCmsFastPath { type Params = CmsParams; const IMPL: &'static str = "lib-fastpath-parallel"; }
impl BenchImpl for ParallelCsFastPath { type Params = CountSketchParams; const IMPL: &'static str = "lib-fastpath-parallel"; }
impl BenchImpl for ParallelHllFastPath { type Params = HllParams; const IMPL: &'static str = "lib-fastpath-parallel"; }
