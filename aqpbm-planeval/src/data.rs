use std::collections::BTreeMap;

use anyhow::Result;
use aqpbm_datagen::{DataDistribution, Sampler};
use rand::{Rng, SeedableRng};
use rand_xoshiro::Xoshiro256PlusPlus;

use crate::spec::{Change, MetricSpec, Phase, SeriesGroup, Spec};

pub struct Series {
    pub labels: BTreeMap<String, String>,
    pub points: Vec<(i64, f64)>,
}

pub type Slice<'a> = (&'a Series, &'a [(i64, f64)]);

impl Series {
    pub fn between(&self, start_exclusive: i64, end_inclusive: i64) -> &[(i64, f64)] {
        let from = self.points.partition_point(|&(t, _)| t <= start_exclusive);
        let to = self.points.partition_point(|&(t, _)| t <= end_inclusive);
        &self.points[from..to.max(from)]
    }

    pub fn up_to(&self, end_inclusive: i64) -> &[(i64, f64)] {
        &self.points[..self.points.partition_point(|&(t, _)| t <= end_inclusive)]
    }
}

pub fn between(series: &[Series], start_exclusive: i64, end_inclusive: i64) -> Vec<Slice<'_>> {
    series
        .iter()
        .map(|s| (s, s.between(start_exclusive, end_inclusive)))
        .filter(|(_, points)| !points.is_empty())
        .collect()
}

pub fn up_to(series: &[Series], end_inclusive: i64) -> Vec<Slice<'_>> {
    series
        .iter()
        .map(|s| (s, s.up_to(end_inclusive)))
        .filter(|(_, points)| !points.is_empty())
        .collect()
}

pub fn intensity(phase: &Phase, offset_ms: i64, shift_ms: i64) -> f64 {
    let start = phase.start_offset_ms + shift_ms;
    let end = phase.end_offset_ms + shift_ms;
    if offset_ms < start || offset_ms >= end {
        return 0.0;
    }
    if phase.ramp_ms == 0 {
        return 1.0;
    }
    let rise = (offset_ms - start + 1) as f64 / phase.ramp_ms as f64;
    let fall = (end - offset_ms) as f64 / phase.ramp_ms as f64;
    rise.min(fall).min(1.0)
}

fn mixed_seed(seed: u64, stream: u64) -> u64 {
    seed ^ stream.wrapping_mul(0x9E37_79B9_7F4A_7C15)
}

struct Stream<'a> {
    metric: &'a MetricSpec,
    group: &'a SeriesGroup,
    index: i64,
    distributions: Vec<(Sampler, Xoshiro256PlusPlus)>,
    phase_distribution: Vec<Option<usize>>,
    choice: Xoshiro256PlusPlus,
}

impl<'a> Stream<'a> {
    fn new(
        spec: &Spec,
        metric: &'a MetricSpec,
        group: &'a SeriesGroup,
        stream: u64,
        index: i64,
    ) -> Result<Stream<'a>> {
        let mut known: Vec<&DataDistribution> = vec![&group.baseline];
        let mut phase_distribution = Vec::new();
        for phase in &group.phases {
            phase_distribution.push(match &phase.change {
                Change::Mix { distribution, .. } => Some(
                    known
                        .iter()
                        .position(|d| *d == distribution)
                        .unwrap_or_else(|| {
                            known.push(distribution);
                            known.len() - 1
                        }),
                ),
                Change::Scale { .. } => None,
            });
        }
        let distributions = known
            .iter()
            .map(|d| {
                Ok((
                    d.sampler()?,
                    Xoshiro256PlusPlus::seed_from_u64(mixed_seed(d.seed(), stream)),
                ))
            })
            .collect::<Result<_>>()?;
        Ok(Stream {
            metric,
            group,
            index,
            distributions,
            phase_distribution,
            choice: Xoshiro256PlusPlus::seed_from_u64(mixed_seed(spec.seed, stream)),
        })
    }

    fn draw(&mut self, offset_ms: i64) -> f64 {
        let u = self.choice.random::<f64>();
        let mut chosen = None;
        let mut cumulative = 0.0;
        let mut scale = 1.0;
        for (phase, distribution) in self.group.phases.iter().zip(&self.phase_distribution) {
            let w = intensity(phase, offset_ms, self.index * phase.stagger_ms);
            if w == 0.0 {
                continue;
            }
            match (&phase.change, distribution) {
                (Change::Mix { share, .. }, Some(index)) => {
                    cumulative += share * w;
                    if chosen.is_none() && u < cumulative {
                        chosen = Some(*index);
                    }
                }
                (Change::Scale { factor }, _) => scale *= 1.0 + w * (factor - 1.0),
                _ => {}
            }
        }
        let (sampler, rng) = &mut self.distributions[chosen.unwrap_or(0)];
        let mut value = sampler.sample(rng) * scale;
        if self.metric.floor {
            value = value.floor();
        }
        if self.metric.non_negative {
            value = value.max(0.0);
        }
        value
    }
}

fn labels(metric: &MetricSpec, group: &SeriesGroup, index: usize) -> BTreeMap<String, String> {
    let mut labels: BTreeMap<String, String> = group
        .labels
        .iter()
        .map(|(k, v)| (k.clone(), v.replace("{i}", &index.to_string())))
        .collect();
    labels.insert("__name__".into(), metric.name.clone());
    labels
}

pub fn generate(spec: &Spec) -> Result<Vec<Series>> {
    let mut out = Vec::new();
    let mut stream = 0u64;
    for metric in &spec.metrics {
        for group in &metric.series {
            let interval_ms = metric.sample_interval_of(group);
            let count = spec.duration_ms / interval_ms;
            if let Some(label) = &metric.label_from_value {
                let mut drawn = Stream::new(spec, metric, group, stream, 0)?;
                stream += 1;
                let mut by_value: BTreeMap<u64, Vec<(i64, f64)>> = BTreeMap::new();
                for i in 0..count {
                    let offset_ms = (i + 1) * interval_ms;
                    let value = drawn.draw(offset_ms) as u64;
                    by_value
                        .entry(value)
                        .or_default()
                        .push((spec.start_ms + offset_ms, 1.0));
                }
                for (value, points) in by_value {
                    let mut labels = labels(metric, group, 0);
                    labels.insert(label.clone(), value.to_string());
                    out.push(Series { labels, points });
                }
                continue;
            }
            for index in 0..group.count {
                let mut drawn = Stream::new(spec, metric, group, stream, index as i64)?;
                stream += 1;
                let points = (0..count)
                    .map(|i| {
                        let offset_ms = (i + 1) * interval_ms;
                        (spec.start_ms + offset_ms, drawn.draw(offset_ms))
                    })
                    .collect();
                out.push(Series {
                    labels: labels(metric, group, index),
                    points,
                });
            }
        }
    }
    Ok(out)
}
