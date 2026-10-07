//! AutoSketch vs. the ASAP planner (ProjectASAP/ASAPQuery#777, paper §6.3).
//!
//! One invocation evaluates one workload and writes one JSON file:
//!
//! ```text
//! autosketch_vs_asap traces --dataset alibaba_v2022 --saturation-dir DIR --out FILE [--runs 5]
//! autosketch_vs_asap synthetic --table FILE --target default --saturation-dir DIR --out FILE [--runs 1]
//!
//! Options for both: --slas-ms 0.01,inf (override the SLA grid), --weights
//! cpu,fargate (a subset of the objective weights), --no-chosen (drop
//! per-RQE choices). The synthetic --target is a strictness level (loose,
//! default, strict).
//! ```
//!
//! Every plan is scored by `analytical_cost_model::score` (#145): mean CPU and
//! memory per phase. ASAP and PerQuery-CostAware minimize
//! `w_cpu · CPU + w_mem · memory_GiB` at each weight setting of #777 §4:
//! CPU only, and Fargate's per-vCPU and per-GB prices. AutoSketch's plan does
//! not depend on the weights and is scored under each.
//!
//! Methods:
//! - **ASAP:** `milp::minimize` over every RQE jointly, with one absolute
//!   latency SLA for every RQE, swept over the SLA grid. RQEs that no eligible
//!   deployment can serve within an SLA are excluded from every method at
//!   that SLA, and listed in the output.
//! - **AutoSketch-Adapted:** `autosketch::plan`, one search and one dedicated
//!   deployment per RQE, accuracy only.
//! - **PerQuery-CostAware:** `milp::minimize` on each RQE alone, under the
//!   same SLA; the per-RQE deployments are kept separate.
//!
//! `traces` reads `data/autosketch-eval/table.json`; `synthetic` reads one
//! table written by `export_autosketch_eval_table.py --synthetic`. A table
//! gives the workload: RQEs, and each stream's groups, rate and data shape
//! (the family's `grid_param`/`grid_K`, the worst case rounded to the grid).
//! Each stream (`traces`: dataset, query and kind; `synthetic`: the table's
//! `stream`) is one metric, which is where sharing comes from.
//!
//! Costs and accuracy come from `--saturation-dir`, the same inputs the
//! planner reads (#174): the cost table `optimizer_cost/rqe_atomic_costs.json`
//! (one row per config, at the evaluation dataset's shape) and the saturation
//! curves. ASAP reads `SaturationCurves::accuracy` (the curve at the
//! lookback's items; KLL and top-k merged from `L/x` shards read the merge
//! curve, #158); AutoSketch reads `autosketch_accuracy` (one unmerged sketch
//! per query window, no saturation requirement).

use std::collections::{BTreeMap, BTreeSet};
use std::time::Instant;

use rqe_optimizer::analytical_cost_model::{score, PlanCost};
use rqe_optimizer::autosketch;
use rqe_optimizer::candidates::{build_all_candidates, eligible_deployments_for, is_eligible};
use rqe_optimizer::milp::{minimize, MilpSolution, Objective};
use rqe_optimizer::saturation::{DataShape, SaturationCurves, COST_TABLE};
use rqe_optimizer::{
    validate_facts, AtomicCostEntry, AtomicCostTable, Capability, Deployment, LabelSet, Mapping,
    MetricFacts, Millis, Raqe, WorkloadFacts,
};
use serde_json::{json, Value};

const TRACES_TABLE: &str = "rqe-optimizer/data/autosketch-eval/table.json";
/// Absolute per-RQE latency SLAs in ms (#777 §5), plus no SLA.
const SLAS_MS: [f64; 5] = [0.01, 0.1, 1.0, 10.0, f64::INFINITY];
/// Objective weights of #777 §4: CPU only, then AWS Fargate (us-east-1,
/// Linux/x86) $/vCPU-hour and $/GB-hour, so the objective reads in $/hour.
const WEIGHTS: [(&str, f64, f64); 2] = [("cpu", 1.0, 0.0), ("fargate", 0.0405, 0.00445)];
/// Every stream is scraped once a second: windows and slides are whole
/// seconds, and the stream's samples/s become its series count.
const SCRAPE_MS: Millis = 1_000;
const BYTES_PER_GIB: f64 = (1u64 << 30) as f64;
const SEED: u64 = 7;
/// AutoSketch's benchmark time per evaluated configuration, at the paper's
/// rate: 1–2 minutes each (NSDI '24, §5.2 and Exp#9); we take 1 minute.
const PAPER_SECS_PER_PROBE: f64 = 60.0;
/// Items per benchmark run in AutoSketch's benchmark-time lower bound:
/// sketch-bench's measurement size.
const N_BENCH: f64 = 1e8;

