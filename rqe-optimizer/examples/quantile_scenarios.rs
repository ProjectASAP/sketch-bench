//! How the optimal deployment of a quantile workload moves with the query's
//! lookback and evaluation interval.
//!
//! Every scenario is one workload: [`QUERY_COUNT`] `quantile_over_time`
//! RAQEs, one per latency metric, all sharing the scenario's lookback and
//! interval. The metrics are distinct, so no deployment is shared across
//! queries. Each scenario is solved with the MILP and printed as one row.
//!
//! Run:
//! `cargo run --release -p rqe-optimizer --example quantile_scenarios --
//!  --cost-dir DIR --saturation-dir DIR [--w-cpu X] [--w-mem Y]
//!  [--allow-undeployable-families]`

use rqe_optimizer::candidates::build_all_candidates;
use rqe_optimizer::enumerate::unservable;
use rqe_optimizer::milp::{minimize, Objective};
use rqe_optimizer::saturation::{DataShape, SaturationCurves};
use rqe_optimizer::{
    validate_facts, AccuracyDirection, AtomicCostTable, Capability, Deployment, LabelSet,
    MetricFacts, Raqe, WorkloadFacts,
};

const QUERY_COUNT: usize = 10;
const MINUTE_MS: u64 = 60_000;
const HOUR_MS: u64 = 60 * MINUTE_MS;
const DAY_MS: u64 = 24 * HOUR_MS;
const LOOKBACKS_MS: [u64; 5] = [5 * MINUTE_MS, HOUR_MS, 6 * HOUR_MS, DAY_MS, 7 * DAY_MS];
const INTERVALS_MS: [u64; 3] = [15_000, MINUTE_MS, 5 * MINUTE_MS];

const RANK_ERROR_METRIC: &str = "mean_rank_err";
const RANK_ERROR_SLA: f64 = 0.05;

fn label_set(names: &[&str]) -> LabelSet {
    names.iter().map(|name| name.to_string()).collect()
}

fn metric_name(query_index: usize) -> String {
    format!("service_{query_index}_request_duration_seconds")
}

/// The flag's value, or `None` when the flag is absent.
fn flag_value(flag: &str) -> Option<String> {
    let args: Vec<_> = std::env::args().collect();
    let index = args.iter().position(|arg| arg == flag)?;
    Some(
        args.get(index + 1)
            .unwrap_or_else(|| panic!("{flag} requires a value"))
            .clone(),
    )
}

fn weight_flag(flag: &str, default: f64) -> f64 {
    flag_value(flag).map_or(default, |value| {
        value
            .parse::<f64>()
            .ok()
            .filter(|weight| *weight >= 0.0)
            .unwrap_or_else(|| panic!("{flag} requires a non-negative number"))
    })
}

fn reject_unknown_args() {
    const SWITCHES: &[&str] = &["--allow-undeployable-families"];
    const WITH_VALUE: &[&str] = &["--cost-dir", "--saturation-dir", "--w-cpu", "--w-mem"];
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        if WITH_VALUE.contains(&arg.as_str()) {
            args.next();
        } else if !SWITCHES.contains(&arg.as_str()) {
            panic!("unknown argument: {arg}");
        }
    }
}

fn load_cost_table() -> AtomicCostTable {
    let cost_dir = flag_value("--cost-dir").expect("--cost-dir DIR is required");
    let path = std::path::Path::new(&cost_dir).join("rqe_atomic_costs.json");
    let raw = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("couldn't read {} ({e})", path.display()));
    serde_json::from_str(&raw)
        .unwrap_or_else(|e| panic!("{} isn't a valid AtomicCostTable: {e}", path.display()))
}

fn load_saturation_curves(cost_table: &AtomicCostTable) -> SaturationCurves {
    let dir = flag_value("--saturation-dir").expect("--saturation-dir DIR is required");
    let curves = SaturationCurves::load(std::path::Path::new(&dir))
        .unwrap_or_else(|e| panic!("couldn't load saturation curves: {e}"));
    // ponytail: warn rather than panic, since a config with no curve is never
    // eligible; panic like small_problem once the cost export matches the grid.
    let check = curves.check_cost_table(cost_table);
    for row in check.unchecked.iter().chain(&check.mismatched) {
        eprintln!("cost table vs. saturation curves: {row}");
    }
    curves
}

