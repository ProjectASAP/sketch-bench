use std::collections::BTreeMap;
use std::path::Path;

use anyhow::{bail, Context, Result};
use aqpbm_datagen::DataDistribution;
use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Spec {
    pub use_case: String,
    pub metric: String,
    pub labels: BTreeMap<String, String>,
    pub start_ms: i64,
    pub duration_ms: i64,
    pub sample_interval_ms: i64,
    pub max_distinct_per_evaluation: u64,
    pub seed: u64,
    pub baseline: DataDistribution,
    pub attack: DataDistribution,
    pub attacks: Vec<Attack>,
    pub queries: Vec<QuerySpec>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Attack {
    pub start_offset_ms: i64,
    pub end_offset_ms: i64,
    pub ramp_ms: i64,
    pub attack_share: f64,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QuerySpec {
    pub id: String,
    pub promql: String,
    pub window_ms: i64,
    pub interval_ms: i64,
    pub threshold: f64,
    pub unit: String,
}

impl Spec {
    pub fn read(path: &Path) -> Result<Spec> {
        let text =
            std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        let spec: Spec =
            serde_json::from_str(&text).with_context(|| format!("parsing {}", path.display()))?;
        spec.check()?;
        Ok(spec)
    }

    pub fn check(&self) -> Result<()> {
        if self.duration_ms <= 0 || self.sample_interval_ms <= 0 {
            bail!("duration_ms and sample_interval_ms must be positive");
        }
        for attack in &self.attacks {
            if !(0.0..=1.0).contains(&attack.attack_share)
                || attack.start_offset_ms >= attack.end_offset_ms
                || attack.ramp_ms < 0
            {
                bail!("attack {attack:?} is not a span with a share in [0, 1]");
            }
        }
        for query in &self.queries {
            if query.window_ms <= 0 || query.interval_ms <= 0 {
                bail!("query {} needs a positive window and interval", query.id);
            }
            if self.start_ms.rem_euclid(query.interval_ms) != 0 {
                bail!("start_ms is not on query {}'s interval grid", query.id);
            }
        }
        Ok(())
    }

    pub fn end_ms(&self) -> i64 {
        self.start_ms + self.duration_ms
    }

    pub fn query(&self, id: &str) -> Result<&QuerySpec> {
        self.queries
            .iter()
            .find(|q| q.id == id)
            .with_context(|| format!("no query {id} in the spec"))
    }

    pub fn evaluation_times(&self, query: &QuerySpec) -> Vec<i64> {
        let first = self.start_ms + query.window_ms;
        let first =
            first + (query.interval_ms - first.rem_euclid(query.interval_ms)) % query.interval_ms;
        (0..)
            .map(|i| first + i * query.interval_ms)
            .take_while(|&t| t <= self.end_ms())
            .collect()
    }
}
