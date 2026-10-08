//! MILP solver for one scalarized RAQE deployment optimization (§4).
//!
//! The model is documented in `docs/rqe_sketch_deployment_v1.md`. It avoids
//! enumerating the Cartesian product of eligible deployment choices.

use crate::analytical_cost_model::{self, score, PhaseCost, PlanCost, BYTES_PER_GIB};
use crate::candidates::eligible_deployments_for;
use crate::{Accuracy, Deployment, Mapping, Raqe, WorkloadFacts};
use good_lp::{
    default_solver, variable, Expression, ProblemVariables, ResolutionError, Solution, SolverModel,
    Variable,
};

/// What a [`minimize`] solve minimizes. Eligibility, assignment and activation
/// rows, and latency SLAs are the same for every objective.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Objective {
    /// `w_cpu · CPU + w_mem · memory`. CPU is mean CPU-sec/sec, the area under
    /// the load curve, so bursts are not priced. Memory is GiB summed over
    /// every phase, as if every query evaluates at once.
    AUCCost { w_cpu: f64, w_mem: f64 },
}

impl Default for Objective {
    /// CPU only, until memory gets a price.
    fn default() -> Self {
        Objective::AUCCost {
            w_cpu: 1.0,
            w_mem: 0.0,
        }
    }
}

impl Objective {
    pub fn value(&self, plan_cost: &PlanCost) -> f64 {
        self.weigh(PhaseCost {
            cpu_secs_per_sec: plan_cost.cpu_secs_per_sec(),
            memory_bytes: plan_cost.memory_bytes(),
        })
    }

    fn weigh(&self, cost: PhaseCost) -> f64 {
        let Objective::AUCCost { w_cpu, w_mem } = *self;
        w_cpu * cost.cpu_secs_per_sec + w_mem * cost.memory_bytes / BYTES_PER_GIB
    }
}

/// The solved plan: the deployments to run and which one serves each RAQE.
#[derive(Debug, Clone)]
pub struct MilpSolution {
    /// Active deployments only, in order of the first RAQE each serves.
    pub deployments: Vec<PlannedDeployment>,
    /// Index-aligned with the input RAQE slice.
    pub raqes: Vec<PlannedRaqe>,
    pub plan_cost: PlanCost,
}

#[derive(Debug, Clone)]
pub struct PlannedDeployment {
    pub deployment: Deployment,
    /// Windows kept per group (per sketch, not × `card(G)`): `x/y` open plus
    /// `(L − x)/y + 1` closed, for the longest lookback `L` it serves.
    pub retained_instance_count: u64,
}

#[derive(Debug, Clone)]
pub struct PlannedRaqe {
    /// Index into [`MilpSolution::deployments`].
    pub deployment: usize,
    /// `n = L/x` windows merged per query; 1 means Direct, more means Merge.
    pub merged_instance_count: u64,
}

