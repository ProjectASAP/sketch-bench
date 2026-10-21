//! Instant queries through the Prometheus HTTP API, which both Prometheus and
//! the data plane serve.

use std::collections::BTreeMap;

use anyhow::Context;
use aqpbm_core::metrics::WallClock;

/// One `GET /api/v1/query` round trip.
#[derive(Debug, Clone)]
pub struct Answer {
    pub status: u16,
    /// The data plane's `x-asap-execution` header (`plan`, `refused`,
    /// `forwarded`); `None` from Prometheus.
    pub execution: Option<String>,
    /// The response's `error` field, if any.
    pub error: Option<String>,
    /// `instance` label -> value, for a vector result.
    pub values: BTreeMap<String, f64>,
    /// Request sent to body fully read, client side.
    pub latency_ms: f64,
}

pub fn instant_query(
    client: &reqwest::blocking::Client,
    base: &str,
    query: &str,
    time_ms: i64,
) -> anyhow::Result<Answer> {
    let time = format!("{:.3}", time_ms as f64 / 1000.0);
    let clock = WallClock::start();
    let response = client
        .get(format!("{base}/api/v1/query"))
        .query(&[("query", query), ("time", time.as_str())])
        .send()
        .with_context(|| format!("GET {base}/api/v1/query"))?;
    let status = response.status().as_u16();
    let execution = response
        .headers()
        .get("x-asap-execution")
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);
    let body = response.bytes()?;
    let latency_ms = clock.elapsed_ns() as f64 / 1e6;

    let json: serde_json::Value = serde_json::from_slice(&body).with_context(|| {
        format!(
            "{base} answered {status} with a body that is not JSON: {}",
            String::from_utf8_lossy(&body)
        )
    })?;
    let error = json
        .get("error")
        .and_then(|e| e.as_str())
        .map(str::to_owned);
    let mut values = BTreeMap::new();
    if let Some(result) = json.pointer("/data/result").and_then(|r| r.as_array()) {
        for item in result {
            let instance = item
                .pointer("/metric/instance")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_owned();
            let value = item
                .pointer("/value/1")
                .and_then(|v| v.as_str())
                .and_then(|v| v.parse::<f64>().ok())
                .unwrap_or(f64::NAN);
            values.insert(instance, value);
        }
    }
    Ok(Answer {
        status,
        execution,
        error,
        values,
        latency_ms,
    })
}

/// GET `url` and parse its JSON body, or `null` if that fails.
pub fn get_json(client: &reqwest::blocking::Client, url: &str) -> serde_json::Value {
    client
        .get(url)
        .send()
        .and_then(|r| r.json::<serde_json::Value>())
        .unwrap_or(serde_json::Value::Null)
}
