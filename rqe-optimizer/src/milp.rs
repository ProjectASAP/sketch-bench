//! MILP solver for one scalarized RQE deployment optimization (§4).
//!
//! The model is documented in `docs/rqe_sketch_deployment_v1.md`. It avoids
//! enumerating the Cartesian product of eligible deployment choices.

use crate::candidates::eligible_deployments_for;
use crate::objectives::{score, MachineFamily, Objectives, BYTES_PER_GIB};
use crate::{Deployment, LabelSetTable, Mapping, Rqe};
use good_lp::{
    default_solver, variable, Expression, ProblemVariables, ResolutionError, Solution, SolverModel,
    Variable,
};

/// Optional hard bounds for a minimum-TCO solve. An empty latency vector means
/// no RQE has a latency bound; otherwise it is index-aligned with `rqes`.
#[derive(Debug, Clone, Default)]
pub struct MilpBounds {
    pub max_peak_query_memory_bytes: Option<f64>,
    pub max_query_latency_secs: Vec<Option<f64>>,
}

#[derive(Debug, Clone)]
pub struct MilpSolution {
    pub mapping: Mapping,
    pub objectives: Objectives,
}

/// Minimize total steady-state CPU subject to optional memory and latency
/// bounds. Missing eligible deployments are a caller input error; use
/// `enumerate::unservable` to report them before calling this function.
pub fn minimize_tco(
    rqes: &[Rqe],
    deployments: &[Deployment],
    label_sets: &LabelSetTable,
    bounds: &MilpBounds,
) -> Result<MilpSolution, ResolutionError> {
    solve(rqes, deployments, label_sets, bounds, None)
}

/// Minimize the hourly price of running the plan on `family`, in fractional
/// instances: `n ≥ CPU / vCPU` and `n ≥ retained memory / GiB`. Same inputs
/// and bounds as [`minimize_tco`].
pub fn minimize_cost(
    rqes: &[Rqe],
    deployments: &[Deployment],
    label_sets: &LabelSetTable,
    bounds: &MilpBounds,
    family: &MachineFamily,
) -> Result<MilpSolution, ResolutionError> {
    solve(rqes, deployments, label_sets, bounds, Some(family))
}

fn solve(
    rqes: &[Rqe],
    deployments: &[Deployment],
    label_sets: &LabelSetTable,
    bounds: &MilpBounds,
    family: Option<&MachineFamily>,
) -> Result<MilpSolution, ResolutionError> {
    assert!(
        bounds.max_query_latency_secs.is_empty()
            || bounds.max_query_latency_secs.len() == rqes.len(),
        "latency bounds must be empty or index-aligned with rqes"
    );

    let eligible: Vec<Vec<usize>> = rqes
        .iter()
        .map(|rqe| eligible_deployments_for(rqe, deployments))
        .collect();
    assert!(
        eligible.iter().all(|choices| !choices.is_empty()),
        "cannot solve a workload with an unservable RQE"
    );

    let mut variables = ProblemVariables::new();
    let active: Vec<Variable> = deployments
        .iter()
        .map(|_| variables.add(variable().binary()))
        .collect();
    let peak_memory = variables.add(variable().min(0));
    let assignments: Vec<Vec<(usize, Variable)>> = eligible
        .iter()
        .map(|choices| {
            choices
                .iter()
                .map(|&deployment_index| (deployment_index, variables.add(variable().binary())))
                .collect()
        })
        .collect();

    let mut objective = Expression::with_capacity(deployments.len() + rqes.len());
    for (deployment, &active_variable) in deployments.iter().zip(&active) {
        let labels = &label_sets[&deployment.labels];
        let fanout = deployment
            .active_instance_count()
            .expect("candidate window and slide align") as f64;
        objective.add_mul(
            labels.arrival_rate_per_sec * fanout * deployment.config.insert_cpu_secs,
            active_variable,
        );
    }
    for (rqe_index, choices) in assignments.iter().enumerate() {
        for &(deployment_index, assignment) in choices {
            objective.add_mul(
                query_and_merge_cpu_per_sec(
                    &rqes[rqe_index],
                    &deployments[deployment_index],
                    label_sets,
                ),
                assignment,
            );
        }
    }

    // Retained GiB per deployment, and fractional instances, priced only when
    // a machine family is given.
    let retained_gib: Vec<Variable> = match family {
        Some(_) => deployments
            .iter()
            .map(|_| variables.add(variable().min(0)))
            .collect(),
        None => Vec::new(),
    };
    let instances = variables.add(variable().min(0));
    let goal = match family {
        Some(f) => f.usd_per_hour * instances,
        None => objective.clone(),
    };

    let mut model = variables.minimise(goal).using(default_solver);
    if let Some(f) = family {
        model.add_constraint((objective - f.vcpu * instances).leq(0));
        let total_gib: Expression = retained_gib.iter().sum();
        model.add_constraint((total_gib - f.memory_gib * instances).leq(0));
    }
    if let Some(memory_limit) = bounds.max_peak_query_memory_bytes {
        model.add_constraint(Expression::from(peak_memory).leq(memory_limit));
    }

    for (rqe_index, choices) in assignments.iter().enumerate() {
        let assignment_sum: Expression = choices.iter().map(|(_, variable)| *variable).sum();
        model.add_constraint(assignment_sum.eq(1));

        for &(deployment_index, assignment) in choices {
            let deployment = &deployments[deployment_index];
            model.add_constraint((assignment - active[deployment_index]).leq(0));
            let memory = label_sets[&deployment.labels].cardinality as f64
                * deployment.config.mem_bytes_per_instance;
            model.add_constraint((memory * assignment - peak_memory).leq(0));
            if family.is_some() {
                // Linearized max over the RQEs a deployment serves.
                let retained = memory
                    * deployment
                        .retained_instance_count(rqes[rqe_index].lookback_secs)
                        .expect("assignment only contains eligible deployments")
                        as f64
                    / BYTES_PER_GIB;
                model.add_constraint(
                    (retained * assignment - retained_gib[deployment_index]).leq(0),
                );
            }
        }

        if let Some(Some(latency_limit)) = bounds.max_query_latency_secs.get(rqe_index) {
            let latency: Expression = choices
                .iter()
                .map(|&(deployment_index, assignment)| {
                    query_latency_secs(&rqes[rqe_index], &deployments[deployment_index], label_sets)
                        * assignment
                })
                .sum();
            model.add_constraint(latency.leq(*latency_limit));
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
                .expect("MILP assigns exactly one deployment to every RQE")
        })
        .collect();
    let objectives = score(rqes, deployments, &mapping, label_sets);
    Ok(MilpSolution {
        mapping,
        objectives,
    })
}

