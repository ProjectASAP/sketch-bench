//! The dataset: series laid out on a label hierarchy, one gauge sample and
//! (optionally) one counter sample per series every `interval_ms`, values
//! drawn by `aqpbm-datagen`, plus the exact answers the comparison is scored
//! against.
//!
//! `aqpbm-datagen` produces typed value columns with no time axis or labels,
//! so the harness lays each `f64` column onto a time grid: row `i` is series
//! `i % series` at `start_ms + (i / series) * interval_ms`.
//!
//! Series sit on a label hierarchy such as `region:2,service:2,instance:4`:
//! 2 regions, 2 services in each, 4 instances in each service, 16 series. A
//! label value names its level and its index across the whole level
//! (`service-3` is the second service of `region-1`), so every child has one
//! parent and a coarser grouping is a union of finer ones.
//!
//! The gauge (the queried metric) takes the column's values as they are. The
//! counter takes a second column, drawn with the next seed, as non-negative
//! increments and accumulates them per series from zero, so it only grows.
//!
//! Values are deterministic in `seed`; timestamps are deterministic given
//! `start_ms`, which the run picks relative to the wall clock (the data plane
//! caps pane closing at the wall clock, so the data must lie in the past).

use std::collections::BTreeMap;

use anyhow::{bail, ensure, Context};
use aqpbm_datagen::{ColumnSpec, DataDistribution, TableDescription, UniformParameter};
use serde::Serialize;

use crate::remote_write::{Label, Sample, TimeSeries, WriteRequest};

/// One level of the label hierarchy: its label name and how many children
/// each parent has (the first level's fan-out is its total count).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Level {
    pub label: String,
    pub fan_out: usize,
}

/// Parses `region:2,service:2,instance:4`.
pub fn parse_hierarchy(spec: &str) -> anyhow::Result<Vec<Level>> {
    let mut levels = Vec::new();
    for part in spec.split(',') {
        let (label, n) = part
            .split_once(':')
            .with_context(|| format!("label level {part:?} is not NAME:COUNT"))?;
        let fan_out: usize = n
            .parse()
            .with_context(|| format!("label level {part:?}: count"))?;
        ensure!(fan_out > 0, "label level {part:?}: count must be positive");
        ensure!(
            !label.is_empty() && label != "__name__",
            "label level {part:?}: bad label name"
        );
        if levels.iter().any(|l: &Level| l.label == label) {
            bail!("label {label:?} appears twice in {spec:?}");
        }
        levels.push(Level {
            label: label.to_owned(),
            fan_out,
        });
    }
    ensure!(!levels.is_empty(), "empty label hierarchy");
    Ok(levels)
}

#[derive(Debug, Clone, Serialize)]
pub struct DatasetSpec {
    /// The gauge metric, the one the query reads.
    pub metric: String,
    /// The counter metric, if any.
    pub counter_metric: Option<String>,
    pub hierarchy: Vec<Level>,
    pub series: usize,
    pub interval_ms: i64,
    /// Timestamp of the first sample of every series.
    pub start_ms: i64,
    /// Samples per series and metric.
    pub points: usize,
    pub seed: u64,
    /// The `aqpbm-datagen` description the gauge values came from.
    pub gauge_table: TableDescription,
    /// The `aqpbm-datagen` description the counter increments came from.
    pub counter_increment_table: Option<TableDescription>,
}

pub struct Dataset {
    pub spec: DatasetSpec,
    /// `labels[s]`: series `s`'s hierarchy labels, in hierarchy order.
    pub labels: Vec<Vec<(String, String)>>,
    /// `values[s][j]` is the gauge of series `s` at `start_ms + j * interval_ms`.
    pub values: Vec<Vec<f64>>,
    /// `counters[s][j]` is the counter of series `s` at the same time.
    pub counters: Option<Vec<Vec<f64>>>,
}

/// The gauge's value distribution: Uniform[1, 1000).
const GAUGE: (f64, f64) = (1.0, 1000.0);
/// The counter's per-sample increment distribution: Uniform[0, 10).
const COUNTER_INCREMENT: (f64, f64) = (0.0, 10.0);

