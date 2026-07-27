//! `Exporter` trait + concrete sinks (`stdout`, `file`, `grpc`, `noop`,
//! `fanout`). Deliberately simple — one `export(&Record)`, no `Result` on the
//! hot path — because embedded benchmarking must NEVER panic the host app, so
//! impls log and drop on error. `&self` lets one `Arc<dyn Exporter>` be shared.

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

use aqpbm_core::report::Record;

/// Handler for the JSONL records produced by the embedded
/// [`Sampler`](crate::sampler::Sampler). Impls must be `Send + Sync` so one
/// `Arc<dyn Exporter>` spans many threads; errors are logged and swallowed.
pub trait Exporter: Send + Sync {
    fn export(&self, record: &Record);
}

impl<E: Exporter + ?Sized> Exporter for std::sync::Arc<E> {
    #[inline]
    fn export(&self, record: &Record) {
        (**self).export(record)
    }
}