/// Each metric: 5 services × 10 endpoints × 600 pods = 30,000 series scraped
/// every 15 s, with durations from 1 ms to 60 s, as in `small_problem`.
fn facts() -> WorkloadFacts {
    (0..QUERY_COUNT)
        .map(|query_index| {
            let metric_facts = MetricFacts {
                labels: label_set(&["service", "endpoint", "pod"]),
                scrape_interval_ms: 15_000,
                cardinality: [
                    (label_set(&["service"]), 5),
                    (label_set(&["service", "endpoint"]), 50),
                    (label_set(&["service", "endpoint", "pod"]), 30_000),
                ]
                .into(),
                value_range: Some((0.001, 60.0)),
                data_shape: [(
                    label_set(&["service", "endpoint"]),
                    DataShape {
                        zipf_s: 1.1,
                        distinct_keys: 1_000.0,
                        tail_index: 1.5,
                    },
                )]
                .into(),
            };
            (metric_name(query_index), metric_facts)
        })
        .collect()
}

fn raqes(lookback_ms: u64, interval_ms: u64) -> Vec<Raqe> {
    (0..QUERY_COUNT)
        .map(|query_index| Raqe {
            id: format!("p99_{query_index}"),
            capability: Capability::Quantile,
            lookback_ms,
            interval_ms,
            metric: metric_name(query_index),
            spatial_filter: String::new(),
            grouping_labels: label_set(&["service", "endpoint"]),
            accuracy_metric: RANK_ERROR_METRIC.to_string(),
            accuracy_sla: RANK_ERROR_SLA,
            accuracy_direction: AccuracyDirection::LowerIsBetter,
            latency_sla_ms: None,
        })
        .collect()
}

fn duration_label(ms: u64) -> String {
    match ms {
        _ if ms.is_multiple_of(DAY_MS) => format!("{}d", ms / DAY_MS),
        _ if ms.is_multiple_of(HOUR_MS) => format!("{}h", ms / HOUR_MS),
        _ if ms.is_multiple_of(MINUTE_MS) => format!("{}m", ms / MINUTE_MS),
        _ => format!("{}s", ms / 1_000),
    }
}

fn describe(deployment: &Deployment) -> String {
    format!(
        "{} {} x={} y={}",
        deployment.config.sketch,
        deployment.config.sketch_config,
        duration_label(deployment.window_ms),
        duration_label(deployment.slide_ms),
    )
}

fn main() {
    reject_unknown_args();
    let cost_table = load_cost_table();
    let curves = load_saturation_curves(&cost_table);
    let facts = facts();
    let objective = Objective::AUCCost {
        w_cpu: weight_flag("--w-cpu", 1.0),
        w_mem: weight_flag("--w-mem", 0.0),
    };
    let allow_undeployable = std::env::args().any(|arg| arg == "--allow-undeployable-families");
    let accuracy = |raqe: &Raqe, deployment: &Deployment| curves.accuracy(raqe, deployment, &facts);

    println!("{QUERY_COUNT} quantile queries per scenario, {objective:?}");
    println!(
        "{:<8} {:<8} {:>10} {:>10} {:>9} {:>8} {:>12}  deployment(s)",
        "lookback", "interval", "objective", "cpu", "mem_MB", "merged", "latency_ms"
    );
    for lookback_ms in LOOKBACKS_MS {
        for interval_ms in INTERVALS_MS {
            let raqes = raqes(lookback_ms, interval_ms);
            if let Err(problems) = validate_facts(&raqes, &facts) {
                panic!("invalid workload facts: {problems:#?}");
            }
            let deployments =
                build_all_candidates(&raqes, &cost_table, &facts, allow_undeployable, &accuracy);
            let scenario = format!(
                "{:<8} {:<8}",
                duration_label(lookback_ms),
                duration_label(interval_ms)
            );
            if !unservable(&raqes, &deployments, &facts, &accuracy).is_empty() {
                println!("{scenario} unservable: no eligible deployment");
                continue;
            }
            let solution = match minimize(&raqes, &deployments, &facts, objective, &accuracy) {
                Ok(solution) => solution,
                Err(error) => {
                    println!("{scenario} infeasible: {error}");
                    continue;
                }
            };
            // The queries are identical up to metric, so their choices
            // normally agree; list every distinct one in case they don't.
            let mut chosen: Vec<String> = solution
                .deployments
                .iter()
                .map(|planned| describe(&planned.deployment))
                .collect();
            chosen.sort();
            chosen.dedup();
            let plan_cost = &solution.plan_cost;
            let max_latency_ms = plan_cost
                .query_latency_ms
                .iter()
                .copied()
                .fold(0.0, f64::max);
            println!(
                "{scenario} {:>10.3e} {:>10.3e} {:>9.1} {:>8} {:>12.3e}  {}",
                objective.value(plan_cost),
                plan_cost.cpu_secs_per_sec(),
                plan_cost.memory_bytes() / 1e6,
                solution.raqes[0].merged_instance_count,
                max_latency_ms,
                chosen.join(" | "),
            );
        }
    }
}
