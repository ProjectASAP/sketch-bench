# Blog Outline: Why Approximate Instead Of Keep Optimizing Exact?

> Status: design outline.
>
> Goal: summarize the benchmark design direction after looking at recent
> benchmark papers and the current AQP docs.

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
- Each scenario family must measure which benefits and costs actually appear.
- The benchmark should be allowed to reject approximation. A result where
  optimized exact execution wins is still a useful result.

## One-Sentence Direction

We want a scenario-family AQP benchmark that measures when approximate
execution is worthwhile against optimized exact alternatives, using native
system interfaces, explicit workload provenance, controlled data-condition
sweeps, exact reference results, and resource accounting that says what is
measured and what is still missing.

## Current AQPBMV2 Positioning

The paragraph above describes the broader AQP benchmark ambition. AQPBMV2 is
now narrower:

```text
AQPBMV2 is a sketch-kernel benchmark, not a full SQL/PromQL/AQP system
benchmark.
```

The V2 story should not start from "we benchmark approximate SQL." It should
start from a more specific gap:

```text
Traditional sketch benchmarks usually evaluate one sketch state over one value
stream. AQP-style workloads often create many sketch states, many answers,
grouping pressure, and partial-state merge pressure.
```

The blog can still use SQL-shaped examples to explain why grouped and
partitioned kernels matter, but it should make clear that AQPBMV2 does not run
SQL or evaluate an optimizer. The benchmark definition should be:

```text
concrete sketch instance
+ benchmark-owned AQP-style kernel
+ controlled data condition
=> answer coverage, failure localization, sensitivity, and cost/fidelity
```

This also sets the honest research bar. AQPBMV2 is only paper-interesting if
the grouped or partitioned kernels reveal conclusions that a single-state
sketch benchmark would miss. If they do not, the toolkit is still useful, but
the VLDB-level contribution is weaker.

## Core Benchmark Unit

A benchmark should be organized as a scenario family. One scenario family
specifies:

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
- `workload model`
  - Where requests and data come from.
  - Examples: product quickstart, real trace, controlled synthetic generator,
    or hybrid generated workload fitted to real statistics.
- `pressure dimensions`
  - Data or workload axes the benchmark is intentionally stressing.
  - Examples: cardinality, group count, skew, temporal drift, query range,
    query concurrency, retention horizon.
- `data condition`
  - One measured point in the family, with source/version, seed, shape, time
    range, replay or generation policy, and pressure-dimension values.
- `exact reference execution`
  - Exact computation used to produce the reference answer for fidelity comparison.
- `exact performance baseline`
  - Optimized exact alternative used for latency and resource comparison.
- `baseline policy`
  - Why the exact performance baseline is strong enough for the claim, and
    which baseline gaps remain.
- `approximate option`
  - Approximate system path, sketch, summary, or configuration being tested.
- `option/config policy`
  - How parameters are chosen and compared.
  - Examples: fixed defaults, grid sweep, best option under requirement,
    calibration data, tuning budget.
- `value hypothesis`
  - Falsifiable reason approximation might help: pressure, expected benefit,
    acceptance rule, and rejection rule.
- `requirements`
  - Error target, latency SLO, memory target, result-shape rule, or other acceptance condition.
- `resource accounting scope`
  - Costs measured, costs estimated, and costs still missing.
- `comparison rules`
  - How to pair runs, align windows, match labels/groups, and compute error.

Output records:

- Scenario-family records:
  - Workload provenance, pressure dimensions, baseline policy, planned and
    completed condition coverage, and option/config policy.
- Performance/cost records:
  - Latency, throughput, CPU time, scan bytes, or other measured cost of running an option.
- Fidelity records:
  - Numeric error and result-shape differences against the exact reference execution.
- Resource accounting records:
  - Memory, storage, ingest/update overhead, added service cost, and explicit missing measurements.
- Outcome records:
  - Run status and execution mode, such as exact, approximate, unsupported, timeout, or error.
  - This is separate from fidelity: fidelity says how wrong the answer was; outcome says what happened during execution.

## Why This Benchmark Is Credible

Do not claim that the benchmark is "realistic" as a slogan. The benchmark is
credible when the workload argument is inspectable:

- Real system boundary:
  - Use the system's native interface.
  - Include planner, serving path, serialization, precompute path, fallback behavior, and deployment shape when they are part of the system being evaluated.
- Workload provenance:
  - Say whether the workload is a product quickstart, real trace, controlled
    generator, or hybrid generator.
  - Record source version, seed, replay policy, and query suite.
- Pressure dimensions:
  - State which data or workload axes the benchmark is meant to stress.
  - Do not imply coverage of axes that were not run.

Data input design should be a first-class benchmark axis:

- Synthetic sweeps:
  - Purpose: controlled breakpoints.
  - Method: vary one or two dimensions at a time.
  - Examples: cardinality, group count, Zipf skew, window length.
- Trace or workload replay:
  - Purpose: realistic correlations and temporal behavior.
  - Requirement: preserve timestamp/window semantics.
  - Metadata: record trace version, source, replay speed, and replay policy.
- Hybrid generated traces:
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

Each scenario family should answer separate questions:

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
- Workload provenance and pressure-dimension coverage.
- Exact-reference versus exact-baseline roles.
- Baseline policy: why Prometheus is the exact comparison for this scenario,
  and what stronger exact deployments are outside the current run.
- Option/config policy: whether approximate parameters are defaults, swept,
  calibrated, or tuned under a declared budget.
- Resource accounting for augmentation/precompute overhead.
- Requirement satisfaction across latency, fidelity, result shape, and resources.
- Output regions showing where exact remains better, where approximation helps, and where approximation fails.

This means AQP benchmark can help ASAPQuery tune and test data inputs more completely:

- It can vary cardinality, skew, pattern mix, query range, concurrency, and retention pressure.
- It can expose when ASAPQuery's approximate path is useful versus when Prometheus exact remains sufficient.
- It can keep these broader input-shape tests outside the product quickstart, while still reusing quickstart as a concrete adapter target.
- It can label the quickstart as a seed condition, so the blog does not
  overclaim from one convenient demo workload.

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

The blog should frame this as a baseline policy, not only a terminology
distinction:

- What exact system is the reference?
- What exact system is the performance baseline?
- Why is that baseline strong enough for this scenario family?
- Which stronger or different exact baselines would change the claim?
- Is the result complete, partial, or blocked by missing resource accounting?

It should also frame approximation configuration as a fairness issue:

- Which sketch or summary parameters were used?
- Were they defaults, tuned, or chosen from a sweep?
- Was the exact baseline given an equivalent tuning opportunity?
- Is the reported approximate option a fixed deployment choice or a best-case
  point selected after seeing the workload?
<!-- 
## Proposed Post Flow

1. Start with the exact-baseline question.
   - Exact execution is the default users understand.
   - Exact systems continue to improve.
   - Approximation must justify its error and any added infrastructure.

2. Define the scenario family.
   - Reuse the scenario-family shape from the AQP problem definition.
   - Add the distinction between exact reference execution and exact performance baseline.
   - Make clear that concrete systems, native interfaces, workload provenance,
     and pressure dimensions are required.

3. Explain realistic data input design.
   - This is a central design point, not a side note or a marketing claim.
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
