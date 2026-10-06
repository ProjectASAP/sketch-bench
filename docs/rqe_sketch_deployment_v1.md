# RQE-to-sketch deployment mapping: v1

## Purpose

Given a workload of repeating query expressions (RQEs), find sketch deployments
that can serve it and report the trade-offs among query working memory, CPU,
and per-RQE latency.

## Inputs

Let `R = {r_1, ..., r_n}` be the RQE workload. Each RQE `r_i` provides:

| Field | Meaning |
|---|---|
| `cap_i` | Required capability: `freq`, `quantile`, `cardinality`, or `topk`. |
| `S_i` | Query-window size. This is the current `lookback` field. |
| `T_i` | Query slide: how often the RQE runs. This is the current `interval` field. |
| `metric_i` | Metric the RQE reads. |
| `G_i` | Group-by label set (`grouping_labels`). |
| `accuracy_metric_i` | Accuracy measurement to check. |
| `tol_i` | Accuracy threshold. |
| `direction_i` | Whether lower or higher values are better. |

For each metric, the caller provides `MetricFacts`:

- `labels`: every label the metric's series carry.
- `scrape_interval`: every series yields one sample per scrape.
- `card(X)`: distinct value combinations of each label set `X` in use,
  including `labels` itself (the raw series count).

The arrival rate is derived as follows (not provided by the user):

```text
lambda(metric) = card(metric.labels) / scrape_interval      samples/second
```

`validate_facts` reports every missing or inconsistent fact up front.

sketch-bench supplies empirical measurements for each sketch configuration:

- memory per sketch instance;
- insert CPU per item;
- query CPU per estimate;
- CPU per pairwise merge; and
- measured accuracy values.

Configurations are associated with the capabilities they can serve. The
implementation uses measured configuration variant names, because variants of
the same algorithm can have different costs.

## Window model

A deployment materializes sliding sketch instances with:

- `x`: sketch-window size;
- `y`: sketch slide.

For every label-value group, it creates an instance for every interval:

```text
[k × y, k × y + x)
```

At time `t`, an instance is **open** while its interval has not ended
(`t < k × y + x`): it still receives samples. After that it is **closed**: it
never changes again and is kept only while some RQE's lookback still needs it.
A deployment has `x / y` open instances per group at any time.

Instances overlap when `x > y`. There is no subtract operation. To answer an
RQE, the system selects and merges only non-overlapping instances, so items
are never counted twice.

An RQE with query window `S` and query slide `T` can use a deployment when:

```text
x % y == 0
S % x == 0
T % y == 0
```

For a query ending at time `t`, use the instances:

```text
[t-S, t-S+x), [t-S+x, t-S+2x), ..., [t-x, t)
```

They exactly cover the query window and do not overlap. The deployment may
have produced other, overlapping instances between these starts. That is okay. Those overlapping instances are not used for this query.

`S % T == 0` is not required. For example, a 10-minute window running every
3 minutes can use `x = 2 minutes` and `y = 1 minute`; each query merges five
non-overlapping two-minute instances.

We assume all RQEs share a common time origin. Query phase offsets are not
modeled.

## Candidate deployments

A candidate deployment is:

```text
D = (capability, configuration, metric, G, x, y)
```

Candidates may be generated from individual RQEs, then deduplicated. This is
only a way to construct the candidate set: when solving a workload, one
selected deployment may serve multiple compatible RQEs.

Intuition: candidates are built in two steps, and each drops only candidates
that another candidate beats, so the best plan is never lost. Generation uses
no measurements. It keeps every legal window and configuration, and drops only
slides that a coarser slide beats for the same RQEs. Pruning then attaches the
measured costs and drops a candidate when another serves all of its RQEs and
is no worse on every cost.

For each (capability, metric, G) group, generate candidates as follows:

```text
for each window x that divides some S_i and is a multiple of the scrape interval:
    for each RQE i whose S_i is divisible by x:
        g_i = gcd(x, T_i)    # the largest slide that serves RQE i
    slides = { gcd(A) : A is a non-empty subset of {g_i} }
    for each y in slides that is a multiple of the scrape interval:
        for each configuration that serves the capability:
            add (capability, configuration, metric, G, x, y)
```

