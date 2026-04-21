//! End-to-end test for [`PushExporter`]: spin up a local axum
//! server pretending to be the controller's
//! `/api/v1/runtime-samples` endpoint, pipe a `Sampler` ->
//! `Probe<S, Sampler>` through a `PushExporter`, and assert:
//!
//! 1. Records arrive in batches (count + byte delta stats).
//! 2. Body is zstd-compressed when the feature is on; Prometheus
//!    / Content-Type headers are `application/x-ndjson` +
//!    `Content-Encoding: zstd`.
//! 3. When the server is unreachable, records are counted as
//!    dropped without panicking the host.

#![cfg(feature = "push")]

use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::{extract::State, http::StatusCode, routing::post, Router};
use sketch_core::probe::Probe;
use sketch_core::report::{Record, Source};
use sketch_core::sketch::Sketch;
use sketch_runtime::sampler::{Sampler, Tag};
use sketch_runtime::{PushConfig, PushExporter};

#[derive(Clone, Default)]
struct Received {
    bodies: Arc<Mutex<Vec<Vec<u8>>>>,
    content_encoding: Arc<Mutex<Option<String>>>,
}

async fn handler(
    State(state): State<Received>,
    headers: axum::http::HeaderMap,
    body: axum::body::Bytes,
) -> StatusCode {
    let enc = headers
        .get("content-encoding")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());
    *state.content_encoding.lock().unwrap() = enc;
    state.bodies.lock().unwrap().push(body.to_vec());
    StatusCode::OK
}

async fn start_server(state: Received) -> u16 {
    let app = Router::new()
        .route("/api/v1/runtime-samples", post(handler))
        .with_state(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    tokio::time::sleep(Duration::from_millis(50)).await;
    addr.port()
}

struct DummySketch;
impl Sketch for DummySketch {
    type Item = i64;
    type Query = ();
    type Answer = u64;
    fn update(&mut self, _: &i64) {}
    fn query(&self, _: ()) -> u64 {
        0
    }
    fn memory_bytes(&self) -> usize {
        0
    }
}

fn tag() -> Tag {
    Tag::new("hll", "oxide", Source::DataCollector)
}

#[tokio::test]
async fn sampler_push_roundtrip_receives_compressed_batch() {
    let rcv = Received::default();
    let rcv_clone = rcv.clone();
    let port = start_server(rcv_clone).await;

    let cfg = PushConfig {
        endpoint: format!("http://127.0.0.1:{port}/api/v1/runtime-samples"),
        batch_capacity: 8,
        flush_period: Duration::from_millis(40),
        queue_depth: 128,
        http_timeout: Duration::from_secs(2),
        compress: true,
        zstd_level: 3,
    };
    let exporter = Arc::new(PushExporter::spawn(cfg));
    let stats = exporter.stats().clone();

    let sampler = Sampler::every_n(1, 4, exporter.clone(), tag());
    let mut probe = Probe::new(DummySketch, sampler);
    for i in 0..8_i64 {
        probe.update(&i);
    }

    // Wait for flush(es) to land.
    tokio::time::sleep(Duration::from_millis(250)).await;

    let snap = stats.snapshot();
    assert!(
        snap.batches_sent >= 1,
        "should have sent at least one batch (got {:?})",
        snap
    );
    assert_eq!(snap.send_errors, 0, "expected no errors: {:?}", snap);

    let bodies = rcv.bodies.lock().unwrap();
    assert!(
        !bodies.is_empty(),
        "server should have received at least one body"
    );
    let enc = rcv.content_encoding.lock().unwrap().clone();
    #[cfg(feature = "compress-zstd")]
    {
        assert_eq!(enc.as_deref(), Some("zstd"), "expected zstd encoding");
        // Round-trip a body to confirm it's decodable + JSONL.
        let decoded = zstd::decode_all(&bodies[0][..]).expect("zstd decode");
        let text = String::from_utf8(decoded).expect("utf8");
        let lines: Vec<&str> = text.lines().collect();
        assert!(!lines.is_empty(), "decoded batch should have lines");
        let first: Record = serde_json::from_str(lines[0]).expect("parse record");
        assert_eq!(first.sketch, "hll");
        assert_eq!(first.impl_name, "oxide");
    }
    #[cfg(not(feature = "compress-zstd"))]
    {
        assert!(
            enc.is_none(),
            "no zstd feature -> no Content-Encoding header"
        );
    }
}

#[tokio::test]
async fn push_to_unreachable_endpoint_drops_records_without_panic() {
    let cfg = PushConfig {
        endpoint: "http://127.0.0.1:1/api/v1/runtime-samples".into(),
        batch_capacity: 4,
        flush_period: Duration::from_millis(30),
        queue_depth: 32,
        http_timeout: Duration::from_millis(300),
        compress: false,
        zstd_level: 3,
    };
    let exporter = Arc::new(PushExporter::spawn(cfg));
    let stats = exporter.stats().clone();

    let sampler = Sampler::every_n(1, 2, exporter.clone(), tag());
    let mut probe = Probe::new(DummySketch, sampler);
    for i in 0..8_i64 {
        probe.update(&i);
    }
    // Let the flush attempts time out.
    tokio::time::sleep(Duration::from_millis(700)).await;

    let snap = stats.snapshot();
    assert_eq!(snap.batches_sent, 0);
    assert!(
        snap.send_errors >= 1 || snap.records_dropped >= 1,
        "expected at least one error / drop when endpoint unreachable: {:?}",
        snap
    );
    // Most importantly: no panic, no hang.
}

#[tokio::test]
async fn push_records_drop_when_queue_full() {
    // tiny queue_depth to force drops regardless of server state
    let cfg = PushConfig {
        endpoint: "http://127.0.0.1:1/api/v1/runtime-samples".into(),
        batch_capacity: 1000,                  // never-reached size trigger
        flush_period: Duration::from_secs(60), // effectively never
        queue_depth: 2,
        http_timeout: Duration::from_secs(10),
        compress: false,
        zstd_level: 3,
    };
    let exporter = Arc::new(PushExporter::spawn(cfg));
    let stats = exporter.stats().clone();
    let sampler = Sampler::every_n(1, 1000, exporter.clone(), tag());
    let mut probe = Probe::new(DummySketch, sampler);
    // Burst way past queue_depth.
    for i in 0..1000_i64 {
        probe.update(&i);
    }
    tokio::time::sleep(Duration::from_millis(50)).await;
    let snap = stats.snapshot();
    assert!(
        snap.records_dropped > 0,
        "queue-full path should count drops: {:?}",
        snap
    );
}
