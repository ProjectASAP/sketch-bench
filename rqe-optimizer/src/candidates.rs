//! Candidate generation and eligibility (§3).

use crate::{AtomicCostEntry, Capability, Deployment, LabelSet, Rqe, Seconds};
use std::collections::{BTreeMap, BTreeSet};

pub fn gcd(a: Seconds, b: Seconds) -> Seconds {
    if b == 0 {
        a
    } else {
        gcd(b, a % b)
    }
}

fn divisors(value: Seconds) -> Vec<Seconds> {
    let mut out = Vec::new();
    let mut factor = 1;
    while factor <= value / factor {
        if value.is_multiple_of(factor) {
            out.push(factor);
            if factor != value / factor {
                out.push(value / factor);
            }
        }
        factor += 1;
    }
    out.sort_unstable();
    out
}

/// Every gcd reachable from a non-empty subset, folded incrementally rather
/// than by enumerating the powerset.
fn subset_gcds(values: impl IntoIterator<Item = Seconds>) -> BTreeSet<Seconds> {
    let mut out = BTreeSet::new();
    for value in values {
        let extensions: Vec<_> = out.iter().map(|&previous| gcd(previous, value)).collect();
        out.insert(value);
        out.extend(extensions);
    }
    out
}

fn candidate_deployments(group: &[&Rqe], costs: &[AtomicCostEntry]) -> Vec<Deployment> {
    if group.is_empty() {
        return Vec::new();
    }
    let capability = group[0].capability;
    let labels = group[0].labels.clone();
    let windows = group
        .iter()
        .flat_map(|r| divisors(r.lookback_secs))
        .collect::<BTreeSet<_>>();
    let mut deployments = Vec::new();
    for window_secs in windows {
        let slides = subset_gcds(
            group
                .iter()
                .filter(|r| r.lookback_secs % window_secs == 0)
                .map(|r| gcd(window_secs, r.interval_secs)),
        );
        for config in costs
            .iter()
            .filter(|c| capability.families().contains(&c.sketch.as_str()))
        {
            deployments.extend(slides.iter().map(|&slide_secs| Deployment {
                capability,
                labels: labels.clone(),
                config: config.clone(),
                window_secs,
                slide_secs,
            }));
        }
    }
    deployments
}

pub fn build_all_candidates(rqes: &[Rqe], costs: &[AtomicCostEntry]) -> Vec<Deployment> {
    let mut groups: BTreeMap<(Capability, LabelSet), Vec<&Rqe>> = BTreeMap::new();
    for rqe in rqes {
        groups
            .entry((rqe.capability, rqe.labels.clone()))
            .or_default()
            .push(rqe);
    }
    groups
        .values()
        .flat_map(|group| candidate_deployments(group, costs))
        .collect()
}

pub fn eligible_deployments_for(r: &Rqe, deployments: &[Deployment]) -> Vec<usize> {
    deployments
        .iter()
        .enumerate()
        .filter(|(_, d)| is_eligible(r, d))
        .map(|(i, _)| i)
        .collect()
}

pub fn is_eligible(r: &Rqe, d: &Deployment) -> bool {
    r.capability == d.capability
        && r.labels == d.labels
        && d.window_secs != 0
        && d.slide_secs != 0
        && d.window_secs.is_multiple_of(d.slide_secs)
        && r.lookback_secs.is_multiple_of(d.window_secs)
        && r.interval_secs.is_multiple_of(d.slide_secs)
        && r.accuracy_ok_for(&d.config)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{AccuracyDirection, LabelSet};
    use std::collections::BTreeMap;
    fn rqe(id: &str, lookback: Seconds, interval: Seconds) -> Rqe {
        Rqe {
            id: id.into(),
            capability: Capability::Freq,
            lookback_secs: lookback,
            interval_secs: interval,
            labels: LabelSet::new(),
            accuracy_metric: "err".into(),
            accuracy_tolerance: 1.0,
            accuracy_direction: AccuracyDirection::LowerIsBetter,
        }
    }
    fn cost() -> AtomicCostEntry {
        AtomicCostEntry {
            sketch: "cms-fastpath-vector2d".into(),
            sketch_config: serde_json::json!(null),
            mem_bytes_per_instance: 1.0,
            insert_cpu_secs: 1.0,
            merge_cpu_secs: 1.0,
            query_cpu_secs: 1.0,
            query_accuracy: BTreeMap::from([("err".into(), 0.1)]),
        }
    }
    #[test]
    fn shared_slide_comes_from_subset_gcd_not_all_divisors() {
        let rqes = vec![rqe("a", 60, 20), rqe("b", 60, 30)];
        let candidates = build_all_candidates(&rqes, &[cost()]);
        let slides: BTreeSet<_> = candidates
            .iter()
            .filter(|d| d.window_secs == 60)
            .map(|d| d.slide_secs)
            .collect();
        assert_eq!(slides, BTreeSet::from([10, 20, 30]));
    }
    #[test]
    fn eligibility_requires_exact_non_overlapping_tiling() {
        let r = rqe("r", 600, 180);
        let d = Deployment {
            capability: Capability::Freq,
            labels: LabelSet::new(),
            config: cost(),
            window_secs: 120,
            slide_secs: 60,
        };
        assert!(is_eligible(&r, &d));
        assert!(!is_eligible(
            &r,
            &Deployment {
                window_secs: 128,
                ..d.clone()
            }
        ));
        assert!(!is_eligible(
            &r,
            &Deployment {
                slide_secs: 70,
                ..d
            }
        ));
    }
}
