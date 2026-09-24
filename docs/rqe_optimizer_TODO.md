# rqe-optimizer TODO

The v1 problem statement lives in `rqe_sketch_deployment_v1.md`. This file
tracks work that is intentionally outside the current implementation.

## Implement the v1 sliding-instance model

The current crate still implements the earlier tumbling-only model. Replace it
with the v1 deployment `(capability, configuration, labels, x, y)` and:

- generate candidates per RQE for every legal `(x, y)` divisor pair;
- make capability, labels, window alignment, and measured accuracy hard
  eligibility constraints;
- allow one selected deployment to serve multiple eligible RQEs;
- use the analytical model for peak query working memory, ingest CPU, merge
  CPU, query CPU, total CPU, and per-RQE latency; and
- use `(peak_query_memory, TCO_cpu, {latency_i})` for the Pareto frontier.

Keep latency index-aligned with the input RQEs rather than keyed by the display
ID. Duplicate IDs must not collapse an objective dimension.

## Validate accuracy after merging

v1 uses each configuration's measured single-instance accuracy. A query may
merge `S / x` instances, so this is not evidence that the final result meets
the RQE's tolerance.

Prefer measuring accuracy at representative merge depths. An analytic
composition rule is acceptable only after validation against measured data.

## Future model extensions

- Query-result sharing when RQE semantics and execution timing make reuse safe.
- Per-RQE latency SLAs as hard eligibility constraints.
- Merge buffers, retained-storage capacity, and concurrent-query memory.
- Additional capabilities such as keyed L1 norm, L2 norm, and entropy.
- A policy for selecting one mapping from the Pareto frontier.
- An ILP solver when exhaustive enumeration no longer fits the workload size.

## External measurement pipeline

`scripts/export_atomic_costs.sh` may have a pass-order problem: its record
merge keeps first values, while the script runs accuracy before cost. That can
misalign timing and throughput sample counts. This pipeline is outside the
optimizer's scope and needs an owner decision before changing it.
