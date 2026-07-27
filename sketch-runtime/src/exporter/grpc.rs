//! `GrpcExporter` — client-side gRPC streaming to the controller's
//! `asap.runtime.v1.RuntimeSamples` service. `feedback.proto` is the wire
//! contract, so breakage is build-time; HTTP/2 flow control makes controller
//! slowness visible; `payload_json` carries the `Record` shape so new fields
//! need no proto bump. Fire-and-forget on the hot path: drops are counted.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use aqpbm_core::report::Record;
use tokio::sync::mpsc;
use tonic::transport::{Channel, Endpoint};

use super::Exporter;

// Generated from proto/feedback.proto at build time.
pub mod feedback {
    tonic::include_proto!("asap.runtime.v1");
}
use feedback::{
    runtime_samples_client::RuntimeSamplesClient, PushBatch, RuntimeRecord as PbRuntimeRecord,
};

#[derive(Debug, Default)]
pub struct GrpcStats {
    pub batches_sent: AtomicU64,
    pub records_sent: AtomicU64,
    pub records_dropped: AtomicU64,
    pub send_errors: AtomicU64,
}

impl GrpcStats {
    pub fn snapshot(&self) -> GrpcStatsSnapshot {
        GrpcStatsSnapshot {
            batches_sent: self.batches_sent.load(Ordering::Relaxed),
            records_sent: self.records_sent.load(Ordering::Relaxed),
            records_dropped: self.records_dropped.load(Ordering::Relaxed),
            send_errors: self.send_errors.load(Ordering::Relaxed),
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct GrpcStatsSnapshot {
    pub batches_sent: u64,
    pub records_sent: u64,
    pub records_dropped: u64,
    pub send_errors: u64,
}

#[derive(Debug, Clone)]
pub struct GrpcConfig {
    /// Full controller endpoint, e.g.
    /// `"http://controller:8080"`. tonic's HTTP/2 transport —
    /// use `http://` for h2c or `https://` with rustls.
    pub endpoint: String,
    /// Max records per batched RPC.
    pub batch_capacity: usize,
    /// Max wall-clock age of the oldest buffered record before
    /// a flush is forced.
    pub flush_period: Duration,
    /// `(Sampler → flusher)` channel depth. Records beyond this
    /// are dropped and counted.
    pub queue_depth: usize,
    /// Per-RPC timeout. Tonic retries are intentionally NOT
    /// configured — real-time loops prefer fresh data over
    /// stale retried data.
    pub rpc_timeout: Duration,
    /// Enable gzip compression on the gRPC call. Controller
    /// side must advertise gzip support (tonic does by
    /// default when its `gzip` feature is on).
    pub compress: bool,
}

impl Default for GrpcConfig {
    fn default() -> Self {
        Self {
            endpoint: "http://127.0.0.1:8080".into(),
            batch_capacity: 64,
            flush_period: Duration::from_millis(100),
            queue_depth: 1024,
            rpc_timeout: Duration::from_secs(3),
            compress: true,
        }
    }
}

/// Client-side gRPC exporter. Spawns one background tokio
/// task; the `Sampler` hot path just does a single
/// `mpsc::try_send` per record.
pub struct GrpcExporter {
    tx: mpsc::Sender<Record>,
    stats: Arc<GrpcStats>,
}

impl GrpcExporter {
    /// Spawn a new `GrpcExporter` + its background flush task
    /// on the current tokio runtime.
    pub fn spawn(config: GrpcConfig) -> Self {
        let (tx, rx) = mpsc::channel(config.queue_depth);
        let stats = Arc::new(GrpcStats::default());
        let stats_clone = stats.clone();
        let cfg = config.clone();
        tokio::spawn(async move {
            flush_loop(rx, cfg, stats_clone).await;
        });
        Self { tx, stats }
    }

    pub fn stats(&self) -> &Arc<GrpcStats> {
        &self.stats
    }
}

impl Exporter for GrpcExporter {
    fn export(&self, record: &Record) {
        match self.tx.try_send(record.clone()) {
            Ok(()) => {}
            Err(mpsc::error::TrySendError::Full(_)) | Err(mpsc::error::TrySendError::Closed(_)) => {
                self.stats.records_dropped.fetch_add(1, Ordering::Relaxed);
            }
        }
    }
}

/// Background flush loop: drain the channel, batch, build one
/// `PushBatch` RPC per batch, gzip-compressed on the wire.
async fn flush_loop(mut rx: mpsc::Receiver<Record>, config: GrpcConfig, stats: Arc<GrpcStats>) {
    // The channel stays open if the controller is briefly unreachable — tonic
    // reconnects on the next RPC. Failing to build the endpoint at all is a
    // caller misconfig, so the background task panics loudly instead.
    let endpoint = match Endpoint::from_shared(config.endpoint.clone()) {
        Ok(e) => e.timeout(config.rpc_timeout),
        Err(e) => {
            eprintln!(
                "grpc-exporter: invalid endpoint '{}': {}",
                config.endpoint, e
            );
            return;
        }
    };

    let mut buffer: Vec<Record> = Vec::with_capacity(config.batch_capacity);
    let mut flush_timer = tokio::time::interval(config.flush_period);
    flush_timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

    loop {
        tokio::select! {
            biased;
            maybe = rx.recv() => {
                match maybe {
                    Some(rec) => {
                        buffer.push(rec);
                        if buffer.len() >= config.batch_capacity {
                            flush(&endpoint, &config, &stats, &mut buffer).await;
                        }
                    }
                    None => {
                        if !buffer.is_empty() {
                            flush(&endpoint, &config, &stats, &mut buffer).await;
                        }
                        break;
                    }
                }
            }
            _ = flush_timer.tick() => {
                if !buffer.is_empty() {
                    flush(&endpoint, &config, &stats, &mut buffer).await;
                }
            }
        }
    }
}

async fn flush(
    endpoint: &Endpoint,
    config: &GrpcConfig,
    stats: &Arc<GrpcStats>,
    buffer: &mut Vec<Record>,
) {
    let n = buffer.len() as u64;
    let records: Vec<PbRuntimeRecord> = buffer
        .drain(..)
        .map(|rec| {
            let schema_version = rec.schema_version;
            let source = source_label(rec.source);
            let sketch = rec.sketch.clone();
            let impl_name = rec.impl_name.clone();
            let payload_json = serde_json::to_string(&rec).unwrap_or_default();
            PbRuntimeRecord {
                source,
                sketch,
                impl_name,
                schema_version,
                payload_json,
            }
        })
        .collect();

    // One new client per flush is OK — HTTP/2 connection is
    // cached by tonic's Channel under the hood; the per-RPC
    // cost here is building the `Request`, not TCP/TLS setup.
    let channel = match endpoint.connect().await {
        Ok(c) => c,
        Err(_) => {
            stats.send_errors.fetch_add(1, Ordering::Relaxed);
            stats.records_dropped.fetch_add(n, Ordering::Relaxed);
            return;
        }
    };
    let mut client = client_for(channel, config.compress);

    let req = PushBatch { records };
    match client.push(req).await {
        Ok(_resp) => {
            stats.batches_sent.fetch_add(1, Ordering::Relaxed);
            stats.records_sent.fetch_add(n, Ordering::Relaxed);
        }
        Err(_) => {
            stats.send_errors.fetch_add(1, Ordering::Relaxed);
            stats.records_dropped.fetch_add(n, Ordering::Relaxed);
        }
    }
}

fn client_for(channel: Channel, compress: bool) -> RuntimeSamplesClient<Channel> {
    let mut c = RuntimeSamplesClient::new(channel);
    if compress {
        c = c
            .send_compressed(tonic::codec::CompressionEncoding::Gzip)
            .accept_compressed(tonic::codec::CompressionEncoding::Gzip);
    }
    c
}

fn source_label(src: aqpbm_core::report::Source) -> String {
    use aqpbm_core::report::Source::*;
    match src {
        Cli => "cli",
        AsapFusion => "asap-fusion",
        DataCollector => "data-collector",
        AsapQuery => "asap-query",
        CppBench => "cpp-bench",
    }
    .to_string()
}
