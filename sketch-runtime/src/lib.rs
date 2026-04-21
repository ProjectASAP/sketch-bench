//! `sketch-runtime` — the embedded half of sketchlib-tool.
//!
//! Downstream apps (asap-fusion, DataCollector, ASAPQuery) wrap
//! their live sketches in a `Probe<S, Sampler>` so throughput /
//! latency samples flow to an `Exporter` (stdout / file / gRPC /
//! prometheus) in the same v1 JSONL shape the offline
//! `sketchlib bench` CLI produces. See `docs/DESIGN.md` §7.1.
//!
//! # Enabling / disabling
//!
//! There are **three** independent off-switches. Pick the one
//! whose cost / ergonomics fit:
//!
//! | Level | How | Hot-path cost |
//! |---|---|---|
//! | Compile-time | `sketch-runtime = { default-features = false }` | 0 (four `#[inline(always)]` no-ops) |
//! | Construction-time | `Sampler::disabled(exporter, tag)` | 1 cold branch per op |
//! | Runtime | `RuntimeSwitch::disable()` | 1 `Relaxed` atomic load per op |
//!
//! Additionally the sampling rate itself is a dial — a
//! `Sampler::every_n(u32::MAX, ...)` effectively never emits
//! until the controller turns it down.

pub mod exporter;
pub mod switch;

// Compile-time disable: when the `enabled` feature is off, the
// public `Sampler` is a zero-sized newtype whose `MetricsSink`
// impl is four empty methods the compiler inlines away.
#[cfg(feature = "enabled")]
pub mod sampler;
#[cfg(not(feature = "enabled"))]
#[path = "noop.rs"]
pub mod sampler;

pub use exporter::{Exporter, FanOutExporter, FileExporter, NoopExporter, StdoutExporter};
#[cfg(feature = "grpc")]
pub use exporter::{GrpcConfig, GrpcExporter, GrpcStats, GrpcStatsSnapshot};
pub use sampler::{Mode, Sampler, Tag};
pub use switch::RuntimeSwitch;
