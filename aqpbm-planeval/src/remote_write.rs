//! Prometheus Remote Write v1 sender: the protobuf messages, snappy framing
//! and headers a Remote Write client sends. Written here rather than taken
//! from the backend so the harness does not link the code under test.

use std::time::Duration;

use anyhow::{bail, Context};
use prost::Message;

#[derive(Clone, PartialEq, Message)]
pub struct WriteRequest {
    #[prost(message, repeated, tag = "1")]
    pub timeseries: Vec<TimeSeries>,
}

#[derive(Clone, PartialEq, Message)]
pub struct TimeSeries {
    #[prost(message, repeated, tag = "1")]
    pub labels: Vec<Label>,
    #[prost(message, repeated, tag = "2")]
    pub samples: Vec<Sample>,
}

#[derive(Clone, PartialEq, Message)]
pub struct Label {
    #[prost(string, tag = "1")]
    pub name: String,
    #[prost(string, tag = "2")]
    pub value: String,
}

#[derive(Clone, PartialEq, Message)]
pub struct Sample {
    #[prost(double, tag = "1")]
    pub value: f64,
    #[prost(int64, tag = "2")]
    pub timestamp: i64,
}

/// Snappy-compressed protobuf body of `request`.
pub fn encode(request: &WriteRequest) -> Vec<u8> {
    snap::raw::Encoder::new()
        .compress_vec(&request.encode_to_vec())
        .expect("snappy compresses any input")
}

/// POSTs one encoded body to `url` (a `/api/v1/write` endpoint). Retries a
/// 503 (the data plane's back-pressure answer) a bounded number of times, as
/// a Remote Write sender does; anything else that is not 2xx is an error.
pub fn send(client: &reqwest::blocking::Client, url: &str, body: &[u8]) -> anyhow::Result<()> {
    let mut backoff = Duration::from_millis(20);
    for _ in 0..50 {
        let response = client
            .post(url)
            .header("Content-Encoding", "snappy")
            .header("Content-Type", "application/x-protobuf")
            .header("X-Prometheus-Remote-Write-Version", "0.1.0")
            .body(body.to_vec())
            .send()
            .with_context(|| format!("POST {url}"))?;
        let status = response.status();
        if status.is_success() {
            return Ok(());
        }
        let text = response.text().unwrap_or_default();
        if status.as_u16() != 503 {
            bail!("POST {url}: {status}: {text}");
        }
        std::thread::sleep(backoff);
        backoff = (backoff * 2).min(Duration::from_secs(1));
    }
    bail!("POST {url}: still 503 after 50 attempts")
}
