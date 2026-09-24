//! Brute-force search over full mappings (§4, §7 step 2).
//!
//! A mapping is one deployment choice per RQE (§4's only constraint is
//! `Σ_D x_{i,D} = 1`; `y_D` is derived). No RQE is eligible outside its own
//! `(capability, labels)` group, so choices are independent and the search
//! space is exactly the cartesian product of the per-RQE eligible lists --
//! nothing to prune against. Sharing falls out when two choices land on the
//! same index.
//!
//! An ILP would replace this module alone: `Mapping` already encodes
//! `x_{i,D}` and `y_D`.

use crate::{candidates::eligible_deployments_for, Deployment, Mapping, Rqe};

/// RQEs nothing in `deployments` can serve (§3.4). Surfaced separately from
/// `brute_force`: an unservable RQE is a modeling gap (tolerance too tight,
/// no such config), not a normal empty result.
pub fn unservable(rqes: &[Rqe], deployments: &[Deployment]) -> Vec<String> {
    rqes.iter()
        .filter(|r| eligible_deployments_for(r, deployments).is_empty())
        .map(|r| r.id.clone())
        .collect()
}

/// Every feasible full mapping. Empty if any RQE is unservable -- check
/// [`unservable`] to tell the two cases apart.
pub fn brute_force(rqes: &[Rqe], deployments: &[Deployment]) -> Vec<Mapping> {
    let choices: Vec<Vec<usize>> = rqes
        .iter()
        .map(|r| eligible_deployments_for(r, deployments))
        .collect();

    if choices.iter().any(|c| c.is_empty()) {
        return Vec::new();
    }

    let mut mappings = vec![Vec::with_capacity(rqes.len())];
    for choice in &choices {
        let mut next = Vec::with_capacity(mappings.len() * choice.len());
        for partial in &mappings {
            for &d in choice {
                let mut extended = partial.clone();
                extended.push(d);
                next.push(extended);
            }
        }
        mappings = next;
    }
    mappings
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::candidates::build_all_candidates;
    use crate::{AtomicCostEntry, Capability, LabelSet};
    use std::collections::BTreeMap;

    fn cost(sketch: &str) -> AtomicCostEntry {
        let mut query_accuracy = BTreeMap::new();
        query_accuracy.insert("err".to_string(), 0.01);
        AtomicCostEntry {
            sketch: sketch.to_string(),
            sketch_config: serde_json::json!(null),
            mem_bytes_per_instance: 1.0,
            insert_cpu_secs: 1.0,
            merge_cpu_secs: 1.0,
            query_cpu_secs: 1.0,
            query_accuracy,
        }
    }

    fn rqe(id: &str, interval: u64, lookback: u64) -> Rqe {
        Rqe {
            id: id.to_string(),
            capability: Capability::Freq,
            lookback_secs: lookback,
            interval_secs: interval,
            labels: LabelSet::new(),
            accuracy_metric: "err".to_string(),
            accuracy_tolerance: 1.0,
            accuracy_direction: crate::AccuracyDirection::LowerIsBetter,
        }
    }

    #[test]
    fn two_rqes_can_share_one_deployment() {
        let rqes = vec![rqe("a", 60, 3_600), rqe("b", 60, 3_600)];
        let table = vec![cost("cms-fastpath-vector2d")];
        let deployments = build_all_candidates(&rqes, &table);

        assert!(unservable(&rqes, &deployments).is_empty());
        let mappings = brute_force(&rqes, &deployments);
        assert!(
            mappings.iter().any(|m| m[0] == m[1]),
            "expected at least one mapping where both identical RQEs land on the same deployment"
        );
    }

    #[test]
    fn unservable_rqe_yields_no_mappings() {
        let rqes = vec![rqe("a", 60, 3_600)];
        let table: Vec<AtomicCostEntry> = Vec::new(); // no configs at all
        let deployments = build_all_candidates(&rqes, &table);

        assert_eq!(unservable(&rqes, &deployments), vec!["a".to_string()]);
        assert!(brute_force(&rqes, &deployments).is_empty());
    }
}
