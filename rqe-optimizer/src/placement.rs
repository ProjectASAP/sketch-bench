//! A plan's load over time, its cost under cost models 1 and 2, and the query
//! placement that gives its batch latency (`docs/rqe_sketch_deployment_v1.md`,
//! "MILP with latency SLA constraints: cost models, query placement and batch
//! latency"). Both cost models use the placement: cost model 1 with unlimited
//! capacity, which has the closed form [`cost_model_1`] uses, and cost model 2
//! with the provisioned `C`.
//!
//! Ingest is a steady load on `⌈ρ⌉` workers per deployment, split by sample.
//! Each closed window is compacted (the workers' partials merged into one), and
//! each RAQE firing runs one query job (merge, then estimate) once the newest
//! window is compacted. A job uses at most one core: sketch-bench's operations
//! are single-threaded, their CPU time equal to their wall time.

use std::collections::BTreeMap;

use crate::analytical_cost_model::{self, BYTES_PER_GIB};
use crate::{Deployment, Mapping, Millis, Raqe, WorkloadFacts};

/// One active deployment's steady load.
#[derive(Debug, Clone)]
pub struct DeploymentLoad {
    /// Ingest CPU, vCPUs.
    pub ingest_cpu: f64,
    /// Open windows on every worker, bytes.
    pub ingest_bytes: f64,
    /// Closed windows kept for the longest lookback served, bytes.
    pub storage_bytes: f64,
    /// CPU-seconds to compact one closed window (0 with one worker).
    pub compaction_secs: f64,
    /// Windows close every slide.
    pub slide_ms: Millis,
}

/// One RAQE's query job, run every interval.
#[derive(Debug, Clone)]
pub struct QueryLoad {
    /// Index into [`PlanLoad::deployments`].
    pub deployment: usize,
    pub interval_ms: Millis,
    /// Merge then estimate, CPU-seconds (one core at most).
    pub work_secs: f64,
    /// Merge accumulators and output, held while the job runs.
    pub memory_bytes: f64,
}

/// A plan as load over time.
#[derive(Debug, Clone)]
pub struct PlanLoad {
    pub deployments: Vec<DeploymentLoad>,
    /// Index-aligned with the plan's RAQEs.
    pub queries: Vec<QueryLoad>,
}

impl PlanLoad {
    /// The load of `mapping` (one candidate index per RAQE).
    pub fn new(
        raqes: &[Raqe],
        candidates: &[Deployment],
        mapping: &Mapping,
        facts: &WorkloadFacts,
    ) -> PlanLoad {
        let mut index: BTreeMap<usize, usize> = BTreeMap::new();
        let mut deployments: Vec<DeploymentLoad> = Vec::new();
        let mut queries = Vec::with_capacity(raqes.len());
        for (raqe, &candidate) in raqes.iter().zip(mapping) {
            let deployment = &candidates[candidate];
            let at = *index.entry(candidate).or_insert_with(|| {
                let ingest = analytical_cost_model::ingest(deployment, facts);
                let workers = analytical_cost_model::ingest_workers(deployment, facts);
                deployments.push(DeploymentLoad {
                    ingest_cpu: ingest.cpu_secs_per_sec,
                    ingest_bytes: ingest.memory_bytes * workers,
                    storage_bytes: 0.0,
                    compaction_secs: analytical_cost_model::compaction_secs(deployment, facts),
                    slide_ms: deployment.slide_ms,
                });
                deployments.len() - 1
            });
            let storage = analytical_cost_model::storage_bytes(raqe, deployment, facts);
            let load = &mut deployments[at];
            load.storage_bytes = load.storage_bytes.max(storage);
            queries.push(QueryLoad {
                deployment: at,
                interval_ms: raqe.interval_ms,
                work_secs: analytical_cost_model::query_latency_ms(raqe, deployment, facts)
                    / 1000.0,
                memory_bytes: analytical_cost_model::merge(raqe, deployment, facts).memory_bytes
                    + analytical_cost_model::query(raqe, deployment, facts).memory_bytes,
            });
        }
        PlanLoad {
            deployments,
            queries,
        }
    }

