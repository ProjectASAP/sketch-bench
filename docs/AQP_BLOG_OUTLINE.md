# Blog Outline: AQPBM

> Status: currently focused on AQPBMV2.
>
> Goal: explain the problem scope of AQPBM and the contribution of the current AQPBMV2 design.

## Query to begin with: SELECT approx_count_distinct(user_id) FROM table;

- Example query:
  - `SELECT approx_count_distinct(user_id) FROM table;`

- Why this query is useful as motivation:
  - It is a common approximate aggregate.
  - The exact table still exists.
  - The system replaces one exact aggregate with an approximate function.
  - Users trade some fidelity for lower latency, memory, or CPU.

- Why AQPBMV2 should not benchmark this SQL query directly:
  - The benchmark target would become the whole SQL system.
  - Different databases expose different functions and execution plans.
  - Parser, optimizer, storage, and execution engine behavior would be mixed in.
  - That belongs to a later system benchmark, not AQPBMV2.

## Raw Sketch BM

- AQPBMV1 already covers raw sketch benchmarking.
  - The benchmark target is a sketch primitive.
  - Example: HLL as one state over one stream.
  - Inputs can be controlled by distribution and cardinality.
  - Metrics include accuracy, throughput, and memory.

- The limitation:
  - Users do not care about an isolated HLL state by itself.
  - Users care about whether a task can be answered well enough.
  - Users also care about what resources are saved for that task.

- The gap:
  - A good sketch primitive is useful evidence.
  - It is not the same as a good approximate function implementation.
  - A raw sketch benchmark is where the evaluation starts.
  - It is not where the evaluation should end.

## middle layer standards

- Standard for the AQPBMV2 middle layer:
  - It must be executable code.
  - If it cannot be implemented as code, it is only a protocol or config design.
  - It should not be only a raw sketch interface.
  - It should not be a database-specific SQL function.

- Current middle-layer interface:

```rust
trait ApproxFunction {
    type Input;
    type State;
    type Output;
    type Query;

    fn create(&self) -> Self::State;
    fn update(&self, state: &mut Self::State, input: Self::Input);
    fn merge(&self, left: &mut Self::State, right: Self::State);
    fn finalize(&self, state: &mut Self::State, query: Self::Query) -> Self::Output;
}
```

- Concrete candidates can be implemented as code:

```text
ExactCountDistinct<HashSet>
HllCountDistinct<HllImpl>

ExactQuantile<Vec>
KllQuantile<KllImpl>
TDigestQuantile<TDigestImpl>

ExactTopK<HashMap>
SpaceSavingTopK<SpaceSavingImpl>
```

- Middle-layer shape:

```text
user-level functionality
  -> executable approximate function candidate
  -> sketch or exact state implementation
```

- What AQPBMV2 does not claim:
  - It does not benchmark Spark's function directly.
  - It does not benchmark Trino's function directly.
  - Those systems show that the functionality class is real.

- What AQPBMV2 should benchmark:
  - Executable function candidates.
  - Controlled benchmark-owned kernels.
  - Data shapes that expose behavior missed by raw sketch benchmarks.

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
- The benchmark must measure which benefits and costs actually appear.
- The benchmark should be allowed to reject approximation. A result where
  optimized exact execution wins is still a useful result.

## Current AQPBMV2 Positioning

```text
AQPBMV2 is an executable approximate-function benchmark, not a full
SQL/PromQL/AQP system benchmark and not a raw sketch benchmark.
```

The V2 story should not start from "we benchmark approximate SQL." It should
start from a more specific gap:

```text
Traditional sketch benchmarks usually evaluate one sketch state over one value
stream. AQP-style workloads often create many sketch states, many answers,
grouping pressure, and partial-state merge pressure.
```

The blog can still use SQL-shaped examples to explain why approximate
functions are real user-facing functionality, but it should make clear that
AQPBMV2 does not run SQL or evaluate an optimizer. The benchmark definition
should be:

```text
executable approximate function candidate
+ benchmark-owned AQP-style function kernel
+ controlled data condition
=> answer coverage, failure localization, sensitivity, and cost/fidelity
```

This also sets the honest research bar. AQPBMV2 is only paper-interesting if
the grouped or partitioned kernels reveal conclusions that a single-state
sketch benchmark would miss. If they do not, the toolkit is still useful, but
the VLDB-level contribution is weaker.

The practical output should help users choose a sketch candidate. That choice
depends on both measured behavior and adoption metadata:

- measured primitive behavior: update/query/merge cost, memory, and raw
  fidelity;
- measured AQP-kernel behavior: answer coverage, failure localization, and
  sensitivity to grouping and merge shape;
- declared adoption metadata: language, implemented sketch families, API
  shape, serialization, dependencies, license, documentation, and maintenance.

The blog should keep these categories separate. API ease and documentation are
real adoption criteria, but they are not measured by the sketch kernel.

## Previous AQPBMV1

- AQPBMV1 is the existing raw-sketch benchmark layer.
  - Target: sketch primitives directly.
  - Input control: data shape and distribution.
  - Metrics: throughput, accuracy, and memory usage.
  - Role: provide primitive-level evidence for AQPBMV2.

## Future AQPBMV3

## Future AQPBMV4
