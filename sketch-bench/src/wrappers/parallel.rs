//! "Octo" parallel-insert wrappers: `cms/lib-fastpath-parallel`,
//! `countsketch/lib-fastpath-parallel`, `hll/lib-fastpath-parallel`.
//!
//! Mirrors the legacy `throughput/octo/` binary: each worker
//! thread builds its own `FastPath` sketch on a disjoint
//! partition of the input. As in the legacy code, deltas
//! emitted via `insert_emit_delta` are passed through
//! `black_box` and the partition sketches are *not* merged —
//! so `accuracy_kind: None` for these impls. The reported
//! throughput is the max worker's elapsed time (= the wall
//! clock for the parallel section), matching legacy octo.
//!
//! Worker count is plumbed via `BenchConfig.threads` (the new
//! `--workers N` CLI flag). Wrappers ignore the family's
//! `ParamSet` knobs; their shape is fixed to the legacy
//! `M5x32K` (5 × 32768) octo matrix.

use std::sync::Barrier;
use std::time::Instant;

use crate::init::BuildError;
use crate::params::{CmsParams, CountSketchParams, HllParams};
use aqpbm_core::config::ParamSet;
use aqpbm_core::sketch::Sketch;
use asap_sketchlib::{
    impl_fixed_matrix, Count, CountMin, DataInput, ErtlMLE, FastPath, HyperLogLog,
};

impl_fixed_matrix!(M5x32K, i32, 5, 32768);

/// CMS, parallel-insert FastPath.
pub struct ParallelCmsFastPath {
    buf: Vec<i64>,
    workers: usize,
}

impl crate::cell::ParallelInit for ParallelCmsFastPath {
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

impl Sketch for ParallelCmsFastPath {
    type Item = i64;
    type Query = i64;
    type Answer = u64;

    #[inline(always)]
    fn update(&mut self, v: &i64) {
        self.buf.push(*v);
    }

    fn finalize_for_query(&mut self) {
        run_parallel_cms(&self.buf, self.workers);
    }

    fn query(&self, _q: i64) -> u64 {
        // Parallel partitions are intentionally not merged
        // (matches legacy octo). Query is a stub.
        0
    }

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

impl crate::cell::ParallelInit for ParallelCsFastPath {
    fn build(config: &ParamSet, workers: usize) -> Result<Self, BuildError> {
        let _p: CountSketchParams = config.parse()?;
        Ok(Self {
            buf: Vec::new(),
            workers: workers.max(1),
        })
    }
}

impl Sketch for ParallelCsFastPath {
    type Item = i64;
    type Query = i64;
    type Answer = u64;

    #[inline(always)]
    fn update(&mut self, v: &i64) {
        self.buf.push(*v);
    }

    fn finalize_for_query(&mut self) {
        run_parallel_cs(&self.buf, self.workers);
    }

    fn query(&self, _q: i64) -> u64 {
        0
    }

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

impl crate::cell::ParallelInit for ParallelHllFastPath {
    fn build(config: &ParamSet, workers: usize) -> Result<Self, BuildError> {
        let _p: HllParams = config.parse()?;
        Ok(Self {
            buf: Vec::new(),
            workers: workers.max(1),
        })
    }
}

impl Sketch for ParallelHllFastPath {
    type Item = i64;
    type Query = ();
    type Answer = f64;

    #[inline(always)]
    fn update(&mut self, v: &i64) {
        self.buf.push(*v);
    }

    fn finalize_for_query(&mut self) {
        run_parallel_hll(&self.buf, self.workers);
    }

    fn query(&self, _q: ()) -> f64 {
        0.0
    }

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

fn run_parallel_cms(items: &[i64], workers: usize) -> u128 {
    let parts = partition(items, workers);
    let barrier = Barrier::new(parts.len());
    std::thread::scope(|s| {
        let handles: Vec<_> = parts
            .iter()
            .map(|part| {
                let barrier = &barrier;
                s.spawn(move || {
                    let mut sketch = CountMin::<M5x32K, FastPath>::from_storage(M5x32K::default());
                    barrier.wait();
                    let start = Instant::now();
                    for &v in *part {
                        sketch.insert_emit_delta(&DataInput::I64(v), &mut |d| {
                            std::hint::black_box(&d);
                        });
                    }
                    std::hint::black_box(&sketch);
                    start.elapsed().as_nanos()
                })
            })
            .collect();
        handles
            .into_iter()
            .map(|h| h.join().unwrap())
            .max()
            .unwrap_or(0)
    })
}

fn run_parallel_cs(items: &[i64], workers: usize) -> u128 {
    let parts = partition(items, workers);
    let barrier = Barrier::new(parts.len());
    std::thread::scope(|s| {
        let handles: Vec<_> = parts
            .iter()
            .map(|part| {
                let barrier = &barrier;
                s.spawn(move || {
                    let mut sketch = Count::<M5x32K, FastPath>::from_storage(M5x32K::default());
                    barrier.wait();
                    let start = Instant::now();
                    for &v in *part {
                        sketch.insert_emit_delta(&DataInput::I64(v), &mut |d| {
                            std::hint::black_box(&d);
                        });
                    }
                    std::hint::black_box(&sketch);
                    start.elapsed().as_nanos()
                })
            })
            .collect();
        handles
            .into_iter()
            .map(|h| h.join().unwrap())
            .max()
            .unwrap_or(0)
    })
}

fn run_parallel_hll(items: &[i64], workers: usize) -> u128 {
    let parts = partition(items, workers);
    let barrier = Barrier::new(parts.len());
    std::thread::scope(|s| {
        let handles: Vec<_> = parts
            .iter()
            .map(|part| {
                let barrier = &barrier;
                s.spawn(move || {
                    let mut sketch = HyperLogLog::<ErtlMLE>::default();
                    barrier.wait();
                    let start = Instant::now();
                    for &v in *part {
                        sketch.insert_emit_delta(&DataInput::I64(v), &mut |d| {
                            std::hint::black_box(&d);
                        });
                    }
                    std::hint::black_box(&sketch);
                    start.elapsed().as_nanos()
                })
            })
            .collect();
        handles
            .into_iter()
            .map(|h| h.join().unwrap())
            .max()
            .unwrap_or(0)
    })
}