    /// Steady ingest CPU, vCPUs.
    pub fn ingest_cpu(&self) -> f64 {
        self.deployments.iter().map(|d| d.ingest_cpu).sum()
    }

    /// Ingest and storage memory, held at all times, bytes.
    pub fn static_bytes(&self) -> f64 {
        self.deployments
            .iter()
            .map(|d| d.ingest_bytes + d.storage_bytes)
            .sum()
    }

    /// Mean CPU, vCPUs: ingest, compaction and queries, by use.
    pub fn auc_cpu(&self) -> f64 {
        self.ingest_cpu()
            + self
                .deployments
                .iter()
                .map(|d| d.compaction_secs / secs(d.slide_ms))
                .sum::<f64>()
            + self
                .queries
                .iter()
                .map(|q| q.work_secs / secs(q.interval_ms))
                .sum::<f64>()
    }

    /// Mean memory, bytes: ingest and storage always, and each query's memory
    /// for its run on one core (`work_secs`) every interval.
    pub fn auc_bytes(&self) -> f64 {
        self.static_bytes()
            + self
                .queries
                .iter()
                .map(|q| q.memory_bytes * q.work_secs / secs(q.interval_ms))
                .sum::<f64>()
    }

    /// One firing's chain on its own cores, seconds: the newest window's
    /// compaction, then the query.
    pub fn chain_secs(&self, raqe: usize) -> f64 {
        let q = &self.queries[raqe];
        self.deployments[q.deployment].compaction_secs + q.work_secs
    }

    /// The longest chain, ms: the batch latency when CPU is elastic (cost
    /// model 1).
    pub fn longest_chain_ms(&self) -> f64 {
        1000.0
            * (0..self.queries.len())
                .map(|i| self.chain_secs(i))
                .fold(0.0, f64::max)
    }

    /// `lcm` of every interval and slide, ms.
    pub fn hyperperiod_ms(&self) -> Millis {
        self.queries
            .iter()
            .map(|q| q.interval_ms)
            .chain(self.deployments.iter().map(|d| d.slide_ms))
            .fold(1, lcm)
    }

    /// Place every job of two hyperperiods on `cpu` vCPUs shared with ingest
    /// (fluid, priority-ordered processor sharing; module docs). `None` when
    /// ingest alone does not fit or the mean load exceeds `cpu`.
    pub fn place(&self, cpu: f64) -> Option<Placement> {
        place(self, cpu)
    }
}

/// The outcome of [`PlanLoad::place`].
#[derive(Debug, Clone)]
pub struct Placement {
    /// Over batches: from the batch's issue until its last query finishes, ms.
    pub worst_batch_ms: f64,
    /// Index-aligned with the RAQEs: its worst firing, ms.
    pub raqe_latency_ms: Vec<f64>,
    /// Peak CPU (ingest plus running jobs), vCPUs.
    pub peak_cpu: f64,
    /// Peak memory (ingest, storage and running queries' memory), bytes.
    pub peak_bytes: f64,
}

/// Cost model 1 (AUC) and cost model 2 (max) of a plan, with the latency
/// that goes with each.
#[derive(Debug, Clone)]
pub struct ModelCost {
    pub cpu: f64,
    pub bytes: f64,
    /// `w_cpu · cpu + w_mem · GiB`.
    pub value: f64,
    pub latency_ms: f64,
}

/// Cost model 1: CPU and memory billed by use. CPU is elastic: the placement
/// with unlimited capacity, where every job starts when ready on its own core,
/// so the latency is the longest chain and each query runs for its work.
pub fn cost_model_1(load: &PlanLoad, w_cpu: f64, w_mem: f64) -> ModelCost {
    let (cpu, bytes) = (load.auc_cpu(), load.auc_bytes());
    ModelCost {
        cpu,
        bytes,
        value: w_cpu * cpu + w_mem * bytes / BYTES_PER_GIB,
        latency_ms: load.longest_chain_ms(),
    }
}

