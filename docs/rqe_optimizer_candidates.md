# RQE optimizer: candidate generation and mapping

This document is the canonical design for candidate construction, eligibility, dominance pruning, finer-to-coarser roll-ups, and the assignment MILP.

## Candidate deployments

A candidate deployment is:

```text
D = (capability, configuration, metric, G, x, y)
```

Candidates may be generated from individual RAQEs, then deduplicated. This is
only a way to construct the candidate set: when solving a workload, one
selected deployment may serve multiple compatible RAQEs.

Intuition: candidates are built in two steps, and each drops only candidates
that another candidate beats, so the best plan is never lost. Generation uses
no measurements. It keeps every legal window and configuration, and drops only
slides that a coarser slide beats for the same RAQEs. Pruning then attaches the
measured costs and drops a candidate when another serves all of its RAQEs and
is no worse on every cost.

For each (capability, metric, G) group, generate candidates as follows:

```text
for each window x that divides some S_i and is a multiple of the scrape interval:
    g = { g_i = gcd(x, T_i) : RAQE i with S_i divisible by x }
    slides = { gcd(A) : A is a non-empty subset of g }
    for each y in slides that is a multiple of the scrape interval:
        for each configuration that serves the capability:
            add (capability, configuration, metric, G, x, y)
```

`g_i = gcd(x, T_i)` is the largest slide that serves RAQE `i`.

**Windows.** An RAQE merges `S_i / x` whole windows, so `x` must divide `S_i`.
Windows and slides finer than the scrape interval would only split one
scrape's samples.

**Slides.** A slide `y` serves RAQE `i` when it divides both `x` and `T_i`,
that is, when it divides `g_i`. A finer slide serves no more RAQEs but holds
more open windows (`x / y`) and more closed ones, so it costs more ingest and
storage at the same latency. Hence:

- for one RAQE, `y = g_i` is strictly better than every finer slide that serves
  it;
- for a set `A` of RAQEs sharing a deployment, `y = gcd(A)`, the largest slide
  dividing every `g_i` in `A`, is strictly better than every finer slide that
  serves them all.

Which RAQEs share is the solver's choice, so every subset's gcd is a candidate.
Every other divisor of `x` is strictly worse than one of these.

Example: `x = 60`, RAQE `a` every 20 s, RAQE `b` every 30 s.

| Served RAQEs | `y` | Open windows `x / y` |
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

An RAQE `r_i` is eligible for a candidate `D` when:

1. `cap_i = D.capability`, `metric_i = D.metric` and `G_i = D.G`.
2. `D.x % D.y = 0`, `S_i % D.x = 0`, and `T_i % D.y = 0`.
3. The accuracy of `D` for `r_i`, in the accuracy metric of `D`'s family
   (`family_properties`, which also gives the direction), clears `tol_i`.

Exact accumulators take that accuracy from the cost table; they merge without
loss, so the window size doesn't matter. Sketches take it
from the saturation study's error-vs-N curves (`SaturationCurves`, #156),
read at the number of items one group receives over the RAQE's whole
lookback: series per group times scrapes per lookback.

- The curve is the configuration's at the metric's fitted `data_shape` for
  `G` (zipf θ and keys `K`, or tail index `a` for quantiles). Between grid
  points, the worst bracketing point; outside the grid, or with no fit, no
  accuracy.
- Between checkpoints, the worse neighbour. Below the first checkpoint, no
  accuracy. Past the last, the plateau if the point saturated, else none.
- The curve's `error_metric` must be the family's metric; `SaturationCurves::load`
  refuses a curve for another metric as a stale study.
