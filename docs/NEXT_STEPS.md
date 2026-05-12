# Next Steps — From Sketch Bench to Approximate Query Benchmark

> Status: design draft. Branch `docs/aqp-next-steps`.
> Last reconciled to `main` on 2026-06-05 (see the freshness notes inline).
> Supersedes the open-ended scope in [`TODO.md`](../TODO.md) "Deferred" sections.
> Companion to [`DESIGN.md`](DESIGN.md) (which stays the source of truth for the
> existing crate layout and the `Sketch` / `Probe` contracts).

---

## 1. Thesis

Today this repo is a **per-sketch microbenchmark**: one sketch, one workload,
one statistic. That is necessary but not sufficient. Real systems
(asap-fusion, ASAPQuery, OLAP engines, streaming monitors) don't issue a
single `COUNT DISTINCT` against a single HLL — they run **mixed
approximate-query workloads** with multiple statistics per query, group-bys,
merges across shards, and continuous re-querying against drifting data.

The next step is to turn this repo into an **Approximate Query Processing
(AQP) benchmark**, with the existing sketch microbench as a strictly scoped
Phase 1. Phase 2's deliverable is a workload-driven, plan-level benchmark that
is **complete** (covers the full AQP design space we care about) and
**realistic** (workloads, error budgets, and merge patterns derived from
production traces), with a defensible **research contribution** beyond
"another sketch shootout".

---

## 2. Phase boundary

### Phase 1 — Sketch microbench  *(already landed; freeze the scope)*

Locked deliverables — no new scope after this doc lands:

- 38 sketch impls across 8 families (`cms`, `countsketch`, `dd`, `elastic`,
  `hll`, `kll`, `nitro`, `univmon`) × default config grids, v1 JSONL,
  accuracy + throughput + CPU + RSS + latency.
- `sketchlib bench --sketch X` single-statistic runs.
- Exact baselines per statistic (`cardinality`, `frequency`, `quantile`).
- `sketch-runtime` sampler + `stdout` / `file` / `noop` / `grpc` / `fanout`
  exporters.

Explicitly **out** of Phase 1 from here forward:
- Prometheus exporter (not built; defer to Phase 3 if a consumer ever needs
  it). *Note: the gRPC exporter — originally cut here — has since landed on
  `main` (`sketch-runtime/src/exporter/grpc.rs` + `fanout.rs` + integration
  test); treat it as a shipped Phase-1 extra, not Phase-2 scope.*
- `sketch-profile` hardware counters (defer indefinitely; cite `perf` /
  `cachegrind` directly in the paper instead).
- C++ harness migration (the Rust matrix already covers the accuracy story).
- Legacy `rust/src/bin/*` retirement (cosmetic; do opportunistically).

Rationale: every hour spent on Phase-1 polish is an hour not spent on the
Phase-2 contribution that actually distinguishes this work.

### Phase 2 — Approximate Query Benchmark  *(this doc)*

A benchmark that takes **query plans**, not sketches, as its unit of work.
See §4 for the scope and §6 for the research claims.

### Phase 3 — Productionization  *(out of scope for the paper)*

Live integration with asap-fusion / ASAPQuery / ASAPController. Tracked
separately; not on the critical path for the benchmark contribution.

---

## 3. What we are cutting

To make room for Phase 2 without growing the repo unboundedly:

| Cut | Where it lives now | Why |
|---|---|---|
| `sketch-profile` crate stub | `docs/MERGE_PLAN.md` Phase 5, `TODO.md` Deferred | Linux+privilege-only; replaceable by direct `perf`/`cachegrind` invocations in the paper. |
| Prometheus exporter | `TODO.md` runtime section | No consumer ships in the paper timeline. (gRPC exporter is **not** cut — it shipped; see §2.) |
| C++ binary v1 JSONL migration | `docs/MERGE_PLAN.md` Phase 8 | Mechanical; not a research contribution. |
| YAML sweep-matrix loader, criterion no-op proof, dispatch warning | `TODO.md` "Minor polish" | All deferrable; nothing in §6 depends on them. |
| ~~`simplified_cumulative_result.txt`, `cumulative_result.txt`, `dir_structures.md`, `design_conversion_conclusion.txt` at repo root~~ — **done** | working files | Already removed from the tree; `docs/archive/` now exists. |

After this trim the repo's "active" surface is: `sketch-core`,
`sketch-bench`, `sketch-cli`, `sketch-runtime` (sampler + `file`/`grpc`
exporters), plus the new `aqp-bench` crate from §5.

---

## 4. Phase 2 scope — what an "approximate query" means here

A **query** is a tuple `(plan, error budget, latency budget)`:

- **Plan**: a small DAG of operators over a stream/relation. Operators we
  cover: `count_distinct`, `frequency`, `top_k`, `quantile`,
  `cardinality_group_by`, `heavy_hitter_group_by`, `join_size_estimate`,
  `set_membership` (Bloom), and `merge` across shards.
- **Error budget**: per-operator (e.g. ε for relative cardinality error, rank
  error for quantiles) plus a **plan-level composed budget** (the novel
  part — see §6.2).
- **Latency budget**: a deadline. The system must answer within it; if it
  can't, it must report which operator blew through, not silently degrade.

A **system under test** is any binding that answers a plan. Initial bindings:

1. **Sketch-AQP** — compose Phase-1 sketches per operator.
2. **Sampling-AQP** — uniform / stratified / reservoir sampling over the same
   inputs, answering operators by scaled aggregation. (BlinkDB-style.)
3. **Exact** — the existing exact baselines, run on the full input.
4. **Hybrid** — sketches for high-cardinality columns, sampling for the rest;
   policy chosen per plan.