/// Minimize `objective` subject to each RAQE's latency SLA. `facts` must pass
/// [`crate::validate_facts`]. Missing eligible deployments are a caller input
/// error; use `enumerate::unservable` to report them before calling this
/// function.
pub fn minimize(
    raqes: &[Raqe],
    deployments: &[Deployment],
    facts: &WorkloadFacts,
    objective: Objective,
    accuracy: &Accuracy,
) -> Result<MilpSolution, ResolutionError> {
    let eligible: Vec<Vec<usize>> = raqes
        .iter()
        .map(|raqe| eligible_deployments_for(raqe, deployments, facts, accuracy))
        .collect();
    assert!(
        eligible.iter().all(|choices| !choices.is_empty()),
        "cannot solve a workload with an unservable RAQE"
    );

    let mut variables = ProblemVariables::new();
    let active: Vec<Variable> = deployments
        .iter()
        .map(|_| variables.add(variable().binary()))
        .collect();
    let assignments: Vec<Vec<(usize, Variable)>> = eligible
        .iter()
        .map(|choices| {
            choices
                .iter()
                .map(|&deployment_index| (deployment_index, variables.add(variable().binary())))
                .collect()
        })
        .collect();
    // Weighted storage cost per deployment: the max over the RAQEs it serves.
    let deployment_storage_cost: Vec<Variable> = deployments
        .iter()
        .map(|_| variables.add(variable().min(0)))
        .collect();

    // Ingest is paid once per active deployment, merge and query once per
    // assignment, and storage once per deployment for its longest lookback.
    let ingest_cost: Vec<f64> = deployments
        .iter()
        .map(|deployment| objective.weigh(analytical_cost_model::ingest(deployment, facts)))
        .collect();
    let merge_and_query_cost = |raqe: &Raqe, deployment: &Deployment| {
        objective.weigh(analytical_cost_model::merge(raqe, deployment, facts))
            + objective.weigh(analytical_cost_model::query(raqe, deployment, facts))
    };
    let storage_cost_for_raqe_and_deployment = |raqe: &Raqe, deployment: &Deployment| {
        objective.weigh(PhaseCost {
            cpu_secs_per_sec: 0.0,
            memory_bytes: analytical_cost_model::storage_bytes(raqe, deployment, facts),
        })
    };

    // Real plans cost ~1e-6 CPU-sec/sec and read in microseconds, below
    // HiGHS's absolute gap (1e-6) and feasibility tolerance (1e-7). So costs
    // are divided by `reference`, a plan's cost without sharing (each RAQE on
    // its cheapest deployment), and latency SLAs become exclusions. The
    // result is re-scored on real costs.
    let reference: f64 = raqes
        .iter()
        .zip(&eligible)
        .map(|(raqe, choices)| {
            choices
                .iter()
                .map(|&deployment_index| {
                    let deployment = &deployments[deployment_index];
                    ingest_cost[deployment_index]
                        + merge_and_query_cost(raqe, deployment)
                        + storage_cost_for_raqe_and_deployment(raqe, deployment)
                })
                .fold(f64::INFINITY, f64::min)
        })
        .sum();
    let reference = if reference.is_normal() {
        reference
    } else {
        1.0
    };

    let mut goal: Expression = deployment_storage_cost.iter().sum();
    for (&cost, &active_variable) in ingest_cost.iter().zip(&active) {
        goal.add_mul(cost / reference, active_variable);
    }
    for (raqe_index, choices) in assignments.iter().enumerate() {
        for &(deployment_index, assignment) in choices {
            goal.add_mul(
                merge_and_query_cost(&raqes[raqe_index], &deployments[deployment_index])
                    / reference,
                assignment,
            );
        }
    }

    let mut model = variables.minimise(goal).using(default_solver);
    for (raqe_index, choices) in assignments.iter().enumerate() {
        let assignment_sum: Expression = choices.iter().map(|(_, variable)| *variable).sum();
        model.add_constraint(assignment_sum.eq(1));

        for &(deployment_index, assignment) in choices {
            let raqe = &raqes[raqe_index];
            let deployment = &deployments[deployment_index];
            model.add_constraint((assignment - active[deployment_index]).leq(0));
            // Zero when `w_mem` is 0; such a row can never bind.
            let scaled_storage_cost =
                storage_cost_for_raqe_and_deployment(raqe, deployment) / reference;
            if scaled_storage_cost > 0.0 {
                model.add_constraint(
                    (scaled_storage_cost * assignment - deployment_storage_cost[deployment_index])
                        .leq(0),
                );
            }
            // Each RAQE takes exactly one deployment, so its latency SLA just
            // forbids the choices over it. Comparing in f64 here, not in the
            // solver, keeps µs latencies clear of its feasibility tolerance.
            let latency_ms = analytical_cost_model::query_latency_ms(raqe, deployment, facts);
            if raqe.latency_sla_ms.is_some_and(|limit| latency_ms > limit) {
                model.add_constraint(Expression::from(assignment).leq(0));
            }
        }
    }

    for (deployment_index, active_variable) in active.iter().enumerate() {
        let assignments_using_deployment: Expression = assignments
            .iter()
            .flat_map(|choices| choices.iter())
            .filter(|(index, _)| *index == deployment_index)
            .map(|(_, assignment)| *assignment)
            .sum();
        model.add_constraint((*active_variable - assignments_using_deployment).leq(0));
    }

    let solved = model.solve()?;
    let mapping = assignments
        .iter()
        .map(|choices| {
            choices
                .iter()
                .find_map(|&(deployment_index, assignment)| {
                    (solved.value(assignment) > 0.5).then_some(deployment_index)
                })
                .expect("MILP assigns exactly one deployment to every RAQE")
        })
        .collect();
    let plan_cost = score(raqes, deployments, &mapping, facts);
    let (deployments, raqes) = plan(raqes, deployments, &mapping);
    Ok(MilpSolution {
        deployments,
        raqes,
        plan_cost,
    })
}

/// Which cost model [`minimize_cost_model`] minimizes
/// (`docs/rqe_sketch_deployment_v1.md`, "Cost models, batch latency and
/// query placement").
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CostModel {
    /// `w_cpu · AUC(CPU) + w_mem · AUC(memory)`: billed by use, CPU elastic.
    /// Exact and linear.
    One,
    /// `w_cpu · max(CPU) + w_mem · max(memory)`: a provisioned `(C, M)`,
    /// billed at the peak. The MILP's batch row is a surrogate, corrected by
    /// the placement (see [`minimize_cost_model`]).
    Two,
}

