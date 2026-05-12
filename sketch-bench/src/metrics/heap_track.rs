//! Per-sketch heap tracking via a counting `GlobalAlloc` shim.
//!
//! Wraps an inner allocator (System or jemalloc) and records
//! `IN_USE` / `PEAK` byte counters on every alloc/dealloc/realloc.
//! The runner takes a `snapshot()` before constructing a sketch
//! and after dropping it, so the delta isolates that sketch's
//! heap footprint from anything else the process is doing.
//!
//! Counters are process-global atomics. The benchmark runner is
//! single-threaded by contract (see `BenchRunner`), so contention
//! is nil and a global counter is precise *for the sketch under
//! test* — a background thread allocating during the measurement
//! window would pollute the numbers, which is why this module
//! must be paired with a single-threaded driver.
//!
//! Cost: two relaxed atomic ops per alloc/dealloc. Off by default
//! — opt in via the `heap-track` Cargo feature on the linking
//! binary, then install `TrackingAllocator` as `#[global_allocator]`.

use std::alloc::{GlobalAlloc, Layout};
use std::sync::atomic::{AtomicI64, Ordering};

/// Currently-allocated bytes attributed to this process by the
/// shim. Signed so transient drift (rare on x86-64, but possible
/// during torn deallocs across feature-gated paths) is visible
/// rather than silently wrapping.
pub static IN_USE: AtomicI64 = AtomicI64::new(0);

/// High-water mark of `IN_USE` since the last `reset_peak()`.
pub static PEAK: AtomicI64 = AtomicI64::new(0);

/// Wraps any `GlobalAlloc`, forwarding every call while updating
/// `IN_USE` / `PEAK`. Use as `#[global_allocator]` in the bin
/// crate, e.g.
///
/// ```ignore
/// #[global_allocator]
/// static A: TrackingAllocator<std::alloc::System> =
///     TrackingAllocator(std::alloc::System);
/// ```
pub struct TrackingAllocator<A>(pub A);

unsafe impl<A: GlobalAlloc> GlobalAlloc for TrackingAllocator<A> {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let p = unsafe { self.0.alloc(layout) };
        if !p.is_null() {
            account(layout.size() as i64);
        }
        p
    }
    unsafe fn dealloc(&self, p: *mut u8, layout: Layout) {
        unsafe { self.0.dealloc(p, layout) };
        IN_USE.fetch_sub(layout.size() as i64, Ordering::Relaxed);
    }
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        let p = unsafe { self.0.alloc_zeroed(layout) };
        if !p.is_null() {
            account(layout.size() as i64);
        }
        p
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        // Delegate to the inner allocator's realloc so any in-place
        // expansion fast path is preserved; account only the delta.
        let p = unsafe { self.0.realloc(ptr, layout, new_size) };
        if !p.is_null() {
            account(new_size as i64 - layout.size() as i64);
        }
        p
    }
}

#[inline]
fn account(delta: i64) {
    let after = IN_USE.fetch_add(delta, Ordering::Relaxed) + delta;
    if delta > 0 {
        PEAK.fetch_max(after, Ordering::Relaxed);
    }
}

/// Snapshot of the global counters. Cheap; two relaxed loads.
#[derive(Copy, Clone, Debug)]
pub struct Snapshot {
    pub in_use: i64,
    pub peak: i64,
}

pub fn snapshot() -> Snapshot {
    Snapshot {
        in_use: IN_USE.load(Ordering::Relaxed),
        peak: PEAK.load(Ordering::Relaxed),
    }
}

/// Pin `PEAK` to the current `IN_USE`. Call before a measurement
/// window so the peak reported afterwards reflects only that
/// window's transient highs, not historical ones.
pub fn reset_peak() {
    PEAK.store(IN_USE.load(Ordering::Relaxed), Ordering::Relaxed);
}

#[cfg(test)]
mod tests {
    use super::*;

    // These tests don't install the shim as the global allocator
    // (the test binary uses whatever the harness picked); they
    // exercise the counter accounting directly.

    #[test]
    fn account_updates_in_use_and_peak() {
        let before = snapshot();
        account(1024);
        let after = snapshot();
        assert_eq!(after.in_use - before.in_use, 1024);
        assert!(after.peak >= after.in_use);
        account(-1024);
        assert_eq!(snapshot().in_use, before.in_use);
    }
}