/// (distinct probed configs, lower-bound seconds) for probes given as
/// (config name, insert CPU per item, query-phase CPU): each distinct config
/// is benchmarked once, inserting N_BENCH items and running one query phase.
fn benchmark_lower_bound(probes: &[(String, f64, f64)]) -> (usize, f64) {
    let mut seen = BTreeSet::new();
    let mut secs = 0.0;
    for (name, insert, phase) in probes {
        if seen.insert(name) {
            secs += N_BENCH * insert + phase;
        }
    }
    (seen.len(), secs)
}

/// One workload: RQEs, one metric per stream, the cost entries each stream
/// may use, and the curves every accuracy is read from.
struct Workload {
    name: String,
    raqes: Vec<Raqe>,
    facts: WorkloadFacts,
    costs: BTreeMap<String, Vec<AtomicCostEntry>>,
    curves: SaturationCurves,
    notes: Vec<String>,
}

impl Workload {
    /// ASAP's accuracy: the planner's own lookup.
    fn asap_accuracy(&self, r: &Raqe, d: &Deployment) -> Option<f64> {
        self.curves.accuracy(r, d, &self.facts)
    }

    /// AutoSketch's: one unmerged sketch per query window.
    fn autosketch_accuracy(&self, r: &Raqe, c: &AtomicCostEntry) -> Option<f64> {
        self.curves.autosketch_accuracy(r, c, &self.facts)
    }
}

fn arg(args: &[String], name: &str) -> Option<String> {
    args.windows(2)
        .find(|pair| pair[0] == name)
        .map(|pair| pair[1].clone())
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let out = arg(&args, "--out").expect("--out FILE is required");
    let runs: usize = arg(&args, "--runs").map_or(5, |r| r.parse().unwrap());
    let saturation_dir = arg(&args, "--saturation-dir").expect("--saturation-dir DIR is required");
    let workload = match args.get(1).map(String::as_str) {
        Some("traces") => from_table(
            TRACES_TABLE,
            Some(&arg(&args, "--dataset").expect("--dataset")),
            None,
            &saturation_dir,
        ),
        Some("synthetic") => from_table(
            &arg(&args, "--table").expect("--table"),
            None,
            Some(arg(&args, "--target").expect("--target")),
            &saturation_dir,
        ),
        _ => panic!("usage: autosketch_vs_asap (traces|synthetic) ... --out FILE"),
    };
    let slas: Vec<f64> = arg(&args, "--slas-ms").map_or(SLAS_MS.to_vec(), |list| {
        list.split(',')
            .map(|x| {
                if x == "inf" {
                    f64::INFINITY
                } else {
                    x.parse().unwrap()
                }
            })
            .collect()
    });
    let weights: Vec<(&str, Objective)> = WEIGHTS
        .iter()
        .filter(|(name, ..)| {
            arg(&args, "--weights").is_none_or(|list| list.split(',').any(|w| w == *name))
        })
        .map(|&(name, w_cpu, w_mem)| (name, Objective::AUCCost { w_cpu, w_mem }))
        .collect();
    let mut result = evaluate(&workload, runs, &slas, &weights);
    if args.iter().any(|a| a == "--no-chosen") {
        for r in result["results"].as_array_mut().unwrap() {
            r.as_object_mut().unwrap().remove("chosen");
        }
    }
    std::fs::write(&out, serde_json::to_string_pretty(&result).unwrap()).unwrap();
    eprintln!("wrote {out}");
}

// ---------------------------------------------------------------- workloads

