use std::collections::BTreeMap;
use std::path::Path;

use anyhow::{bail, Context, Result};
use aqpbm_datagen::DataDistribution;
use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Spec {
    pub use_case: String,
    pub start_ms: i64,
    pub duration_ms: i64,
    pub seed: u64,
    pub metrics: Vec<MetricSpec>,
    pub queries: Vec<QuerySpec>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MetricSpec {
    pub name: String,
    pub sample_interval_ms: i64,
    #[serde(default)]
    pub max_distinct_per_evaluation: Option<u64>,
    #[serde(default)]
    pub floor: bool,
    #[serde(default)]
    pub non_negative: bool,
    #[serde(default)]
    pub label_from_value: Option<String>,
    #[serde(default)]
    pub planned_series_count: Option<u64>,
    pub series: Vec<SeriesGroup>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SeriesGroup {
    #[serde(default = "one")]
    pub count: usize,
    #[serde(default)]
    pub sample_interval_ms: Option<i64>,
    pub labels: BTreeMap<String, String>,
    pub baseline: DataDistribution,
    #[serde(default)]
    pub phases: Vec<Phase>,
}

fn one() -> usize {
    1
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Phase {
    pub start_offset_ms: i64,
    pub end_offset_ms: i64,
    pub ramp_ms: i64,
    #[serde(default)]
    pub stagger_ms: i64,
    pub change: Change,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "snake_case")]
pub enum Change {
    Mix {
        share: f64,
        distribution: DataDistribution,
    },
    Scale {
        factor: f64,
    },
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QuerySpec {
    pub id: String,
    pub promql: String,
    pub window_ms: i64,
    pub interval_ms: i64,
    #[serde(default)]
    pub threshold: Option<f64>,
    pub unit: String,
    #[serde(default)]
    pub accuracy_metric: Option<String>,
}

impl MetricSpec {
    pub fn series_count(&self) -> u64 {
        self.planned_series_count
            .unwrap_or_else(|| self.series.iter().map(|g| g.count as u64).sum())
    }

    pub fn planned_sample_interval_ms(&self) -> i64 {
        match self.label_from_value {
            Some(_) => self.sample_interval_ms * self.series_count() as i64,
            None => self.sample_interval_ms,
        }
    }

    pub fn sample_interval_of(&self, group: &SeriesGroup) -> i64 {
        group.sample_interval_ms.unwrap_or(self.sample_interval_ms)
    }

    pub fn rows_per_second(&self) -> f64 {
        self.series
            .iter()
            .map(|g| {
                let streams = match self.label_from_value {
                    Some(_) => 1,
                    None => g.count,
                };
                streams as f64 * 1000.0 / self.sample_interval_of(g) as f64
            })
            .sum()
    }
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
        if self.duration_ms <= 0 {
            bail!("duration_ms must be positive");
        }
        for metric in &self.metrics {
            if metric.sample_interval_ms <= 0
                || metric
                    .series
                    .iter()
                    .any(|g| metric.sample_interval_of(g) <= 0)
            {
                bail!("metric {} needs a positive sample_interval_ms", metric.name);
            }
            if metric.label_from_value.is_some() && metric.planned_series_count.is_none() {
                bail!(
                    "metric {} draws its series from values, so it needs planned_series_count",
                    metric.name
                );
            }
            for phase in metric.series.iter().flat_map(|g| &g.phases) {
                let share = match phase.change {
                    Change::Mix { share, .. } => share,
                    Change::Scale { .. } => 0.0,
                };
                if !(0.0..=1.0).contains(&share)
                    || phase.start_offset_ms >= phase.end_offset_ms
                    || phase.ramp_ms < 0
                {
                    bail!(
                        "phase {phase:?} of {} is not a span with a share in [0, 1]",
                        metric.name
                    );
                }
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
