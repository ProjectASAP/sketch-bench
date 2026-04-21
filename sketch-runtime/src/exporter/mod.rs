//! `Exporter` trait + concrete sinks (`stdout`, `file`, `grpc`,
//! `noop`, `fanout`).
//!
//! The trait is deliberately simple — one `export(&Record)`
//! method, no `Result` on the hot path — because embedded
//! benchmarking must NEVER panic the host app. Concrete impls
//! log and drop records on error rather than propagate.
//!
//! Interior mutability: `&self` (not `&mut self`) lets a
//! single `Arc<dyn Exporter>` be shared across many `Probe`s
//! without forcing the caller to wrap it themselves.

pub mod fanout;
pub mod file;
#[cfg(feature = "grpc")]
pub mod grpc;
pub mod noop;
pub mod stdout;

pub use fanout::FanOutExporter;
pub use file::FileExporter;
#[cfg(feature = "grpc")]
pub use grpc::{GrpcConfig, GrpcExporter, GrpcStats, GrpcStatsSnapshot};
pub use noop::NoopExporter;
pub use stdout::StdoutExporter;

use sketch_core::report::Record;

/// Handler for v1 JSONL records produced by the embedded
/// [`Sampler`](crate::sampler::Sampler).
///
/// Impls must be `Send + Sync` so one `Arc<dyn Exporter>` can be
/// shared across many worker threads / sketches. Errors are
/// logged internally and swallowed — the hot path never
/// propagates them.
pub trait Exporter: Send + Sync {
    fn export(&self, record: &Record);
}

impl<E: Exporter + ?Sized> Exporter for std::sync::Arc<E> {
    #[inline]
    fn export(&self, record: &Record) {
        (**self).export(record)
    }
}