fn query_and_merge_cpu_per_sec(
    rqe: &Rqe,
    deployment: &Deployment,
    label_sets: &LabelSetTable,
) -> f64 {
    query_latency_secs(rqe, deployment, label_sets) / rqe.interval_secs as f64
}

fn query_latency_secs(rqe: &Rqe, deployment: &Deployment, label_sets: &LabelSetTable) -> f64 {
    let instances = deployment
        .query_instance_count(rqe.lookback_secs)
        .expect("assignment only contains eligible deployments");
    let cardinality = label_sets[&deployment.labels].cardinality as f64;
    cardinality
        * (deployment.config.query_cpu_secs
            + instances.saturating_sub(1) as f64 * deployment.config.merge_cpu_secs)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::enumerate::brute_force;
    use crate::{AccuracyDirection, AtomicCostEntry, Capability, LabelSet, LabelSetInfo};
    use std::collections::BTreeMap;

    fn deployment(insert_cpu_secs: f64, memory: f64, query_cpu_secs: f64) -> Deployment {
        Deployment {
            capability: Capability::Freq,
            labels: LabelSet::new(),
            config: AtomicCostEntry {
                sketch: "cms-fastpath-vector2d".into(),
                sketch_config: serde_json::json!(null),
                mem_bytes_per_instance: memory,
                insert_cpu_secs,
                merge_cpu_secs: 1.0,
                query_cpu_secs,
                query_accuracy: BTreeMap::from([("err".into(), 0.0)]),
            },
            window_secs: 60,
            slide_secs: 60,
        }
    }

    fn rqe() -> Rqe {
        Rqe {
            id: "r".into(),
            capability: Capability::Freq,
            lookback_secs: 60,
            interval_secs: 60,
            labels: LabelSet::new(),
            accuracy_metric: "err".into(),
            accuracy_tolerance: 1.0,
            accuracy_direction: AccuracyDirection::LowerIsBetter,
        }
    }

    #[test]
    fn minimizes_cpu_and_respects_a_memory_bound() {
        let rqes = vec![rqe()];
        let deployments = vec![deployment(1.0, 20.0, 1.0), deployment(2.0, 10.0, 1.0)];
        let label_sets = BTreeMap::from([(
            LabelSet::new(),
            LabelSetInfo {
                cardinality: 1,
                arrival_rate_per_sec: 1.0,
            },
        )]);

        let unconstrained = minimize_tco(&rqes, &deployments, &label_sets, &MilpBounds::default())
            .expect("feasible MILP");
        assert_eq!(unconstrained.mapping, vec![0]);

        let constrained = minimize_tco(
            &rqes,
            &deployments,
            &label_sets,
            &MilpBounds {
                max_peak_query_memory_bytes: Some(10.0),
                max_query_latency_secs: Vec::new(),
            },
        )
        .expect("feasible MILP under memory bound");
        assert_eq!(constrained.mapping, vec![1]);
    }

    #[test]
    fn matches_brute_force_minimum_with_shared_deployment_costs() {
        let mut frequent = rqe();
        frequent.id = "frequent".into();
        frequent.interval_secs = 60;
        let mut infrequent = rqe();
        infrequent.id = "infrequent".into();
        let rqes = vec![frequent, infrequent];
        let deployments = vec![deployment(1.0, 10.0, 400.0), deployment(3.0, 10.0, 1.0)];
        let label_sets = BTreeMap::from([(
            LabelSet::new(),
            LabelSetInfo {
                cardinality: 1,
                arrival_rate_per_sec: 1.0,
            },
        )]);

        let brute_force_best = brute_force(&rqes, &deployments)
            .into_iter()
            .min_by(|left, right| {
                score(&rqes, &deployments, left, &label_sets)
                    .tco_cpu_secs_per_sec
                    .total_cmp(&score(&rqes, &deployments, right, &label_sets).tco_cpu_secs_per_sec)
            })
            .expect("test workload is servable");
        let expected = score(&rqes, &deployments, &brute_force_best, &label_sets);

        let milp = minimize_tco(&rqes, &deployments, &label_sets, &MilpBounds::default())
            .expect("feasible MILP");

        assert_eq!(milp.mapping, brute_force_best);
        assert!(
            (milp.objectives.tco_cpu_secs_per_sec - expected.tco_cpu_secs_per_sec).abs() < 1e-12
        );
    }

    fn family(memory_gib: f64) -> MachineFamily {
        MachineFamily {
            family: format!("{memory_gib} GiB"),
            vcpu: 4.0,
            memory_gib,
            usd_per_hour: 1.0,
        }
    }

    #[test]
    fn minimize_cost_matches_brute_force_minimum() {
        let mut frequent = rqe();
        frequent.id = "frequent".into();
        let mut long = rqe();
        long.id = "long".into();
        long.lookback_secs = 600;
        let rqes = vec![frequent, long];
        let deployments = vec![
            deployment(1.0, 0.5 * BYTES_PER_GIB, 400.0),
            deployment(3.0, 0.1 * BYTES_PER_GIB, 1.0),
            deployment(0.2, 2.0 * BYTES_PER_GIB, 1.0),
        ];
        let label_sets = BTreeMap::from([(
            LabelSet::new(),
            LabelSetInfo {
                cardinality: 1,
                arrival_rate_per_sec: 1.0,
            },
        )]);

        for family in [family(8.0), family(32.0)] {
            let best = brute_force(&rqes, &deployments)
                .iter()
                .map(|mapping| {
                    family.usd_per_hour(&score(&rqes, &deployments, mapping, &label_sets))
                })
                .min_by(f64::total_cmp)
                .expect("test workload is servable");
            let milp = minimize_cost(
                &rqes,
                &deployments,
                &label_sets,
                &MilpBounds::default(),
                &family,
            )
            .expect("feasible MILP");
            assert!((family.usd_per_hour(&milp.objectives) - best).abs() < 1e-9);
        }
    }

    #[test]
    fn binding_resource_decides_the_plan_per_family() {
        let rqes = vec![rqe()];
        // Holds 2 instances: CPU-heavy with 2 GiB, or CPU-light with 4 GiB.
        let deployments = vec![
            deployment(1.0, 1.0 * BYTES_PER_GIB, 0.0),
            deployment(0.05, 2.0 * BYTES_PER_GIB, 0.0),
        ];
        let label_sets = BTreeMap::from([(
            LabelSet::new(),
            LabelSetInfo {
                cardinality: 1,
                arrival_rate_per_sec: 1.0,
            },
        )]);
        let solve = |memory_gib| {
            minimize_cost(
                &rqes,
                &deployments,
                &label_sets,
                &MilpBounds::default(),
                &family(memory_gib),
            )
            .expect("feasible MILP")
            .mapping
        };

        assert_eq!(solve(8.0), vec![0]); // memory is scarce: 0.25 vs 0.5 instances
        assert_eq!(solve(32.0), vec![1]); // CPU is scarce: 0.25 vs 0.125 instances
    }
}
