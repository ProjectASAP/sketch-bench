//! Analytical scoring built from measured per-operation costs (§5).
//!
//! Costs split into four phases, each with CPU and memory: ingest (open
//! windows), merge and query (per evaluation), and storage (closed windows,
//! no CPU). The per-phase functions here are the one copy of the formulas;
//! [`score`] and the MILP both use them.
//!
//! Every measured cost is per instance, and a deployment holds `card(G)`
//! instances per window. Exact multi-group accumulators are measured as one
//! instance over many groups, but the export divides their memory and merge
//! by `groups_per_instance` (`aqpbm_core::atomic_costs`), so `card(G) ×`
//! prices them right too.
// ponytail: a family holding every group in one fixed-size instance (HydraKLL,
// sketch-bench#142) needs a per-family shape so it isn't multiplied by card(G).

use crate::{Capability, Deployment, Mapping, Rqe, WorkloadFacts};
use std::collections::{BTreeMap, BTreeSet};

pub(crate) const BYTES_PER_GIB: f64 = (1u64 << 30) as f64;

/// Query output per group: one f64 (one quantile for Quantile).
// ponytail: an estimate, not measured; tiny next to sketch state.
pub const OUTPUT_BYTES_PER_VALUE: f64 = 8.0;
/// Query output per top-k entry: a 64-bit key hash and a count.
pub const OUTPUT_BYTES_PER_TOPK_ENTRY: f64 = 16.0;
/// Top-k entries per group: sketch-bench's `CMS_HEAP_TOP_K`, the heap size
/// every top-k row was measured at.
// ponytail: fixed k. The RQE has no `k`, and the heap is left out of measured
// memory; a large k needs both, plus a benchmark that varies the heap.
pub const TOPK_ENTRIES: f64 = 32.0;

/// One phase's resource use.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct PhaseCost {
    /// Mean CPU-seconds per second: the area under the CPU curve over time.
    pub cpu_secs_per_sec: f64,
    pub memory_bytes: f64,
}

impl std::ops::AddAssign for PhaseCost {
    fn add_assign(&mut self, other: Self) {
        self.cpu_secs_per_sec += other.cpu_secs_per_sec;
        self.memory_bytes += other.memory_bytes;
    }
}

#[derive(Debug, Clone)]
pub struct Objectives {
    /// Inserts into open windows, and the open windows' state.
    pub ingest: PhaseCost,
    /// Folding each query's windows together, and the merged state, summed
    /// over RQEs as if every query evaluates at once.
    pub merge: PhaseCost,
    /// Reading the merged state, and the output, summed like `merge`.
    pub query: PhaseCost,
    /// Closed windows still inside the longest lookback each deployment
    /// serves. No CPU.
    pub storage: PhaseCost,
    /// Index-aligned with the input RQE slice; display IDs need not be unique.
    pub query_latency_secs: Vec<f64>,
}

impl Objectives {
    pub fn cpu_secs_per_sec(&self) -> f64 {
        self.phases().map(|p| p.cpu_secs_per_sec).sum()
    }

    pub fn memory_bytes(&self) -> f64 {
        self.phases().map(|p| p.memory_bytes).sum()
    }

    fn phases(&self) -> impl Iterator<Item = &PhaseCost> {
        [&self.ingest, &self.merge, &self.query, &self.storage].into_iter()
    }
}

/// `card(G)`: instances per window.
fn groups(d: &Deployment, facts: &WorkloadFacts) -> f64 {
    facts[&d.metric].cardinality[&d.grouping_labels] as f64
}

fn open_windows(d: &Deployment) -> f64 {
    d.active_instance_count()
        .expect("candidate window and slide align") as f64
}

/// `L / x`: windows merged for one evaluation.
fn merged_windows(r: &Rqe, d: &Deployment) -> f64 {
    d.query_instance_count(r.lookback_secs)
        .expect("only eligible pairs are costed") as f64
}

/// Every sample goes into its group's instance in each open window:
/// `λ · (x/y) · c_ins` CPU, `card(G) · m · x/y` memory.
pub(crate) fn ingest(d: &Deployment, facts: &WorkloadFacts) -> PhaseCost {
    PhaseCost {
        cpu_secs_per_sec: facts[&d.metric].arrival_rate_per_sec()
            * open_windows(d)
            * d.config.insert_cpu_secs,
        memory_bytes: groups(d, facts) * d.config.mem_bytes_per_instance * open_windows(d),
    }
}

/// Per group, `L/x − 1` folds every `T`, holding one merged instance:
/// `card(G) · (L/x − 1) · c_mrg / T` CPU, `card(G) · m` memory.
pub(crate) fn merge(r: &Rqe, d: &Deployment, facts: &WorkloadFacts) -> PhaseCost {
    let groups = groups(d, facts);
    PhaseCost {
        cpu_secs_per_sec: groups * (merged_windows(r, d) - 1.0) * d.config.merge_cpu_secs
            / r.interval_secs as f64,
        memory_bytes: groups * d.config.mem_bytes_per_instance,
    }
}

/// Per group, one query every `T` and its output:
/// `card(G) · c_qry / T` CPU, `card(G) ·` output bytes memory.
pub(crate) fn query(r: &Rqe, d: &Deployment, facts: &WorkloadFacts) -> PhaseCost {
    let groups = groups(d, facts);
    let output_bytes = match r.capability {
        Capability::TopK => TOPK_ENTRIES * OUTPUT_BYTES_PER_TOPK_ENTRY,
        _ => OUTPUT_BYTES_PER_VALUE,
    };
    PhaseCost {
        cpu_secs_per_sec: groups * d.config.query_cpu_secs / r.interval_secs as f64,
        memory_bytes: groups * output_bytes,
    }
}

