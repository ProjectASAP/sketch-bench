mod asapquery_backend;
mod victoriametrics;

use anyhow::{bail, Context, Result};
use serde_json::Value;

pub use asapquery_backend::AsapqueryBackend;
pub use victoriametrics::Victoriametrics;

pub enum Sent {
    Accepted,
    Busy,
}

pub trait System {
    fn id(&self) -> &'static str;

    fn name(&self) -> &'static str;

    fn write(&self, agent: &ureq::Agent, base: &str, body: &[u8]) -> Result<Sent> {
        let url = format!("{base}/api/v1/write");
        let response = agent
            .post(&url)
            .set("Content-Encoding", "snappy")
            .set("Content-Type", "application/x-protobuf")
            .set("X-Prometheus-Remote-Write-Version", "0.1.0")
            .send_bytes(body);
        match response {
            Ok(_) => Ok(Sent::Accepted),
            Err(ureq::Error::Status(429 | 503, _)) => Ok(Sent::Busy),
            Err(ureq::Error::Status(code, response)) => {
                let text = response.into_string().unwrap_or_default();
                bail!("{url} answered {code}: {text}");
            }
            Err(error) => Err(error).with_context(|| format!("writing to {url}")),
        }
    }

    fn make_visible(&self, agent: &ureq::Agent, base: &str, through_ms: i64) -> Result<()>;

    fn status(&self, _agent: &ureq::Agent, _base: &str) -> Result<Option<Value>> {
        Ok(None)
    }
}

pub const SYSTEMS: &[&dyn System] = &[&AsapqueryBackend, &Victoriametrics];

pub fn ids() -> Vec<&'static str> {
    SYSTEMS.iter().map(|system| system.id()).collect()
}

pub fn by_id(id: &str) -> Option<&'static dyn System> {
    SYSTEMS.iter().copied().find(|system| system.id() == id)
}