/// Cost model 2: CPU and memory billed at the peak of a provisioned
/// `(C, M)`. `C` is the smallest at which the placement's worst batch meets
/// `sla_ms` (the mean CPU with no SLA), `M` the placement's peak memory.
/// `None` when a chain alone exceeds the SLA.
pub fn cost_model_2(
    load: &PlanLoad,
    w_cpu: f64,
    w_mem: f64,
    sla_ms: Option<f64>,
) -> Option<ModelCost> {
    let floor = load.auc_cpu();
    let placed = |cpu: f64| load.place(cpu).expect("at or above the mean load");
    let (cpu, placement) = match sla_ms {
        None => (floor, placed(floor)),
        Some(sla) => {
            if load.longest_chain_ms() > sla {
                return None;
            }
            let meets = |cpu: f64| placed(cpu).worst_batch_ms <= sla;
            let mut hi = floor.max(f64::MIN_POSITIVE);
            while !meets(hi) {
                hi *= 2.0;
            }
            let mut lo = floor;
            if meets(lo) {
                hi = lo;
            }
            while hi - lo > 1e-4 * hi {
                let mid = 0.5 * (lo + hi);
                if meets(mid) {
                    hi = mid;
                } else {
                    lo = mid;
                }
            }
            (hi, placed(hi))
        }
    };
    let bytes = placement.peak_bytes;
    Some(ModelCost {
        cpu,
        bytes,
        value: w_cpu * cpu + w_mem * bytes / BYTES_PER_GIB,
        latency_ms: placement.worst_batch_ms,
    })
}

fn secs(ms: Millis) -> f64 {
    ms as f64 / 1000.0
}

fn gcd(a: Millis, b: Millis) -> Millis {
    if b == 0 {
        a
    } else {
        gcd(b, a % b)
    }
}