fn uniform_column(name: &str, (low, high): (f64, f64), seed: u64, rows: u64) -> TableDescription {
    TableDescription::single(
        name,
        ColumnSpec {
            distribution: DataDistribution::Uniform(UniformParameter {
                lower_bound: low,
                upper_bound: high,
                seed,
            }),
            shift: None,
            cardinality: None,
            special_rule: 0,
            data_type: "f64".into(),
            string: None,
        },
        rows,
    )
}

/// Generates `table` and deals its one `f64` column out to `series` series.
fn deal(table: &TableDescription, series: usize, points: usize) -> anyhow::Result<Vec<Vec<f64>>> {
    let generated = table.generate().context("aqpbm-datagen")?;
    let column = generated.data[0].as_f64().context("aqpbm-datagen column")?;
    let mut values = vec![Vec::with_capacity(points); series];
    for (i, &v) in column.iter().enumerate() {
        values[i % series].push(v);
    }
    Ok(values)
}

/// Every series' labels: series `s` is the mixed-radix number whose digits
/// are its index at each level under its parent.
fn hierarchy_labels(hierarchy: &[Level]) -> Vec<Vec<(String, String)>> {
    let series: usize = hierarchy.iter().map(|l| l.fan_out).product();
    (0..series)
        .map(|s| {
            // Series per node at each level: a level-k node covers the
            // product of the fan-outs below it.
            let mut below = series;
            hierarchy
                .iter()
                .map(|level| {
                    below /= level.fan_out;
                    // The node's index across the whole level.
                    let index = s / below;
                    (level.label.clone(), format!("{}-{index}", level.label))
                })
                .collect()
        })
        .collect()
}

/// A canonical key for a label set: `name=value` pairs sorted by name and
/// joined by `,`, without `__name__`. Both systems' answers are matched to
/// the generated series by it.
pub fn label_key<'a>(labels: impl IntoIterator<Item = (&'a str, &'a str)>) -> String {
    let sorted: BTreeMap<&str, &str> = labels
        .into_iter()
        .filter(|(name, _)| *name != "__name__")
        .collect();
    sorted
        .iter()
        .map(|(name, value)| format!("{name}={value}"))
        .collect::<Vec<_>>()
        .join(",")
}

impl Dataset {
    pub fn generate(
        metric: &str,
        counter_metric: Option<&str>,
        hierarchy: Vec<Level>,
        interval_ms: i64,
        start_ms: i64,
        points: usize,
        seed: u64,
    ) -> anyhow::Result<Self> {
        let series: usize = hierarchy.iter().map(|l| l.fan_out).product();
        let rows = (series * points) as u64;
        let gauge_table = uniform_column(metric, GAUGE, seed, rows);
        let values = deal(&gauge_table, series, points)?;
        let (counter_increment_table, counters) = match counter_metric {
            Some(name) => {
                let table = uniform_column(name, COUNTER_INCREMENT, seed.wrapping_add(1), rows);
                let mut counters = deal(&table, series, points)?;
                for row in &mut counters {
                    let mut total = 0.0;
                    for v in row.iter_mut() {
                        total += *v;
                        *v = total;
                    }
                }
                (Some(table), Some(counters))
            }
            None => (None, None),
        };
        Ok(Self {
            labels: hierarchy_labels(&hierarchy),
            spec: DatasetSpec {
                metric: metric.into(),
                counter_metric: counter_metric.map(str::to_owned),
                hierarchy,
                series,
                interval_ms,
                start_ms,
                points,
                seed,
                gauge_table,
                counter_increment_table,
            },
            values,
            counters,
        })
    }

    pub fn timestamp(&self, j: usize) -> i64 {
        self.spec.start_ms + j as i64 * self.spec.interval_ms
    }

