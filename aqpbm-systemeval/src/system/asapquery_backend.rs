use anyhow::{Context, Result};
use serde_json::Value;

use super::System;

pub struct AsapqueryBackend;

impl System for AsapqueryBackend {
    fn id(&self) -> &'static str {
        "asapquery-backend"
    }

    fn name(&self) -> &'static str {
        "ASAPQuery-backend"
    }

    fn make_visible(&self, agent: &ureq::Agent, base: &str, through_ms: i64) -> Result<()> {
        let url = format!("{base}/api/v1/precompute/watermark?time_ms={through_ms}");
        agent
            .post(&url)
            .call()
            .with_context(|| format!("closing panes through {through_ms}"))?;
        Ok(())
    }

    fn status(&self, agent: &ureq::Agent, base: &str) -> Result<Option<Value>> {
        Ok(Some(serde_json::from_str::<Value>(
            &agent
                .get(&format!("{base}/api/v1/status/asap"))
                .call()?
                .into_string()?,
        )?))
    }
}