**Windows.** An RQE merges `S_i / x` whole windows, so `x` must divide `S_i`.
Windows and slides finer than the scrape interval would only split one
scrape's samples.

**Slides.** A slide `y` serves RQE `i` when it divides both `x` and `T_i`,
that is, when it divides `g_i`. A finer slide serves no more RQEs but holds
more open windows (`x / y`) and more closed ones, so it costs more ingest and
storage at the same latency. Hence:

- for one RQE, `y = g_i` is strictly better than every finer slide that serves
  it;
- for a set `A` of RQEs sharing a deployment, `y = gcd(A)`, the largest slide
  dividing every `g_i` in `A`, is strictly better than every finer slide that
  serves them all.

Which RQEs share is the solver's choice, so every subset's gcd is a candidate.
Every other divisor of `x` is strictly worse than one of these.

Example: `x = 60`, RQE `a` every 20 s, RQE `b` every 30 s.

| Served RQEs | `y` | Open windows `x / y` |
|---|---|---|
| `a` | gcd(60, 20) = 20 | 3 |
| `b` | gcd(60, 30) = 30 | 2 |
| `a` and `b` | gcd(20, 30) = 10 | 6 |

The slides are `{10, 20, 30}`. `y = 5` would also serve both, but holds 12 open
windows instead of 6. With a 15 s scrape interval only `y = 30` remains: no
multiple of 15 divides both 20 and 30, so `a` and `b` cannot share at
`x = 60`.

The subset gcds take one pass, without listing subsets: keep the gcds found so
far, and for each new `g` add `g` and `gcd(found, g)` for every `found`. For
`{20, 30}`: `{20}`, then `{20, 30, 10}`. One pass suffices because
`gcd(A ∪ {g}) = gcd(gcd(A), g)`.

An RQE `r_i` is eligible for a candidate `D` when:

1. `cap_i = D.capability`, `metric_i = D.metric` and `G_i = D.G`.
2. `D.x % D.y = 0`, `S_i % D.x = 0`, and `T_i % D.y = 0`.
3. `D.configuration` contains a measured value for `accuracy_metric_i` that clears
   `tol_i` in `direction_i`.

For lower-is-better metrics, passing means `measured <= tol_i`. For
higher-is-better metrics, passing means `measured >= tol_i`. Direction is
explicit on the RQE; it is never inferred from a metric name. A missing metric
does not pass.

Before mapping, prune a candidate only if another candidate can serve every
RQE it can and is no worse in ingest CPU and memory, and in latency, merge
memory and stored memory for each such RQE. This is safe because any mapping using
the removed candidate can substitute the remaining one without weakening a
modeled objective.

Closed instances must be stored long enough to answer the RQEs assigned to a
deployment. Retention is not a candidate parameter: a deployment stores the
history needed by the largest assigned query window, costed as the storage
phase (below).

## Workload mapping

Let `z_{i,D}` be 1 when RQE `i` uses candidate deployment `D`; let `u_D` be 1
when deployment `D` is active.

```text
Σ_D z_{i,D} = 1       for every RQE i
z_{i,D} <= u_D        for every eligible pair (i, D)
```

Every RQE selects exactly one eligible deployment. Multiple RQEs may select
the same deployment and therefore share its ingest work. A query may merge
multiple instances from its selected deployment, but v1 does not combine
results from several deployments to satisfy one RQE.

### MILP formulation

Let `E` be the eligible `(i, D)` pairs. The solver has one binary `z_{i,D}`
for each pair in `E`, one binary `u_D` for each candidate, and a continuous
`stored_D` for each candidate's closed-window storage.

```text
z_{i,D}  ∈ {0, 1}    for (i, D) ∈ E
u_D      ∈ {0, 1}    for D ∈ candidates
stored_D ≥ 0

Σ_{D: (i,D) ∈ E} z_{i,D} = 1                    for every RQE i
z_{i,D} ≤ u_D                                    for (i, D) ∈ E
u_D ≤ Σ_{i: (i,D) ∈ E} z_{i,D}                   for every candidate D
stored_D ≥ w(storage_{i,D}) × z_{i,D}            for (i, D) ∈ E
```

