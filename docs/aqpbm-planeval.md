# `aqpbm-planeval` design

`aqpbm-planeval` is sketch-bench's end-to-end, system-level evaluation. It
answers one question for a user who has PromQL queries: how does
ASAPQuery-backend, which runs ASAPPlanner's plan, compare with Prometheus on
the same data, in accuracy, latency and throughput?

How to run it, what is pinned, what is measured and the latest results are
in [`aqpbm-planeval/README.md`](../aqpbm-planeval/README.md).

## Components

- **Systems under test.** ASAPQuery-backend (`control_plane` plans the
  workload with ASAPPlanner and writes `plan.json`; `data_plane` executes it
  over Remote Write data) and Prometheus. Both are built or downloaded at
  pinned versions.
- **Data.** Values from `aqpbm-datagen`, laid onto series and timestamps by
  the harness and sent to both systems over Remote Write.
- **Ground truth.** The harness computes exact answers from the generated
  samples, and also checks Prometheus against them, so either can serve as
  the reference.
- **Measurement.** Query latency and throughput are timed by the client;
  ingest throughput, memory and CPU are read from outside each process with
  `aqpbm-core`'s timer and `/proc`.
- **Arms.** One per deployment configuration the control plane plans with:
  the `DeploymentCapabilities` (for example whether raw data is kept) and
  whether accuracy guarantees are required.

## Next

- More queries in one workload, each with its own repeat interval, planned
  jointly by the backend after each is checked alone.
- Data with a label hierarchy (region, service, job) and counters, so
  grouped and `rate` queries have meaning.
- Accuracy measured against both references: Prometheus and the harness's
  own exact answer.
- A report per query that a user can read without the raw tables.

The earlier in-process design of this crate, which interpreted ASAPPlanner's
pre- and post-ASAP IR with sketch-bench's own evaluators, was removed when
the end-to-end harness moved here. It is in the git history before this
change.
