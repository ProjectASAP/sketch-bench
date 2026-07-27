//! End-to-end test for [`GrpcExporter`]: spin up a local
//! tonic server implementing `asap.runtime.v1.RuntimeSamples`,
//! pipe a `Sampler` → `Probe<S, Sampler>` through a
//! `GrpcExporter`, assert the server receives batches with the
//! right tuple (source/sketch/impl) + payload JSON.

#![cfg(feature = "grpc")]

use std::sync::{Arc, Mutex};
use std::time::Duration;

use aqpbm_core::accumulator::Accumulator;
use aqpbm_core::probe::Probe;
use aqpbm_core::report::{Source, SCHEMA_VERSION};
use sketch_runtime::sampler::{Sampler, Tag};
use sketch_runtime::{GrpcConfig, GrpcExporter};

// Re-import the generated proto the same way the exporter
// does. Test uses the same generated module.
pub mod feedback {
    tonic::include_proto!("asap.runtime.v1");
}
use feedback::{
    runtime_samples_server::{RuntimeSamples, RuntimeSamplesServer},
    PushAck, PushBatch,
};

#[derive(Default)]
struct CapturingService {
    batches: Arc<Mutex<Vec<PushBatch>>>,
}

#[tonic::async_trait]
impl RuntimeSamples for CapturingService {
    async fn push(
        &self,
        request: tonic::Request<PushBatch>,
    ) -> Result<tonic::Response<PushAck>, tonic::Status> {
        let batch = request.into_inner();
        let n = batch.records.len() as u64;
        self.batches.lock().unwrap().push(batch);
        Ok(tonic::Response::new(PushAck { accepted: n }))
    }
}

struct DummySketch;
impl Accumulator for DummySketch {
    type Item = i64;
    fn update(&mut self, _: &i64) {}
}

async fn spin_server() -> (String, Arc<Mutex<Vec<PushBatch>>>) {
    let batches: Arc<Mutex<Vec<PushBatch>>> = Arc::new(Mutex::new(Vec::new()));
    let svc = CapturingService {
        batches: Arc::clone(&batches),
    };
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let stream = tokio_stream::wrappers::TcpListenerStream::new(listener);
    tokio::spawn(async move {
        tonic::transport::Server::builder()
            .add_service(
                RuntimeSamplesServer::new(svc)
                    .accept_compressed(tonic::codec::CompressionEncoding::Gzip)
                    .send_compressed(tonic::codec::CompressionEncoding::Gzip),
            )
            .serve_with_incoming(stream)
            .await
            .unwrap();
    });
    // Let tonic finish binding.
    tokio::time::sleep(Duration::from_millis(80)).await;
    (format!("http://{addr}"), batches)
}

fn tag() -> Tag {
    Tag::new("hll", "oxide", Source::DataCollector)
}

#[tokio::test]
async fn grpc_exporter_pushes_compressed_batch_to_server() {
    let (endpoint, batches) = spin_server().await;
    let cfg = GrpcConfig {
        endpoint,
        batch_capacity: 4,
        flush_period: Duration::from_millis(40),
        queue_depth: 128,
        rpc_timeout: Duration::from_secs(2),
        compress: true,
    };
    let exporter = Arc::new(GrpcExporter::spawn(cfg));
    let stats = exporter.stats().clone();

    let sampler = Sampler::every_n(1, 4, exporter.clone(), tag());
    let mut probe = Probe::new(DummySketch, sampler);
    for i in 0..8_i64 {
        probe.update(&i);
    }
    tokio::time::sleep(Duration::from_millis(300)).await;

    let snap = stats.snapshot();
    assert!(
        snap.batches_sent >= 1,
        "expected ≥1 batch sent, got {:?}",
        snap
    );
    assert_eq!(snap.send_errors, 0, "unexpected errors: {:?}", snap);

    let b = batches.lock().unwrap();
    assert!(!b.is_empty(), "server received no batches");
    let first = &b[0];
    assert!(!first.records.is_empty());
    let rec = &first.records[0];
    assert_eq!(rec.sketch, "hll");
    assert_eq!(rec.impl_name, "oxide");
    assert_eq!(rec.source, "data-collector");
    // Against the constant, not a literal: what this test owns is that
    // the exporter *passes the version through* — same number on the
    // wire, in the proto field and inside the payload. Pinning the
    // number itself is a separate job, done by
    // `aqpbm_core::report::tests::schema_version_is_v2`, which
    // explains why it matters. A literal here duplicated that gate
    // badly: it went red on the 1 → 2 bump with nothing to say for
    // itself, and got written off as a broken test.
    assert_eq!(rec.schema_version, SCHEMA_VERSION);
    // payload_json is the full Record — parse it back to prove.
    let parsed: serde_json::Value = serde_json::from_str(&rec.payload_json).unwrap();
    assert_eq!(parsed["schema_version"], SCHEMA_VERSION);
    assert_eq!(parsed["sketch"], "hll");
}

#[tokio::test]
async fn grpc_exporter_unreachable_endpoint_drops_without_panic() {
    // Point at a port nothing's listening on.
    let cfg = GrpcConfig {
        endpoint: "http://127.0.0.1:1".into(),
        batch_capacity: 2,
        flush_period: Duration::from_millis(30),
        queue_depth: 32,
        rpc_timeout: Duration::from_millis(300),
        compress: false,
    };
    let exporter = Arc::new(GrpcExporter::spawn(cfg));
    let stats = exporter.stats().clone();
    let sampler = Sampler::every_n(1, 2, exporter.clone(), tag());
    let mut probe = Probe::new(DummySketch, sampler);
    for i in 0..8_i64 {
        probe.update(&i);
    }
    tokio::time::sleep(Duration::from_millis(700)).await;
    let snap = stats.snapshot();
    assert_eq!(snap.batches_sent, 0);
    assert!(
        snap.send_errors >= 1 || snap.records_dropped >= 1,
        "unreachable endpoint should drop / error, got {:?}",
        snap
    );
}

#[tokio::test]
async fn grpc_exporter_queue_full_drops_records() {
    let cfg = GrpcConfig {
        endpoint: "http://127.0.0.1:1".into(),
        batch_capacity: 10_000, // never-reached
        flush_period: Duration::from_secs(60),
        queue_depth: 2,
        rpc_timeout: Duration::from_secs(10),
        compress: false,
    };
    let exporter = Arc::new(GrpcExporter::spawn(cfg));
    let stats = exporter.stats().clone();
    let sampler = Sampler::every_n(1, 1000, exporter.clone(), tag());
    let mut probe = Probe::new(DummySketch, sampler);
    for i in 0..1000_i64 {
        probe.update(&i);
    }
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert!(
        stats.snapshot().records_dropped > 0,
        "expected drops when queue full"
    );
}