The `u_D ≤ Σ z` constraint prevents a deployment from becoming active when no
RQE selected it. It is not required for feasibility, but makes ingest cost
unambiguous. `stored_D` is a linearized max: a deployment stores enough for
the longest lookback it serves.

The only objective, `Objective::AUCCost { w_cpu, w_mem }`, weighs each phase
cost (below) as `w(c) = w_cpu × c.cpu + w_mem × c.memory_GiB`:

```text
minimize  Σ_D w(ingest_D) × u_D
        + Σ_(i,D)∈E (w(merge_{i,D}) + w(query_{i,D})) × z_{i,D}
        + Σ_D stored_D
```

CPU is the area under the CPU curve (mean CPU-sec/sec), so plans are sized
for the mean load, not for bursts. Memory sums every phase, as if every query
evaluates at once. The default weights are `(1, 0)`: CPU only.

Per-RQE latency bounds forbid the pairs over them (`z_{i,D} = 0`). The solver
never enumerates full mappings.

## Analytical cost model

The analytical model combines the empirical per-operation Sketch Bench
measurements with workload properties such as group cardinality, arrival rate,
window size, and query frequency.

For deployment `D` serving RQE `i`:

- `lambda`: samples/sec arriving for `D`'s metric, `card(metric.labels) / scrape_interval`.
- `card(G)`: cardinality of `G`, so the number of parallel accumulator instances per window.
- `x`, `y`: `D`'s window and slide.
- `a_D = x / y`: open windows; each sample is inserted into `a_D` instances of its group.
- `S_i`, `T_i`: RQE `i`'s lookback and interval.
- `n_i = S_i / x`: windows merged per query.
- `m`: memory per instance, measured.
- `c_ins`: CPU per insert, measured.
- `c_mrg`: CPU per pairwise merge, measured.
- `c_qry`: CPU per query of one instance, measured.

Costs split into four phases, each with CPU (mean CPU-sec/sec) and memory
(bytes):

| Phase | CPU | Memory |
|---|---|---|
| Ingest, per active `D` | `lambda × a_D × c_ins` | `card(G) × m × a_D` (open windows) |
| Merge, per RQE | `card(G) × (n_i − 1) × c_mrg / T_i` | `card(G) × m` (one accumulator per group); 0 when `n_i = 1` |
| Query, per RQE | `card(G) × c_qry / T_i` | `card(G) ×` output bytes |
| Storage, per active `D` | 0 | `card(G) × m × ((max_i S_i − x) / y + 1)` (closed windows) |

