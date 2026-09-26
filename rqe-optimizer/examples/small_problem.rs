//! Toy instance, solved end to end: builds candidates, brute-forces
//! every feasible mapping, scores each one, then reports the
//! Pareto-optimal subset. Picking one mapping *off* the Pareto front is still
//! the parked scalarization question (problem statement, "Open for later").
//!
//! Cost data is real, not fabricated: `scripts/export_rqe_optimizer_costs.sh`
//! measures it via `sketch-bench`/`aqpbm-cli` and writes
//! `out/rqe_atomic_costs.json`, an `AtomicCostTable` loaded here verbatim --
//! no translation layer, since `rqe_optimizer::Deployment` embeds
//! `AtomicCostEntry` directly.
//!
//! Every other number this example uses (RQE definitions, label-set
//! cardinality/arrival-rate) lives in the two tables below (`label_sets`,
//! `rqes`) and nowhere else in this file.
//!
//! Run: `scripts/export_rqe_optimizer_costs.sh` once, then
//! `cargo run -p rqe-optimizer --example small_problem`. Add
//! `--candidates-only` to inspect candidate pruning safely, without starting
//! mapping enumeration.

use std::collections::BTreeSet;

use rqe_optimizer::candidates::{
    build_all_candidates, build_all_candidates_unpruned, eligible_deployments_for,
};
use rqe_optimizer::enumerate::{brute_force, for_each_mapping, unservable};
use rqe_optimizer::objectives::score;
use rqe_optimizer::pareto::{pareto_front, ParetoFront};
use rqe_optimizer::{
    AccuracyDirection, AtomicCostTable, Capability, LabelSet, LabelSetInfo, LabelSetTable, Rqe,
};

/// Accuracy metric keys the real comparators actually report (checked
/// against `aqpbm-core/src/accuracy/{frequency,quantile,cardinality}.rs`,
/// via `curve.rs`'s `error_curve` for frequency). These three are
/// lower-is-better errors; each RQE below pairs its metric with the matching
/// [`AccuracyDirection`] so the comparison can't be applied the wrong way
/// round.
///
/// Frequency uses `are_top100` (mean relative error on the 100 heaviest
/// keys), not `relative_error_mean`/`are_all`: those average over every
/// probed key including the long zipf tail, where relative error is
/// inherently unstable (dividing by a near-zero true count) and the real
/// numbers came back in the tens-to-hundreds, not a fraction -- useless as a
/// bound. `req_rate_*`'s use case (request rate by service+endpoint) cares
/// about the heavily-trafficked keys, which `are_top100` actually measures.
const FREQ_ERR: &str = "are_top100";
const RANK_ERR: &str = "mean_rank_err";
const CARDINALITY_ERR: &str = "relative_error";
/// Top-k's metric is the odd one out: a *score*, not an error, so its
/// tolerance below is a floor and its direction is `HigherIsBetter`.
const TOPK_PRECISION: &str = "precision_at_k";

const COST_TABLE_PATH: &str = "out/rqe_atomic_costs.json";

fn label_set(names: &[&str]) -> LabelSet {
    names.iter().map(|s| s.to_string()).collect()
}

fn load_cost_table() -> AtomicCostTable {
    let raw = std::fs::read_to_string(COST_TABLE_PATH).unwrap_or_else(|e| {
        panic!(
            "couldn't read {COST_TABLE_PATH} ({e}) -- run \
             `scripts/export_rqe_optimizer_costs.sh` first to generate it"
        )
    });
    serde_json::from_str(&raw)
        .unwrap_or_else(|e| panic!("{COST_TABLE_PATH} isn't a valid AtomicCostTable: {e}"))
}

fn label_sets() -> LabelSetTable {
    let mut t = LabelSetTable::new();
    // labels                    cardinality  arrival_rate (items/sec)
    t.insert(
        label_set(&["service", "endpoint"]),
        LabelSetInfo {
            cardinality: 50,
            arrival_rate_per_sec: 2_000.0,
        },
    );
    t.insert(
        label_set(&["service"]),
        LabelSetInfo {
            cardinality: 5,
            arrival_rate_per_sec: 2_000.0,
        },
    );
    t
}