/// Per-capability accuracy targets for a strictness level of the synthetic
/// workload (#777 §5). Exact accumulators must have zero error.
fn strictness_target(level: &str, capability: Capability) -> f64 {
    let column = match level {
        "loose" => 0,
        "default" => 1,
        "strict" => 2,
        other => panic!("unknown strictness level {other}"),
    };
    let targets = match capability {
        Capability::Quantile => [0.02, 0.01, 0.005],
        Capability::TopKByValue | Capability::TopKByCount => [0.90, 0.95, 0.99],
        Capability::Cardinality => [0.05, 0.02, 0.01],
        Capability::Sum
        | Capability::Count
        | Capability::Min
        | Capability::Max
        | Capability::RateOrIncrease => [0.0; 3],
    };
    targets[column]
}

/// The table's capability for an RQE. `traces` key queries are group sums,
/// served by exact accumulators; its tables list sketch families only, so
/// those RQEs stay unservable until the table carries an exact row.
fn capability_of(r: &Value) -> Capability {
    match (r["capability"].as_str(), r["kind"].as_str().unwrap()) {
        (Some("sum"), _) | (Some("freq"), _) | (None, "keys") => Capability::Sum,
        (Some("rate"), _) => Capability::RateOrIncrease,
        // `topk(k, sum by (...) (...))`: ranked by summed value.
        (Some("topk"), _) => Capability::TopKByValue,
        (Some("quantile"), _) | (None, "values") => Capability::Quantile,
        other => panic!("unknown capability {other:?}"),
    }
}

fn label_set(names: &[&str]) -> LabelSet {
    names.iter().map(|s| s.to_string()).collect()
}