/// A [`minimize_cost_model`] solution, priced by its cost model.
#[derive(Debug, Clone)]
pub struct CostModelSolution {
    pub milp: MilpSolution,
    /// One candidate index per RAQE.
    pub mapping: Mapping,
    pub cost: crate::placement::ModelCost,
    /// MILP solves (more than one only for cost model 2).
    pub solves: usize,
}

/// Correction passes for cost model 2, beyond the first solve.
const COST_MODEL_2_CORRECTIONS: usize = 3;

/// Minimize `cost_model` with weights `w_cpu`, `w_mem` (per vCPU, per GiB),
/// optionally under a batch latency SLA `sla_ms`.
///
/// Cost model 1 is solved exactly. Cost model 2's batch row
/// (`Σ ℓ + Σ compaction ≤ L · (C − ingest)`) treats the heaviest batch as
/// divisible, and its memory row holds every query at once; the placement
/// then sizes the chosen mapping's real `(C, M)`. The gap between the two is
/// added back to the rows (`C ≥ … + δ_C`, `M ≥ … + δ_M`) and the MILP solved
/// again, keeping the mapping whose placed cost is lowest, until a mapping
/// repeats or after [`COST_MODEL_2_CORRECTIONS`] passes.
#[allow(clippy::too_many_arguments)]
pub fn minimize_cost_model(
    raqes: &[Raqe],
    deployments: &[Deployment],
    facts: &WorkloadFacts,
    w_cpu: f64,
    w_mem: f64,
    cost_model: CostModel,
    accuracy: &Accuracy,
    sla_ms: Option<f64>,
) -> Result<CostModelSolution, ResolutionError> {
    use crate::placement::{cost_model_1, cost_model_2, PlanLoad};
    let price = |mapping: &Mapping| {
        let load = PlanLoad::new(raqes, deployments, mapping, facts);
        match cost_model {
            CostModel::One => Some(cost_model_1(&load, w_cpu, w_mem)),
            CostModel::Two => cost_model_2(&load, w_cpu, w_mem, sla_ms),
        }
    };
    let mut corrections = (0.0, 0.0);
    let mut best: Option<CostModelSolution> = None;
    let mut seen: Vec<Mapping> = Vec::new();
    for solve in 1..=1 + COST_MODEL_2_CORRECTIONS {
        let (milp, mapping) = solve_cost_model(
            raqes,
            deployments,
            facts,
            w_cpu,
            w_mem,
            cost_model,
            accuracy,
            sla_ms,
            corrections,
        )?;
        let cost = price(&mapping).ok_or(ResolutionError::Infeasible)?;
        if cost_model == CostModel::One {
            return Ok(CostModelSolution {
                milp,
                mapping,
                cost,
                solves: 1,
            });
        }
        let load = PlanLoad::new(raqes, deployments, &mapping, facts);
        let (surrogate_cpu, surrogate_bytes) = surrogate_peaks(&load, sla_ms);
        corrections = (cost.cpu - surrogate_cpu, cost.bytes - surrogate_bytes);
        let repeated = seen.contains(&mapping);
        seen.push(mapping.clone());
        if best.as_ref().is_none_or(|b| cost.value < b.cost.value) {
            best = Some(CostModelSolution {
                milp,
                mapping,
                cost,
                solves: solve,
            });
        }
        if repeated {
            break;
        }
    }
    let mut best = best.expect("at least one solve");
    best.solves = seen.len();
    Ok(best)
}

/// Cost model 2's surrogate `(C, M)` of a placed plan, the MILP's rows
/// without corrections: `C = max(mean load, ingest + batch work / L)`, and `M`
/// with every query's memory at once.
fn surrogate_peaks(load: &crate::placement::PlanLoad, sla_ms: Option<f64>) -> (f64, f64) {
    let mean = load.auc_cpu();
    let cpu = match sla_ms {
        Some(sla) => {
            let batch: f64 = load.queries.iter().map(|q| q.work_secs).sum::<f64>()
                + load
                    .deployments
                    .iter()
                    .map(|d| d.compaction_secs)
                    .sum::<f64>();
            mean.max(load.ingest_cpu() + batch / (sla / 1000.0))
        }
        None => mean,
    };
    let bytes = load.static_bytes() + load.queries.iter().map(|q| q.memory_bytes).sum::<f64>();
    (cpu, bytes)
}

