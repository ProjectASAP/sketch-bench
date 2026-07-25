//! End-to-end check for the `heap-track` allocator shim.
//!
//! Installs `TrackingAllocator(System)` as the test binary's
//! global allocator and asserts that an alloc/drop pair moves
//! `IN_USE` by approximately the right delta.
//!
//! The counters are process-global atomics — fine for a
//! single-threaded benchmark runner, but the cargo test harness
//! itself allocates on background paths (panic plumbing, output
//! capture) between any two snapshots. So we cannot assert
//! "after drop, in_use == before" in absolute terms; we assert
//! the *drop delta* is at least the size we just freed. The
//! BenchRunner's invariant ("any drift is a leak") holds because
//! it runs single-threaded with no other allocators of note.
//!
//! The test only compiles under the `heap-track` feature.
//!     cargo test -p sketch-bench --features heap-track --test heap_track_leak

#![cfg(feature = "heap-track")]

use aqpbm_core::metrics::heap_track::{reset_peak, snapshot, TrackingAllocator};

#[global_allocator]
static A: TrackingAllocator<std::alloc::System> = TrackingAllocator(std::alloc::System);

#[test]
fn alloc_then_drop_releases_bytes() {
    reset_peak();
    let before = snapshot();

    let v: Vec<u8> = vec![0u8; 1 << 16];
    let after_alloc = snapshot();
    assert!(
        after_alloc.in_use - before.in_use >= (1 << 16),
        "expected at least 64 KiB tracked; got delta={}",
        after_alloc.in_use - before.in_use,
    );
    assert!(after_alloc.peak >= after_alloc.in_use);

    drop(v);
    let after_drop = snapshot();
    assert!(
        after_alloc.in_use - after_drop.in_use >= (1 << 16),
        "drop did not release the buffer: pre-drop={} post-drop={}",
        after_alloc.in_use,
        after_drop.in_use,
    );
}

#[test]
fn realloc_grow_accounts_delta() {
    reset_peak();
    let small_start = snapshot();

    let mut v: Vec<u64> = Vec::with_capacity(8);
    let small = snapshot();
    for i in 0..4096 {
        v.push(i);
    }
    let big = snapshot();

    assert!(
        big.in_use - small.in_use >= (4096 * 8 - 64),
        "growth not tracked: small={} big={}",
        small.in_use,
        big.in_use,
    );
    assert!(big.peak >= big.in_use);

    drop(v);
    let after_drop = snapshot();
    assert!(
        big.in_use - after_drop.in_use >= (4096 * 8 - 64),
        "drop did not release the buffer",
    );
    let _ = small_start;
}
