//! `sketch-runtime` — the embedded half of sketchlib-tool. Downstream apps wrap
//! live sketches in a `Probe<S, Sampler>` so samples flow to an `Exporter` in
//! the same JSONL shape the offline CLI produces. Three independent off-switches:
//! compile-time (`default-features = false`, free), construction-time
//! (`Sampler::disabled`, one cold branch), runtime (`RuntimeSwitch`, one load).

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
