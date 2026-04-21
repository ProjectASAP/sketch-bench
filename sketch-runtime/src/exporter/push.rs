//! `PushExporter` — HTTP POST a batched JSONL body (optionally
//! zstd-compressed) to the controller's
//! `/api/v1/runtime-samples` endpoint.
//!
//! Why HTTP POST and not gRPC: the controller already exposes
//! HTTP for plan control, and a single batched POST body is
//! schema-evolution-friendly (the `Record` type in `sketch-core`
//! carries `schema_version: 1`). A full `tonic` / `.proto` stack
//! adds 100+ transitive crates for marginal wire-efficiency
//! gains vs. `JSONL + zstd`. See `TODO.md`.
//!
//! # Delivery semantics
//!
//! * Fire-and-forget: the `Sampler` hot path enqueues records;
//!   an async flusher sends them out-of-band.
//! * Batch trigger: whichever of `batch_capacity` (record count)
//!   or `flush_period` (wall time) fires first. Default 64
//!   records / 100 ms — low enough floor that decision latency
//!   in the controller's real-time loop isn't bottlenecked here.
//! * Drop-policy: if the send-channel is full (controller
//!   unreachable + `queue_depth` exceeded), new records are
//!   dropped and counted in `PUSH_DROPPED_TOTAL`. Hot path
//!   never blocks.
//! * Compression: when the `compress-zstd` feature is on,
//!   request bodies are zstd-level-3 compressed with
//!   `Content-Encoding: zstd` header so the server can decode.
//!
//! # Counters
//!
//! * `sketchruntime_push_batches_sent_total`
//! * `sketchruntime_push_bytes_sent_total`
//! * `sketchruntime_push_dropped_total`
//! * `sketchruntime_push_errors_total`

use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc,
};
use std::time::Duration;

use sketch_core::report::Record;
use tokio::sync::mpsc;

use super::Exporter;

/// Process-wide counters. Simple atomics keep the hot path
/// allocation-free; a Prometheus-style exposer can scrape them
/// via `push_stats()`.
#[derive(Debug, Default)]
pub struct PushStats {
    pub batches_sent: AtomicU64,
    pub bytes_sent: AtomicU64,
    pub records_sent: AtomicU64,
    pub records_dropped: AtomicU64,
    pub send_errors: AtomicU64,
}