/// One MILP solve for [`minimize_cost_model`], with cost model 2's
/// corrections `(δ_C vCPU, δ_M bytes)`.
#[allow(clippy::too_many_arguments)]
fn solve_cost_model(
    raqes: &[Raqe],
    deployments: &[Deployment],
    facts: &WorkloadFacts,
    w_cpu: f64,
    w_mem: f64,
    cost_model: CostModel,
    accuracy: &Accuracy,
    sla_ms: Option<f64>,
    (cpu_correction, bytes_correction): (f64, f64),
) -> Result<(MilpSolution, Mapping), ResolutionError> {
    let secs = |ms: u64| ms as f64 / 1000.0;
    // Per candidate: ingest vCPUs, compaction per window, its rate, ingest bytes.
    let ingest_cpu: Vec<f64> = deployments
        .iter()
        .map(|d| analytical_cost_model::ingest(d, facts).cpu_secs_per_sec)
        .collect();
    let compaction: Vec<f64> = deployments
        .iter()
        .map(|d| analytical_cost_model::compaction_secs(d, facts))
        .collect();
    let ingest_bytes: Vec<f64> = deployments
        .iter()
        .map(|d| {
            analytical_cost_model::ingest(d, facts).memory_bytes
                * analytical_cost_model::ingest_workers(d, facts)
        })
        .collect();
    // Eligible pairs, less those whose chain exceeds the SLA.
    let eligible: Vec<Vec<usize>> = raqes
        .iter()
        .map(|raqe| {
            eligible_deployments_for(raqe, deployments, facts, accuracy)
                .into_iter()
                .filter(|&d| {
                    sla_ms.is_none_or(|sla| {
                        1000.0 * compaction[d]
                            + analytical_cost_model::query_latency_ms(raqe, &deployments[d], facts)
                            <= sla
                    })
                })
                .collect()
        })
        .collect();
    if eligible.iter().any(Vec::is_empty) {
        return Err(ResolutionError::Infeasible);
    }
    // Per pair: query work (CPU-sec), its memory and the storage it needs.
    let work = |i: usize, d: usize| {
        analytical_cost_model::query_latency_ms(&raqes[i], &deployments[d], facts) / 1000.0
    };
    let query_bytes = |i: usize, d: usize| {
        analytical_cost_model::merge(&raqes[i], &deployments[d], facts).memory_bytes
            + analytical_cost_model::query(&raqes[i], &deployments[d], facts).memory_bytes
    };
    let storage = |i: usize, d: usize| {
        analytical_cost_model::storage_bytes(&raqes[i], &deployments[d], facts)
    };

    // Scale: the cost of each RAQE alone on its cheapest pair (cost model 1),
    // so HiGHS sees magnitudes near 1.
    let cpu_of = |i: usize, d: usize| {
        ingest_cpu[d]
            + compaction[d] / secs(deployments[d].slide_ms)
            + work(i, d) / secs(raqes[i].interval_ms)
    };
    let gib = |bytes: f64| bytes / BYTES_PER_GIB;
    let reference: f64 = (0..raqes.len())
        .map(|i| {
            eligible[i]
                .iter()
                .map(|&d| {
                    w_cpu * cpu_of(i, d)
                        + w_mem * gib(ingest_bytes[d] + storage(i, d) + query_bytes(i, d))
                })
                .fold(f64::INFINITY, f64::min)
        })
        .sum();
    let reference = if reference.is_normal() {
        reference
    } else {
        1.0
    };
    let cpu_scale = (0..raqes.len())
        .map(|i| {
            eligible[i]
                .iter()
                .map(|&d| cpu_of(i, d))
                .fold(f64::INFINITY, f64::min)
        })
        .sum::<f64>()
        .max(f64::MIN_POSITIVE);
    let bytes_scale = (0..raqes.len())
        .map(|i| {
            eligible[i]
                .iter()
                .map(|&d| ingest_bytes[d] + storage(i, d) + query_bytes(i, d))
                .fold(f64::INFINITY, f64::min)
        })
        .sum::<f64>()
        .max(1.0);

    let mut variables = ProblemVariables::new();
    let active: Vec<Variable> = deployments
        .iter()
        .map(|_| variables.add(variable().binary()))
        .collect();
    let assignments: Vec<Vec<(usize, Variable)>> = eligible
        .iter()
        .map(|choices| {
            choices
                .iter()
                .map(|&d| (d, variables.add(variable().binary())))
                .collect()
        })
        .collect();
    // Storage per deployment, in units of `bytes_scale`.
    let stored: Vec<Variable> = deployments
        .iter()
        .map(|_| variables.add(variable().min(0)))
        .collect();
    // Cost model 2: C and M, in units of `cpu_scale` and `bytes_scale`.
    let peaks = (cost_model == CostModel::Two).then(|| {
        (
            variables.add(variable().min(0)),
            variables.add(variable().min(0)),
        )
    });

    // CPU and memory terms, in scaled units.
    let mut mean_cpu = Expression::from(0.0);
    let mut batch_work = Expression::from(0.0);
    let mut ingest = Expression::from(0.0);
    let mut held_bytes: Expression = stored.iter().sum();
    let mut query_bytes_all = Expression::from(0.0);
    let mut query_bytes_auc = Expression::from(0.0);
    for (d, &u) in active.iter().enumerate() {
        let deployment = &deployments[d];
        mean_cpu.add_mul(
            (ingest_cpu[d] + compaction[d] / secs(deployment.slide_ms)) / cpu_scale,
            u,
        );
        ingest.add_mul(ingest_cpu[d] / cpu_scale, u);
        batch_work.add_mul(compaction[d] / cpu_scale, u);
        held_bytes.add_mul(ingest_bytes[d] / bytes_scale, u);
    }
    for (i, choices) in assignments.iter().enumerate() {
        let interval = secs(raqes[i].interval_ms);
        for &(d, z) in choices {
            mean_cpu.add_mul(work(i, d) / interval / cpu_scale, z);
            batch_work.add_mul(work(i, d) / cpu_scale, z);
            query_bytes_all.add_mul(query_bytes(i, d) / bytes_scale, z);
            query_bytes_auc.add_mul(query_bytes(i, d) * work(i, d) / interval / bytes_scale, z);
        }
    }
    let cpu_weight = w_cpu * cpu_scale / reference;
    let bytes_weight = w_mem * gib(bytes_scale) / reference;
    let goal = match peaks {
        None => {
            cpu_weight * mean_cpu.clone() + bytes_weight * (held_bytes.clone() + query_bytes_auc)
        }
        Some((c, m)) => cpu_weight * c + bytes_weight * m,
    };

    let mut model = variables.minimise(goal).using(default_solver);
    for (i, choices) in assignments.iter().enumerate() {
        let sum: Expression = choices.iter().map(|&(_, z)| z).sum();
        model.add_constraint(sum.eq(1));
        for &(d, z) in choices {
            model.add_constraint((z - active[d]).leq(0));
            let needed = storage(i, d) / bytes_scale;
            if needed > 0.0 {
                model.add_constraint((needed * z - stored[d]).leq(0));
            }
        }
    }
    for (d, &u) in active.iter().enumerate() {
        let used: Expression = assignments
            .iter()
            .flat_map(|choices| choices.iter())
            .filter(|(index, _)| *index == d)
            .map(|&(_, z)| z)
            .sum();
        model.add_constraint((u - used).leq(0));
    }
    if let Some((c, m)) = peaks {
        model.add_constraint((mean_cpu - c).leq(0));
        model.add_constraint(
            (held_bytes + query_bytes_all + bytes_correction / bytes_scale - m).leq(0),
        );
        if let Some(sla) = sla_ms {
            // C ≥ ingest + (batch work) / L + δ_C.
            let l = sla / 1000.0;
            model.add_constraint(
                (ingest + batch_work * (1.0 / l) + cpu_correction / cpu_scale - c).leq(0),
            );
        }
    }

    let solved = model.solve()?;
    let mapping: Mapping = assignments
        .iter()
        .map(|choices| {
            choices
                .iter()
                .find_map(|&(d, z)| (solved.value(z) > 0.5).then_some(d))
                .expect("the MILP assigns every RAQE one deployment")
        })
        .collect();
    let plan_cost = score(raqes, deployments, &mapping, facts);
    let (planned, planned_raqes) = plan(raqes, deployments, &mapping);
    Ok((
        MilpSolution {
            deployments: planned,
            raqes: planned_raqes,
            plan_cost,
        },
        mapping,
    ))
}