This binding set is the minimum to make claims about *when* sketches win
over sampling and vice versa — the question existing sketch papers don't
answer because they only compare sketch-to-sketch.

---

## 5. Crate additions

```
sketch-bench/
├── aqp-core/           # plan AST, error budget algebra, workload trace format
├── aqp-bench/          # plan executor + 4 system bindings (sketch/sample/exact/hybrid)
├── aqp-workloads/      # trace ingest + synthetic generators (see §7)
└── sketch-cli/         # gains `aqp` subcommand alongside `bench`
```

`aqp-core` depends on `sketch-core`. `aqp-bench` depends on `sketch-bench`
for the sketch binding. No reverse deps — Phase 1 stays self-contained.

CLI surface:

```
sketchlib aqp run --workload <name> --system sketch|sample|exact|hybrid \
                  --budget 'eps=0.01 deadline=50ms' --report aqp.jsonl
sketchlib aqp pareto --workload <name> --systems all --report pareto.jsonl
```

The `pareto` subcommand sweeps each system's tunables and emits the
accuracy/cost frontier — this is what §6.3 plots.

---

## 6. Research contributions (the part that has to be defensible)

### 6.1 An **operator-level** AQP benchmark, not a sketch-level one

Existing benchmarks either (a) measure a single sketch on synthetic streams
(this repo's Phase 1; most sketch papers) or (b) measure a whole AQP system
end-to-end on TPC-H-with-sampling (BlinkDB, VerdictDB). Neither isolates
**which operator** in a plan dominates the error or the latency. Phase 2
does, by carrying per-operator error/latency attribution through the
executor.

### 6.2 **Composed error budgets** across operator DAGs

The non-obvious claim: per-operator ε bounds **do not compose** by simple
addition for plans that share inputs or feed one operator's output into
another's predicate. We define a composition algebra (worst-case +
empirically calibrated) and the benchmark **measures the gap between the
two** across our workload set. This gap, if non-trivial, is the paper's
core empirical contribution and tells practitioners how much slack to leave
when sizing sketches in production plans.

### 6.3 **Pareto frontier across system bindings**, not "which sketch is
best"

For each workload, plot (accuracy, p99 latency, peak memory) for every
system × tunable. The interesting axes are:
- skew sensitivity (where do sampling-based systems collapse?),
- merge cost (sketches' selling point — is it real at our scale?),
- update/query ratio (sampling wins on read-heavy; verify the crossover).

Existing literature asserts these tradeoffs; nobody has measured them on
the **same harness with the same workloads**. That is the deliverable.

### 6.4 **Drift-aware accuracy**

Workloads in §7 include non-stationary segments (concept drift,
distribution shift). The benchmark reports accuracy *as a function of time
since last reset/merge*, not a single aggregate number. This is the
metric production users actually care about and the one current sketch
papers do not report.

### 6.5 What we are explicitly **not** claiming

- Not proposing new sketches.
- Not proposing a new AQP system.
- Not claiming the benchmark is "the" benchmark — claiming it is the first
  open one that answers §6.2–§6.4 on a shared harness.

---

## 7. Workloads — the realism requirement

A benchmark is only as defensible as its workloads. Three tiers, each
required for the paper:

1. **Synthetic, parameterized** — Zipf, uniform, churn, drift. Already
   present in `sketch-core/src/workload.rs`. Extend with skew schedules and
   merge-shard counts.
2. **Replayed traces** — at least two of: a CDN access log, a network
   telemetry trace (CAIDA), a database query log, a Kafka topic capture
   from asap-fusion staging. Released as a normalized `.aqp-trace` format
   (schema in `aqp-core`).
3. **Plan corpus** — ~30 hand-curated plans derived from real ASAPQuery
   uses + published AQP papers' example queries. Versioned, with expected
   error/latency bands.

Tier 2 is the bottleneck. We need at least one trace we can redistribute
(license-clean). The CAIDA anonymized traces qualify; confirm before
committing.

---

## 8. Concrete next 6 commits (to validate the plan, not to ship Phase 2)

1. This doc (`docs/NEXT_STEPS.md`). *(done)*
2. Archive cuts from §3 under `docs/archive/` *(done — stray root files
   removed, `docs/archive/` created)*; update `README.md` to point at this
   doc *(still pending — `README.md` has no AQP reference yet)*.
3. `aqp-core` crate stub: plan AST + budget types, no executor yet.
   *(not started — no `aqp-*` crate in the workspace.)*
4. One end-to-end plan (`count_distinct ∘ group_by`) running on the
   sketch binding, emitting per-operator attribution into a v1-compatible
   JSONL extension.
5. Same plan on the sampling binding. Confirm the harness can A/B them.
6. A first §6.2 measurement on synthetic Zipf — published as a notebook in
   `visualization/`, not as a finished result, to sanity-check that the
   composition gap is measurable.

If step 6 shows the composition gap is below measurement noise on every
plan, §6.2 is not a paper and we revisit the contribution claim before
investing further. That is the gate.

---

## 9. Open questions

- **Trace licensing** — which production trace can we ship? Without one,
  Tier 2 collapses to CAIDA-only.
- **Sampling binding scope** — do we implement stratified sampling
  ourselves or wrap an existing library? Implementation cost vs. baseline
  fidelity.
- **Streaming vs batch** — Phase 1 assumes batch insert-then-query. AQP
  needs interleaved. The `Probe` decorator already supports it; the
  executor needs to schedule it. Decide at step 4.
- **Hardware** — Phase 1 numbers are single-machine. Merge claims (§6.3)
  need multi-shard. Decide whether to simulate shards in-process or run
  distributed. In-process is cheaper and likely enough for the paper.
