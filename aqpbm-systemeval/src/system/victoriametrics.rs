use anyhow::{Context, Result};

use super::System;

pub struct Victoriametrics;

impl System for Victoriametrics {
    fn id(&self) -> &'static str {
        "victoriametrics"
    }

    fn name(&self) -> &'static str {
        "VictoriaMetrics"
    }

    fn make_visible(&self, agent: &ureq::Agent, base: &str, _through_ms: i64) -> Result<()> {
        agent
            .get(&format!("{base}/internal/force_flush"))
            .call()
            .context("flushing VictoriaMetrics")?;
        Ok(())
    }
}
