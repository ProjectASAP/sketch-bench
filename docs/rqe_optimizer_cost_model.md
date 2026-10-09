# RQE optimizer: cost models

This document is the canonical design for cost-by-use, batch latency, the analytical phase model, and the measurements required before Hydra can participate.

## Cost by use and batch latency

Status: agreed design (ProjectASAP/ASAPQuery#777), implemented in
`rqe-optimizer/src/usage.rs` (resource use, cost, latency) and
`milp::minimize_usage_cost`. The MILP minimizes the cost of a plan billed by
use, in two versions:

- **Version 1, no latency constraint:** the cheapest plan; its latency is
  reported. Sweeping an optional latency bound traces each method's
  cost–latency Pareto frontier.
- **Version 2, a batch latency SLA:** the cheapest plan whose batch latency,
  from the job placement (§6), is at most the SLA `L`, for a grid of SLAs.
Evaluated first on the synthetic mixed template set; methods: ASAP (this
MILP, with sharing), PerQuery (the same MILP without sharing) and
AutoSketch-Adapted (fixed configs chosen by memory).

#### 1. Definitions

| Symbol | Meaning |
|---|---|
| `D`, `i` | a candidate deployment; a RAQE |
| `x_D`, `y_D` | `D`'s window and slide; a window closes every `y_D` |
| `S_i`, `T_i` | RAQE `i`'s lookback and interval; it fires at `t = k · T_i` |
| `card(G)` | groups of `D`'s grouping |
| `inst_D` | instances per window: `card(G)`, or 1 for a sketch shared by all groups, plus `card(G)` for a key tracker if the family needs one |
| `λ_D` | samples/s arriving for `D`'s metric: `card(series) / scrape interval`, already a rate |
| `c_ins`, `c_mrg`, `c_qry` | measured CPU-seconds per insert, per pairwise merge, per query of one instance |
| `m` | measured bytes per instance; `w_D = Σ_parts inst · m`, one window of all instances |
| `n_{i,D} = S_i / x_D` | windows a query merges |
| batch | all query jobs issued at one instant |

**Measurement assumption (sketch-bench as is).** Every sketch operation is
single-threaded and compute-bound: sketch-bench's CPU time equals its wall
time (e.g. an insert phase of 62.65 ms CPU and 62.65 ms wall). So a job uses
at most one core and, on a full core, takes its CPU time. The cost table's
`*_cpu_secs` are all the model needs.

#### 2. Cost model

`cost = w1 · AUC(CPU) + w2 · AUC(memory)`: the mean vCPUs and GiB over time,
priced per vCPU and per GiB (CPU only: `(1, 0)`; Fargate's prices). CPU is
elastic: a job gets a core whenever it is ready.

This is billing by use, as on fine-grained autoscaling platforms (Cloud Run
with request-based billing, Dataflow streaming, Flink on Kubernetes with an
autoscaler), idealized: capacity follows the load at once. Billing for
provisioned capacity (VMs, or containers billed by allocation such as
Fargate) is not modeled. Since a job holds its memory only while it runs, its
memory × time is the same whenever it runs, and queueing a job would save
nothing.

#### 3. CPU: parts and how each is computed

Every "mean vCPUs" below is a long-run average rate, CPU-seconds per second,
over the same period: a whole number of hyperperiods `H = lcm(T_i, y_D)`,
over which every job pattern repeats exactly. A part with `w` CPU-seconds per
event every `P` seconds runs `H / P` times per hyperperiod, so its mean is
`(H / P) · w / H = w / P`; ingest is already a rate. So the three columns add.

| Part | When | CPU-seconds | Mean vCPUs |
|---|---|---|---|
| Ingest (the precompute) | continuously, as samples arrive | — | `ρ_D = λ_D · (x_D / y_D) · c_ins` |
| Compaction | at each window close | `c_D = (k_D − 1) · Σ_parts inst · c_mrg` | `c_D / y_D` |
| Query job of RAQE `i` | at each firing | `ℓ_{i,D} = card(G) · c_qry + Σ_parts inst · (n_{i,D} − 1) · c_mrg` | `ℓ_{i,D} / T_i` |

- Ingest runs on `k_D = ⌈ρ_D⌉` parallel workers (at least one; each uses at
  most one core), **split by sample**: each worker reads a share of the input
  (e.g. some Kafka partitions or scrape targets) with no shuffle by group, so
  it keeps its own open window instance of every group it sees.
- When a window closes (the watermark passes its end), a **compaction** job
  merges the `k_D` partial instances of each group into one stored instance.
  With `k_D = 1` there is none.
- Each firing of RAQE `i` issues one **query job**: merge the `n_{i,D}`
  stored instances of each group for the query window, then estimate. Merging
  is part of the query. It reads the newest window, which closes at the firing
  time, so it starts after that window's compaction.

`AUC(CPU) = Σ_D ρ_D + Σ_D c_D / y_D + Σ_i ℓ_{i,D} / T_i`: ingest is already
a rate; only compaction (per window close, every `y_D`) and queries (per
firing, every `T_i`) are divided by their period. It is fixed by the plan,
whatever the schedule.

In words:

- `ρ_D`: every second, `λ_D` samples arrive (the scrape interval is already
  inside `λ_D`), and each is inserted into the `x_D / y_D` windows still
  open, at `c_ins` CPU-seconds each: CPU-seconds per second.
- `c_D`: closing a window merges, for every instance, the `k_D` workers'
  partial copies into one, which takes `k_D − 1` merges. This happens once
  per slide.
- `ℓ_{i,D}`: a query merges each instance's `n_{i,D}` stored windows
  (`n − 1` merges per instance), then answers once per group. This happens
  once per interval. (A sketch shared by all groups merges one instance but
  still answers every group.)
- `AUC(CPU)` adds the three rates: work per second, in vCPUs.

#### 4. Memory: parts and how each is computed

Each part is counted once: a window counts as ingest memory while open and as
storage once compacted, and a query reads stored instances in place.

| Part | What | Bytes | Held |
|---|---|---|---|
| Ingest | open windows, one copy per worker | `I_D = w_D · (x_D / y_D) · k_D` | always |
| Storage | closed windows for the longest lookback served (one compacted copy) | `stored_D = max_i w_D · ((S_i − x_D) / y_D + 1)` | always |
| Compaction | the closed window's `k_D` partial copies, until merged | `k_D · w_D` (0 when `k_D = 1`) | while the compaction job runs |
| Query | the accumulators the merge creates (none when `n = 1`) and the output | `q_{i,D} = w_D · [n_{i,D} > 1] + card(G) · output bytes` | while the query job runs |

`AUC(memory) = Σ_D (I_D + stored_D + k_D · w_D · (compaction run time) / y_D)
+ Σ_i q_{i,D} · (query run time) / T_i`. CPU is elastic, so every job runs on its
own core: a compaction runs for `c_D` and a query for `ℓ_{i,D}`.

In words:

- `I_D`: every worker keeps one instance per group for each of the
  `x_D / y_D` windows still open, and there are `k_D` workers.
- `stored_D`: closed windows are kept until the longest lookback that `D`
  serves no longer needs them. That is `(S − x)/y + 1` windows per group.
- Compaction: when a window closes, a new one opens, so ingest still holds
  `x_D / y_D` open windows. The closed window's `k_D` partial copies stay in
  memory until the compaction has merged them. The merged copy takes the
  storage slot that the oldest window frees at the same moment, so it is not
  counted twice.
- `q_{i,D}`: a merge builds one accumulator per instance (a direct query,
  with `n = 1`, reads the stored window and needs none), plus the answer for
  every group.
- `AUC(memory)`: ingest and storage are held all the time. A compaction's
  and a query's memory are held only while they run, a fraction
  `run time / y_D` or `run time / T_i` of the time.

#### 5. Batch latency and the SLA

A batch's **latency** is the time from its issue until its last query job
finishes, compaction included. A plan's **query latency** is its worst batch
latency, given by the job placement (§6). Version 1 reports it; version 2
requires it to be at most the SLA `L`. Per-RAQE latency bounds
(`Raqe::latency_sla_ms`) are not used.

CPU is elastic, so no job waits for a core, and a batch's latency is its
longest **chain**: the newest window's compaction, then the query, each on a
full core, `c_D + ℓ_{i,D}` (`analytical_cost_model::chain_ms`). A plan's query
latency is the longest chain over its RAQEs.

In words: the trade-off between cost and latency comes from the plan. A cheap
plan keeps short windows and merges many of them at query time (long
chains); a faster one precomputes more at ingest (more open windows, more CPU
and memory). With no bound the MILP picks the cheapest plan, whatever its
latency; a bound `L` asks for the cheapest plan at least that fast.

#### 6. Job placement algorithm

The job placement decides when each job runs on the available CPU, and so
gives every batch's latency. It is defined for any CPU capacity; billing by
use makes CPU elastic, so it runs with unlimited capacity.

```text
capacity for jobs  K = C − Σ_D ρ_D      (C = ∞ when CPU is elastic)
event-driven, over the hyperperiod H = lcm(T_i, y_D), twice
(events: query issues, window closes, completions):
  ready = compaction jobs (triggered at window closes)
        + query jobs whose newest window is compacted
  order: issue / trigger time (older first), then compaction before query
         (it unblocks queries), then longest remaining work first
  give rates in that order: job j gets a_j = min(1, K − rates already given)
  a job at rate a_j finishes when its remaining work / a_j elapses
  recompute the rates at every event
outputs: per-RAQE and per-batch latency, worst batch latency
```

In words:

1. **Reserve ingest.** Ingest runs all the time and takes `Σ ρ_D` vCPUs; the
   rest, `K`, is shared by compaction and query jobs.
2. **Issue jobs.** Each window close triggers a compaction job for its
   deployment. Each firing issues a query job, which becomes ready only once
   its deployment's newest window is compacted.
3. **Prioritize.** Ready jobs run oldest batch first, so no batch starves.
   Within a batch, compaction runs before queries, because queries wait for
   it. Then the longest remaining work runs first, so a long job doesn't end
   up running alone at the end of the batch.
4. **Share capacity as rates.** In that order, each job gets up to one core
   (a single-threaded job can't use more) from the capacity still free. The
   last one served may get only a fraction and runs slower in proportion.
5. **Advance to the next event.** Time jumps to the next query issue, window
   close or completion; remaining work drops by rate × elapsed time, and the
   rates are recomputed.
6. **Read off the results.** A query's latency is its finish minus its
   issue, and a batch's latency is that of its last query.

With elastic CPU (`K = ∞`) every job starts as soon as it is ready on its own
core, so the placement has a closed form: a compaction runs for `c_D`, a
query for `ℓ_{i,D}`, and a batch's latency is its longest chain
`c_D + ℓ_{i,D}`. The code uses this closed form (`usage::usage_cost`), and
it is what makes both MILP versions exact.

#### 7. MILP formulation

Variables: `z_{i,D} ∈ {0,1}` (RAQE `i` uses `D`, for eligible pairs `E`),
`u_D ∈ {0,1}` (`D` is active), `stored_D ≥ 0`. Constants per candidate and
pair are §3 and §4's.

```text
minimize  w1 · [ Σ_D (ρ_D + c_D / y_D) · u_D + Σ_(i,D) (ℓ_{i,D} / T_i) · z_{i,D} ]
        + w2 · [ Σ_D ((I_D + k_D · w_D · c_D / y_D) · u_D + stored_D)
                 + Σ_(i,D) (q_{i,D} · ℓ_{i,D} / T_i) · z_{i,D} ]
s.t.      Σ_D z_{i,D} = 1                        for every RAQE i
          z_{i,D} ≤ u_D ≤ Σ_i z_{i,D}             for (i, D) ∈ E
          stored_D ≥ w_D · ((S_i − x_D) / y_D + 1) · z_{i,D}
version 2, a batch latency SLA L (or version 1's frontier bound L):
          z_{i,D} = 0  if c_D + ℓ_{i,D} > L
```

In words:

- The objective is the price of the mean vCPUs (ingest and compaction per
  active deployment, plus each RAQE's query work per second), plus the price
  of the mean memory (ingest and storage per active deployment, each
  compaction's partial copies for the fraction of time it runs, and each
  query's memory for the fraction of time it runs).
- Every RAQE is served by exactly one eligible deployment.
- A deployment is active if and only if some RAQE uses it, so its ingest is
  paid once, however many RAQEs share it.
- A deployment stores enough closed windows for the longest lookback it
  serves (a linearized max).
- Version 2 (and version 1's frontier bound): a pair whose chain (the newest
  window's compaction, then the query, each on a full core) is longer than
  `L` is ruled out. By the placement's closed form, the plan's batch latency
  is then at most `L`, exactly: the SLA is met. Chains are
  compared with `L` with a relative slack of `1e-9`, so float error can't rule
  out a chain equal to `L`.

#### 8. Solution method

Every term is linear in `u` and `z`, with or without a bound, so the MILP is
solved exactly
(`milp::minimize_usage_cost`; costs are scaled by each RAQE's cheapest pair
so the solver sees magnitudes near 1). The solved plan's cost and latency are
then computed from its resource use (`usage::usage_cost`).

- **PerQuery** is the same MILP with sharing ruled out. `minimize_usage_cost`
  takes `allowed`, the candidate indices each RAQE may use. PerQuery gives
  each RAQE its own candidates, with a separate copy of any deployment that
  two RAQEs could both use. Two RAQEs share a deployment only by choosing the
  same candidate index (`u_D` is per index), so their copies stay apart and
  each pays its own ingest.
- **AutoSketch** keeps its memory-chosen configs and is priced by the same
  function: its cost by use and its latency, the longest chain. It ignores
  latency, so it is one point, not a frontier.
- **Version 1:** ASAP and PerQuery are solved with no bound (the cheapest
  plan), and for a sweep of bounds `L` from the smallest feasible one (the
  largest, over RAQEs, of each RAQE's fastest chain) up to the unbounded
  plan's latency. Each solve gives a (latency, cost) point; together they are
  the method's frontier. `L` = AutoSketch's latency gives each method's cost
  at no more than AutoSketch's latency.
- **Version 2:** ASAP and PerQuery are solved at each SLA of a fixed grid;
  an SLA below the smallest feasible bound has no plan. AutoSketch ignores
  the SLA; its latency either meets it or not.

#### 9. Outputs

Per (method, weight setting, bound or SLA): cost; mean CPU and memory, by
part; query latency and per-RAQE latency; planning time. Figures: version 1,
each method's cost–latency frontier for each workload (ASAP and PerQuery as
lines, AutoSketch as a point); version 2, each method's cost at each SLA.

## Analytical cost model

The analytical model combines the empirical per-operation Sketch Bench
measurements with workload properties such as group cardinality, arrival rate,
window size, and query frequency.

For deployment `D` serving RAQE `i`:

- `lambda`: samples/sec arriving for `D`'s metric, `card(metric.labels) / scrape_interval`.
- `card(G)`: cardinality of `G`, so the number of parallel accumulator instances per window (one for a shared fixed-size sketch; see below).
- `x`, `y`: `D`'s window and slide.
- `a_D = x / y`: open windows; each sample is inserted into `a_D` instances of its group.
- `S_i`, `T_i`: RAQE `i`'s lookback and interval.
- `n_i = S_i / x`: windows merged per query.
- `m`: memory per instance, measured. For DDSketch on a metric with a
  `value_range`, `m = (floor(ln(hi / lo) / ln((1 + alpha) / (1 - alpha))) + 1) × 8`
  bytes instead: one bucket count per `gamma`-power in the range, capped by
  the values one instance sees (`λ · x / card(G)` for a window, `λ · L / card(G)`
  for a query's merge accumulator), as sketch-bench's `dd_footprint` counts them. The range is the metric's, so `m` is an upper
  bound for a group whose own values span less.
- `c_ins`: CPU per insert, measured.
- `c_mrg`: CPU per pairwise merge, measured.
- `c_qry`: CPU per query of one instance, measured.

Costs split into four phases, each with CPU (mean CPU-sec/sec) and memory
(bytes):

| Phase | CPU | Memory |
|---|---|---|
| Ingest, per active `D` | `lambda × a_D × c_ins` | `card(G) × m × a_D` (open windows) |
| Merge, per RAQE | `card(G) × (n_i − 1) × c_mrg / T_i` | `card(G) × m` (one accumulator per group); 0 when `n_i = 1` |
| Query, per RAQE | `card(G) × c_qry / T_i` | `card(G) ×` output bytes |
| Storage, per active `D` | 0 | `card(G) × m × ((max_i S_i − x) / y + 1)` (closed windows) |

Closed windows wholly inside a lookback start in `[t − S, t − x]`, hence
`(S − x) / y + 1` of them; storage takes the longest lookback `D` serves.
Query output is estimated at 8 bytes per group, or 32 × 16 bytes per group for
top-k (sketch-bench's heap size, with 64-bit key hashes); it is not measured.

### Instance shape and size law

Each row of the cost table (from
[`scripts/study_saturation.py --phase optimizer-cost`](../scripts/study_saturation.py))
is measured on one instance, at whatever key count the benchmark fed it
(`measured_keys`). A key is one distinct entry an instance stores exactly,
such as one group's running sum in an exact accumulator. Turning a row into a
deployment's cost needs two properties of the family:

- **Shape**: instances per window. `PerGroup` keeps one per group, so
  `card(G)`; `Shared` keeps one for all groups.
- **Law**: how one instance's size grows. `Fixed` is set by the configuration
  (e.g. CMS rows × columns); `PerKey` stores one entry per key, so it grows
  linearly with keys.

Memory per window is instances × instance size:

| | Fixed | PerKey |
|---|---|---|
| **PerGroup** | `card(G) × m`: CMS, CountSketch, KLL, DDSketch, HLL, UnivMon, top-k (heap `m · k`) | `card(G) × m × keys_per_group / measured_keys`: none yet (e.g. an exact top-k map) |
| **Shared** | `m`: HydraKLL and other Hydra sketches | `m × card(G) / measured_keys`: exact sum, min, max, increase (one value per group, so keys = groups) |

The same factor scales merge and query CPU. Insert CPU is per sample and does
not scale.

Quantile sketches have no keys, but are not strictly fixed either: KLL grows
slowly with the number of values inserted (about `k × log(n / k)`), and
DDSketch with the range of values. The model treats KLL as Fixed at the size
the export measured (1,000,000 values per instance), so it misstates instances
that see far fewer or far more values. DDSketch is sized from the metric's
`value_range` when given (capped by values per instance), else from the
measured size.

The model uses `I × m`, where `I` is instances per window:

- PerGroup + Fixed: `I = card(G)`.
- Shared + PerKey reduces to `I = card(G)` because the export divides the
  exact accumulators' memory and merge cost by `measured_keys`
  (`groups_per_instance`), making `m` a per-group cost. Their query cost is
  already per group.
- Shared + Fixed: `I = 1`. In code this is the family property
  `one_fixed_size_sketch_for_all_groups`, true only for HydraKLL. It applies
  to memory and merge; query stays `card(G) × c_qry`, one probe per group.
  A fixed-size sketch degrades as groups grow, so it is eligible only when
  `card(G)` is at most the `subpopulations` its row was measured at. This
  models a sketch keyed by the joined `G` value alone; the current rows fan
  out to every label subset (sketch-bench#165), and merging several windows
  is not yet measured (sketch-bench#166).

PerGroup + PerKey is deferred until a family needs it: it needs the key labels
`K`, the metric's labels minus `G`. So for every family in use, nothing scales
with keys below the group: a per-service top-k CMS costs the same however
many endpoints it counts.

Side by side, with `G = card(G)` and `C` closed windows:

| | KLL (per group) | exact-sum (per-group normalized) | HydraKLL (shared, fixed) |
|---|---|---|---|
| `m`, `c_mrg` in the row | one group's | whole ÷ groups | whole sketch |
| Ingest CPU | `λ·a_D·c_ins` | `λ·a_D·c_ins` | `λ·a_D·c_ins` |
| Ingest memory | `G·m·a_D` | `G·m·a_D` | `m·a_D` |
| Merge CPU | `G·(n_i−1)·c_mrg/T_i` | same | `(n_i−1)·c_mrg/T_i` |
| Merge memory | `G·m` | same | `m` |
| Query CPU | `G·c_qry/T_i` | same | `G·c_qry/T_i` |
| Query memory | `G ×` output bytes | same | `G ×` output bytes |
| Storage memory | `G·m·C` | same | `m·C` |
| Latency | `G·(c_qry + (n_i−1)·c_mrg)` | same | `(n_i−1)·c_mrg + G·c_qry` |

### Family properties and key tracker

Each family has properties, hardcoded by variant in `family_properties`
(an unknown variant panics rather than getting a default):

- `mergeable_across_windows`: a family without it only serves `S_i = x`.
  Every family in use merges.
- `one_fixed_size_sketch_for_all_groups`: the Shared + Fixed cell above.
- `needs_delta_set_key_tracker`: the sketch can't list its groups, so a
  DeltaSet (`exact-delta-set`) records each window's keys for the query to
  probe. True for HydraKLL. A candidate needing one carries the DeltaSet row
  and is dropped if the cost table has none; a deployment without its tracker
  is never eligible. The cost table may hold at most one DeltaSet row.

The tracker is priced as an exact accumulator on the same `x`, `y` and `G`,
in every phase: ingest, merge (union of the `n_i` windows' key sets), query
(enumerate keys) and storage. Its `m` is per key, so it adds `card(G) × m_ds`
per window. The AutoSketch baseline skips shared fixed-size families for now
(sketch-bench#159).

Per-RAQE latency is the serial CPU time of one query:

```text
latency_i = card(G) × c_qry + I × (n_i − 1) × c_mrg
```

summed over the sketch and its key tracker, if any.

On its own it is not a wall-clock SLA: it assumes no parallel execution across
groups and no cheaper k-way merge. In "Cost by use and batch latency" above it
is one firing's query job `ℓ_{i,D}` (merge, then estimate) on one core.


## Future Hydra cost and measurement requirements

It does not make sense to apply the "fine-to-coarse rollup strategy" to Hydra.

The required accuracy contract is an empirical predicate:

```text
hydra_error(config, schema H, requested grouping G_r,
            target-shape(G_r), items per target group,
            subpopulation population/fan-out, merged_windows)
    <= RAQE accuracy SLA
```

The result must be reported in the family metric (for Hydra-KLL, `mean_rank_err`) with the same direction as the RAQE SLA. It is insufficient to record only a single `subpopulations` ceiling: two target groupings with the same cardinality can have different skew, fan-out, and query selectivity. No analytic composition from the KLL cell's guarantee to the Hydra grid's error is assumed. In particular, cross-window Hydra merge accuracy must be measured for every allowed merge depth; until then a Hydra candidate may serve only `L == x`.

### Proposed Hydra cost model

Let `m_h`, `c_ins,h`, `c_mrg,h`, and `c_probe,h(G_r)` be measured for a particular Hydra configuration and label schema. Hydra's shared grid has one state per open window, so for slide `y` it has `a = x/y` live states.

| Phase | CPU | Memory |
| --- | --- | --- |
| Ingest | `lambda * a * c_ins,h` | `a * m_h` |
| Merge a RAQE | `(L/x - 1) * c_mrg,h / T` | `m_h` when `L/x > 1` |
| Query a RAQE | `C_r * c_probe,h(G_r) / T` | `C_r * output_bytes` |
| Storage | `0` | `ceil((max L - x)/y + 1) * m_h` |

This is deliberately not the #190 formula: a Hydra merge combines whole windows, not `C_d` independently addressable states. It assumes the measured merge cost does not depend on the number of requested probes; Sketch Bench must confirm that assumption or report a probe-count term.

Hydra also needs a way to enumerate the groups to query. If it still relies on `exact-delta-set`, the plan must add that tracker's ingest, merge, query, and storage costs. A fine-group tracker used to derive coarse group keys may cost `C_d`, not `C_r`; alternatively, a per-requested-group tracker must be specified and measured. This is a design choice, not a free operation.

### Sketch Bench requirements before admitting Hydra

1. Accept a named label schema and query grouping/predicate, rather than interpreting semicolon position as the meaning of a grouping. Preserve label names and their order in the emitted record.
2. Generate hierarchical data with exact answers for every queried grouping; include cardinality, per-parent fan-out, skew, total items, and each target group's item count in the result metadata.
3. Sweep Hydra configuration (outer grid and cell parameters), schema width, target grouping, cardinality/fan-out, and distribution shape. Measure accuracy separately for each target grouping.
4. Measure insert, footprint, one-window query, and merges over the intended shard counts. For a merged query, measure both sketch merge and the requested number of probes, so the planner can distinguish a whole-sketch merge from per-group probe work.
5. Benchmark and export the key-enumeration path. State whether it uses a DeltaSet, what grouping it stores, and its exact input/output cardinality.
6. Emit a machine-readable saturation key containing the fields in the accuracy contract above. The optimizer must reject a Hydra candidate when no matching point (or explicitly approved interpolation) exists; it must not borrow a KLL or a different-grouping curve.
7. Add regression fixtures for equal grouping, a direct coarse predicate, and a multi-window coarse predicate. Each fixture must compare to exact answers and assert the exported accuracy and cost dimensions.