/// Keeps the deployments `mapping` uses, renumbered, with their instance
/// counts. Every pair in `mapping` is eligible, so the counts exist.
fn plan(
    raqes: &[Raqe],
    candidates: &[Deployment],
    mapping: &Mapping,
) -> (Vec<PlannedDeployment>, Vec<PlannedRaqe>) {
    let mut planned_index: Vec<Option<usize>> = vec![None; candidates.len()];
    let mut planned_deployments: Vec<PlannedDeployment> = Vec::new();
    let mut planned_raqes = Vec::with_capacity(raqes.len());
    // `retained_instance_count` holds the closed windows until the open ones
    // are added below, once per deployment.
    for (raqe, &candidate_index) in raqes.iter().zip(mapping) {
        let deployment = &candidates[candidate_index];
        let index = *planned_index[candidate_index].get_or_insert_with(|| {
            planned_deployments.push(PlannedDeployment {
                deployment: deployment.clone(),
                retained_instance_count: 0,
            });
            planned_deployments.len() - 1
        });
        let closed = deployment
            .closed_instance_count(raqe.lookback_ms)
            .expect("eligible deployments have whole window counts");
        let planned = &mut planned_deployments[index];
        planned.retained_instance_count = planned.retained_instance_count.max(closed);
        planned_raqes.push(PlannedRaqe {
            deployment: index,
            merged_instance_count: deployment
                .query_instance_count(raqe.lookback_ms)
                .expect("eligible deployments divide the lookback"),
        });
    }
    for planned in &mut planned_deployments {
        planned.retained_instance_count += planned
            .deployment
            .active_instance_count()
            .expect("eligible deployments have whole window counts");
    }
    (planned_deployments, planned_raqes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::candidates::build_all_candidates;
    use crate::enumerate::brute_force;
    use crate::test_support::{self, facts};
    use crate::{table_accuracy, AtomicCostEntry, Capability};

    /// Window and slide 60 s, merge 1 CPU-sec.
    fn deployment(insert_cpu_secs: f64, memory: f64, query_cpu_secs: f64) -> Deployment {
        test_support::deployment(memory, insert_cpu_secs, 1.0, query_cpu_secs, 60_000, 60_000)
    }

    fn raqe(id: &str, lookback_ms: u64) -> Raqe {
        Raqe {
            id: id.into(),
            ..test_support::raqe(lookback_ms, 60_000)
        }
    }

    /// The candidate index serving each RAQE.
    fn chosen(milp: &MilpSolution, candidates: &[Deployment]) -> Mapping {
        milp.raqes
            .iter()
            .map(|planned| {
                let deployment = &milp.deployments[planned.deployment].deployment;
                candidates
                    .iter()
                    .position(|candidate| candidate == deployment)
                    .expect("planned deployments come from the candidates")
            })
            .collect()
    }

    fn weights(w_cpu: f64, w_mem: f64) -> Objective {
        Objective::AUCCost { w_cpu, w_mem }
    }

    /// The MILP's plan scores as well as the best of every mapping.
    fn assert_matches_brute_force(
        raqes: &[Raqe],
        deployments: &[Deployment],
        objective: Objective,
    ) {
        let facts = facts(1, 1);
        let best = brute_force(raqes, deployments, &facts, &table_accuracy)
            .iter()
            .map(|mapping| objective.value(&score(raqes, deployments, mapping, &facts)))
            .min_by(f64::total_cmp)
            .expect("test workload is servable");
        let milp = minimize(raqes, deployments, &facts, objective, &table_accuracy)
            .expect("feasible MILP");
        let got = objective.value(&milp.plan_cost);
        assert!(
            (got - best).abs() <= 1e-9 * best,
            "{objective:?}: {got} vs {best}"
        );
    }

    /// Capabilities the plan serves from one deployment, solving `a` and `b`
    /// queries at the same cadence from one `sketch` cost row.
    fn shared_deployments(a: Capability, b: Capability, sketch: &str) -> usize {
        let raqes = [a, b].map(|capability| Raqe {
            capability,
            ..raqe("r", 60_000)
        });
        let costs = [AtomicCostEntry {
            sketch: sketch.into(),
            accuracy_metric: crate::test_support::metric_of(sketch),
            ..deployment(1.0, 1.0, 1.0).config
        }];
        let candidates = build_all_candidates(&raqes, &costs, &facts(1, 1), false, &table_accuracy);
        let milp = minimize(
            &raqes,
            &candidates,
            &facts(1, 1),
            Objective::default(),
            &table_accuracy,
        )
        .expect("feasible MILP");
        raqes.len() - milp.deployments.len()
    }

    #[test]
    fn plan_never_serves_paired_capabilities_from_one_deployment() {
        let pairs = [
            (Capability::Sum, Capability::Count, "exact-sum"),
            (
                Capability::TopKByValue,
                Capability::TopKByCount,
                "cms-heap-topk-fastpath-vector2d",
            ),
        ];
        for (a, b, sketch) in pairs {
            // Sharing halves ingest, so the MILP shares whenever it may.
            assert_eq!(shared_deployments(a, a, sketch), 1, "{a:?} twice");
            assert_eq!(shared_deployments(a, b, sketch), 0, "{a:?} and {b:?}");
        }
    }

    #[test]
    fn minimizes_cpu_and_respects_a_latency_sla() {
        let raqes = vec![raqe("r", 60_000)];
        // CPU 1 + 3/60 with latency 3, or 2 + 1/60 with latency 1.
        let deployments = vec![deployment(1.0, 20.0, 3.0), deployment(2.0, 10.0, 1.0)];
        let solve = |raqes: &[Raqe]| {
            let milp = minimize(
                raqes,
                &deployments,
                &facts(1, 1),
                Objective::default(),
                &table_accuracy,
            )
            .expect("feasible MILP");
            chosen(&milp, &deployments)
        };

        assert_eq!(solve(&raqes), vec![0]);
        let bounded = Raqe {
            latency_sla_ms: Some(2_000.0),
            ..raqes[0].clone()
        };
        assert_eq!(solve(&[bounded]), vec![1]);
    }

    /// The cheapest mapping under `price` (None: unservable), by brute force.
    fn brute_force_cost(
        raqes: &[Raqe],
        deployments: &[Deployment],
        facts: &WorkloadFacts,
        price: impl Fn(&Mapping) -> Option<f64>,
    ) -> f64 {
        brute_force(raqes, deployments, facts, &table_accuracy)
            .iter()
            .filter_map(price)
            .min_by(f64::total_cmp)
            .expect("servable")
    }

    /// Two RAQEs, 100 groups, and candidates that trade ingest for queries.
    fn cost_model_workload() -> (Vec<Raqe>, Vec<Deployment>, WorkloadFacts) {
        let raqes = vec![raqe("frequent", 60_000), raqe("long", 600_000)];
        // (memory, insert, merge, query, window, slide): cheap ingest with
        // slow queries; costlier ingest with fast ones; a 10-minute window
        // that answers `long` without merging but keeps 10 windows open.
        let deployments = vec![
            test_support::deployment(0.5 * BYTES_PER_GIB, 1e-3, 1e-4, 4e-3, 60_000, 60_000),
            test_support::deployment(0.1 * BYTES_PER_GIB, 3e-3, 1e-5, 1e-5, 60_000, 60_000),
            test_support::deployment(2.0 * BYTES_PER_GIB, 2e-4, 1e-5, 1e-5, 600_000, 60_000),
        ];
        (raqes, deployments, facts(100, 100))
    }

    #[test]
    fn cost_model_1_matches_brute_force_with_and_without_an_sla() {
        use crate::placement::{cost_model_1, PlanLoad};
        let (raqes, deployments, facts) = cost_model_workload();
        for (w_cpu, w_mem) in [(1.0, 0.0), (1.0, 4.0)] {
            for sla_ms in [None, Some(1e5), Some(300.0)] {
                let price = |mapping: &Mapping| {
                    let cost = cost_model_1(
                        &PlanLoad::new(&raqes, &deployments, mapping, &facts),
                        w_cpu,
                        w_mem,
                    );
                    sla_ms
                        .is_none_or(|l| cost.latency_ms <= l)
                        .then_some(cost.value)
                };
                let best = brute_force_cost(&raqes, &deployments, &facts, price);
                let got = minimize_cost_model(
                    &raqes,
                    &deployments,
                    &facts,
                    w_cpu,
                    w_mem,
                    CostModel::One,
                    &table_accuracy,
                    sla_ms,
                )
                .expect("feasible");
                assert!(
                    (got.cost.value - best).abs() <= 1e-9 * best,
                    "({w_cpu}, {w_mem}), {sla_ms:?}: {} vs {best}",
                    got.cost.value
                );
                assert!(sla_ms.is_none_or(|l| got.cost.latency_ms <= l));
            }
        }
    }

    #[test]
    fn cost_model_2_meets_the_sla_and_corrections_find_the_best_mapping() {
        use crate::placement::{cost_model_2, PlanLoad};
        let (raqes, deployments, facts) = cost_model_workload();
        for (w_cpu, w_mem) in [(1.0, 0.0), (1.0, 4.0)] {
            for sla_ms in [None, Some(1e5), Some(2_000.0), Some(300.0)] {
                let price = |mapping: &Mapping| {
                    cost_model_2(
                        &PlanLoad::new(&raqes, &deployments, mapping, &facts),
                        w_cpu,
                        w_mem,
                        sla_ms,
                    )
                    .map(|cost| cost.value)
                };
                let best = brute_force_cost(&raqes, &deployments, &facts, price);
                let got = minimize_cost_model(
                    &raqes,
                    &deployments,
                    &facts,
                    w_cpu,
                    w_mem,
                    CostModel::Two,
                    &table_accuracy,
                    sla_ms,
                )
                .expect("feasible");
                assert!(sla_ms.is_none_or(|l| got.cost.latency_ms <= l));
                assert!(
                    (got.cost.value - best).abs() <= 1e-3 * best,
                    "({w_cpu}, {w_mem}), {sla_ms:?}: {} vs {best} after {} solves",
                    got.cost.value,
                    got.solves
                );
            }
        }
    }

    #[test]
    fn matches_brute_force_with_shared_deployment_costs() {
        let raqes = vec![raqe("frequent", 60_000), raqe("long", 600_000)];
        let deployments = vec![
            deployment(1.0, 0.5 * BYTES_PER_GIB, 400.0),
            deployment(3.0, 0.1 * BYTES_PER_GIB, 1.0),
            deployment(0.2, 2.0 * BYTES_PER_GIB, 1.0),
        ];
        for objective in [weights(1.0, 0.0), weights(0.0, 1.0), weights(1.0, 4.0)] {
            assert_matches_brute_force(&raqes, &deployments, objective);
        }
    }

    #[test]
    fn memory_weight_trades_cpu_for_memory() {
        let raqes = vec![raqe("r", 60_000)];
        // CPU-heavy and small, or CPU-light and twice the size.
        let deployments = vec![
            deployment(1.0, 1.0 * BYTES_PER_GIB, 0.0),
            deployment(0.05, 2.0 * BYTES_PER_GIB, 0.0),
        ];
        let solve = |objective| {
            let milp = minimize(
                &raqes,
                &deployments,
                &facts(1, 1),
                objective,
                &table_accuracy,
            )
            .expect("feasible MILP");
            chosen(&milp, &deployments)
        };

        assert_eq!(solve(weights(1.0, 0.0)), vec![1]);
        assert_eq!(solve(weights(0.0, 1.0)), vec![0]);
    }

    #[test]
    fn tiny_magnitudes_match_brute_force_and_respect_latency_slas() {
        // Real plans cost ~1e-6 CPU-sec/sec with µs latencies: below HiGHS's
        // absolute gap and feasibility tolerances unless the model is scaled.
        let raqes = vec![raqe("frequent", 60_000), raqe("long", 600_000)];
        let tiny = 1e-9;
        let deployments = vec![
            deployment(1.0 * tiny, 0.5 * tiny * BYTES_PER_GIB, 400.0 * tiny),
            deployment(3.0 * tiny, 0.1 * tiny * BYTES_PER_GIB, 1.0 * tiny),
            deployment(0.2 * tiny, 2.0 * tiny * BYTES_PER_GIB, 1.0 * tiny),
        ]
        .into_iter()
        .map(|mut candidate| {
            candidate.config.merge_cpu_secs = tiny;
            candidate
        })
        .collect::<Vec<_>>();
        for objective in [weights(1.0, 0.0), weights(1.0, 4.0)] {
            assert_matches_brute_force(&raqes, &deployments, objective);
        }

        // The cheap deployment's latency is 1.5e-7 s, over a 1e-7 s SLA by
        // less than HiGHS's absolute feasibility tolerance.
        let raqes = vec![Raqe {
            latency_sla_ms: Some(1e-4),
            ..raqe("r", 60_000)
        }];
        let deployments = vec![
            deployment(tiny, tiny, 1.5e-7),
            deployment(10.0 * tiny, tiny, 0.5e-7),
        ];
        let milp = minimize(
            &raqes,
            &deployments,
            &facts(1, 1),
            Objective::default(),
            &table_accuracy,
        )
        .expect("feasible MILP");
        assert_eq!(chosen(&milp, &deployments), vec![1]);
        assert!(milp.plan_cost.query_latency_ms[0] <= 1e-4);
    }

    #[test]
    fn plan_lists_active_deployments_with_their_instance_counts() {
        // Window 60 s, slide 20 s; the unused candidate 0 is CPU-heavy.
        let shared = test_support::deployment(1.0, 1.0, 1.0, 1.0, 60_000, 20_000);
        let unused = test_support::deployment(1.0, 1e6, 1.0, 1.0, 60_000, 20_000);
        let raqes = vec![raqe("short", 60_000), raqe("long", 600_000)];
        let milp = minimize(
            &raqes,
            &[unused, shared.clone()],
            &facts(1, 1),
            Objective::default(),
            &table_accuracy,
        )
        .expect("feasible MILP");

        assert_eq!(milp.deployments.len(), 1);
        assert_eq!(milp.deployments[0].deployment, shared);
        // 60/20 open + (600 − 60)/20 + 1 closed, for the longer lookback.
        assert_eq!(milp.deployments[0].retained_instance_count, 3 + 28);
        let merged: Vec<_> = milp
            .raqes
            .iter()
            .map(|planned| (planned.deployment, planned.merged_instance_count))
            .collect();
        assert_eq!(merged, vec![(0, 1), (0, 10)]);
    }
}