/// Closed windows `d` holds so `r` can be answered: `card(G) · m ·
/// ((L − x)/y + 1)`. A deployment holds the most any of its RQEs needs.
pub(crate) fn storage_bytes(r: &Rqe, d: &Deployment, facts: &WorkloadFacts) -> f64 {
    groups(d, facts)
        * d.config.mem_bytes_per_instance
        * d.closed_instance_count(r.lookback_secs)
            .expect("only eligible pairs are costed") as f64
}

/// One evaluation's wall time on one core: `card(G) · (c_qry + (L/x − 1) · c_mrg)`.
pub(crate) fn query_latency_secs(r: &Rqe, d: &Deployment, facts: &WorkloadFacts) -> f64 {
    groups(d, facts)
        * (d.config.query_cpu_secs + (merged_windows(r, d) - 1.0) * d.config.merge_cpu_secs)
}

pub fn score(
    rqes: &[Rqe],
    deployments: &[Deployment],
    mapping: &Mapping,
    facts: &WorkloadFacts,
) -> Objectives {
    assert_eq!(
        mapping.len(),
        rqes.len(),
        "mapping must have one deployment per RQE"
    );
    let active: BTreeSet<usize> = mapping.iter().copied().collect();
    let mut ingest_cost = PhaseCost::default();
    for &di in &active {
        ingest_cost += ingest(&deployments[di], facts);
    }

    let mut stored: BTreeMap<usize, f64> = BTreeMap::new();
    let mut merge_cost = PhaseCost::default();
    let mut query_cost = PhaseCost::default();
    let query_latency_secs = rqes
        .iter()
        .zip(mapping)
        .map(|(r, &di)| {
            let d = &deployments[di];
            let held = stored.entry(di).or_default();
            *held = held.max(storage_bytes(r, d, facts));
            merge_cost += merge(r, d, facts);
            query_cost += query(r, d, facts);
            query_latency_secs(r, d, facts)
        })
        .collect();

    Objectives {
        ingest: ingest_cost,
        merge: merge_cost,
        query: query_cost,
        storage: PhaseCost {
            cpu_secs_per_sec: 0.0,
            memory_bytes: stored.values().sum(),
        },
        query_latency_secs,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{deployment, facts, rqe};

    #[test]
    fn scores_each_phase() {
        // 4 groups, 7 samples/sec; window 20 s sliding by 10 s, so 2 open
        // windows; lookback 60 s every 30 s, so 3 windows merged and
        // (60 − 20)/10 + 1 = 5 closed windows held.
        let d = deployment(10.0, 2.0, 3.0, 5.0, 20, 10);
        let r = rqe(60, 30);
        let result = score(&[r], &[d], &vec![0], &facts(4, 7));

        assert_eq!(result.ingest.cpu_secs_per_sec, 28.0); // 7 × 2 × 2
        assert_eq!(result.ingest.memory_bytes, 80.0); // 4 × 10 × 2
        assert_eq!(result.merge.cpu_secs_per_sec, 24.0 / 30.0); // 4 × 2 × 3 / 30
        assert_eq!(result.merge.memory_bytes, 40.0); // 4 × 10
        assert_eq!(result.query.cpu_secs_per_sec, 20.0 / 30.0); // 4 × 5 / 30
        assert_eq!(result.query.memory_bytes, 4.0 * 32.0 * 16.0); // top-k output
        assert_eq!(result.storage.cpu_secs_per_sec, 0.0);
        assert_eq!(result.storage.memory_bytes, 200.0); // 4 × 10 × 5
        assert_eq!(result.query_latency_secs, vec![44.0]); // 4 × (5 + 2 × 3)
        assert!((result.cpu_secs_per_sec() - (28.0 + 44.0 / 30.0)).abs() < 1e-12);
        assert_eq!(result.memory_bytes(), 80.0 + 40.0 + 2048.0 + 200.0);
    }

    #[test]
    fn a_direct_query_holds_one_closed_window() {
        // L == x: one closed window, nothing to merge.
        let d = deployment(10.0, 1.0, 1.0, 1.0, 60, 60);
        let result = score(&[rqe(60, 60)], &[d], &vec![0], &facts(1, 1));
        assert_eq!(result.storage.memory_bytes, 10.0);
        assert_eq!(result.merge.cpu_secs_per_sec, 0.0);
    }

    #[test]
    fn shared_deployment_stores_for_its_longest_lookback() {
        let d = deployment(10.0, 1.0, 1.0, 1.0, 10, 10);
        let result = score(
            &[rqe(60, 10), rqe(600, 10)],
            &[d],
            &vec![0, 0],
            &facts(2, 2),
        );
        // One copy of state, sized for 600 s: 2 × 10 × ((600 − 10)/10 + 1).
        assert_eq!(result.storage.memory_bytes, 1200.0);
        // Ingest is paid once; merge and query once per RQE.
        assert_eq!(result.ingest.cpu_secs_per_sec, 2.0);
        assert_eq!(result.merge.memory_bytes, 2.0 * 2.0 * 10.0);
    }

    #[test]
    fn arrival_rate_is_series_over_scrape_interval() {
        let mut f = facts(4, 600);
        f.get_mut(crate::test_support::METRIC)
            .unwrap()
            .scrape_interval_secs = 15;
        assert_eq!(f[crate::test_support::METRIC].arrival_rate_per_sec(), 40.0);
    }
}
