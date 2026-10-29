use std::collections::BTreeMap;

use anyhow::Result;
use rand::{Rng, SeedableRng};
use rand_xoshiro::Xoshiro256PlusPlus;

use crate::spec::Spec;

pub struct Series {
    pub labels: BTreeMap<String, String>,
    pub points: Vec<(i64, f64)>,
}

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

pub fn attack_share(spec: &Spec, offset_ms: i64) -> f64 {
    spec.attacks
        .iter()
        .filter(|a| a.start_offset_ms <= offset_ms && offset_ms < a.end_offset_ms)
        .map(|a| {
            if a.ramp_ms == 0 {
                return a.attack_share;
            }
            let rise = (offset_ms - a.start_offset_ms + 1) as f64 / a.ramp_ms as f64;
            let fall = (a.end_offset_ms - offset_ms) as f64 / a.ramp_ms as f64;
            a.attack_share * rise.min(fall).min(1.0)
        })
        .fold(0.0, f64::max)
}

pub fn generate(spec: &Spec) -> Result<Series> {
    let baseline = spec.baseline.sampler()?;
    let attack = spec.attack.sampler()?;
    let mut baseline_rng = Xoshiro256PlusPlus::seed_from_u64(spec.baseline.seed());
    let mut attack_rng = Xoshiro256PlusPlus::seed_from_u64(spec.attack.seed());
    let mut choice_rng = Xoshiro256PlusPlus::seed_from_u64(spec.seed);
    let mut labels = spec.labels.clone();
    labels.insert("__name__".into(), spec.metric.clone());
    let count = spec.duration_ms / spec.sample_interval_ms;
    let mut points = Vec::with_capacity(count as usize);
    for i in 0..count {
        let offset_ms = (i + 1) * spec.sample_interval_ms;
        let value = if choice_rng.random::<f64>() < attack_share(spec, offset_ms) {
            attack.sample(&mut attack_rng)
        } else {
            baseline.sample(&mut baseline_rng)
        };
        points.push((spec.start_ms + offset_ms, value.floor()));
    }
    Ok(Series { labels, points })
}
