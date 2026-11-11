//! `approxbench`, the approximate query processing benchmark suite.
//!
//! `sketchbench` measures one row of the sketch bundle, and `sketchbench
//! --list-impls` enumerates that bundle's `(variant, library)` pairs.

// Global allocator selection across the `heap-jemalloc` / `heap-track` feature
// pair: bare jemalloc, `TrackingAllocator` wrapping jemalloc or System, or the
// implicit System allocator. The static is what `#[global_allocator]` needs.
#[cfg(all(feature = "heap-jemalloc", not(feature = "heap-track")))]
#[global_allocator]
static GLOBAL: tikv_jemallocator::Jemalloc = tikv_jemallocator::Jemalloc;

#[cfg(all(feature = "heap-jemalloc", feature = "heap-track"))]
#[global_allocator]
static GLOBAL: aqpbm_core::metrics::heap_track::TrackingAllocator<tikv_jemallocator::Jemalloc> =
    aqpbm_core::metrics::heap_track::TrackingAllocator(tikv_jemallocator::Jemalloc);

#[cfg(all(feature = "heap-track", not(feature = "heap-jemalloc")))]
#[global_allocator]
static GLOBAL: aqpbm_core::metrics::heap_track::TrackingAllocator<std::alloc::System> =
    aqpbm_core::metrics::heap_track::TrackingAllocator(std::alloc::System);

use anyhow::Result;
use aqpbm_cli::cli::{Cli, Cmd};
use aqpbm_cli::{atomic_costs_cmd, erp_cmd, flatten_cmd, sketchbench};
use clap::Parser;

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Cmd::Sketchbench(args) => sketchbench::run_sketchbench(args),
        Cmd::AtomicCosts(args) => atomic_costs_cmd::run(args),
        Cmd::Erp(args) => erp_cmd::run(args),
        Cmd::Flatten(args) => flatten_cmd::run(args),
    }
}
