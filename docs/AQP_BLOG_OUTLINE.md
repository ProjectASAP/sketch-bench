# Blog Outline: Why Approximate Instead Of Keep Optimizing Exact?

> Status: design outline.
>
> Goal: summarize the benchmark design direction.

## Starting Point

The post should start from one skeptical question:

```text
Why use approximation instead of continuing to optimize exact execution?
```

Approximation is useful only when reduced fidelity buys something concrete. The benchmark should measure that tradeoff instead of assuming it.

What approximation may buy:

- Lower tail query latency.
- Lower query CPU.
- Smaller exact-state growth in replacement-style designs.
- Longer retention through summaries or tiered storage.
- Higher query concurrency under the same SLO.
- Lower effective serving cost for a workload.

What approximation may add:

- Extra ingest or update CPU.
- Extra memory for sketches, summaries, or services.
- Extra storage for precomputed state.
- Extra network or remote-write traffic.
- Freshness lag or maintenance lag.
- Operational complexity.
- Fidelity loss, such as numeric error or missing groups.

Important caveat:

- These are possible value dimensions, not claims.
- Each benchmark instance must measure which benefits and costs actually appear.

## One-Sentence Direction

We want a system-level AQP benchmark that measures when approximate execution is worthwhile against optimized exact alternatives, using native system interfaces, realistic data inputs and dynamics, explicit resource accounting, and exact reference results for fidelity measurement.

## Core Benchmark Unit

A benchmark instance specifies:

- `task`
  - Analytical task or query family.
  - Examples: quantile, count distinct, frequency, top-k, grouped aggregate.
- `benchmark context`
  - System/domain/interface where the task is issued.
  - Examples: PromQL telemetry, SQL analytics, DataFusion plans, sketch API.
- `system under test`
  - Concrete exact or approximate system being exercised.
- `native interface`
  - Interface the system normally exposes.
  - The benchmark should not translate every system into one artificial query language.
- `data condition`
  - Data source, data shape, time range, skew, cardinality, and replay or generation policy.
- `exact reference execution`
  - Exact computation used to produce the reference answer for fidelity comparison.
- `exact performance baseline`
  - Optimized exact alternative used for latency and resource comparison.
- `approximate option`
  - Approximate system path, sketch, summary, or configuration being tested.
- `requirements`
  - Error target, latency SLO, memory target, result-shape rule, or other acceptance condition.
- `resource accounting scope`
  - Costs measured, costs estimated, and costs still missing.
- `comparison rules`
  - How to pair runs, align windows, match labels/groups, and compute error.

Output records:

- Performance/cost records:
  - Latency, throughput, CPU time, scan bytes, or other measured cost of running an option.
- Fidelity records:
  - Numeric error and result-shape differences against the exact reference execution.
- Resource accounting records:
  - Memory, storage, ingest/update overhead, added service cost, and explicit missing measurements.
- Outcome records:
  - Run status and execution mode, such as exact, approximate, unsupported, timeout, or error.
  - This is separate from fidelity: fidelity says how wrong the answer was; outcome says what happened during execution.

## Why This Benchmark Is Realistic

The benchmark is realistic when it preserves both:

- Real system boundary:
  - Use the system's native interface.
  - Include planner, serving path, serialization, precompute path, fallback behavior, and deployment shape when they are part of the system being evaluated.
- Realistic data input pressure:
  - Use data inputs and dynamics that create the kinds of pressure exact systems face in real deployments.

Data input design should be a first-class benchmark axis:

- Synthetic sweeps:
  - Purpose: controlled breakpoints.
  - Method: vary one or two dimensions at a time.
  - Examples: cardinality, group count, Zipf skew, window length.
- Trace or workload replay (mental model: CAIDA-style trace/replay corpus):
  - Purpose: realistic correlations and temporal behavior.
  - Requirement: preserve timestamp/window semantics.
  - Metadata: record trace version, source, replay speed, and replay policy.
- Hybrid generated traces (mental model: synthetic CAIDA-like inputs, combining the first two):
  - Purpose: realistic-enough inputs when real traces are unavailable or incomplete.
  - Requirement: encode phenomena such as bursts, drift, skew, and rare groups, not only random rows.

Data and workload pressures to include, not exhaustive:

- Cardinality.
- Group count and group-size skew.
- Heavy tails and rare keys.
- Temporal drift.
- Bursts.
- Query window or range length.
- Dashboard-style query concurrency.
- Update/query ratio.
- Retention horizon.
- Late, missing, or out-of-order data when relevant.

The point is to produce regions like:

