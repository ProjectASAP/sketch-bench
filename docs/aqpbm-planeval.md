# `aqpbm-planeval` design

`aqpbm-planeval` is sketch-bench's end-to-end, system-level evaluation. It
answers one question for a user who has PromQL queries: which of them does
ASAPQuery-backend, which runs ASAPPlanner's plan, support, and how does it
compare with Prometheus on the same data, query by query, in accuracy,
latency and throughput?

How to run it, what is pinned, what is measured and the latest results are
in [`aqpbm-planeval/README.md`](../aqpbm-planeval/README.md).

## Components

- **Systems under test.** ASAPQuery-backend (`control_plane` plans the
  workload with ASAPPlanner and writes `plan.json`; `data_plane` executes it
  over Remote Write data) and Prometheus. Both are built or downloaded at
  pinned versions.
- **Workload.** A Planner `PlanningWorkload` of repeating queries, each
  with its own interval; by default ten templates over a gauge and a
  counter (`workloads/templates.json`).
- **Data.** Values from `aqpbm-datagen`, laid by the harness onto series on
  a label hierarchy (by default `region:2,service:4,instance:2`) and onto
  timestamps, as a gauge and a counter that only grows, each service scaled
  by its own factor (`scale_by`), and sent to both systems over Remote
  Write.
- **Probe and joint plans.** Per arm, every query is planned alone first,
  which tells whether the backend supports it; the supported ones are then
  planned jointly in batches, each batch run on a fresh pair of systems.
- **Reference.** Prometheus's answer, series by series, matched by label
  set: PromQL's semantics are Prometheus's, and it answers from raw samples.
  The harness does not evaluate PromQL. For a quantile, whose requirement
  is a rank error, it also asks Prometheus for the raw samples the quantile
  is taken over and measures ASAP's rank in them. Each query is judged
  against its own requirement metric.
- **Measurement.** Query latency and throughput are timed by the client;
  ingest throughput, memory and CPU are read from outside each process with
  `aqpbm-core`'s timer and `/proc`.
- **Arms.** One per deployment configuration the control plane plans with:
  the `DeploymentCapabilities` (for example whether raw data is kept) and
  whether accuracy guarantees are required.

## Next

- A report per query that a user can read without the raw tables.

The earlier in-process design of this crate, which interpreted ASAPPlanner's
pre- and post-ASAP IR with sketch-bench's own evaluators, was removed when
the end-to-end harness moved here. It is in the git history before this
change.
