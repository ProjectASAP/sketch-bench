# RQE-to-sketch deployment mapping: v1

## Purpose

Given a workload of repeating query expressions (RQEs), find sketch deployments
that can serve it and report the trade-offs among query working memory, CPU,
and per-RQE latency.

v1 is static and small-scale. It enumerates feasible mappings rather than
calling an ILP solver, but states the problem in solver-friendly terms so that
enumeration can be replaced later.

## Inputs

Let `R = {r_1, ..., r_n}` be the RQE workload. Each RQE `r_i` provides:

| Field | Meaning |
|---|---|
| `cap_i` | Required capability: `freq`, `quantile`, `cardinality`, or `topk`. |
| `S_i` | Query-window size. This is the current `lookback` field. |
| `T_i` | Query slide: how often the RQE runs. This is the current `interval` field. |
| `labels_i` | Group-by label set. |
| `accuracy_metric_i` | Accuracy measurement to check. |
| `tol_i` | Accuracy threshold. |
| `direction_i` | Whether lower or higher values are better. |

For each label set, the caller provides:

- `card(labels)`: number of distinct label-value groups.
- `lambda(labels)`: aggregate item arrival rate across those groups, in
  items/second.

These are inputs to the optimizer; estimating them is outside v1.

Sketch Bench supplies empirical measurements for each sketch configuration:

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
have produced other, overlapping instances between these starts; this query
does not merge them.

`S % T == 0` is not required. For example, a 10-minute window running every
3 minutes can use `x = 2 minutes` and `y = 1 minute`; each query merges five
non-overlapping two-minute instances.

v1 assumes all RQEs share a common time origin. Query phase offsets are not
modeled. It also enumerates every valid divisor pair; there are no heuristic
candidate bounds yet.

## Candidate deployments

A candidate deployment is:

```text
D = (capability, configuration, labels, x, y)
```

Candidates may be generated from individual RQEs, then deduplicated. This is
only a way to construct the candidate set: when solving a workload, one
selected deployment may serve multiple compatible RQEs.

For each RQE `r_i`, generate candidates as follows:

```text
for each measured configuration that serves cap_i:
    for each x that divides S_i:
        for each y that divides gcd(x, T_i):
            add (cap_i, configuration, labels_i, x, y)
```

An RQE `r_i` is eligible for a candidate `D` when:

1. `cap_i = D.capability` and `labels_i = D.labels`.
2. `D.x % D.y = 0`, `S_i % D.x = 0`, and `T_i % D.y = 0`.
3. `D.configuration` contains a measured value for `accuracy_metric_i` that clears
   `tol_i` in `direction_i`.

For lower-is-better metrics, passing means `measured <= tol_i`. For
higher-is-better metrics, passing means `measured >= tol_i`. Direction is
explicit on the RQE; it is never inferred from a metric name. A missing metric
does not pass.

Historical instances must be retained long enough to answer the RQEs assigned
to a deployment. Retention is an execution/storage detail in v1, not a
candidate parameter or a scored objective. After selecting a mapping, retain
the history needed by the largest assigned query window.

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

## Analytical cost model

The analytical model combines the empirical per-operation Sketch Bench
measurements with workload properties such as group cardinality, arrival rate,
window size, and query frequency.

For deployment `D`, let `a_D = D.x / D.y`. Each incoming item is inserted into
`a_D` concurrently active sketch instances per group.

### Query working memory

v1 models query working memory, not retained-storage footprint. For RQE `i`:

```text
query_memory_i = card(labels_i) × mem_bytes(D(i).configuration)
```

This assumes one instance per group is loaded and groups are processed
concurrently. It does not model merge buffers or concurrent queries.

The mapping-level memory objective is the worst individual query:

```text
peak_query_memory = max_i query_memory_i
```

### CPU

Ingest CPU is paid once for every active deployment:

```text
ingest_cpu = Σ_{D: u_D=1}
             lambda(D.labels) × a_D × insert_cpu_secs(D.configuration)
```

For RQE `i`, let `n_i = S_i / D(i).x`. One query performs one final estimate
and `n_i - 1` pairwise merges for each group:

```text
query_cpu_i = card(labels_i) × query_cpu_secs(D(i).configuration)
merge_cpu_i = card(labels_i) × (n_i - 1) × merge_cpu_secs(D(i).configuration)
latency_i   = query_cpu_i + merge_cpu_i
```

`latency_i` is estimated serial CPU time for one query, not a wall-clock SLA.
It assumes no parallel execution across groups and no cheaper k-way merge.

The repeating query load is:

```text
query_cpu = Σ_i query_cpu_i / T_i
merge_cpu = Σ_i merge_cpu_i / T_i
```

Total steady-state CPU is:

```text
TCO_cpu = ingest_cpu + merge_cpu + query_cpu
```

## Procedure

1. Generate candidate deployments for each input RQE, then deduplicate them.
2. Build the eligible candidate–RQE pairs. Reject pairs that fail capability,
   labels, window alignment, or empirical accuracy.
3. Enumerate feasible workload mappings. Every RQE must select one eligible
   candidate; a single selected candidate may serve multiple RQEs.
4. Score every complete mapping with the analytical cost model.
5. Compute and report the Pareto frontier.

The Pareto vector is:

```text
(peak_query_memory, TCO_cpu, {latency_i})
```

The report also includes `ingest_cpu`, `merge_cpu`, and `query_cpu` as the
breakdown of `TCO_cpu`, together with the selected deployment mapping.

## v1 scope and TODOs

- **Accuracy after merging:** v1 uses the measured, single-instance accuracy
  of a configuration. Measure or model merged accuracy before relying on a
  selected mapping as truly accuracy-feasible.
- **Query-result sharing:** v1 charges every RQE its own query and merge CPU.
  Revisit when RQE semantics and execution timing identify safe reuse cases.
- **Latency SLAs:** v1 reports per-RQE latency but does not reject a mapping
  for exceeding a target. Add optional per-RQE maximum latency as a hard
  constraint when workloads supply targets.
- **Memory model:** merge buffers, retained-storage capacity, and concurrent
  queries are not modeled.
- **Static planning:** no RQE churn, replanning, or migration cost.
- **Rollups:** v1 does not precompute merged rollups. Queries merge their
  selected base instances when they run.
- **Selection policy:** the optimizer reports the Pareto frontier; selecting a
  single point through weights or budgets is deferred.

## Implementation status

`rqe-optimizer/` currently implements the earlier tumbling-only model with a
single base-slide parameter. It does not yet implement the `(x, y)` sliding
instance model in this document.

To implement this v1 design, update the deployment type, candidate generation,
eligibility checks, analytical objective calculations, examples, and tests.
The existing separation between candidate generation, enumeration, objectives,
and Pareto filtering remains the intended implementation boundary.

Exporter bugs, benchmark-dataset choices, and historic run counts belong in
`rqe_optimizer_TODO.md` or issue tracking, not in this problem statement.