/// One workload of an evaluation table: `traces`'s `dataset`, or a
/// `synthetic` table's only workload. `target` sets every RQE's accuracy
/// target to a strictness level.
fn from_table(
    path: &str,
    dataset: Option<&str>,
    target: Option<String>,
    saturation_dir: &str,
) -> Workload {
    let table: Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    let dir = std::path::Path::new(saturation_dir);
    let curves = SaturationCurves::load(dir)
        .unwrap_or_else(|e| panic!("couldn't load saturation curves from {saturation_dir}: {e}"));
    let cost_path = dir.join(COST_TABLE);
    let cost_table: AtomicCostTable =
        serde_json::from_str(&std::fs::read_to_string(&cost_path).unwrap_or_else(|e| {
            panic!(
                "couldn't read {} ({e}); run study_saturation.py --phase optimizer-cost",
                cost_path.display()
            )
        }))
        .unwrap();
    let workload = table["workloads"]
        .as_array()
        .unwrap()
        .iter()
        .find(|w| dataset.is_none_or(|d| w["dataset"] == d))
        .unwrap_or_else(|| panic!("no workload {dataset:?} in {path}"));
    let dataset = workload["dataset"].as_str().unwrap();
    let mut raqes = Vec::new();
    let mut facts = WorkloadFacts::new();
    // Queries one evaluation issues per instance, per stream (max over its RQEs).
    let mut queries_per_stream: BTreeMap<String, f64> = BTreeMap::new();
    let mut skipped_families = 0;
    let mut rounded_up_streams = BTreeSet::new();
    for r in workload["rqes"].as_array().unwrap() {
        let id = r["id"].as_str().unwrap().to_string();
        let stream = match r["stream"].as_str() {
            Some(stream) => format!("{dataset}/{stream}"),
            None => format!(
                "{}/{}/{}",
                dataset,
                r["query_id"].as_str().unwrap(),
                r["kind"].as_str().unwrap()
            ),
        };
        let capability = capability_of(r);
        let rate = r["label_set"]["arrival_rate_per_sec"].as_f64().unwrap();
        let groups = r["label_set"]["groups"].as_u64().unwrap();
        let grouping = if groups > 1 {
            label_set(&["g"])
        } else {
            LabelSet::new()
        };
        // One metric per stream, labels {g, x}: `g` is the grouping (card =
        // groups), and with one scrape a second `{g, x}` has one series per
        // sample/s. A stream's RQEs differ only in range, so take the
        // largest per-RQE rate (conservative). Rates below `groups`
        // samples/s are rounded up so every group has a series.
        let series = (rate * SCRAPE_MS as f64 / 1000.0).round() as u64;
        if series < groups {
            rounded_up_streams.insert(stream.clone());
        }
        let metric = facts.entry(stream.clone()).or_insert_with(|| MetricFacts {
            labels: label_set(&["g", "x"]),
            scrape_interval_ms: SCRAPE_MS,
            cardinality: [
                (label_set(&["g", "x"]), 0),
                (label_set(&["g"]), groups),
                (LabelSet::new(), 1),
            ]
            .into(),
            value_range: None,
            data_shape: BTreeMap::new(),
        });
        let total = metric.cardinality.get_mut(&label_set(&["g", "x"])).unwrap();
        *total = (*total).max(series).max(groups);
        // Queries one evaluation issues per instance (the table's q_r); tables
        // written before it existed charge one query phase per evaluation.
        let queries = r["queries_per_instance"].as_f64().unwrap_or(1.0);
        let q = queries_per_stream.entry(stream.clone()).or_default();
        *q = q.max(queries);

        // The first family of the RQE's capability gives its target and the
        // stream's data shape; the table rounds both to the grid.
        let mut rqe_target = None;
        for family in r["families"].as_array().unwrap() {
            let sketch = family["sketch"].as_str().unwrap();
            if !capability.families().contains(&sketch) {
                // e.g. `traces`' frequency sketches, or top-k for a sum query.
                skipped_families += 1;
                continue;
            }
            if rqe_target.is_none() {
                rqe_target = Some(match target.as_deref() {
                    Some(level) => strictness_target(level, capability),
                    None => family["target"].as_f64().unwrap(),
                });
            }
            if let Some(shape) = data_shape(family) {
                // Several RQEs of a stream: the harder side of each parameter.
                metric
                    .data_shape
                    .entry(grouping.clone())
                    .and_modify(|old| {
                        old.zipf_s = old.zipf_s.min(shape.zipf_s);
                        old.distinct_keys = old.distinct_keys.max(shape.distinct_keys);
                        old.tail_index = old.tail_index.min(shape.tail_index);
                    })
                    .or_insert(shape);
            }
        }
        raqes.push(Raqe {
            id: id.clone(),
            capability,
            lookback_ms: (r["lookback_secs"].as_f64().unwrap() * 1000.0) as Millis,
            interval_ms: (r["interval_secs"].as_f64().unwrap() * 1000.0) as Millis,
            metric: stream,
            spatial_filter: String::new(),
            grouping_labels: grouping,
            // An RQE with no family of its capability is unservable; its
            // target is never read.
            accuracy_sla: rqe_target.unwrap_or(0.0),
            latency_sla_ms: None,
        });
    }
    // Every stream may use every row; one evaluation runs q_r queries per
    // instance.
    let costs = queries_per_stream
        .into_iter()
        .map(|(stream, queries)| {
            let rows = cost_table
                .iter()
                .map(|row| AtomicCostEntry {
                    query_cpu_secs: queries * row.query_cpu_secs,
                    ..row.clone()
                })
                .collect();
            (stream, rows)
        })
        .collect();
    let name = match &target {
        Some(t) => format!("{dataset}/t{t}"),
        None => format!("traces/{dataset}"),
    };
    Workload {
        name,
        raqes,
        facts,
        costs,
        curves,
        notes: vec![
            format!("{skipped_families} families skipped: they serve another capability"),
            "one metric per stream: groups from the table, samples/s = max over the stream's RQEs, scraped once a second".into(),
            format!("streams below one sample/s per group, rounded up: {rounded_up_streams:?}"),
            format!("costs: {COST_TABLE} under the saturation dir, one row per config"),
            "data shape per stream: the families' grid_param/grid_K, the harder side over its RQEs".into(),
            "query_cpu_secs = q_r queries per instance per evaluation".into(),
        ],
    }
}

/// A table family's grid point as a curve key: Zipf θ and K, or the Pareto
/// tail index for quantiles. The unused fields are never read.
fn data_shape(family: &Value) -> Option<DataShape> {
    let param = family["grid_param"].as_f64()?;
    let keys = &family["grid_K"];
    match keys
        .as_f64()
        .or_else(|| keys.as_str().and_then(|k| k.parse().ok()))
    {
        Some(keys) => Some(DataShape {
            zipf_s: param,
            distinct_keys: keys,
            tail_index: f64::INFINITY,
        }),
        None => Some(DataShape {
            zipf_s: f64::INFINITY,
            distinct_keys: 0.0,
            tail_index: param,
        }),
    }
}

// --------------------------------------------------------------- evaluation

