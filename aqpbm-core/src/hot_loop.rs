//! CPU warm-up + **the** timed insert loop — the mechanical core a runner
//! drives, sitting here in `aqpbm-core` rather than in any one bench domain.
//!
//! Two of the `aqpbm-core` targets in `docs/component_walk_through.md` live
//! here: "warm-up of cpu" ([`warmup_cpu_once`]) and the "timing functionality"
//! of the hot path ([`insert_loop`]). Both are generic over the [`Sketch`]
//! trait — which already lives in this crate — so moving them in introduces no
//! new dependency.
//!
//! ## Why moving the hot loop across a crate boundary is inlining-safe
//!
//! [`insert_loop`] is the one function every throughput measurement times, and
//! it must fold the caller's `insert` closure (the wrapper's `update`) into the
//! loop or the number is wrong — see the 5.1% history on [`insert_loop`]
//! itself. That fold **already** depended on `#[inline(always)]` + thin LTO,
//! not on living in the same crate as the wrapper: the closure is monomorphised
//! at the call site in the bench crate and passed in generically, and thin LTO
//! (`lto = "thin"`, `codegen-units = 1` in the workspace release profile)
//! inlines it across the crate boundary at link time. The boundary between
//! `aqpbm-core` and the bench crate is therefore no different from the boundary
//! the closure already crossed. Keeping the `#[inline(always)]` attribute and
//! the single-copy structure is what preserves the guarantee; the crate it is
//! compiled in does not enter into it.

use std::sync::Once;
use std::time::{Duration, Instant};

use crate::sketch::Sketch;

/// Ramp the CPU **once per process**, before the first measured loop of any
/// pass.
///
/// This used to be called from the throughput fast path only, which made the
/// warm-up an accident of which `--metrics` flags were passed: a THROUGHPUT
/// pass that also carried the CPU/MEMORY bits took the other branch and was
/// timed with no governor ramp at all. Two passes measured at two different
/// clock states is not a comparison. Gating on `Once` also stops each of a
/// cell's metric passes from re-burning the warm-up duration — the ramp only
/// the first pass in the process actually needs.
pub fn warmup_cpu_once() {
    static WARMED: Once = Once::new();
    WARMED.call_once(warmup_cpu_from_env);
}

/// Burn CPU on the current core so the cpufreq governor ramps to max turbo
/// before timing starts. External shell warmups don't work reliably because
/// the governor can drop frequency during the bench process's exec/startup
/// window.
///
/// Duration is read from `BENCH_WARMUP_SECS`. **Defaults to 0** — a library
/// must not burn ten seconds of a caller's CPU because it was linked. The
/// measurement default lives in `sketchlib`'s `main`, which sets the variable
/// when the operator hasn't; every integration test and downstream embedder
/// therefore pays nothing. (`cfg!(test)` cannot express this: an integration
/// test links this crate as a plain dependency, compiled without `cfg(test)`.)
fn warmup_cpu_from_env() {
    let secs: u64 = std::env::var("BENCH_WARMUP_SECS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    if secs == 0 {
        return;
    }
    let deadline = Instant::now() + Duration::from_secs(secs);
    let mut x: u64 = 0xdeadbeef;
    while Instant::now() < deadline {
        for _ in 0..10_000 {
            x = x
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
        }
        std::hint::black_box(x);
    }
}

/// **The** insert loop. Times `insert` over every item and returns the
/// elapsed nanoseconds.
///
/// Every pass that reports throughput goes through here, and nothing else
/// is inside the timed region — no metric snapshot, no `finalize_for_query`,
/// no `memory_bytes`. This function existing exactly once is a correctness
/// property, not tidiness:
///
/// The throughput fast path and the CPU/MEMORY path used to carry their own
/// copies of this loop, one calling a closure supplied by `aqpbm-cli` and
/// the other calling `sketch.update(it)` from inside `sketch-bench`. Both
/// timed the right region, so the bug was invisible to review — but the
/// cross-crate call in the second copy cost the asap_sketchlib FixedMatrix
/// FastPath its inlining, and `cms/lib-fixedmatrix-fast-32k` reported
/// **5.1% lower throughput** under `--metrics throughput,cpu,memory` than
/// under `--metrics throughput` (5 alternating rounds, non-overlapping
/// ranges). The penalty scaled with how much an implementation relies on
/// inlining — ~1.5% for `hll/oxide` — so it did not cancel out: it changed
/// the ranking *between* implementations, and the slow path is the one the
/// default `--metrics` selects.
#[inline(always)]
pub fn insert_loop<S, Insert>(sketch: &mut S, items: &[S::Item], insert: &mut Insert) -> u64
where
    S: Sketch,
    Insert: FnMut(&mut S, &S::Item),
{
    let start = Instant::now();
    for it in items {
        insert(sketch, it);
    }
    let ns = start.elapsed().as_nanos() as u64;
    std::hint::black_box(&*sketch);
    ns
}