Closed windows wholly inside a lookback start in `[t − S, t − x]`, hence
`(S − x) / y + 1` of them; storage takes the longest lookback `D` serves.
Query output is estimated at 8 bytes per group, or 32 × 16 bytes per group for
top-k (sketch-bench's heap size, with 64-bit key hashes); it is not measured.

### Instance shape and size law

Each exported cost row is measured on one instance, at whatever key count the
benchmark fed it (`measured_keys`). Turning that into a deployment's cost
needs two properties of the family:

- **Shape**: instances per window. `PerGroup` keeps one per group, so
  `card(G)`; `Shared` keeps one for all groups.
- **Law**: how one instance's size grows with its keys. `Fixed` is set by the
  configuration (e.g. CMS rows × columns); `PerKey` grows with every key.

Memory per window is instances × instance size:

| | Fixed | PerKey |
|---|---|---|
| **PerGroup** | `card(G) × m`: CMS, CountSketch, KLL, DDSketch, HLL, UnivMon, top-k (heap fixed at k = 32) | `card(G) × m × keys_per_group / measured_keys`: none yet (e.g. an exact top-k map) |
| **Shared** | `m`: HydraKLL and other Hydra sketches | `m × card(G) / measured_keys`: exact sum, min, max, increase (one value per group, so keys = groups) |

The same factor scales merge and query CPU. Insert CPU is per sample and does
not scale.

The model uses one formula, `card(G) × m`, which is right for both cells in
use:

- PerGroup + Fixed is that formula as is.
- Shared + PerKey reduces to it because the export divides the exact
  accumulators' memory and merge cost by `measured_keys`
  (`groups_per_instance`), making `m` a per-group cost. Their query cost is
  already per group.

The other two cells are deferred until a family needs them: Shared + Fixed
would be overpriced by `card(G)` (sketch-bench#142), and PerGroup + PerKey
needs the key labels `K`, the metric's labels minus `G`.

Per-RQE latency is the serial CPU time of one query:

```text
latency_i = card(G) × (c_qry + (n_i − 1) × c_mrg)
```

It is not a wall-clock SLA: it assumes no parallel execution across groups and
no cheaper k-way merge.

### What each cost is counted per

| Cost | Shared by RQEs on one deployment? | × groups `card(G)` | × windows | Per key inside an instance |
|---|---|---|---|---|
| Ingest CPU | yes, paid once per deployment | no, `lambda` already totals all groups | × `x/y` open | each sample is one insert, whatever the family |
| Ingest memory | yes | yes | × `x/y` open | not modeled |
| Storage memory | yes, sized for the longest `S` it serves | yes | × `(S−x)/y + 1` closed | not modeled |
| Merge CPU | no, per RQE | yes | × `(S/x − 1)` folds | not modeled |
| Merge memory | no, per RQE | yes | × 1 accumulator, 0 when `n_i = 1` | not modeled |
| Query CPU | no, per RQE | yes | — | one probe per group |
| Query memory | no, per RQE | yes | — | 8 B, or 32 × 16 B for top-k |
| Latency | per RQE | yes | `S/x − 1` merges | — |

Keys inside an instance matter only through the size law above. For every
family in use, nothing scales with keys below the group: a per-service top-k
CMS costs the same however many endpoints it counts.

Not shared, though it may look shareable:

- **The stream read.** Two deployments on the same metric each pay full
  ingest. Only RQEs on the same deployment share it.
- **Merges and query results.** Two RQEs with the same lookback on one
  deployment each pay their own merge and query.
- **Groupings.** A `{service, endpoint}` deployment does not serve
  `by (service)`: `G` must match exactly, with no regrouping at query time.
- **Capabilities.** RQEs with different capabilities never share a
  deployment, even on the same metric and `G`.

## Procedure

1. Generate candidate deployments for each input RQE, then deduplicate them.
2. Build the eligible candidate–RQE pairs. Reject pairs that fail capability,
   metric, grouping, window alignment, or empirical accuracy.
3. Enumerate feasible workload mappings. Every RQE must select one eligible
   candidate; a single selected candidate may serve multiple RQEs. Small
   instances may retain every mapping eagerly; larger ones stream mappings
   through incremental Pareto filtering, retaining only the current frontier.
4. Score every complete mapping with the analytical cost model.
5. Compute and report the Pareto frontier.

The Pareto vector is:

```text
(CPU, memory, {latency_i})
```

CPU and memory are the sums over the four phases. The report also includes
each phase's CPU and memory, together with the selected deployment mapping.

## v1 scope and TODOs

- **Accuracy after merging:** v1 uses the measured, single-instance accuracy
  of a configuration. Measure or model merged accuracy before relying on a
  selected mapping as truly accuracy-feasible.
- **Query-result sharing:** v1 charges every RQE its own query and merge CPU.
  Revisit when RQE semantics and execution timing identify safe reuse cases.
- **Latency SLAs:** the MILP takes optional per-RQE latency bounds; the
  enumerator reports latency but does not reject a mapping for it.
- **Memory model:** query memory sums every RQE's merge and output memory, as
  if all queries run at once; real concurrency is not modeled. Query output
  size is an estimate.
- **Bursts:** CPU is a mean over time, so plans are sized for average load.
- **Static planning:** no RQE churn, replanning, or migration cost.
- **Rollups:** v1 does not precompute merged rollups. Queries merge their
  selected base instances when they run.
- **Selection policy:** the enumerator reports the Pareto frontier; the MILP
  selects one point by the `w_cpu`/`w_mem` weights.

## Implementation status

`rqe-optimizer/` implements the `(x, y)` sliding-instance model, candidate
generation and pruning, eligibility checks, analytical objectives, and
streaming Pareto filtering. The separation between candidate generation,
enumeration, objectives, and Pareto filtering remains the intended boundary
for future solver work.

Exporter bugs, benchmark-dataset choices, and historic run counts belong in
`rqe_optimizer_TODO.md` or issue tracking, not in this problem statement.