fn median(mut v: Vec<f64>) -> f64 {
    v.sort_by(f64::total_cmp);
    v[v.len() / 2]
}

fn group_by_stream(raqes: &[Raqe]) -> BTreeMap<&str, Vec<Raqe>> {
    let mut out: BTreeMap<&str, Vec<Raqe>> = BTreeMap::new();
    for r in raqes {
        out.entry(r.metric.as_str()).or_default().push(r.clone());
    }
    out
}

/// ASAP candidates, built per stream from that stream's entries.
fn candidates(w: &Workload, raqes: &[Raqe]) -> Vec<Deployment> {
    group_by_stream(raqes)
        .into_iter()
        .flat_map(|(stream, group)| {
            let costs = w.costs.get(stream).map_or(&[][..], Vec::as_slice);
            build_all_candidates(&group, costs, &w.facts, false, &|r, d| {
                w.asap_accuracy(r, d)
            })
        })
        .collect()
}

/// AutoSketch, searched per stream over that stream's entries.
fn autosketch_plan(
    w: &Workload,
    raqes: &[Raqe],
) -> Result<autosketch::AutoSketchPlan, Vec<String>> {
    let mut deployments = Vec::new();
    let mut searches = Vec::new();
    let mut unservable = Vec::new();
    let mut order = Vec::new();
    for (stream, group) in group_by_stream(raqes) {
        let costs = w.costs.get(stream).map_or(&[][..], Vec::as_slice);
        let oracle = |r: &Raqe, c: &AtomicCostEntry| w.autosketch_accuracy(r, c);
        match autosketch::plan(&group, costs, SEED, false, oracle) {
            Ok(plan) => {
                deployments.extend(plan.deployments);
                searches.extend(plan.searches);
                order.extend(group.iter().map(|r| r.id.clone()));
            }
            Err(ids) => unservable.extend(ids),
        }
    }
    if !unservable.is_empty() {
        return Err(unservable);
    }
    // Back to the caller's RQE order.
    let position: BTreeMap<&str, usize> = order
        .iter()
        .enumerate()
        .map(|(i, id)| (id.as_str(), i))
        .collect();
    let mapping: Mapping = raqes.iter().map(|r| position[r.id.as_str()]).collect();
    Ok(autosketch::AutoSketchPlan {
        deployments,
        mapping,
        searches,
    })
}

/// Modeled latency of `r` on `d`, in ms.
fn latency_ms(r: &Raqe, d: &Deployment, facts: &WorkloadFacts) -> f64 {
    score(
        std::slice::from_ref(r),
        std::slice::from_ref(d),
        &vec![0],
        facts,
    )
    .query_latency_ms[0]
}

fn phase(p: &rqe_optimizer::analytical_cost_model::PhaseCost) -> Value {
    json!({"cpu": p.cpu_secs_per_sec, "gib": p.memory_bytes / BYTES_PER_GIB})
}

fn summarize(
    w: &Workload,
    raqes: &[Raqe],
    deployments: &[Deployment],
    mapping: &Mapping,
    cost: &PlanCost,
    objective: &Objective,
    sla_ms: Option<f64>,
) -> Value {
    let active: BTreeSet<usize> = mapping.iter().copied().collect();
    let violations = cost
        .query_latency_ms
        .iter()
        .filter(|&&l| sla_ms.is_some_and(|sla| l > sla * (1.0 + 1e-9)))
        .count();
    let chosen: Vec<Value> = raqes
        .iter()
        .zip(mapping)
        .zip(&cost.query_latency_ms)
        .map(|((r, &di), &latency)| {
            let d = &deployments[di];
            json!({
                "rqe": r.id,
                "sketch": d.config.sketch,
                "params": d.config.sketch_config["params"],
                "window_secs": d.window_ms / 1000,
                "slide_secs": d.slide_ms / 1000,
                "merged_shards": r.lookback_ms / d.window_ms,
                "latency_ms": latency,
                "deployment": di,
                "asap_accuracy": w.asap_accuracy(r, d),
            })
        })
        .collect();
    json!({
        "objective": objective.value(cost),
        "cpu": cost.cpu_secs_per_sec(),
        "gib": cost.memory_bytes() / BYTES_PER_GIB,
        "phases": {
            "ingest": phase(&cost.ingest),
            "merge": phase(&cost.merge),
            "query": phase(&cost.query),
            "storage": phase(&cost.storage),
        },
        "latency_violations": violations,
        "max_latency_ms": cost.query_latency_ms.iter().copied().fold(0.0, f64::max),
        "median_latency_ms": (!cost.query_latency_ms.is_empty())
            .then(|| median(cost.query_latency_ms.clone())),
        "feasible": violations == 0,
        "active_deployments": active.len(),
        "chosen": chosen,
    })
}