fn lcm(a: Millis, b: Millis) -> Millis {
    a / gcd(a, b) * b
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Kind {
    /// Compaction runs before queries: it unblocks them.
    Compaction,
    Query,
}

#[derive(Debug, Clone)]
struct Job {
    /// When the job can start: a query's issue, or a compaction's window close.
    issued: f64,
    kind: Kind,
    /// Query: its RAQE; compaction: its deployment.
    owner: usize,
    remaining: f64,
    memory: f64,
    /// Index of the compaction job this query waits for.
    after: Option<usize>,
    done_at: Option<f64>,
}

/// Work below this many CPU-seconds counts as done (float residue).
const EPSILON_SECS: f64 = 1e-15;

fn place(load: &PlanLoad, cpu: f64) -> Option<Placement> {
    let capacity = cpu - load.ingest_cpu();
    if capacity <= 0.0 || load.auc_cpu() > cpu * (1.0 + 1e-12) {
        return None;
    }
    let horizon = 2 * load.hyperperiod_ms();
    let mut jobs: Vec<Job> = Vec::new();
    // Compaction of the window closing at `t`, per deployment.
    let mut compaction_at: BTreeMap<(usize, Millis), usize> = BTreeMap::new();
    for (d, deployment) in load.deployments.iter().enumerate() {
        if deployment.compaction_secs <= 0.0 {
            continue;
        }
        for t in (0..horizon).step_by(deployment.slide_ms as usize) {
            compaction_at.insert((d, t), jobs.len());
            jobs.push(Job {
                issued: secs(t),
                kind: Kind::Compaction,
                owner: d,
                remaining: deployment.compaction_secs,
                memory: 0.0,
                after: None,
                done_at: None,
            });
        }
    }
    for (i, query) in load.queries.iter().enumerate() {
        for t in (0..horizon).step_by(query.interval_ms as usize) {
            jobs.push(Job {
                issued: secs(t),
                kind: Kind::Query,
                owner: i,
                remaining: query.work_secs,
                memory: query.memory_bytes,
                after: compaction_at.get(&(query.deployment, t)).copied(),
                done_at: None,
            });
        }
    }
    let mut issues: Vec<f64> = jobs.iter().map(|j| j.issued).collect();
    issues.sort_by(f64::total_cmp);
    issues.dedup();

    let static_bytes = load.static_bytes();
    let (mut peak_cpu, mut peak_bytes) = (load.ingest_cpu(), static_bytes);
    let mut now = 0.0;
    let mut next_issue = 0;
    loop {
        while next_issue < issues.len() && issues[next_issue] <= now {
            next_issue += 1;
        }
        let mut ready: Vec<usize> = (0..jobs.len())
            .filter(|&j| {
                let job = &jobs[j];
                job.done_at.is_none()
                    && job.issued <= now
                    && job.after.is_none_or(|c| jobs[c].done_at.is_some())
            })
            .collect();
        if ready.is_empty() && next_issue == issues.len() {
            break;
        }
        // Older first, compaction before queries, then longest first.
        ready.sort_by(|&a, &b| {
            let (a, b) = (&jobs[a], &jobs[b]);
            a.issued
                .total_cmp(&b.issued)
                .then(a.kind.cmp(&b.kind))
                .then(b.remaining.total_cmp(&a.remaining))
        });
        let mut left = capacity;
        let mut rates: Vec<(usize, f64)> = Vec::new();
        for &j in &ready {
            if left <= 0.0 {
                break;
            }
            let rate = left.min(1.0);
            rates.push((j, rate));
            left -= rate;
        }
        let running_cpu: f64 = rates.iter().map(|&(_, r)| r).sum();
        let running_bytes: f64 = rates.iter().map(|&(j, _)| jobs[j].memory).sum();
        peak_cpu = peak_cpu.max(load.ingest_cpu() + running_cpu);
        peak_bytes = peak_bytes.max(static_bytes + running_bytes);
        let finish = rates
            .iter()
            .map(|&(j, r)| jobs[j].remaining / r)
            .fold(f64::INFINITY, f64::min);
        let issue = issues.get(next_issue).copied().unwrap_or(f64::INFINITY);
        let step = finish.min(issue - now);
        for &(j, r) in &rates {
            let job = &mut jobs[j];
            job.remaining -= r * step;
            if job.remaining <= EPSILON_SECS.max(1e-12 * r * step) {
                job.remaining = 0.0;
                job.done_at = Some(now + step);
            }
        }
        now += step;
    }

    let mut batches: BTreeMap<u64, f64> = BTreeMap::new();
    let mut raqe_latency_ms = vec![0.0f64; load.queries.len()];
    for job in jobs.iter().filter(|j| j.kind == Kind::Query) {
        let latency_ms = 1000.0 * (job.done_at.expect("every job finishes") - job.issued);
        let batch = batches.entry(job.issued.to_bits()).or_insert(0.0);
        *batch = batch.max(latency_ms);
        raqe_latency_ms[job.owner] = raqe_latency_ms[job.owner].max(latency_ms);
    }
    Some(Placement {
        worst_batch_ms: batches.values().copied().fold(0.0, f64::max),
        raqe_latency_ms,
        peak_cpu,
        peak_bytes,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn load(ingest_cpu: f64, compaction_secs: f64, queries: &[(Millis, f64)]) -> PlanLoad {
        PlanLoad {
            deployments: vec![DeploymentLoad {
                ingest_cpu,
                ingest_bytes: 10.0,
                storage_bytes: 5.0,
                compaction_secs,
                slide_ms: 1_000,
            }],
            queries: queries
                .iter()
                .map(|&(interval_ms, work_secs)| QueryLoad {
                    deployment: 0,
                    interval_ms,
                    work_secs,
                    memory_bytes: 100.0,
                })
                .collect(),
        }
    }

    #[test]
    fn a_job_uses_at_most_one_core_and_slows_on_a_fraction() {
        // Ingest 0.5 vCPU; one 0.1 s query every second.
        let plan = load(0.5, 0.0, &[(1_000, 0.1)]);
        let full = plan.place(1.5).unwrap();
        assert!(
            (full.worst_batch_ms - 100.0).abs() < 1e-6,
            "{}",
            full.worst_batch_ms
        );
        // More capacity doesn't help a one-core job.
        assert!((plan.place(4.0).unwrap().worst_batch_ms - 100.0).abs() < 1e-6);
        // 0.25 vCPU left for it: 0.4 s.
        let quarter = plan.place(0.75).unwrap();
        assert!(
            (quarter.worst_batch_ms - 400.0).abs() < 1e-6,
            "{}",
            quarter.worst_batch_ms
        );
        assert!((quarter.peak_cpu - 0.75).abs() < 1e-9);
        assert_eq!(quarter.peak_bytes, 15.0 + 100.0);
    }

    #[test]
    fn a_batch_shares_the_capacity_and_longest_runs_first() {
        // Two queries of 0.3 s and 0.1 s every second on one free core: the
        // batch ends at 0.4 s, the long one first (0.3 s).
        let plan = load(0.0, 0.0, &[(1_000, 0.1), (1_000, 0.3)]);
        let placed = plan.place(1.0).unwrap();
        assert!((placed.worst_batch_ms - 400.0).abs() < 1e-6);
        assert!((placed.raqe_latency_ms[1] - 300.0).abs() < 1e-6);
        // Two cores: each on its own.
        assert!((plan.place(2.0).unwrap().worst_batch_ms - 300.0).abs() < 1e-6);
    }

    #[test]
    fn a_query_waits_for_the_newest_windows_compaction() {
        // Compaction 0.2 s each second, then a 0.1 s query: 0.3 s.
        let plan = load(0.0, 0.2, &[(1_000, 0.1)]);
        assert!((plan.place(2.0).unwrap().worst_batch_ms - 300.0).abs() < 1e-6);
        assert!((plan.longest_chain_ms() - 300.0).abs() < 1e-9);
    }

    #[test]
    fn the_aligned_batch_is_the_worst_and_backlog_carries_over() {
        // Every second 0.5 s of work, every 2 s another 0.5 s, on one core:
        // the aligned batch (t = 0, 2 s) takes 1 s; the other batches 0.5 s.
        let plan = load(0.0, 0.0, &[(1_000, 0.5), (2_000, 0.5)]);
        let placed = plan.place(1.0).unwrap();
        assert!(
            (placed.worst_batch_ms - 1000.0).abs() < 1e-6,
            "{}",
            placed.worst_batch_ms
        );
        // Below the mean load (0.75 vCPU) there is no placement.
        assert!(plan.place(0.7).is_none());
    }

    #[test]
    fn unlimited_capacity_gives_cost_model_1s_closed_form() {
        // Compaction 0.2 s each second; queries of 0.1 s, 0.5 s and 0.3 s.
        let plan = load(0.5, 0.2, &[(1_000, 0.1), (2_000, 0.5), (1_000, 0.3)]);
        let placed = plan.place(1e9).unwrap();
        assert!((placed.worst_batch_ms - plan.longest_chain_ms()).abs() < 1e-6);
        assert!((cost_model_1(&plan, 1.0, 0.0).latency_ms - placed.worst_batch_ms).abs() < 1e-6);
    }

    #[test]
    fn cost_models_bill_use_and_peak() {
        let plan = load(0.5, 0.0, &[(1_000, 0.1)]);
        let one = cost_model_1(&plan, 1.0, 0.0);
        assert!((one.cpu - 0.6).abs() < 1e-12);
        // Memory by use: 15 bytes always, 100 bytes for 0.1 s each second.
        assert!((one.bytes - 25.0).abs() < 1e-12);
        assert!((one.latency_ms - 100.0).abs() < 1e-9);
        // Cost model 2 with no SLA: the mean CPU, and the queue it gives.
        let two = cost_model_2(&plan, 1.0, 0.0, None).unwrap();
        assert!((two.cpu - 0.6).abs() < 1e-12);
        assert!((two.latency_ms - 1000.0).abs() < 1e-6, "{}", two.latency_ms);
        // A 200 ms SLA: the query needs 0.5 vCPU beside ingest, C = 1.0.
        let tight = cost_model_2(&plan, 1.0, 0.0, Some(200.0)).unwrap();
        assert!((tight.cpu - 1.0).abs() < 1e-3, "{}", tight.cpu);
        assert!(tight.latency_ms <= 200.0);
        // Below the chain, infeasible.
        assert!(cost_model_2(&plan, 1.0, 0.0, Some(50.0)).is_none());
    }
}