impl PushStats {
    pub fn snapshot(&self) -> PushStatsSnapshot {
        PushStatsSnapshot {
            batches_sent: self.batches_sent.load(Ordering::Relaxed),
            bytes_sent: self.bytes_sent.load(Ordering::Relaxed),
            records_sent: self.records_sent.load(Ordering::Relaxed),
            records_dropped: self.records_dropped.load(Ordering::Relaxed),
            send_errors: self.send_errors.load(Ordering::Relaxed),
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct PushStatsSnapshot {
    pub batches_sent: u64,
    pub bytes_sent: u64,
    pub records_sent: u64,
    pub records_dropped: u64,
    pub send_errors: u64,
}

#[derive(Debug, Clone)]
pub struct PushConfig {
    /// Full URL, e.g. `"http://controller.svc:8080/api/v1/runtime-samples"`.
    pub endpoint: String,
    /// Max records per batched POST.
    pub batch_capacity: usize,
    /// Max wall-clock age of the oldest buffered record before
    /// flush is forced.
    pub flush_period: Duration,
    /// `(Sampler → flusher)` channel depth. Beyond this, records
    /// are dropped and counted.
    pub queue_depth: usize,
    /// Per-POST HTTP timeout. On expiry the batch is counted as
    /// `send_errors` and dropped (no retry — real-time loops
    /// prefer fresh data over stale retried data).
    pub http_timeout: Duration,
    /// Compress the request body with zstd + set
    /// `Content-Encoding: zstd`. Requires the `compress-zstd`
    /// feature (on by default).
    pub compress: bool,
    /// zstd compression level (1-22). Level 3 is the sweet spot
    /// for JSON-heavy observability payloads (≈4× ratio,
    /// ≈500 MB/s encoder).
    pub zstd_level: i32,
}

impl Default for PushConfig {
    fn default() -> Self {
        Self {
            endpoint: "http://127.0.0.1:8080/api/v1/runtime-samples".into(),
            batch_capacity: 64,
            flush_period: Duration::from_millis(100),
            queue_depth: 1024,
            http_timeout: Duration::from_secs(3),
            compress: true,
            zstd_level: 3,
        }
    }
}

/// `Exporter` impl that streams batches to a remote HTTP
/// collector. Spawns one background tokio task for the flush
/// loop; construction requires an active tokio runtime.
pub struct PushExporter {
    tx: mpsc::Sender<Record>,
    stats: Arc<PushStats>,
}

impl PushExporter {
    /// Spawn a new `PushExporter` + its background flush task on
    /// the current tokio runtime. Returns an exporter ready to
    /// accept records via the [`Exporter`] trait.
    pub fn spawn(config: PushConfig) -> Self {
        let (tx, rx) = mpsc::channel(config.queue_depth);
        let stats = Arc::new(PushStats::default());
        let stats_clone = stats.clone();
        let config_clone = config.clone();
        tokio::spawn(async move {
            flush_loop(rx, config_clone, stats_clone).await;
        });
        Self { tx, stats }
    }

    pub fn stats(&self) -> &Arc<PushStats> {
        &self.stats
    }
}

impl Exporter for PushExporter {
    fn export(&self, record: &Record) {
        // try_send is non-blocking — if the queue is full, drop
        // and count. The Sampler hot path MUST NOT block.
        match self.tx.try_send(record.clone()) {
            Ok(()) => {}
            Err(mpsc::error::TrySendError::Full(_)) => {
                self.stats.records_dropped.fetch_add(1, Ordering::Relaxed);
            }
            Err(mpsc::error::TrySendError::Closed(_)) => {
                self.stats.records_dropped.fetch_add(1, Ordering::Relaxed);
            }
        }
    }
}

/// Background flush loop: drains the channel, batches records,
/// POSTs them. One task per `PushExporter`.
async fn flush_loop(mut rx: mpsc::Receiver<Record>, config: PushConfig, stats: Arc<PushStats>) {
    let client = match reqwest::Client::builder()
        .timeout(config.http_timeout)
        .build()
    {
        Ok(c) => c,
        Err(_) => {
            // Best-effort — if we can't even build the client,
            // mark all upcoming records as errored and exit. The
            // exporter handle will still accept records (they
            // land in the channel, get dropped when full) but
            // no HTTP work will happen.
            stats.send_errors.fetch_add(1, Ordering::Relaxed);
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
                            flush(&client, &config, &stats, &mut buffer).await;
                        }
                    }
                    None => {
                        // Channel closed — drain + final flush.
                        if !buffer.is_empty() {
                            flush(&client, &config, &stats, &mut buffer).await;
                        }
                        break;
                    }
                }
            }
            _ = flush_timer.tick() => {
                if !buffer.is_empty() {
                    flush(&client, &config, &stats, &mut buffer).await;
                }
            }
        }
    }
}

async fn flush(
    client: &reqwest::Client,
    config: &PushConfig,
    stats: &Arc<PushStats>,
    buffer: &mut Vec<Record>,
) {
    let n = buffer.len();
    // Serialise to JSONL (one record per line).
    let mut body = Vec::with_capacity(n * 512);
    for rec in buffer.drain(..) {
        let line = rec.to_jsonl();
        body.extend_from_slice(line.as_bytes());
        body.push(b'\n');
    }

    let (payload, compressed) = maybe_compress(&body, config);
    let content_length = payload.len() as u64;

    let mut req = client.post(&config.endpoint).body(payload);
    if compressed {
        req = req.header("Content-Encoding", "zstd");
    }
    req = req.header("Content-Type", "application/x-ndjson");

    match req.send().await {
        Ok(resp) if resp.status().is_success() => {
            stats.batches_sent.fetch_add(1, Ordering::Relaxed);
            stats.records_sent.fetch_add(n as u64, Ordering::Relaxed);
            stats
                .bytes_sent
                .fetch_add(content_length, Ordering::Relaxed);
        }
        Ok(_) | Err(_) => {
            // No retry — real-time loops prefer fresh data.
            stats.send_errors.fetch_add(1, Ordering::Relaxed);
            stats.records_dropped.fetch_add(n as u64, Ordering::Relaxed);
        }
    }
}

#[cfg(feature = "compress-zstd")]
fn maybe_compress(body: &[u8], config: &PushConfig) -> (Vec<u8>, bool) {
    if !config.compress {
        return (body.to_vec(), false);
    }
    match zstd::encode_all(body, config.zstd_level) {
        Ok(compressed) => (compressed, true),
        Err(_) => (body.to_vec(), false),
    }
}

#[cfg(not(feature = "compress-zstd"))]
fn maybe_compress(body: &[u8], _config: &PushConfig) -> (Vec<u8>, bool) {
    (body.to_vec(), false)
}
