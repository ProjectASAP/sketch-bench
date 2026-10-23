//! The dataset: `series` series of one metric, one sample per `interval_ms`,
//! values drawn by `aqpbm-datagen`, plus the exact answers the comparison is
//! scored against.
//!
//! `aqpbm-datagen` produces typed value columns with no time axis or labels,
//! so the harness lays its single `f64` column onto a time grid: row `i` is
//! series `i % series` at `start_ms + (i / series) * interval_ms`. Values are
//! deterministic in `seed`; timestamps are deterministic given `start_ms`,
//! which the run picks relative to the wall clock (the data plane caps pane
//! closing at the wall clock, so the data must lie in the past).

use anyhow::Context;
use aqpbm_datagen::{ColumnSpec, DataDistribution, TableDescription, UniformParameter};
use serde::Serialize;

use crate::remote_write::{Label, Sample, TimeSeries, WriteRequest};

#[derive(Debug, Clone, Serialize)]
pub struct DatasetSpec {
    pub metric: String,
    pub series: usize,
    pub interval_ms: i64,
    /// Timestamp of the first sample of every series.
    pub start_ms: i64,
    /// Samples per series.
    pub points: usize,
    pub value_low: f64,
    pub value_high: f64,
    pub seed: u64,
    /// The `aqpbm-datagen` description the values came from.
    pub table: TableDescription,
}

pub struct Dataset {
    pub spec: DatasetSpec,
    /// `values[s][j]` is series `s` at `start_ms + j * interval_ms`.
    pub values: Vec<Vec<f64>>,
}

impl Dataset {
    pub fn generate(
        metric: &str,
        series: usize,
        interval_ms: i64,
        start_ms: i64,
        points: usize,
        seed: u64,
    ) -> anyhow::Result<Self> {
        let (value_low, value_high) = (1.0, 1000.0);
        let table = TableDescription::single(
            metric,
            ColumnSpec {
                distribution: DataDistribution::Uniform(UniformParameter {
                    lower_bound: value_low,
                    upper_bound: value_high,
                    seed,
                }),
                shift: None,
                cardinality: None,
                special_rule: 0,
                data_type: "f64".into(),
                string: None,
            },
            (series * points) as u64,
        );
        let generated = table.generate().context("aqpbm-datagen")?;
        let column = generated.data[0].as_f64().context("aqpbm-datagen column")?;
        let mut values = vec![Vec::with_capacity(points); series];
        for (i, &v) in column.iter().enumerate() {
            values[i % series].push(v);
        }
        Ok(Self {
            spec: DatasetSpec {
                metric: metric.into(),
                series,
                interval_ms,
                start_ms,
                points,
                value_low,
                value_high,
                seed,
                table,
            },
            values,
        })
    }

    pub fn timestamp(&self, j: usize) -> i64 {
        self.spec.start_ms + j as i64 * self.spec.interval_ms
    }

    /// The `instance` label of series `s`, which keys answers from both systems.
    pub fn instance(s: usize) -> String {
        format!("host-{s:03}")
    }

    /// Index range of points with `lo < t <= hi`.
    pub fn points_in(&self, lo_ms: i64, hi_ms: i64) -> std::ops::Range<usize> {
        let idx = |t: i64| -> usize {
            // First j with timestamp(j) > t.
            let d = t - self.spec.start_ms;
            if d < 0 {
                0
            } else {
                ((d / self.spec.interval_ms) as usize + 1).min(self.spec.points)
            }
        };
        idx(lo_ms)..idx(hi_ms)
    }

    /// A Remote Write request holding every series' points with
    /// `lo < t <= hi`, labels sorted by name.
    pub fn write_request(&self, lo_ms: i64, hi_ms: i64) -> WriteRequest {
        let range = self.points_in(lo_ms, hi_ms);
        let timeseries = (0..self.spec.series)
            .map(|s| TimeSeries {
                labels: vec![
                    Label {
                        name: "__name__".into(),
                        value: self.spec.metric.clone(),
                    },
                    Label {
                        name: "instance".into(),
                        value: Self::instance(s),
                    },
                    Label {
                        name: "job".into(),
                        value: "planeval".into(),
                    },
                ],
                samples: range
                    .clone()
                    .map(|j| Sample {
                        value: self.values[s][j],
                        timestamp: self.timestamp(j),
                    })
                    .collect(),
            })
            .collect();
        WriteRequest { timeseries }
    }

    /// Sorted values of series `s` in the PromQL range `(end - window, end]`.
    pub fn window(&self, s: usize, end_ms: i64, window_ms: i64) -> Vec<f64> {
        let mut v = self.values[s][self.points_in(end_ms - window_ms, end_ms)].to_vec();
        v.sort_by(f64::total_cmp);
        v
    }
}

/// `quantile_over_time` as Prometheus computes it: linear interpolation
/// between the closest ranks of `sorted`, rank `q * (n - 1)`.
pub fn prometheus_quantile(sorted: &[f64], q: f64) -> Option<f64> {
    if sorted.is_empty() {
        return None;
    }
    let rank = q * (sorted.len() - 1) as f64;
    let lower = rank.floor().max(0.0) as usize;
    let upper = (lower + 1).min(sorted.len() - 1);
    let weight = rank - rank.floor();
    Some(sorted[lower] * (1.0 - weight) + sorted[upper] * weight)
}

/// Rank error of `estimate` as a `q`-quantile of `sorted`: the distance from
/// `q` to the nearest normalised rank `estimate` can claim, where its ranks
/// run from `#{x < estimate} / n` to `#{x <= estimate} / n`. This is the
/// planner's `rank` accuracy metric for this query.
pub fn rank_error(sorted: &[f64], q: f64, estimate: f64) -> Option<f64> {
    if sorted.is_empty() {
        return None;
    }
    let n = sorted.len() as f64;
    let below = sorted.partition_point(|&x| x < estimate) as f64 / n;
    let at_or_below = sorted.partition_point(|&x| x <= estimate) as f64 / n;
    Some(if q < below {
        below - q
    } else if q > at_or_below {
        q - at_or_below
    } else {
        0.0
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quantile_interpolates_like_prometheus() {
        let v = [1.0, 2.0, 3.0, 4.0, 5.0];
        assert_eq!(prometheus_quantile(&v, 0.5), Some(3.0));
        assert_eq!(prometheus_quantile(&v, 0.99), Some(4.96));
        assert_eq!(prometheus_quantile(&v, 1.0), Some(5.0));
    }

    #[test]
    fn rank_error_is_zero_inside_the_estimates_ranks() {
        let v: Vec<f64> = (1..=100).map(f64::from).collect();
        assert_eq!(rank_error(&v, 0.99, 99.0), Some(0.0));
        assert!((rank_error(&v, 0.99, 97.0).unwrap() - 0.02).abs() < 1e-12);
    }

    #[test]
    fn window_is_left_open_right_closed() {
        let d = Dataset::generate("m", 2, 1000, 1000, 10, 7).unwrap();
        // Points at 1000..=10000; (3000, 6000] holds 4000, 5000, 6000.
        assert_eq!(d.points_in(3000, 6000), 3..6);
        assert_eq!(d.window(0, 6000, 3000).len(), 3);
    }
}