    /// Series `s`'s key ([`label_key`]), which matches it to both systems'
    /// answers.
    pub fn series_key(&self, s: usize) -> String {
        label_key(self.labels[s].iter().map(|(n, v)| (n.as_str(), v.as_str())))
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

    fn time_series(
        &self,
        metric: &str,
        s: usize,
        points: impl Iterator<Item = (i64, f64)>,
    ) -> TimeSeries {
        let mut labels = vec![Label {
            name: "__name__".into(),
            value: metric.to_owned(),
        }];
        labels.extend(self.labels[s].iter().map(|(name, value)| Label {
            name: name.clone(),
            value: value.clone(),
        }));
        // Remote Write wants labels sorted by name.
        labels.sort_by(|a, b| a.name.cmp(&b.name));
        TimeSeries {
            labels,
            samples: points
                .map(|(timestamp, value)| Sample { value, timestamp })
                .collect(),
        }
    }

    /// A Remote Write request holding every series' points with
    /// `lo < t <= hi`, gauge and counter.
    pub fn write_request(&self, lo_ms: i64, hi_ms: i64) -> WriteRequest {
        let range = self.points_in(lo_ms, hi_ms);
        let mut timeseries = Vec::new();
        for s in 0..self.spec.series {
            timeseries.push(
                self.time_series(
                    &self.spec.metric,
                    s,
                    range
                        .clone()
                        .map(|j| (self.timestamp(j), self.values[s][j])),
                ),
            );
            if let (Some(metric), Some(counters)) = (&self.spec.counter_metric, &self.counters) {
                timeseries.push(self.time_series(
                    metric,
                    s,
                    range.clone().map(|j| (self.timestamp(j), counters[s][j])),
                ));
            }
        }
        WriteRequest { timeseries }
    }

    /// Sorted gauge values of series `s` in the PromQL range `(end - window, end]`.
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

    fn flat(n: usize) -> Vec<Level> {
        parse_hierarchy(&format!("instance:{n}")).unwrap()
    }

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
        let d = Dataset::generate("m", None, flat(2), 1000, 1000, 10, 7).unwrap();
        // Points at 1000..=10000; (3000, 6000] holds 4000, 5000, 6000.
        assert_eq!(d.points_in(3000, 6000), 3..6);
        assert_eq!(d.window(0, 6000, 3000).len(), 3);
    }

    /// Each child has one parent, and a level's values run across the level.
    #[test]
    fn hierarchy_labels_nest() {
        let h = parse_hierarchy("region:2,service:3,instance:2").unwrap();
        let labels = hierarchy_labels(&h);
        assert_eq!(labels.len(), 12);
        assert_eq!(
            labels[0],
            [
                ("region".into(), "region-0".into()),
                ("service".into(), "service-0".into()),
                ("instance".into(), "instance-0".into()),
            ]
        );
        assert_eq!(
            labels[11],
            [
                ("region".into(), "region-1".into()),
                ("service".into(), "service-5".into()),
                ("instance".into(), "instance-11".into()),
            ]
        );
        // service-k always sits under the same region.
        let mut parent = BTreeMap::new();
        for l in &labels {
            assert_eq!(
                *parent.entry(l[1].1.clone()).or_insert(l[0].1.clone()),
                l[0].1
            );
        }
        assert_eq!(parent.len(), 6);
    }

    /// The counter never decreases and starts from its first increment.
    #[test]
    fn counter_only_grows() {
        let d = Dataset::generate("g", Some("c_total"), flat(3), 1000, 1000, 50, 7).unwrap();
        for row in d.counters.as_ref().unwrap() {
            assert!(row[0] >= 0.0);
            assert!(row.windows(2).all(|w| w[1] >= w[0]));
        }
        let req = d.write_request(0, 5000);
        assert_eq!(req.timeseries.len(), 6, "a gauge and a counter per series");
    }

    #[test]
    fn label_keys_ignore_name_and_order() {
        assert_eq!(
            label_key([("b", "2"), ("__name__", "m"), ("a", "1")]),
            "a=1,b=2"
        );
    }

    #[test]
    fn bad_hierarchies_are_refused() {
        assert!(parse_hierarchy("region:0").is_err());
        assert!(parse_hierarchy("region").is_err());
        assert!(parse_hierarchy("a:2,a:3").is_err());
    }
}
