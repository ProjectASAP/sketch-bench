//! CPU warm-up + **the** timed insert loop — the mechanical core a runner drives.
//!
//! [`insert_loop`] must fold the caller's `insert` closure into the loop or the
//! number is wrong. Thin LTO (`lto = "thin"`, `codegen-units = 1`) inlines it
//! across the crate boundary; `#[inline(always)]` and one copy preserve that.

use std::sync::Once;
use std::time::{Duration, Instant};

use crate::accumulator::Accumulator;

/// Ramp the CPU **once per process**, before the first measured loop of any
/// pass — two passes timed at two different clock states is not a comparison.
/// `Once` also stops a cell's later passes from re-burning the warm-up.
pub fn warmup_cpu_once() {
    static WARMED: Once = Once::new();
    WARMED.call_once(warmup_cpu_from_env);
}

/// Burn CPU so the cpufreq governor ramps to max turbo before timing starts.
/// Duration from `BENCH_WARMUP_SECS`, **defaulting to 0** — a library must not
/// burn a caller's CPU; `sketchlib`'s `main` sets the measurement default.
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

/// **The** insert loop. Times `insert` over every item, returns elapsed ns.
/// Nothing else is inside the timed region — no snapshot, no `prepare`. One
/// copy only: a duplicate calling across a crate boundary costs 5.1%.
#[inline(always)]
pub fn insert_loop<S, Insert>(sketch: &mut S, items: &[S::Item], insert: &mut Insert) -> u64
where
    S: Accumulator,
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