- For sketches that merge exactly, the window size doesn't matter: a merged
  answer reads the curve at the lookback's item count, like a single sketch.
  KLL and top-k merge lossily (#131): an answer merged from `m = S_i / D.x`
  windows reads the study's merge curve (`saturation_merge_curve.csv`, the
  sketch merged from `m` shards of the same items) at the lookback's item
  count. Between measured shard counts, the worse. Past the largest
  measured count, or without a merge curve, a merged KLL or top-k answer has
  no accuracy: the study must measure the merge counts a workload needs
  (`--merge-shards-list`) up to the N it reads at: past the merge curve's
  last N, the merged answer has no accuracy either (#158).
  `SaturationCurves::load` refuses a study with no merge curves for a
  candidate KLL sketch.
- A top-k RAQE asks for its own `k` (`Raqe::topk_k`, default 32). Heap
  top-k (CMS-heap, CountSketch-heap) keeps a heap of `m · k` in a deployment
  answering from `m = S_i / D.x` merged windows (`m = 1` gives `k`).
  Candidates carry one heap per distinct `m · k` among the RAQEs on a window,
  and a RAQE is eligible only where the heap holds its `m · k`. Such a merged answer reads as one sketch: the
  plain curve, then the guarantee fallback. It assumes the merged heaps still
  hold the true top `k`; it is not a guarantee. So heap top-k needs no merge
  curves. Costs come from the cost table's rows at heaps 32, 128, 512 and
  2048 (`heap=`), linear in the heap between them and past them. Curves are
  measured per `k` (`topk_k=` in the config; 10, 32 and 100 by default) at a
  heap of `k`, and larger heaps read the same curve. A RAQE reads the curve
  at its `k`, else at the next larger measured `k` (harder); past every
  measured `k`, only the guarantee at its `k`. Its query outputs `k`
  entries per group.
- Where the study measured nothing for a bracketing point at or above its
  first N (past an unsaturated curve, past the measured merge counts or N,
  or with no merge curve), the accuracy falls back to the algorithm's
  guarantee at 95% confidence (`rqe_optimizer::theory`; two-sided for
  rank and relative errors, one-sided for CMS overestimates). It is never
  better than the worst last measurement of the curves it extends. The fallbacks are:
  - KLL: rank error `2.296 / k^0.9723` (DataSketches' 99% value), scaled
    to 95% two-sided;
  - DDSketch: relative value error `α` (unbounded store);
  - HLL: `1.96 · 1.04 / sqrt(2^lg_k)`;
  - CMS-heap top-k, unmerged: the share of the top `k` Zipf(θ, K) keys kept
    ranked for every key at once. With Markov per row, independent rows,
    and a union bound over the `K − k` other keys, key `i` fails with
    probability at most `(cols · (p_i − p_{k+1}))^−rows`. Keys are counted
    from the most frequent while the summed failure stays within 5%. It
    assumes the heap ranks keys by their final CMS estimates.

  A sketch with no published bound, a merged top-k answer, a point below
  the first N, or a shape outside the grid stays unknown.
  `accuracy_with_source` reports which points used a bound.

The cost table's sketch accuracies are not read, but they are one point on
each curve: a row's `measured_at` (items, Zipf θ and population, or Pareto
`a`) is a grid point's key. `SaturationCurves::check_cost_table` reports every
row whose `accuracy_metric` is not its family's or not its curve's
`error_metric`, whose configuration is not in the grid, or whose accuracy
disagrees with the curve at `measured_at` (beyond 3 seed standard errors plus
5%). The study adds the cost table's shape to the grid as a full row and
column (θ = 1.1 at every K, K = 1e4 at every θ), so every sketch row has a
grid point and the grid stays a full cross. `SaturationCurves::load`
refuses curves whose Zipf grid isn't a full cross of its θ and K values and
names the missing points: a hole would leave every shape around it without
accuracy (for example, curves from `--points-from` or a narrowed `--resume`). A row without a grid point (a study run
with `--no-cost-shape`) is reported as unchecked. `small_problem` refuses to plan
when any row disagrees.

For lower-is-better metrics, passing means `measured <= tol_i`. For
higher-is-better metrics, passing means `measured >= tol_i`. Direction is
explicit on the RAQE; it is never inferred from a metric name. A missing metric
does not pass.

Before mapping, prune a candidate only if another candidate can serve every
RAQE it can and is no worse in ingest CPU and memory, and in latency, merge
memory and stored memory for each such RAQE. This is safe because any mapping using
the removed candidate can substitute the remaining one without weakening a
modeled objective.

Closed instances must be stored long enough to answer the RAQEs assigned to a
deployment. Retention is not a candidate parameter: a deployment stores the
history needed by the largest assigned query window, costed as the storage
phase (below).

## Workload mapping

Let `z_{i,D}` be 1 when RAQE `i` uses candidate deployment `D`; let `u_D` be 1
when deployment `D` is active.

```text
Σ_D z_{i,D} = 1       for every RAQE i
z_{i,D} <= u_D        for every eligible pair (i, D)
```

Every RAQE selects exactly one eligible deployment. Multiple RAQEs may select
the same deployment and therefore share its ingest work. A query may merge
multiple instances from its selected deployment, but v1 does not combine
results from several deployments to satisfy one RAQE.

### MILP formulation

Let `E` be the eligible `(i, D)` pairs. The solver has one binary `z_{i,D}`
for each pair in `E`, one binary `u_D` for each candidate, and a continuous
`stored_D` for each candidate's closed-window storage.

```text
z_{i,D}  ∈ {0, 1}    for (i, D) ∈ E
u_D      ∈ {0, 1}    for D ∈ candidates
stored_D ≥ 0

Σ_{D: (i,D) ∈ E} z_{i,D} = 1                    for every RAQE i
z_{i,D} ≤ u_D                                    for (i, D) ∈ E
u_D ≤ Σ_{i: (i,D) ∈ E} z_{i,D}                   for every candidate D
stored_D ≥ w(storage_{i,D}) × z_{i,D}            for (i, D) ∈ E
```

The `u_D ≤ Σ z` constraint prevents a deployment from becoming active when no
RAQE selected it. It is not required for feasibility, but makes ingest cost
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

Per-RAQE latency bounds forbid the pairs over them (`z_{i,D} = 0`). The solver
never enumerates full mappings.


## Fine-to-coarse group roll-ups (#190)

### Vocabulary

A deployment has a source grouping `G_d`; a RAQE asks for a result grouped by `G_r`. `C_d = card(G_d)` and `C_r = card(G_r)`. A conventional, per-group sketch can answer `G_r subseteq G_d` by merging the fine states that belong to each coarse group. We call that operation a *roll-up*.

Hydra is different. It ingests a labelled record into one shared grid and answers a subpopulation predicate from that grid. It does not expose one KLL per fine group for the optimizer to merge. A coarse Hydra answer, if the Hydra layout supports its predicate, is a direct query of the shared sketch, not the generic roll-up operation.

### What #190 guarantees today

For the families marked `mergeable_across_groups`, #190 permits `G_r subseteq G_d`. Thus, one can never use a coarse deployment for a fine RAQE. The eligible families are exact sum/min/max, HLL, cardinality UnivMon, KLL, and DDSketch. Rate/increase and top-k remain exact-grouping only i.e. `G_r == G_d`.

For a conventional per-group sketch, a query over `L` using base windows `x` folds this many states:

```text
M = C_d * L/x - C_r
```

Thus merge CPU is `M * c_mrg / T`, merge memory is `C_r * m(L)`, and query CPU and output are priced at `C_r`. Ingest and retained base-window storage remain priced at `C_d`. This reduces to the old model when the groupings are equal, and it correctly charges a direct roll-up (`L = x`) for `C_d - C_r` merges.

Eligibility also reads accuracy at the requested grouping: its data shape and item count are those of `G_r`. For KLL, the measured merge curve is read at the average fan-out `ceil(C_d / C_r) * L/x`. That is an empirical estimate, not a worst-group bound.
This is a known cost-modeling issue caused due to uneven fan-out, logged in #189.

### Future Hydra candidate strategy

The existing candidate map remains the right shape for conventional roll-ups: generate a deployment at every RAQE grouping already present; for a mergeable family let that deployment consider RAQEs at subset groupings when choosing windows and slides; then add one eligible edge `(RAQE, deployment)` for every SLA-valid subset relation. The MILP itself is unchanged: one `z[i,D]` per eligible edge, one `u[D]` per deployment, and the same assignment and activation constraints. Its coefficients use the roll-up costs above.

Hydra should enter through a distinct candidate/read strategy, for example `DirectSubpopulationQuery`, with these properties:

- candidate identity includes the complete label schema `H`, Hydra config, window and slide, plus its group-enumeration strategy;
- it may cover a RAQE only when `G_r` is a supported predicate/grouping of `H`, its exact measurement clears the RAQE SLA, and its merge depth was measured;
- its edge coefficients use the [Hydra cost model](rqe_optimizer_cost_model.md#proposed-hydra-cost-model) plus any tracker costs;
- it is not marked `mergeable_across_groups`, and is never charged or validated as a fine-state roll-up.

This keeps candidate generation, eligibility, cost scoring, enumeration, and the MILP on one shared edge set while making the meaning of an edge explicit. The first Hydra implementation can conservatively support only `G_r = H` (or only explicitly benchmarked prefix groupings), then expand the supported predicate relation as measurements land.

### Decisions needed

1. Is coarse Hydra group enumeration derived from a fine DeltaSet, or do we store keys at every requested grouping?
2. Which predicates are legal: only prefix label groupings, or arbitrary subsets once the IR has a label-to-layout mapping?
3. Do we require an observed worst-parent fan-out curve (issue #189) before admitting Hydra, or accept an average-fan-out planning estimate like #190's KLL model?
4. What interpolation/extrapolation, if any, is permitted between measured Hydra saturation points?