fn rqes() -> Vec<Rqe> {
    let se = label_set(&["service", "endpoint"]);
    let s = label_set(&["service"]);
    vec![
        Rqe {
            id: "req_rate_1h".to_string(),
            capability: Capability::Freq,
            lookback_secs: 3_600,
            interval_secs: 60,
            labels: se.clone(),
            accuracy_metric: FREQ_ERR.to_string(),
            accuracy_tolerance: 0.1,
            accuracy_direction: AccuracyDirection::LowerIsBetter,
        },
        Rqe {
            id: "req_rate_1d".to_string(),
            capability: Capability::Freq,
            lookback_secs: 86_400,
            interval_secs: 60,
            labels: se.clone(),
            accuracy_metric: FREQ_ERR.to_string(),
            accuracy_tolerance: 0.1,
            accuracy_direction: AccuracyDirection::LowerIsBetter,
        },
        Rqe {
            id: "req_rate_5m_tick".to_string(),
            capability: Capability::Freq,
            lookback_secs: 3_600,
            interval_secs: 300,
            labels: se.clone(),
            accuracy_metric: FREQ_ERR.to_string(),
            accuracy_tolerance: 0.1,
            accuracy_direction: AccuracyDirection::LowerIsBetter,
        },
        Rqe {
            id: "latency_p99_1h".to_string(),
            capability: Capability::Quantile,
            lookback_secs: 3_600,
            interval_secs: 60,
            labels: se.clone(),
            accuracy_metric: RANK_ERR.to_string(),
            accuracy_tolerance: 0.05,
            accuracy_direction: AccuracyDirection::LowerIsBetter,
        },
        Rqe {
            id: "distinct_services_1h".to_string(),
            capability: Capability::Cardinality,
            lookback_secs: 3_600,
            interval_secs: 60,
            labels: s.clone(),
            accuracy_metric: CARDINALITY_ERR.to_string(),
            accuracy_tolerance: 0.1,
            accuracy_direction: AccuracyDirection::LowerIsBetter,
        },
        // The higher-is-better case: 0.9 is a *floor* on precision@k, not a
        // ceiling on error. Served by cms-heap, whose measured precision is
        // 1.0 at both configs under the zipf workload (it was 0.0 at both
        // under uniform -- top-k is meaningless without real skew).
        Rqe {
            id: "top_endpoints_1h".to_string(),
            capability: Capability::TopK,
            lookback_secs: 3_600,
            interval_secs: 60,
            labels: se.clone(),
            accuracy_metric: TOPK_PRECISION.to_string(),
            accuracy_tolerance: 0.9,
            accuracy_direction: AccuracyDirection::HigherIsBetter,
        },
    ]
}

fn main() {
    let candidates_only = std::env::args().any(|arg| arg == "--candidates-only");
    let rqes = rqes();
    let cost_table = load_cost_table();
    let label_sets = label_sets();

    let unpruned_count =
        candidates_only.then(|| build_all_candidates_unpruned(&rqes, &cost_table).len());
    let deployments = build_all_candidates(&rqes, &cost_table);
    println!(
        "{} RQEs, {} candidate deployments (from {} real cost-table rows)",
        rqes.len(),
        deployments.len(),
        cost_table.len(),
    );

    if let Some(unpruned_count) = unpruned_count {
        println!(
            "candidate dominance: {unpruned_count} generated -> {} retained ({} removed)",
            deployments.len(),
            unpruned_count - deployments.len(),
        );
        let mut possible_mappings = Some(1_u64);
        for rqe in &rqes {
            let eligible = eligible_deployments_for(rqe, &deployments);
            println!("  {}: {} eligible deployments", rqe.id, eligible.len());
            possible_mappings = possible_mappings.and_then(|count| {
                count.checked_mul(eligible.len().try_into().expect("usize fits in u64"))
            });
        }
        match possible_mappings {
            Some(count) => println!("Cartesian mapping space: {count}"),
            None => println!("Cartesian mapping space: exceeds u64"),
        }
        return;
    }

    let missing = unservable(&rqes, &deployments);
    if !missing.is_empty() {
        println!("unservable (no eligible deployment): {missing:?}");
        return;
    }

    if std::env::args().any(|arg| arg == "--streaming") {
        let mut front = ParetoFront::new();
        let mapping_count = for_each_mapping(&rqes, &deployments, |mapping| {
            front.consider(mapping, score(&rqes, &deployments, mapping, &label_sets));
        });
        println!(
            "streamed {mapping_count} feasible mappings; {} remain on the Pareto front",
            front.entries().len()
        );
        return;
    }

    let mappings = brute_force(&rqes, &deployments);
    println!("{} feasible full mappings", mappings.len());

    let objectives: Vec<_> = mappings
        .iter()
        .map(|m| score(&rqes, &deployments, m, &label_sets))
        .collect();
    let front = pareto_front(&objectives);
    println!("{} on the Pareto front\n", front.len());

    let mut front = front;
    front.sort_unstable();
    for &i in &front {
        let obj = &objectives[i];
        let distinct_deployments: BTreeSet<usize> = mappings[i].iter().copied().collect();
        println!(
            "mapping {i}: {} deployments, peak_query_mem={:.0}MB, ingest={:.3e}, \
             merge={:.3e}, query={:.3e}, total={:.3e} cpu-sec/sec",
            distinct_deployments.len(),
            obj.peak_query_memory_bytes / 1e6,
            obj.ingest_cpu_secs_per_sec,
            obj.merge_cpu_secs_per_sec,
            obj.query_cpu_secs_per_sec,
            obj.tco_cpu_secs_per_sec,
        );
        for (rqe, latency) in rqes.iter().zip(&obj.query_latency_secs) {
            let rqe_id = &rqe.id;
            println!("    {rqe_id}: query_latency={latency:.3e} sec");
        }
    }
}