- Exact wins below this cardinality.
- Approximation wins under this latency SLO and error target.
- Approximation is fast but misses rare groups.
- Approximation is accurate but too expensive to maintain.
- Precompute helps high-query-rate dashboards but not low-query-rate workloads.

These are target analysis shapes, not current claims.

## Resource And Value Model

Do not describe approximation value as one arithmetic score. The units differ: latency, CPU, memory, storage, network, freshness, and fidelity are not directly additive.

Each benchmark instance should answer separate questions:

- Benefit:
  - What improved relative to the exact performance baseline?
  - Examples: p95 latency, query CPU, memory growth, retention, concurrency.
- Added cost:
  - What extra resource did the approximate option require?
  - Examples: ingest CPU, sketch memory, summary storage, extra service memory.
- Fidelity loss:
  - What error or result-shape difference appeared relative to the exact reference execution?
  - Examples: relative error, rank error, missing groups, extra label sets.
- Requirement satisfaction:
  - Did the approximate option meet the declared error, latency, memory, or result-shape requirement?
- Deployment interpretation:
  - Is the option a replacement, augmentation layer, precompute path, tiered retention design, or direct sketch primitive?
  - The resource model depends on this deployment pattern.

This makes results interpretable without pretending all costs can be reduced to one scalar.

## Relationship To ASAPQuery Quickstart

ASAPQuery quickstart is a concrete system entry point, not the whole AQP benchmark.

What quickstart provides:

- A runnable ASAPQuery deployment.
- A native PromQL-facing interface.
- Prometheus as the exact path for paired comparison.
- A starting workload and fake-exporter data source.

What the AQP benchmark layer adds:

- More data input variation.
- Explicit data-condition identifiers and replay/generation policy.
- Exact-reference versus exact-baseline roles.
- Resource accounting for augmentation/precompute overhead.
- Requirement satisfaction across latency, fidelity, result shape, and resources.
- Output regions showing where exact remains better, where approximation helps, and where approximation fails.

This means AQP benchmark can help ASAPQuery tune and test data inputs more completely:

- It can vary cardinality, skew, pattern mix, query range, concurrency, and retention pressure.
- It can expose when ASAPQuery's approximate path is useful versus when Prometheus exact remains sufficient.
- It can keep these broader input-shape tests outside the product quickstart, while still reusing quickstart as a concrete adapter target.

## Exact Reference Versus Exact Performance Baseline

These are two roles, not necessarily two different systems.

- `exact reference execution`
  - Role: answer correctness.
  - Question: what is the exact result?
  - Used for: fidelity comparison.
  - Can be slow if needed, as long as it is semantically precise.

- `exact performance baseline`
  - Role: exact alternative.
  - Question: what would a reasonable exact system cost?
  - Used for: latency, CPU, memory, storage, and SLO comparison.
  - Should be a fair optimized exact path, not a toy baseline.

They often can be the same:

- Sketch primitive:
  - Reference: exact `HashMap`, `HashSet`, or sorted vector.
  - Performance baseline: the same exact structure measured as an option.
- PromQL:
  - Reference: Prometheus at the paired timestamp.
  - Performance baseline: Prometheus serving the same workload.

They may need to differ:

- A slow offline exact computation may be best for reference answers, while an indexed/serving exact system is the fair performance baseline.
- An exact serving system may timeout for a hard condition; that timeout is a baseline result, but the benchmark may still need an offline exact reference to measure approximation error.
- A benchmark script may record enough data for latency but not enough complete result values for fidelity.
<!-- 
## Proposed Post Flow

1. Start with the exact-baseline question.
   - Exact execution is the default users understand.
   - Exact systems continue to improve.
   - Approximation must justify its error and any added infrastructure.

2. Define the benchmark instance.
   - Reuse the high-level AQP problem definition.
   - Add the distinction between exact reference execution and exact performance baseline.
   - Make clear that concrete systems and native interfaces are required.

3. Explain realistic data input design.
   - This is a central design point, not a side note.
   - Separate synthetic sweeps, trace replay, and hybrid generated traces.
   - Explain which exact-system pressures each data input is meant to create.

4. Explain resource accounting.
   - Measure what approximation buys.
   - Measure what approximation adds.
   - Measure fidelity loss separately.
   - Interpret the result according to the deployment pattern.

5. Describe the desired benchmark outputs.
   - Cost/fidelity/resource/outcome records.
   - Break-even regions.
   - Requirement satisfaction regions.
   - Cases where exact remains better.
   - Cases where approximation is useful only under specific requirements. -->