/// A MILP solution's active deployments, and which one serves each RQE.
fn planned(s: &MilpSolution) -> (Vec<Deployment>, Mapping) {
    (
        s.deployments.iter().map(|p| p.deployment.clone()).collect(),
        s.raqes.iter().map(|p| p.deployment).collect(),
    )
}

fn sla_label(sla_ms: f64) -> Value {
    if sla_ms.is_finite() {
        json!(sla_ms)
    } else {
        json!("inf")
    }
}

fn evaluate(w: &Workload, runs: usize, slas: &[f64], weights: &[(&str, Objective)]) -> Value {
    let asap_accuracy = |r: &Raqe, d: &Deployment| w.asap_accuracy(r, d);
    // Drop RQEs that either method cannot serve, so both plan the same batch.
    let all_candidates = candidates(w, &w.raqes);
    let asap_unservable: Vec<String> = w
        .raqes
        .iter()
        .filter(|r| {
            eligible_deployments_for(r, &all_candidates, &w.facts, &asap_accuracy).is_empty()
        })
        .map(|r| r.id.clone())
        .collect();
    let autosketch_unservable = autosketch_plan(w, &w.raqes).err().unwrap_or_default();
    let dropped: BTreeSet<String> = asap_unservable
        .iter()
        .chain(&autosketch_unservable)
        .cloned()
        .collect();
    let raqes: Vec<Raqe> = w
        .raqes
        .iter()
        .filter(|r| !dropped.contains(&r.id))
        .cloned()
        .collect();
    eprintln!(
        "{}: {} RQEs, {} dropped",
        w.name,
        raqes.len(),
        dropped.len()
    );
    if let Err(problems) = validate_facts(&raqes, &w.facts) {
        panic!("invalid facts: {problems:?}");
    }

    // Fastest latency each RQE can reach on any eligible deployment. An RQE
    // over an SLA is excluded from every method at that SLA.
    let min_latency: Vec<f64> = raqes
        .iter()
        .map(|r| {
            eligible_deployments_for(r, &all_candidates, &w.facts, &asap_accuracy)
                .into_iter()
                .map(|di| latency_ms(r, &all_candidates[di], &w.facts))
                .fold(f64::INFINITY, f64::min)
        })
        .collect();

    // AutoSketch: one plan, independent of the weights and the SLA.
    let mut search_secs = Vec::new();
    let mut plan = None;
    for _ in 0..runs {
        let started = Instant::now();
        plan = Some(autosketch_plan(w, &raqes).expect("servable RQEs only"));
        search_secs.push(started.elapsed().as_secs_f64());
    }
    let plan = plan.unwrap();
    let probes: usize = plan.searches.iter().map(|s| s.probes.len()).sum();
    let probed: Vec<(String, f64, f64)> = plan
        .searches
        .iter()
        .flat_map(|s| {
            let r = raqes.iter().find(|r| r.id == s.raqe_id).unwrap();
            let costs = &w.costs[&r.metric];
            s.probes.iter().map(move |&p| {
                let c = &costs[p];
                // One query of one instance stands in for the query phase.
                let name = format!("{}/{}", c.sketch, c.sketch_config["params"]);
                (name, c.insert_cpu_secs, c.query_cpu_secs)
            })
        })
        .collect();
    let (distinct_probes, bench_secs) = benchmark_lower_bound(&probed);
    let paper_secs = distinct_probes as f64 * PAPER_SECS_PER_PROBE;

    // PerQuery-CostAware's candidates: each RQE alone. Solved per SLA below.
    let started = Instant::now();
    let singles: Vec<Vec<Deployment>> = raqes
        .iter()
        .map(|r| candidates(w, std::slice::from_ref(r)))
        .collect();
    let singles_build_secs = started.elapsed().as_secs_f64();

    let mut results = Vec::new();
    let mut sanity = Vec::new();
    let mut excluded_by_sla = serde_json::Map::new();
    for &sla in slas {
        let sla_ms = sla.is_finite().then_some(sla);
        let kept: Vec<usize> = (0..raqes.len())
            .filter(|&i| min_latency[i] <= sla)
            .collect();
        let excluded: Vec<&str> = (0..raqes.len())
            .filter(|i| !kept.contains(i))
            .map(|i| raqes[i].id.as_str())
            .collect();
        excluded_by_sla.insert(
            sla_label(sla).to_string().trim_matches('"').to_string(),
            json!(excluded),
        );
        if kept.is_empty() {
            continue;
        }
        let sla_raqes: Vec<Raqe> = kept
            .iter()
            .map(|&i| Raqe {
                latency_sla_ms: sla_ms,
                ..raqes[i].clone()
            })
            .collect();
        let identity: Mapping = (0..sla_raqes.len()).collect();

        let mut build_secs = Vec::new();
        let mut asap_candidates = Vec::new();
        for _ in 0..runs {
            let started = Instant::now();
            asap_candidates = candidates(w, &sla_raqes);
            build_secs.push(started.elapsed().as_secs_f64());
        }
        let auto_deployments: Vec<Deployment> = kept
            .iter()
            .map(|&i| plan.deployments[plan.mapping[i]].clone())
            .collect();
        let auto_cost = score(&sla_raqes, &auto_deployments, &identity, &w.facts);

        for (weight_name, objective) in weights {
            let mut solve_secs = Vec::new();
            let mut solution = None;
            for _ in 0..runs {
                let started = Instant::now();
                let solved = minimize(
                    &sla_raqes,
                    &asap_candidates,
                    &w.facts,
                    *objective,
                    &asap_accuracy,
                );
                solve_secs.push(started.elapsed().as_secs_f64());
                solution = Some(solved);
            }
            let asap = match solution.unwrap() {
                Ok(s) => {
                    let (deployments, mapping) = planned(&s);
                    let mut v = summarize(
                        w,
                        &sla_raqes,
                        &deployments,
                        &mapping,
                        &s.plan_cost,
                        objective,
                        sla_ms,
                    );
                    v["planning_secs"] =
                        json!(median(build_secs.clone()) + median(solve_secs.clone()));
                    v["candidate_build_secs"] = json!(median(build_secs.clone()));
                    v["milp_solve_secs"] = json!(median(solve_secs.clone()));
                    v["candidates"] = json!(asap_candidates.len());
                    v
                }
                Err(e) => json!({"error": e.to_string()}),
            };
            let mut auto = summarize(
                w,
                &sla_raqes,
                &auto_deployments,
                &identity,
                &auto_cost,
                objective,
                sla_ms,
            );
            auto["planning_secs"] = json!(median(search_secs.clone()));
            auto["probes"] = json!(probes);
            auto["benchmark_secs_lower_bound_nbench1e8"] = json!(bench_secs);
            auto["benchmark_secs_paper_rate"] = json!(paper_secs);
            // PerQuery-CostAware: each kept RQE alone under the SLA.
            let started = Instant::now();
            let perquery_deployments: Result<Vec<Deployment>, String> = kept
                .iter()
                .zip(&sla_raqes)
                .map(|(&i, r)| {
                    minimize(
                        std::slice::from_ref(r),
                        &singles[i],
                        &w.facts,
                        *objective,
                        &asap_accuracy,
                    )
                    .map(|solved| {
                        let (deployments, mapping) = planned(&solved);
                        deployments[mapping[0]].clone()
                    })
                    .map_err(|e| format!("{}: {e}", r.id))
                })
                .collect();
            let perquery_secs = started.elapsed().as_secs_f64() + singles_build_secs;
            let mut perq = match perquery_deployments {
                Ok(deployments) => {
                    let cost = score(&sla_raqes, &deployments, &identity, &w.facts);
                    summarize(
                        w,
                        &sla_raqes,
                        &deployments,
                        &identity,
                        &cost,
                        objective,
                        sla_ms,
                    )
                }
                Err(e) => json!({"error": e}),
            };
            perq["planning_secs"] = json!(perquery_secs);

            // ASAP meets every SLA, and costs no more than AutoSketch whenever
            // AutoSketch meets it too, nor than PerQuery.
            if asap.get("error").is_some() || asap["latency_violations"] != 0 {
                sanity.push(json!({
                    "check": "asap meets every SLA", "weights": weight_name,
                    "sla_ms": sla_label(sla), "asap": asap,
                }));
            }
            if let (Some(a), Some(b)) = (asap["objective"].as_f64(), auto["objective"].as_f64()) {
                if auto["feasible"] == true && a > b * (1.0 + 1e-6) {
                    let not_eligible: Vec<&str> = sla_raqes
                        .iter()
                        .zip(&auto_deployments)
                        .filter(|(r, d)| !is_eligible(r, d, &w.facts, &asap_accuracy))
                        .map(|(r, _)| r.id.as_str())
                        .collect();
                    sanity.push(json!({
                        "check": "asap <= autosketch", "weights": weight_name,
                        "sla_ms": sla_label(sla), "asap": a, "autosketch": b,
                        "autosketch_choices_ineligible_for_asap": not_eligible,
                    }));
                }
            }
            if perq.get("error").is_some() || perq["latency_violations"] != 0 {
                sanity.push(json!({"check": "perquery meets every SLA", "weights": weight_name, "sla_ms": sla_label(sla), "perquery": perq}));
            }
            if let (Some(a), Some(b)) = (asap["objective"].as_f64(), perq["objective"].as_f64()) {
                if a > b * (1.0 + 1e-6) {
                    sanity.push(json!({"check": "asap <= perquery", "weights": weight_name, "sla_ms": sla_label(sla), "asap": a, "perquery": b}));
                }
            }
            for (method, value) in [("asap", asap), ("autosketch", auto), ("perquery", perq)] {
                let mut v = value;
                v["method"] = json!(method);
                v["weights"] = json!(weight_name);
                v["sla_ms"] = sla_label(sla);
                v["rqes"] = json!(sla_raqes.len());
                v["excluded_by_sla"] = json!(excluded.len());
                results.push(v);
            }
        }
    }
    json!({
        "workload": w.name,
        "rqes": raqes.len(),
        "dropped_unservable": {"asap": asap_unservable, "autosketch": autosketch_unservable},
        "streams": raqes.iter().map(|r| &r.metric).collect::<BTreeSet<_>>().len(),
        "runs": runs,
        "notes": w.notes,
        "sla_grid_ms": slas.iter().map(|&l| sla_label(l)).collect::<Vec<_>>(),
        "weights": weights.iter().map(|(name, o)| {
            let Objective::AUCCost { w_cpu, w_mem } = o;
            json!({"name": name, "w_cpu": w_cpu, "w_mem": w_mem})
        }).collect::<Vec<_>>(),
        "min_latency_ms": raqes.iter().zip(&min_latency).map(|(r, &l)| (r.id.clone(), json!(l))).collect::<serde_json::Map<_, _>>(),
        "excluded_by_sla": excluded_by_sla,
        "autosketch": {
            "probes": probes,
            "search_secs": median(search_secs),
            "benchmark_secs_lower_bound_nbench1e8": bench_secs,
            "distinct_probes": distinct_probes,
            "benchmark_secs_paper_rate": paper_secs,
        },
        "sanity_violations": sanity,
        "results": results,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn benchmark_bound_counts_each_config_once_at_n_bench() {
        let probes = [
            ("cms/a".to_string(), 2e-8, 0.5),
            ("cms/b".to_string(), 1e-8, 0.25),
            ("cms/a".to_string(), 2e-8, 0.5),
        ];
        let (distinct, secs) = benchmark_lower_bound(&probes);
        assert_eq!(distinct, 2);
        assert!((secs - (1e8 * 2e-8 + 0.5 + 1e8 * 1e-8 + 0.25)).abs() < 1e-12);
    }

    #[test]
    fn exact_accumulators_must_have_zero_error_at_every_level() {
        for level in ["loose", "default", "strict"] {
            assert_eq!(strictness_target(level, Capability::Sum), 0.0);
            assert_eq!(strictness_target(level, Capability::RateOrIncrease), 0.0);
        }
        assert_eq!(strictness_target("strict", Capability::Quantile), 0.005);
    }
}
