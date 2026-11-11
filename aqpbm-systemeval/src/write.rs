use std::collections::HashMap;
use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};
use aqpbm_planeval::data::Series;
use serde_json::{json, Value};

use crate::remote_write::{encode, Label, Sample, TimeSeries, WriteRequest};
use crate::System;

pub struct Options {
    pub batch_samples: usize,
    pub speed: Option<f64>,
    pub flush_ms: i64,
}

pub struct Writer<'a> {
    system: System,
    base: String,
    agent: ureq::Agent,
    options: &'a Options,
    start_ms: i64,
    order: Vec<(i64, u32, f64)>,
    labels: Vec<Vec<Label>>,
    next: usize,
    started: Instant,
    pub requests: usize,
    pub compressed_bytes: usize,
    pub retries: u64,
    pub send_seconds: f64,
    pub barrier_seconds: f64,
    pub barriers: usize,
    pub max_lag_seconds: f64,
}

fn labels_of(series: &Series) -> Vec<Label> {
    series
        .labels
        .iter()
        .map(|(name, value)| Label {
            name: name.clone(),
            value: value.clone(),
        })
        .collect()
}

impl<'a> Writer<'a> {
    pub fn new(
        series: &[Series],
        system: System,
        base: &str,
        start_ms: i64,
        options: &'a Options,
    ) -> Writer<'a> {
        let mut order: Vec<(i64, u32, f64)> = series
            .iter()
            .enumerate()
            .flat_map(|(i, s)| s.points.iter().map(move |&(t, v)| (t, i as u32, v)))
            .collect();
        order.sort_by_key(|&(t, i, _)| (t, i));
        Writer {
            system,
            base: base.to_string(),
            agent: ureq::AgentBuilder::new().build(),
            options,
            start_ms,
            order,
            labels: series.iter().map(labels_of).collect(),
            next: 0,
            started: Instant::now(),
            requests: 0,
            compressed_bytes: 0,
            retries: 0,
            send_seconds: 0.0,
            barrier_seconds: 0.0,
            barriers: 0,
            max_lag_seconds: 0.0,
        }
    }

    pub fn samples(&self) -> usize {
        self.order.len()
    }

    pub fn start_clock(&mut self) {
        self.started = Instant::now();
    }

    fn batch_end(&self, through_ms: i64) -> usize {
        let first_ms = self.order[self.next].0;
        let mut end = self.next;
        while end < self.order.len()
            && self.order[end].0 <= through_ms
            && end - self.next < self.options.batch_samples
            && (self.options.speed.is_none()
                || self.order[end].0 - first_ms < self.options.flush_ms)
        {
            end += 1;
        }
        end
    }

    fn body(&self, end: usize) -> Vec<u8> {
        let mut request = WriteRequest::default();
        let mut slot: HashMap<u32, usize> = HashMap::new();
        for &(t, i, v) in &self.order[self.next..end] {
            let at = *slot.entry(i).or_insert_with(|| {
                request.timeseries.push(TimeSeries {
                    labels: self.labels[i as usize].clone(),
                    samples: Vec::new(),
                });
                request.timeseries.len() - 1
            });
            request.timeseries[at].samples.push(Sample {
                value: v,
                timestamp: t,
            });
        }
        encode(&request)
    }

    fn pace(&mut self, data_ms: i64) {
        let Some(speed) = self.options.speed else {
            return;
        };
        let due = Duration::from_secs_f64((data_ms - self.start_ms).max(0) as f64 / 1e3 / speed);
        let now = self.started.elapsed();
        if due > now {
            std::thread::sleep(due - now);
        } else {
            self.max_lag_seconds = self.max_lag_seconds.max((now - due).as_secs_f64());
        }
    }

    fn send(&mut self, body: &[u8]) -> Result<()> {
        let url = format!("{}/api/v1/write", self.base);
        let started = Instant::now();
        loop {
            let response = self
                .agent
                .post(&url)
                .set("Content-Encoding", "snappy")
                .set("Content-Type", "application/x-protobuf")
                .set("X-Prometheus-Remote-Write-Version", "0.1.0")
                .send_bytes(body);
            match response {
                Ok(_) => break,
                Err(ureq::Error::Status(429 | 503, _)) => {
                    self.retries += 1;
                    std::thread::sleep(Duration::from_millis(10));
                }
                Err(ureq::Error::Status(code, response)) => {
                    let text = response.into_string().unwrap_or_default();
                    bail!("{url} answered {code}: {text}");
                }
                Err(error) => return Err(error).with_context(|| format!("writing to {url}")),
            }
        }
        self.send_seconds += started.elapsed().as_secs_f64();
        Ok(())
    }

    pub fn write_through(&mut self, through_ms: i64) -> Result<()> {
        while self.next < self.order.len() && self.order[self.next].0 <= through_ms {
            let end = self.batch_end(through_ms);
            let body = self.body(end);
            self.pace(self.order[end - 1].0);
            self.send(&body)?;
            self.requests += 1;
            self.compressed_bytes += body.len();
            self.next = end;
        }
        Ok(())
    }

    pub fn make_visible(&mut self, through_ms: i64) -> Result<()> {
        self.pace(through_ms);
        let started = Instant::now();
        match self.system {
            System::AsapqueryBackend => {
                let url = format!(
                    "{}/api/v1/precompute/watermark?time_ms={through_ms}",
                    self.base
                );
                self.agent
                    .post(&url)
                    .call()
                    .with_context(|| format!("closing panes through {through_ms}"))?;
            }
            System::Victoriametrics => {
                self.agent
                    .get(&format!("{}/internal/force_flush", self.base))
                    .call()
                    .context("flushing VictoriaMetrics")?;
            }
        }
        self.barrier_seconds += started.elapsed().as_secs_f64();
        self.barriers += 1;
        Ok(())
    }

    pub fn summary(&self) -> Result<Value> {
        let status = match self.system {
            System::AsapqueryBackend => Some(serde_json::from_str::<Value>(
                &self
                    .agent
                    .get(&format!("{}/api/v1/status/asap", self.base))
                    .call()?
                    .into_string()?,
            )?),
            System::Victoriametrics => None,
        };
        let write_seconds = self.send_seconds + self.barrier_seconds;
        Ok(json!({
            "samples": self.next,
            "requests": self.requests,
            "compressed_bytes": self.compressed_bytes,
            "retries": self.retries,
            "speed": self.options.speed,
            "batch_samples": self.options.batch_samples,
            "send_seconds": self.send_seconds,
            "barrier_seconds": self.barrier_seconds,
            "barriers": self.barriers,
            "samples_per_second": self.next as f64 / write_seconds,
            "max_lag_seconds": self.options.speed.map(|_| self.max_lag_seconds),
            "status": status,
        }))
    }
}
