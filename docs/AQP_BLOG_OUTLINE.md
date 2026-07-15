# Blog Outline: AQPBM

> Status: example queries that can be used by AQPBMV2:
> why care about these queries?
> what middle layer can be shared among these queries?
>
> Goal: explain the problem scope of AQPBM and the contribution of the current AQPBMV2 design.

## Quick Access

Example queries (each follows the 5-part template: SQL, intent, physical plan, approximate plan, what the benchmark measures):

- [Example 1: distinct users per (region, OS)](#example-1-distinct-users-per-region-os)
- [Example 2: top-5 regions by distinct users, over the middle 50% of page loads](#example-2-top-5-regions-by-distinct-users-over-the-middle-50-of-page-loads)
- [Example 3: top-5 most active users per region — and the same query on `URL`](#example-3-top-5-most-active-users-per-region--and-the-same-query-on-url)
- [Example 4: how many users are on mobile — the negative control](#example-4-how-many-users-are-on-mobile--the-negative-control)
- [Example 5: which regions have the most mobile users — the control for Example 2](#example-5-which-regions-have-the-most-mobile-users--the-control-for-example-2)
- [Example 6: median event time per site — partitioning decides the answer](#example-6-median-event-time-per-site--partitioning-decides-the-answer)
- [Example 7: p99 latency per site — the benchmark's own metric picks the winner](#example-7-p99-latency-per-site--the-benchmarks-own-metric-picks-the-winner)
- [Example 8: audience overlap — excellent sketches, worthless answer](#example-8-audience-overlap--excellent-sketches-worthless-answer)
- [Example 9: top-10 search phrases by distinct users — the cheap plan is not just approximate, it is wrong](#example-9-top-10-search-phrases-by-distinct-users--the-cheap-plan-is-not-just-approximate-it-is-wrong)
- [Example 10: merge does not compose with join](#example-10-merge-does-not-compose-with-join)

## Abstract

Every major analytical engine ships approximate aggregates, and most are backed by a sketch.
But the available evidence sits at two extremes.
Sketch benchmarks measure one sketch state over one stream.
System benchmarks measure a whole database, with the sketch buried under a parser, an optimizer, and a scheduler.
Neither answers whether a given approximate implementation holds up in the shape a query actually runs it: one state per group, or many partial states merged across a shuffle.

We argue the missing unit of measurement is the *runnable implementation of an approximate functionality* — code that creates a state, updates it, merges it, and finalizes an answer.
Under this definition an exact `HashSet` and a HyperLogLog are both count-distinct implementations, and compare directly.
A sketch becomes an implementation detail rather than the benchmark subject.

AQPBMV2 is a toolkit built on that definition.
It exposes one executable interface, owns the execution modes candidates are driven through, and reports each candidate against an exact baseline.
It covers count distinct, heavy hitters, and quantile, with three sketch-backed candidates each.

What we claim today is integration, not discovery.
AQPBMV2 becomes a research contribution only when its execution modes expose behavior a raw sketch benchmark structurally cannot see.

## Intuition (can be skipped)

### Query to begin with: SELECT approx_count_distinct(user_id) FROM table;

- Example query:
  - `SELECT approx_count_distinct(user_id) FROM table;`

- Reason to pick this query as a motivation:
  - It is a common approximate aggregate (already supported in many places).
  - The exact table still exists.
  - The system replaces one exact aggregate with an approximate function.
  - Users trade some fidelity for lower latency, memory, or CPU.
  - See `Existing Approximate Query Support` at the end for examples.

- Why AQPBMV2 should not benchmark this SQL query directly:
  - In that case, the benchmark target would become the whole SQL system.
  - Different databases expose different functions and execution plans.
  - The question would become:
    - Are we benchmarking Spark?
    - Are we benchmarking Trino?
    - Are we benchmarking ClickHouse?
  - Each system-supported version of this query deserves its own system benchmark.
  - Parser, optimizer, storage, and execution engine behavior would be mixed in.
  - That belongs to a later system benchmark, not AQPBMV2.

### Raw Sketch BM

- AQPBMV1 already covers raw sketch benchmarking.
  - The benchmark target is a sketch primitive.
  - Example: HLL as one state over one stream.
  - Inputs can be controlled by distribution and cardinality.
  - Metrics include accuracy, throughput, and memory.

- The limitation:
  - Users ultimately want good query or task performance.
  - Good component or sketch performance may suggest good query performance.
    - But it is only the starting point.
  - The benchmark needs to preserve the path from sketch behavior to user-visible functionality.

- The gap:
  - A raw sketch benchmark can help users reason about candidate sketches.
  - It cannot by itself answer whether an approximate function is useful for a user-facing task.
  - AQPBMV2 starts from raw sketch evidence.
  - It then evaluates runnable implementations of approximate functionality.

- Mental experiment:
  - Suppose a query is implemented using three HLL states.
  - Each individual HLL state may look bad in isolation.
    - Example: 30% relative error per raw sketch state.
  - The final query answer may still be good.
    - Example: 1% relative error after the full function logic.
  - A user would choose the full approximate function because it gives good query performance.
    - Not because every raw sketch component looks good in isolation.
  - This is why AQPBMV2 should evaluate approximate-function implementations.

## Current: AQPBMV2

- Scope:
  - AQPBMV2 is a runnable approximate-function benchmark toolkit.
  - It is not a full SQL, PromQL, or AQP system benchmark.
  - It is not a raw sketch benchmark.
  - Its benchmark unit is a runnable implementation of a functionality.
    - Example: count distinct implemented by an exact `HashSet`.
    - Example: count distinct implemented by Apache DataSketches HLL.
    - Example: quantile implemented by `asap_sketchlib` KLL.
  - A sketch can be part of the implementation.
  - The sketch API alone is not the benchmark unit.

- Main toolkit contribution:
  - AQPBMV2 should provide a reusable toolkit.
  - The toolkit should let users implement and compare approximate-function implementations.
  - The toolkit should provide execution modes and a report format.
  - The toolkit should make it easy to add:
    - more sketch libraries;
    - more exact baselines;
    - more functionality classes;
    - more sketch compositions;
    - more workload generators;
    - more comparison metrics.
  - An implementation may wrap one sketch.
  - An implementation may also compose multiple sketches.
  - This is about implementing an approximate functionality.
  - It is not yet a claim about supporting complex SQL.

- Current executable interface:

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

## Table Header

The following exampls will rely on the ClickBench table.
There are 2 benefits:

- It has public playground to quickly test for queries.
- It is from real data.

- Tables to look for examples on:
  - The point is the header, not the row count.
  - A concrete header makes it possible to ask: given this table, what example query can I write?
  - Column cardinality is what decides whether the middle layer shows up at all.

  - ClickBench `hits` (real web-log table, heavy-tailed).
  - Cardinalities below are measured on the real 100M-row table, not guessed from the type.
    - Source: ClickHouse public playground, `https://play.clickhouse.com/?user=explorer`.
    - Total rows: 99,997,497.

```sql
WatchID            Int64      -- near-unique per row
EventTime          DateTime
EventDate          Date
CounterID          Int32      -- site id, 6,506 distinct
UserID             Int64      -- 17.6M distinct, heavy-tailed
ClientIP           Int32
RegionID           Int32      -- 9,040 distinct
URL                String     -- high cardinality, heavy-tailed
Referer            String
SearchPhrase       String     -- high cardinality, Zipf, many empty
SearchEngineID     Int16
AdvEngineID        Int16      -- very low cardinality
OS                 Int16      -- 91 distinct
UserAgent          Int16
MobilePhone        Int16      -- low cardinality
MobilePhoneModel   String     -- pairs with MobilePhone as a 2-column key
ResolutionWidth    Int16      -- 2,159 distinct
ResolutionHeight   Int16
Age                Int16      -- 6 distinct (bucketed)
Sex                Int16      -- 3 distinct
Income             Int16      -- 4 distinct: {0, 1, 2, 3} (bucketed level, 0 looks like "unknown")
Interests          Int16      -- 9,834 distinct (looks like a bitmask, not an ordinal)
IsRefresh          Int16      -- 0/1
IsMobile           Int16      -- 0/1
ResponseEndTiming  Int32      -- latency, heavy-tailed, 28,376 distinct
SendTiming         Int32
ConnectTiming      Int32
```



## What the middle layer is: the physical-plan space

Roughly speaking, a query will go through logical plan (optimization) physical plan, and being executed.
Logical plan is just describing what functionality the query will do, but not how the functionality is achieved.
The benchmark is trying to reveal how approximation can help.
Targeting the logical plan is missing detail about how the query is executed.
Targeting the real execution is not feasible at this moment.
Physical plan (and more accurately, possible potential physical plan) is the middle-layer.
It omitts the detail about which database is translating query to such physical plan or not.
Instead, the middle-layer is saying, for a certain functionality (where a query can represent), the possible physical plan will use xxx resources and provide yyy benefits from approximation.

This section is the frame for the ten examples that follow.
Each example is one probe of the space described here.

- Which plan layer is the benchmark about?
  - A query engine lowers SQL through roughly two plan layers:
    - **Logical plan**: relational algebra.
      It says *what*.
      `Scan -> Filter -> Aggregate(COUNT DISTINCT UserID) GROUP BY RegionID`.
      It does not commit to an algorithm, to parallelism, or to how data moves.
    - **Physical plan**: it says *how*.
      `ParallelScan -> PartialAggregate(which state type) -> Exchange(repartition by RegionID) -> FinalAggregate(merge states) -> Sort -> Limit`.
    - "Execution plan" is, in most systems, just another name for the physical plan (the thing `EXPLAIN` prints).
      A few systems (Spark AQE) use it to mean the physical plan plus runtime adaptation.
  - The middle layer is the **physical plan**.
  - Evidence: look at what the ten examples actually vary.
    - Example 1 (one HLL per group vs. a hybrid of exact and HLL by group size), Example 2 (one pass vs. two), Example 6 (range partition vs. hash partition), Example 7 (KLL vs. DDSketch) all keep the **same logical plan** and change only the physical one.
      Example 6 literally varies the `Exchange` operator.
  - The `ApproxFunction` trait above is already a physical operator interface.
    - `create / update / merge / finalize` is the standard shape of a physical aggregate operator: ClickHouse `IAggregateFunction`, Spark `TypedImperativeAggregate`, DataFusion `Accumulator` all have exactly this shape.

- Why approximation breaks the logical/physical layering.
  - Classic optimizers rest on one invariant:
    - **Every physical plan of a given logical plan returns the same answer, and they differ only in cost.** The optimizer's job is "find the cheapest plan whose answer is identical."
    - Hash join vs. merge join: same answer.
      Broadcast vs. shuffle: same answer.
  - Under approximation this invariant is false.
    - HLL vs. exact vs. t-digest return **different answers with different error**.
    - So the optimizer's objective changes from `min(cost)` to a **(cost, error) Pareto frontier**.
      There is no single optimal plan, only a frontier.
    - Which point on the frontier is right depends on how much error the user will accept, and that information is **not in the logical plan**.
  - Example 10 is stronger still.
    - It is a **logical rewrite** (push the aggregation below the join, i.e. eager aggregation) whose **correctness depends on a physical choice** (whether `merge` is idempotent: it is for HLL, it is not for t-digest or KLL).
    - In the classic architecture the legality of a logical rewrite must not depend on the physical implementation.
      Approximation violates exactly that.

- When the benchmark has something to measure, and when it does not.
  - The dividing line is not query complexity.
    It is **precomputability**.
  - If the answer is already materialized as metadata, the benchmark is pointless, because the answer is a lookup, not a computation.
    - `SELECT COUNT(DISTINCT UserID) FROM hits` is syntactically trivial, but it is exactly what an Iceberg puffin file stores as a Theta sketch.
      Nothing to benchmark.
  - If the answer is **not** precomputable, no stored metadata can serve it, so it must be computed at query time from raw data.
    That is the benchmark's territory.
    - `COUNT(DISTINCT UserID) GROUP BY RegionID` is not precomputed anywhere: nobody stores 9,040 per-group HLLs in advance.
  - Note the two cases differ only in grouping granularity, not in the aggregate function.
    - Same `COUNT(DISTINCT)`.
      Global is a lookup; per-region is a live computation.
    - So complexity is the wrong axis.
      Precomputability is the right one.
  - This also sharpens the earlier open question "what should be considered as input".
    - Input is not only the data.
      It includes **what has already been precomputed**.
    - The same query lands on different sides of the line on an Iceberg table that stores a sketch versus a bare Parquet file that does not.

- What the toolkit provides, and what it deliberately does not.
  - It does **not** provide the optimizer and does **not** pick the plan.
  - No approximation-aware optimizer exists yet, so every candidate physical plan has to be preparable and runnable by hand.
  - The deliverable is a **toolkit / harness** that can run and measure *any* physical plan on three axes: cost, error, and soundness.
  - The benchmark draws the (cost, error, soundness) map.
    An optimizer is a downstream consumer that walks it.

- Why this is a contribution and not a stopgap.
  - "Prepare all the physical plans by hand because no optimizer picks them yet" is not a temporary inconvenience.
    It is the reason the benchmark has to exist.
  - A classic cost-based optimizer can exist because decades of selectivity estimation and join-cost modeling gave it a map to walk.
    The approximate side has no such map.
  - So the map has to come first.
    AQPBMV2 is the precondition for a future approximation-aware optimizer, not a placeholder for one.
  - This gives the research bar (later in this document) a concrete target: the map AQPBMV2 produces is the calibration data a future optimizer's cost/error model would be built on.


<!--
Per-example template (each example below follows this 5-part structure):
1. The SQL.
2. What the query does, and who would actually ask it.
3. The physical plan (from ClickHouse EXPLAIN).
4. How a sketch / approximation reaches the same result.
5. What the benchmark can measure, and why this is an AQP BM question.
-->

### Example 1: distinct users per (region, OS)

```sql
SELECT RegionID, OS, COUNT(DISTINCT UserID)
FROM   hits
GROUP BY RegionID, OS;
```

Count the distinct users in each (region, operating system) cell.

- **What it does, and who asks it:**
  - "How many unique users per region, per operating system" is a normal web-analytics question.
    The group key is a 2-column composite, so the input is already multi-dimensional.

- **Physical plan (from ClickHouse `EXPLAIN`):**

```text
Aggregating (Keys: RegionID, OS; Aggregates: uniq(UserID))
 └─ ReadFromMergeTree (default.hits)

Pipeline:
  MergeTreeSelect x192      -- 192 parallel scan streams
   └─ AggregatingTransform x192   -- each stream builds its own per-group states
       └─ Resize 24 -> 24         -- partial states merged down to final
```

  - This is the shape the abstract calls "many partial states merged across a shuffle": each scan stream keeps one aggregate state per group it sees, and those partial states are merged per group at the end.
  - `uniq` here is the aggregate.
    The exact baseline uses an exact set as the state; every candidate swaps in a different state type behind the same plan.

- **How approximation plays here:**
  - The naive approximate plan: one HLL per `(RegionID, OS)` group, updated with every `UserID` in the group, finalized to one cardinality per group.
  - But the group sizes decide whether that is a good idea.
    Measured on the real table:
    - Group count: 63,467.
    - Median rows per group: 9.
      Groups with fewer than 100 rows: 52,709 (83%).
    - Largest group: 9,724,473 rows, 1,429,492 distinct users (`RegionID=229, OS=44`).
    - So a handful of huge groups sit on top of a long tail of tiny ones.
  - For a 9-row group: the exact set is free and always correct.
    An HLL there costs *more* memory than the exact set and is less accurate.
    Approximation is a strict loss.
  - For the 9.7M-row group: the exact set must store 1.43M distinct user ids — tens of MB.
    The HLL is 16KB no matter how many distinct users there are.
    **That memory cap is why the large group wants a sketch**: the sketch's cost does not grow with cardinality, the exact set's does.
  - So the honest plan is a hybrid: exact state for small groups, sketch state for large ones.
    The per-group decision is "which state type", made at runtime from the group's size.
  - Real HLL implementations already do a miniature version of this internally, called **sparse-to-dense promotion**: while a group has few distinct values, the HLL is stored as a short list of the registers that were actually touched (cheap); once it fills up, it is promoted to the full fixed-size dense array.
    That is the same "small = cheap/exact-ish, large = sketch" idea, automated inside one sketch.

- **What the benchmark measures, and why it is AQP BM:**
  - Run all-exact, all-HLL, and the size-aware hybrid through the same plan, and report per-group memory and error.
    The interesting output is the crossover: the group size above which the sketch starts paying off.
  - That crossover, and the policy that acts on it, live *above* the sketch API — the sketch does not know how many groups there are or how big each is.
    A raw sketch benchmark has one state and one stream, so it cannot see this at all.

### Example 2: top-5 regions by distinct users, over the middle 50% of page loads

```sql
WITH (SELECT quantile(0.25)(ResponseEndTiming) FROM hits WHERE ResponseEndTiming > 0) AS lo,
     (SELECT quantile(0.75)(ResponseEndTiming) FROM hits WHERE ResponseEndTiming > 0) AS hi
SELECT RegionID, COUNT(DISTINCT UserID) AS u
FROM   hits
WHERE  ResponseEndTiming BETWEEN lo AND hi
GROUP BY RegionID
ORDER BY u DESC
LIMIT 5;
```

Drop the fastest 25% and the slowest 25% of page loads, then report the 5 regions with the most unique users among what is left.

- **What it does, and who asks it:**
  - The real intent is "which regions have the biggest *typical* audience".
    An analyst does not want the count polluted by non-representative page loads: the fastest quarter are often bot hits or cache hits that never rendered a real page, and the slowest quarter are timeouts and broken sessions.
    Trimming both tails by response time keeps the ordinary page loads, and the per-region distinct-user count over *those* is the number that goes on the dashboard.
  - Trimming both tails before aggregating is a common de-noising pattern.

- **Physical plan (from ClickHouse `EXPLAIN`):**

```text
-- the two quantile subqueries are evaluated FIRST, as scalar constants:
Prewhere filter column: ResponseEndTiming >= 15 AND ResponseEndTiming <= 231

Limit 5
 └─ Sorting (uniq(UserID) DESC, Limit 5)
     └─ Aggregating (Keys: RegionID; Aggregates: uniq(UserID))
         └─ ReadFromMergeTree (default.hits)   -- with the prewhere above
```

  - Note what ClickHouse actually did: it computed `lo=15` and `hi=231` in a **separate pass** over the data, folded them into a constant predicate, and only then ran the main aggregation.
    The two-pass structure is not a design choice we imposed; the engine's own plan has it.

- **How approximation plays here — this is a three-stage composition:**
  - a quantile sketch over `ResponseEndTiming` produces the thresholds `lo`, `hi`;
  - those thresholds become a filter predicate;
  - the surviving rows feed one count-distinct state per region;
  - the estimated cardinalities feed a top-5 ranking.
  - So one approximate state's *output* becomes another approximate state's *input condition*, and the final correctness question is about the *ranking*, not about relative error on any one number.
    None of this is expressible as "one sketch over one stream".

- **What the benchmark measures, and why it is AQP BM:**
  - The interface strain: the `create / update / merge / finalize` trait assumes a single pass, but here the filter predicate does not exist until the quantile state is finalized.
    Expressing this query forces the middle-layer abstraction to grow a second pass (or the fan-out trick in `Middle-layer finding: filter cardinality decides whether one pass is possible`).
  - The benchmark measures the end-to-end cost of the whole composition and whether the top-5 ranking survives approximation — not the accuracy of any single sketch.
    That end-to-end, cross-stage view is exactly what a raw sketch benchmark cannot assemble.
  - Example 5 is the deliberate control for this one: same shape, but a predicate that is known before the scan, which isolates the cost of a predicate *derived from* a sketch.

### Example 3: top-5 most active users per region — and the same query on `URL`

```sql
-- 3a: top 5 users by hit count, per region
SELECT RegionID, UserID, count() AS c FROM hits GROUP BY RegionID, UserID
--     ... keep the top 5 rows per RegionID

-- 3b: identical shape, different target column
SELECT RegionID, URL, count() AS c FROM hits GROUP BY RegionID, URL
--     ... keep the top 5 rows per RegionID
```

For each region, report the 5 most frequent values of a column — users in 3a, URLs in 3b.

- **What it does, and who asks it:**
  - Top-k most active users per region: bot and abuse detection, power-user identification.
  - Top-k URLs per region: bread-and-butter web analytics ("most-visited pages here").
  - Both are per-group heavy hitters.
    Same query shape.
    Only the target column differs.

- **Physical plan (from ClickHouse `EXPLAIN`):**

```text
Aggregating (Keys: RegionID; Aggregates: topK(5)(UserID))
 └─ ReadFromMergeTree (default.hits)
```

  - The exact plan is different and heavier: `GROUP BY RegionID, UserID` to count every (region, user) pair, then keep the top 5 per region — a state proportional to the number of distinct users per region.
  - ClickHouse's `topK` is *already* a heavy-hitter sketch (SpaceSaving).
    So the exact-vs-sketch choice is not hypothetical here; it is literally two different aggregate functions the engine ships, behind the same `Aggregating` operator.

- **How approximation plays here — same sketch, opposite outcomes on two columns:**
  - A frequency sketch (SpaceSaving, Frequent-Items, CMS) has count error bounded by `N/k`, where `N` = stream length of the group and `k` = number of counters.
  - Measured on region 229 (the largest region, 18,295,832 rows):

| top-5 target | 5th place count | share of the region's stream | 5th vs 6th |
| --- | --- | --- | --- |
| `UserID` | 3,448 | 0.019% | 3,448 vs 3,120 (9.5% apart) |
| `URL` | 114,482 | 0.626% | 114,482 vs 97,884 (17% apart) |

  - The ground truth is well-defined in both cases: no mass ties at the top.
    Median user in region 229 appears only 2 times; the top user appears 10,597 times.
  - For **top-5 URLs**: to resolve 5th place (114,482 hits) needs `18.3M / k < 114,482`, so `k > 160`; to separate 5th from 6th, `k > 2,205`.
    Cheap.
    The sketch wins easily.
  - For **top-5 users**: to resolve 5th place (3,448 hits) needs `k > 5,307`; to separate 5th from 6th (a gap of only 328) needs `k > 111,560`.
    At the common default `k = 1,000` the error bound is 18,300 — **5x larger than the count of the item being ranked**.
    The guarantee is vacuous.
  - Same query shape, same sketch, same region.
    Swap the target column and the answer flips: one needs 160 counters, the other needs 111,560.

- **What the benchmark measures, and why it is AQP BM:**
  - Whether a heavy-hitter sketch works is **not a property of the sketch**.
    It is a property of the target column's frequency distribution relative to the stream length — and that is visible only when the sketch runs on the real column inside the real query.
  - A raw sketch benchmark feeds a synthetic Zipf stream and reports `error = N/k`.
    It structurally cannot tell you top-5-URL is easy while top-5-user is hopeless.
  - Note this breaks the naive plan in a **different place than Example 1**: Example 1 fails on the many *tiny* groups (state wasted); Example 3a fails on the single *largest* group (the error bound is meaningless there).

### Example 4: how many users are on mobile — the negative control

```sql
SELECT IsMobile, COUNT(DISTINCT UserID) FROM hits GROUP BY IsMobile;
```

Split users into mobile vs. non-mobile and count the distinct users on each side.

- **What it does, and who asks it:**
  - "How many unique users are on mobile" — a headline metric on any product dashboard.

- **Physical plan (from ClickHouse `EXPLAIN`):**
  - `Aggregating (Keys: IsMobile; Aggregates: uniq(UserID))` over a MergeTree scan.
  - Two groups, so two states.
    Nothing else in the plan.

- **How approximation plays here:**
  - Approximation wins overwhelmingly: an exact `HashSet` over 17.6M users costs hundreds of MB per group; an HLL costs about 16KB and is accurate to ~1%.
  - But there is exactly **one** sensible plan.
    No state-allocation decision (2 groups), no merge strategy to pick, no composition.
    The middle layer contributes nothing.

- **What the benchmark measures, and why it is AQP BM — it is the negative control:**
  - This is AQPBMV1 with two states instead of one, and that is the point of including it.
  - It marks the other side of the boundary: a real query where the middle layer is provably irrelevant.
    Having it in the set makes the examples where the middle layer *does* matter sharper, and gives the benchmark a case whose "correct" answer is "just use the sketch, there is nothing to decide".

### Example 5: which regions have the most mobile users — the control for Example 2

```sql
SELECT RegionID, COUNT(DISTINCT UserID) AS mobile_users
FROM   hits
WHERE  MobilePhone != 0
GROUP BY RegionID
ORDER BY mobile_users DESC
LIMIT 5;
```

Among mobile visits only, report the 5 regions with the most unique users.

- **What it does, and who asks it:**
  - "Where is my mobile audience biggest" — a standard segmentation question.
  - Note on the column: `MobilePhone` is a phone vendor/brand id (used with `MobilePhoneModel`), not a phone number.
    Measured, `MobilePhone = 0` covers 92,775,562 rows (**92.78%**), so `MobilePhone != 0` is a mobile-visit filter that keeps only 7.22% of rows.

- **Physical plan (from ClickHouse `EXPLAIN`):**

```text
Limit 5
 └─ Sorting (uniq(UserID) DESC, Limit 5)
     └─ Aggregating (Keys: RegionID; Aggregates: uniq(UserID))
         └─ ReadFromMergeTree (Prewhere: MobilePhone != 0)
```

  - The predicate is a plain constant, so it is pushed into the scan as a prewhere and applied in a **single pass**.
    Compare this directly to Example 2's plan, where the predicate constants had to be computed in a separate pass first.

- **How approximation plays here:**
  - Same as Example 2 downstream of the filter: one count-distinct state per region, then a top-5.
  - The one difference is upstream: the predicate is known before the scan, so there is no quantile stage and no second pass.

- **What the benchmark measures, and why it is AQP BM — it is the control for Example 2:**
  - The query shape is identical to Example 2 (filter, count-distinct per region, top-5).
    The only difference is where the predicate comes from:

| | Example 5 | Example 2 |
| --- | --- | --- |
| predicate | `MobilePhone != 0` | `ResponseEndTiming BETWEEN lo AND hi` |
| known before the scan? | yes — a constant, pushed down | no — `lo`/`hi` do not exist until a quantile state is finalized |
| passes required | one | two, or one with state fan-out |

  - Same shape, same sketches, same output type, so the cost difference isolates exactly one thing: **the price of a predicate derived from an approximate state**.
    Without Example 5 as a baseline, Example 2's cost is not attributable to anything.

### Example 6: median event time per site — partitioning decides the answer

```sql
SELECT CounterID, quantile(0.5)(EventTime) AS median_t
FROM   hits
GROUP BY CounterID;
```

For each site, find the median moment of its traffic during the month.

- **What it does, and who asks it:**
  - "When during the month was this site's traffic centred" — a real question for a site-analytics product (e.g. spotting sites whose activity clusters around a launch).

- **Physical plan (from ClickHouse `EXPLAIN`):**

```text
Aggregating (Keys: CounterID; Aggregates: quantile(0.5)(EventTime))
 └─ ReadFromMergeTree (default.hits)
```

  - The plan looks trivial, but the *interesting* physical detail is not printed by `EXPLAIN`: how the scan is split into partitions, and how the partial states are merged.
    That choice is what changes the answer here.

- **How approximation plays here — the table's physical order is the whole story:**
  - Declared sorting key: `CounterID, EventDate, UserID, EventTime, WatchID`.
    Verified on the real table, not just read from metadata:
    - The table has 3 parts; the largest holds 99,368,738 of 99,997,497 rows (99.4%).
    - Sampling `CounterID` by physical offset in that part shows it is **strictly non-decreasing** across all 99M rows (`17 -> 3,922 -> 7,525 -> ... -> 258,631`); descents over the first 2M physical rows: **0**.
    - `EventTime` is **not** globally sorted — `EventDate` is only the second sort key, so it is sorted only within a `CounterID`.
      Sampling shows `EventDate` jumping around (07-15, 07-03, 07-31, ...) as physical offset increases.
  - **Partition by physical range** (what an engine does when it splits a scan): because the table is sorted by `CounterID`, each group lives entirely inside one partition, so `merge` degenerates to a no-op, and inside a group the rows arrive in `EventTime` order — the quantile sketch sees **perfectly sorted input**.
  - **Partition by hash of the group key**: each group's rows scatter across all P partitions, every group needs a real P-way merge of partial states, and each partial state sees a random sample rather than a sorted run.
  - t-digest and KLL are both sensitive to arrival order, so the two partitioning schemes do **not** produce the same answer.

- **What the benchmark measures, and why it is AQP BM:**
  - Same query, same sketch, same data — change only the partitioning scheme, and both cost (no shuffle vs. full shuffle; zero merges vs. one merge per group) and accuracy (sorted vs. random input; no merge error vs. accumulated merge error) change.
  - The partitioning scheme is chosen by the middle layer, not by the sketch.
    This is exactly what the existing `partitioned_merge` execution mode should measure, and it is invisible to a raw sketch benchmark fed a single random stream.

### Example 7: p99 latency per site — the benchmark's own metric picks the winner

```sql
SELECT CounterID, quantile(0.99)(ResponseEndTiming) AS p99
FROM   hits
WHERE  ResponseEndTiming > 0
GROUP BY CounterID;
```

For each site, estimate the 99th-percentile page-load time.

- **What it does, and who asks it:**
  - Per-site tail latency is the canonical SLO query.
    Every latency dashboard is this query, and nobody doubts that people ask for p99.

- **Physical plan (from ClickHouse `EXPLAIN`):**

```text
Aggregating (Keys: CounterID; Aggregates: quantile(0.99)(ResponseEndTiming))
 └─ ReadFromMergeTree (Prewhere: ResponseEndTiming > 0)
```

  - One quantile state per site.
    The exact baseline holds all values (or a full sorted array) per group; every candidate swaps in a different quantile sketch behind the same operator.

- **How approximation plays here — two sketches that guarantee different things:**
  - KLL guarantees **rank error**: the returned value's true rank is within `q ± eps`.
    Space is `O(1/eps)`, independent of the distribution.
    It makes **no promise** about how far the returned *value* is from the true value.
  - DDSketch guarantees **relative value error**: `|v_hat - v| / v <= alpha`, uniformly at every quantile including the tail.
    Space depends on the *dynamic range* of the values, not the row count.
    It makes **no promise** about rank.
  - Why that difference is invisible at p50 and catastrophic at p99:
  - A rank error translates into a value error through the slope of the quantile function, which is `1 / density`.
    Where the data is dense, rank error is harmless.
    Where the data is sparse (the tail), rank error explodes into value error.
  - Measured exactly on the 25,438,863 rows with `ResponseEndTiming > 0`:

```text
p0.1   p1   p10  p25  p50  p75  p90  p95   p98   p99   p99.5   p99.9
   1    1     4   15   68  233  704  1398  3433  6600  12329   30000
```

  - Reading a 1% rank error off that table as a *value* error:

| target quantile | value moves | relative value error from a 1% rank error |
| --- | --- | --- |
| p10 | 4 -> 4 | 0% |
| p25 | 15 -> 16 | 6.7% |
| p50 | 68 -> 73 | 7.4% |
| p75 | 233 -> 242 | 3.9% |
| p90 | 704 -> 792 | 12.5% |
| p95 | 1,398 -> 1,735 | 24.1% |
| **p98 -> p99** | 3,433 -> 6,600 | **92.2%** |

  - So a KLL with a perfectly respectable 1% rank guarantee can return a p99 latency that is off by nearly 2x in value.
    Its guarantee is intact.
    It is simply not the guarantee the query needed.
  - Note the *low* tail is not the problem here: p0.1 through p2 are all `1`, because the data is quantized and dense there.
    A rank error at the bottom costs almost nothing.
  - The general rule is about **density**, not about "small values": rank error is fatal wherever the density is low.
    For right-skewed latency that is the right tail.
    For a left-skewed column it would be the left tail.

- **The finding, and it is about the benchmark itself:**
  - If AQPBMV2 reports "relative error of the returned value", KLL looks catastrophic at p99 and DDSketch looks perfect.
  - If AQPBMV2 reports "rank error", KLL is comfortably within spec and DDSketch offers no guarantee at all.
  - **Neither metric is wrong.
    They measure different contracts.**
  - So the benchmark's choice of metric silently decides the winner.
  - A raw sketch benchmark that reports one accuracy number is making an unstated choice about which guarantee matters — and that choice belongs to the *query*, which a raw sketch benchmark does not have.
  - This is the most direct answer so far to the open question "what should be considered as output": for quantiles, accuracy is not one number, and picking one number is already taking a side.

- What AQPBMV2 must therefore do:
  - Report rank error **and** relative value error, per quantile level, not just at p50.
  - Let the example query declare which guarantee it actually needs.
  - Treat "which metric does this query care about" as part of the query definition, not as a benchmark-wide constant.

### Example 8: audience overlap — excellent sketches, worthless answer

```sql
-- distinct users who visited BOTH site A and site B
SELECT COUNT(DISTINCT UserID) FROM (
    SELECT UserID FROM hits WHERE CounterID = 199550
    INTERSECT
    SELECT UserID FROM hits WHERE CounterID = 105857
);
```

Count the distinct users who visited **both** site A and site B.

- **What it does, and who asks it:**
  - Audience overlap between two properties is a standard ad-targeting and cross-site question: "how many of my users also use their site" is asked constantly.

- **Physical plan (from ClickHouse `EXPLAIN`):**

```text
Aggregating (Aggregates: uniq(UserID))
 └─ IntersectOrExcept
    ├─ ReadFromMergeTree (Prewhere: CounterID = 199550)   -- set A
    └─ ReadFromMergeTree (Prewhere: CounterID = 105857)   -- set B
```

  - The exact plan needs a genuine physical set intersection: materialize both user sets, intersect them, then count.
    The sketch plan cannot use this operator at all — it has to compute the intersection a completely different way, which is the whole point.

- **How approximation plays here:**
  - HLL supports **union** (registers are max-ed) but has **no intersection operator**.
    The only route is inclusion-exclusion, `|A n B| = |A| + |B| - |A u B|`, so the answer is a *difference of three separate estimates*.
  - Measured on the two largest sites:

```text
|A|        = 3,498,632     (CounterID = 199550)
|B|        = 2,234,995     (CounterID = 105857)
|A u B|    = 5,615,904
|A n B|    =   117,723     <- only 2.1% of the union
```

- What inclusion-exclusion does to a *good* HLL.
  **Measured**, using ClickHouse's own `uniqHLL12` and `uniqTheta` against `uniqExact` as ground truth:

```text
                        estimate      exact       error
|A|       uniqHLL12    3,459,552   3,498,632      1.12%
|B|       uniqHLL12    2,240,716   2,234,995      0.26%
|A u B|   uniqHLL12    5,651,422   5,615,904      0.63%
------------------------------------------------------------
|A n B|   inclusion-exclusion from the three HLLs above:
                          48,846     117,723     58.51%
|A n B|   uniqTheta, native intersection:
                          84,998     117,723     27.80%
```

- The finding:
  - Each individual HLL is accurate to between **0.26% and 1.12%**.
    These are good sketches.
  - The intersection derived from them is off by **58.5%** — it reports 48,846 where the truth is 117,723, missing more than half the audience.
  - The error is amplified roughly **50x**, purely by the subtraction.
  - The cause is structural: the intersection is only 2.1% of the union, so the subtraction cancels the signal and leaves nothing but the accumulated noise of the three estimates.

- And the honest part: **Theta does not rescue this query either.**
  - Theta has a native intersection operator, and it is about **2x better** than inclusion-exclusion.
  - But it is still **27.8%** off.
  - So the conclusion is not "use Theta and you are fine".
    It is that a small intersection relative to the sets is hard for *every* count-distinct sketch, and the benchmark should say so.
  - This is still a discriminating result — Theta beats HLL by 2x here — but the more important result is that **both fail**, which is exactly the kind of thing an honest benchmark exists to report.

- Note this is the **exact inverse** of the mental experiment recorded earlier in this document:
  - That one says: individually bad sketches (30% error each) can still yield a good final answer (1%).
  - This one says: individually excellent sketches (~1% error each) can yield a worthless final answer (58.5% off), and this one is measured rather than imagined.
  - Both are true.
    Both are invisible to a raw sketch benchmark, which only ever reports the ~1%.
  - Together they are the strongest argument that the *implementation*, not the sketch, is the thing that has to be measured.

### Example 9: top-10 search phrases by distinct users — the cheap plan is not just approximate, it is wrong

```sql
SELECT SearchPhrase, COUNT(DISTINCT UserID) AS users
FROM   hits
WHERE  SearchPhrase != ''
GROUP BY SearchPhrase
ORDER BY users DESC
LIMIT 10;
```

Report the 10 search phrases with the most distinct users.

- **What it does, and who asks it:**
  - "What are people searching for", ranked by **unique users** rather than raw hits, precisely so that one bot hammering a single phrase does not dominate the report.
    Ranking by distinct users instead of by hits is the standard de-botting move.

- **Physical plan (from ClickHouse `EXPLAIN`):**

```text
Limit 10
 └─ Sorting (uniq(UserID) DESC, Limit 10)
     └─ Aggregating (Keys: SearchPhrase; Aggregates: uniq(UserID))
         └─ ReadFromMergeTree (Prewhere: notEmpty(SearchPhrase))
```

  - The plan keeps one count-distinct state **per phrase**, and `SearchPhrase` is a very high-cardinality key — millions of groups.
    That is what makes the naive approximate plan unaffordable, and what tempts everyone into a cheaper, unsound plan.

- **How approximation plays here — the cheap plan is not just approximate, it is wrong:**
  - One HLL per phrase means millions of 16KB states — not affordable.
    So the plan everyone reaches for is two-level: first use a cheap heavy-hitter sketch on **hit counts** to find candidate phrases, then build HLLs only for those candidates.
    This assumes hit count is a usable proxy for distinct-user count.
    **It is not.**
  - Measured.
    Top-10 by distinct users, with each phrase's rank by hit count:

| rank by users | phrase | users | hits | **rank by hits** |
| --- | --- | --- | --- | --- |
| 1 | карелки | 23,673 | 70,263 | 1 |
| 2 | смотреть онлайн | 19,747 | 24,580 | 3 |
| 3 | албатрутдин | 18,394 | 34,675 | 2 |
| 4 | смотреть онлайн бесплатно | 17,553 | 21,647 | 4 |
| 5 | смотреть | 14,603 | 19,707 | 5 |
| 6 | экзоидные | 14,529 | 16,620 | 9 |
| 7 | мангу в зарабей грама | 14,198 | 19,195 | 6 |
| 8 | сколько мытищи | 9,007 | 12,317 | 10 |
| 9 | дружке помещение | 8,792 | 17,284 | 7 |
| 10 | комбинирование смотреть | 7,572 | 9,545 | **12** |

  - The true #10 by distinct users sits at rank **12** by hits.
  - A candidate set of "top-10 by hits" therefore **misses a true top-10 answer**.
  - It also wastes candidates: `galaxy table` is rank 8 by hits but has only 7,088 users, and `3dnewsru` is rank 20 by hits with just 2,698 users.
  - The ranking is also reordered inside the top 5: `албатрутдин` is #2 by hits but #3 by users.

- Why it breaks: the hits-per-user ratio is not constant.
  - `карелки`: 3.0 hits/user.
    `экзоидные`: 1.14 hits/user.
    `3dnewsru`: 2.7 hits/user.
  - A ~2.6x spread across phrases.
  - Hit count is therefore **not a monotone proxy** for distinct-user count, so a prefilter on hit count cannot be trusted to contain the true top-k by users.

- The finding:
  - The memory-saving plan is not merely *less accurate*.
    It can return a **flatly wrong answer**, and it will do so silently.
  - To make it safe you must widen the candidate set — and how far you must widen depends on the maximum hits-per-user spread in the data, which you do not know in advance and which would take a *different* sketch to measure.
  - So the plan choice, its soundness, and the data property that decides its soundness all live in the middle layer.
    None of them are visible from the sketch API.

### Example 10: merge does not compose with join

```sql
-- hits joined to a small site dimension table, grouped by the site's category
SELECT d.category,
       COUNT(DISTINCT h.UserID)              AS users,
       quantile(0.99)(h.ResponseEndTiming)   AS p99
FROM   hits h
JOIN   site_dim d ON h.CounterID = d.CounterID
GROUP BY d.category;
```

Join `hits` to a small site dimension table and, per site category, report both the distinct users and the p99 latency.

- **What it does, and who asks it:**
  - Rolling a fact table up to a business dimension ("group sites by category, then aggregate") is the normal shape of a warehouse query.
    A fact table joined to small dimension tables is the standard star-schema pattern, not a contrivance.
  - This is the one example that steps outside the single ClickBench table, because it needs a dimension table (`site_dim`) to join against.

- **Physical plan (described, not run — the playground has no `site_dim`):**

```text
Aggregating (Keys: d.category; Aggregates: uniq(h.UserID), quantile(0.99)(h.ResponseEndTiming))
 └─ Join (h.CounterID = d.CounterID)
    ├─ ReadFromMergeTree (default.hits)
    └─ ReadFromMemory (site_dim)
```

  - The standard optimization here is **eager aggregation**: push the aggregation below the join.
    Pre-compute one sketch per `CounterID`, then per category `merge` the sketches of the `CounterID`s in that category — avoiding a re-scan of `hits` per category.
    This is exactly what a pre-aggregated sketch cube (Druid-style rollup) exists to do.

- **How approximation plays here — the rewrite is safe for one column and wrong for the other:**
  - `merge` in the sketch world implicitly assumes its inputs are **disjoint partitions**.
    A join does not respect that: if a `CounterID` maps to more than one category, or the dimension table has more than one row per `CounterID`, the same partial state gets folded in more than once.
  - **HLL merge is idempotent.** Registers are max-ed, so `merge(H, H) = H`.
    Double-folding the same state changes nothing — the `COUNT(DISTINCT)` column survives.
  - **KLL, t-digest, and CMS merge are not idempotent.** They are multiset unions: `merge(K, K)` doubles the weight.
    Double-folding silently corrupts the answer — the `quantile` column does **not** survive.

- **What the benchmark measures, and why it is AQP BM:**
  - The same query plan is **correct for the `COUNT(DISTINCT)` column and wrong for the `quantile` column**, in the same query, over the same join.
    The word `merge` is hiding two different algebras: a set union (idempotent) and a multiset union (not idempotent).
  - The current trait has exactly one `merge` signature:

```rust
fn merge(&self, left: &mut Self::State, right: Self::State);
```

  - That signature cannot express which algebra it implements, so the middle layer cannot tell a planner which merges are safe to reorder, duplicate, or push below a join.
    This is stronger than "merge loses accuracy": **merge does not have a single well-defined semantics across functionality classes, and the interface pretends it does.** Measuring this needs a query with a join and two different aggregates — precisely what a raw sketch benchmark never has.

- Open questions this raises:
  - Should the interface expose the merge algebra (idempotent / additive / lossy) as a property?
  - Theta sketches carry a richer algebra (union, intersection, difference).
    Are they the only family whose merge actually composes with relational operators?
  - Does any of this change if the join is a strict 1:1 dimension lookup?
    (Probably yes — the unsafe case needs fan-out.
    That would make fan-out itself a benchmark knob.)

### Middle-layer finding: filter cardinality decides whether one pass is possible

> Status: this came out of Example 2. Recorded here because it may be worth more
> than Example 2 itself. Not yet validated by running anything.

- The setup:
  - Example 2 needs a threshold before it can filter.
  - The threshold comes from a quantile state.
  - A quantile state only produces a threshold when it is finalized.
  - So the obvious reading is: you must read the data twice.

- Implementation A: two passes.
  - Pass 1: build one quantile state over `ResponseEndTiming`.
    Finalize it.
    Get `lo` and `hi`.
  - Pass 2: read the data again.
    Keep rows where `ResponseEndTiming` is in `[lo, hi]`.
    Feed the surviving `UserID`s into one count-distinct state per `RegionID`.
  - Cost: two scans of a 100M-row table.
  - State: one quantile state, plus 9,040 count-distinct states.

- Implementation B: one pass, only possible when the filter column is low-cardinality.
  - Suppose the filter column were `Income`, which has exactly 4 distinct values.
  - Then during the single pass, do not try to decide whether a row passes the filter.
  - Instead, *promote the filter column into the group key*.
    - Keep one count-distinct state per `(RegionID, Income)` cell, not per `RegionID`.
    - That is 9,040 x 4 cells.
    - Every row goes into exactly one cell.
      No filtering decision is needed yet.
  - Also keep one quantile state over `Income` during the same pass.
  - At finalize time:
    - Finalize the quantile state.
      Now `lo` and `hi` exist.
    - For each `RegionID`, `merge` together only the cells whose `Income` value falls in `[lo, hi]`.
    - That merged state is exactly the answer for that region.
  - Cost: one scan.
  - The filtering decision was *deferred* from update time to finalize time.

- Why B works only for low-cardinality filter columns:
  - B pays for the deferral by keeping one state per distinct filter value, per group.
  - With `Income` (4 values), that is 4x more states.
    Affordable.
  - With `ResponseEndTiming` (28,376 values), that would be 9,040 x 28,376 states.
    - That is roughly 256 million count-distinct states.
    - Not affordable.
      B is dead.
  - So with `ResponseEndTiming` you are forced back to A, and you pay the second scan.

- The finding:
  - The cardinality of the filter column decides which implementations even exist.
  - Low-cardinality filter column: one-pass is available, at the cost of state fan-out.
  - High-cardinality filter column: one-pass is unavailable, and a second scan is mandatory.
  - The trade is: *extra state* versus *an extra pass over the data*.

- Why this is a middle-layer statement and not a sketch statement:
  - Both A and B use the exact same sketches, with the exact same accuracy per state.
  - A raw sketch benchmark would report them as identical.
  - They are not identical: one reads the table once, the other twice, and their peak memory differs by orders of magnitude.
  - The difference is entirely in how the states are laid out and when they are merged.
  - Note also that B uses `merge` as a *semantic* operation - unioning across income buckets is part of answering the query - not as a shuffle artifact.

- Why this is worth recording:
  - It turns into a benchmark knob directly.
  - Keep one query template.
    Vary only the cardinality of the filter column.
    - `Sex` (3 values), `Income` (4), `OS` (91), `ResolutionWidth` (2,159), `ResponseEndTiming` (28,376).
  - Then measure, for each candidate implementation:
    - Does it still run at all?
    - Peak state size.
    - Number of passes.
    - End-to-end latency.
    - Accuracy of the final top-5 ranking.
  - The expected result is a crossover point where one-pass stops being worth it.
  - Locating that crossover is a claim a raw sketch benchmark structurally cannot make.

- Open questions:
  - Does the extra `merge` in B degrade accuracy?
    Merging 4 HLLs is not free of error.
  - Is there an intermediate design?
    For example, bucket the filter column coarsely, fan out over the buckets, and only re-scan rows in the boundary bucket.
    - That would be a partial second pass, not a full one.
    - It would also make the filter column's *distribution*, not just its cardinality, matter.

benchmark the thing defined above

try to give more examples like this

then, we can pick which example is good and which example is bad

then, we can have a better definition/scope of middlelayer from those examples

input/output can both be multi dimension the data shape/schema, data distribution can matter

the way middlelayer represents query expression

what should be considered as input/output is a problem

output can include performance, resource usage, accuracy, etc.



  - TPC-H `lineitem` (familiar, but uniformly generated):


```sql
l_orderkey         -- high cardinality
l_partkey          -- mid-high cardinality
l_suppkey          -- mid cardinality
l_linenumber
l_quantity         -- small domain (1..50)
l_extendedprice    -- numeric, usable for quantile
l_discount         -- 0.00..0.10
l_tax
l_returnflag       -- 3 values
l_linestatus       -- 2 values
l_shipdate
l_commitdate
l_receiptdate
l_shipinstruct     -- 4 values
l_shipmode         -- 7 values
```

  - Caveat on TPC-H:
    - TPC-H data is generated uniformly.
    - Heavy-hitter examples degenerate on it, because there is no hitter.
    - It may still be useful as a uniform control against the skewed `hits` table.



- Middle-layer shape:

```text
user-level functionality
  -> runnable approximate-function implementation
  -> sketch or exact state implementation
```

- Current functionality classes:
  - Count distinct.
  - Heavy hitters.
  - Quantile.

- Current exact baselines:
  - `ExactCountDistinct<HashSet>`
  - `ExactHeavyHitters<HashMap>`
  - `ExactQuantile<Vec>`

- Current sketch-backed implementations:
  - Apache DataSketches HLL for count distinct.
  - Apache DataSketches FrequentItems for heavy hitters.
  - Apache DataSketches TDigest for quantile.
  - `sketch_oxide` HLL for count distinct.
  - `sketch_oxide` SpaceSaving for heavy hitters.
  - `sketch_oxide` TDigest for quantile.
  - `asap_sketchlib` HLL for count distinct.
  - `asap_sketchlib` CMSHeap for heavy hitters.
  - `asap_sketchlib` KLL for quantile.

- Current benchmark-owned execution modes:
  - `grouped_state`
    - Keep one implementation state per group.
    - Update that state from all rows in the group.
    - Finalize one answer per group.
  - `partitioned_merge`
    - Build implementation states inside partitions.
    - Merge partition-local states for each group.
    - Finalize one answer per group after merge.

- Current report metrics:
  - For count distinct and quantile:
    - How many group-level answers stay within the chosen error threshold.
    - Mean and max relative error for numeric outputs.
  - For heavy hitters:
    - Whether returned top-k items are true top-k items.
    - Whether true top-k items are missing from the returned result.

- Current smoke/demo path:
  - `cargo run -p aqp-core --example aqpbmv2_functions`
  - The current generated data is only a smoke test.
  - It proves implementations can be wired into the middle layer and run.
  - It is not yet benchmark-grade workload evidence.
  - The current implementations are still close to one-sketch-per-function cases.
  - The next step is to add workloads and implementations where the middle layer matters more than the raw sketch API.

- What AQPBMV2 does not claim:
  - It does not benchmark Spark's function directly.
  - It does not benchmark Trino's function directly.
  - It does not benchmark BigQuery's function directly.
  - It does not benchmark ClickHouse's function directly.
  - Those systems show that the functionality classes are real.

- Research bar:
  - AQPBMV2 is interesting only if benchmark-owned execution modes reveal new behavior.
    - The behavior should be something raw sketch benchmarks miss.
  - If they do not, AQPBMV2 is still a useful toolkit but a weaker paper contribution.

## AQPBMV2 Preliminary Result

- Command:
  - `cargo run -q -p aqp-core --example aqpbmv2_functions`

- Functionality coverage supported by this preliminary result:
  - Count distinct:
    - Apache DataSketches HLL supports the count-distinct functionality.
    - `sketch_oxide` HLL supports the count-distinct functionality.
    - `asap_sketchlib` HLL supports the count-distinct functionality.
  - Heavy hitters:
    - Apache DataSketches FrequentItems supports the heavy-hitter functionality.
    - `sketch_oxide` SpaceSaving supports the heavy-hitter functionality.
    - `asap_sketchlib` CMSHeap supports the heavy-hitter functionality.
  - Quantile:
    - Apache DataSketches TDigest supports the quantile functionality.
    - `sketch_oxide` TDigest supports the quantile functionality.
    - `asap_sketchlib` KLL supports the quantile functionality.
  - This is a capability and integration result.
  - It does not yet support a claim about general benchmark quality or winner libraries.

- What the input is:
  - Each input row has two logical fields.
    - `group_key`
    - `value`
  - A group means one distinct value of the synthetic `group_key`.
    - Example: `group_000`, `group_001`, ..., `group_031`.
    - This mimics one output group from `GROUP BY service`.
  - Current demo group assignment is hand-written in the synthetic generator.
    - It assigns rows to groups with `idx % group_count`.
    - It is not produced by a SQL interpreter, query planner, or real data schema.
  - The benchmark groups rows by `group_key`.
    - It then runs the target functionality over the `value` field inside each group.
    - For count distinct, it counts distinct `value`s inside each synthetic group.
    - For heavy hitters, it finds frequent `value`s inside each synthetic group.
    - For quantile, it estimates the p95 of `value`s inside each synthetic group.
  - Each group owns one independent aggregate state for the implementation being tested.
  - Any group-level claim below only refers to these synthetic demo groups.
    - It is not a claim about production groups or all possible group-by workloads.
  - Count distinct input:
    - 50,000 rows.
    - 32 synthetic group-by groups.
    - Deterministic value generator with value cardinality 20,000.
  - Heavy hitter input:
    - 80,000 rows.
    - 24 synthetic group-by groups.
    - Deterministic top-k pattern.
    - Per group, the top items are intentionally clear.
      - Roughly 45%, 20%, and 15% for the top three items.
      - Remaining rows are long-tail noise.
  - Quantile input:
    - 80,000 rows.
    - 16 synthetic group-by groups.
    - Deterministic periodic values with a small tail bump.
    - Query target is p95.

- What the output is:
  - A JSON report.
  - Exact baseline outputs for each functionality class.
  - Candidate outputs under the `grouped_state` execution mode.
  - Candidate outputs under the `partitioned_merge` execution mode.
  - Numeric summaries for count distinct and quantile.
    - Fraction of groups whose approximate answer is within the chosen error threshold.
    - Mean relative error.
    - Max relative error.
  - Set summaries for heavy hitters.
    - Fraction of returned top-k items that are actually correct.
    - Fraction of exact top-k items that were returned.

- What "within threshold" means here:
  - It is a workload-level metric.
  - It is not a statistical confidence interval.
  - It means the fraction of produced answers within the declared error threshold.
  - Example:
    - 32 count-distinct groups.
    - 32 groups within the relative-error threshold.
    - The run has `100%` of answers within threshold.

- Count distinct result:
  - Exact output has 32 groups.
  - All implementations have `100%` of group-level answers within the chosen error threshold.
  - Apache DataSketches HLL:
    - Mean relative error: about `0.76%`.
    - Max relative error: about `2.40%`.
  - `sketch_oxide` HLL:
    - Mean relative error: about `0.87%`.
    - Max relative error: about `2.27%`.
  - `asap_sketchlib` HLL:
    - Mean relative error: about `0.55%`.
    - Max relative error: about `1.47%`.

- Heavy hitter result:
  - Exact output has 24 groups.
  - Apache DataSketches FrequentItems:
    - Every returned top-k item is correct.
    - No exact top-k item is missing.
  - `sketch_oxide` SpaceSaving:
    - Every returned top-k item is correct.
    - No exact top-k item is missing.
  - `asap_sketchlib` CMSHeap:
    - Every returned top-k item is correct.
    - No exact top-k item is missing.

- Quantile result:
  - Exact output has 16 groups.
  - Apache DataSketches TDigest:
    - All group-level answers are within the chosen error threshold.
    - Mean relative error: approximately `0`.
  - `sketch_oxide` TDigest:
    - All group-level answers are within the chosen error threshold.
    - Mean relative error: about `0.39%`.
    - Max relative error: about `0.63%`.
  - `asap_sketchlib` KLL:
    - All group-level answers are within the chosen error threshold.
    - Mean relative error: `0`.

- What this result represents:
  - It shows the toolkit wiring works.
  - Exact baselines and sketch-backed implementations run through the same interface.
  - Multiple sketch libraries can be compared under the same benchmark-owned execution modes.
  - `grouped_state` and `partitioned_merge` both execute successfully.
  - It is still close to raw sketch comparison because each current implementation mostly wraps one sketch.
  - Its purpose is to show the middle-layer interface can host those implementations.

- What this result does not prove:
  - It does not prove one library is generally better.
  - It does not prove AQPBMV2 is already benchmark-grade.
  - It does not yet prove that AQPBMV2 reveals behavior missed by raw sketch benchmarks.
  - It does not validate the group-generation logic.
  - The generated data is too easy.
  - The result is a smoke test and preliminary validation of the toolkit.

- Next steps:
  - Replace toy generators with benchmark-grade workload generators.
  - Use explicit distributions.
    - Uniform.
    - Zipf.
    - Lognormal.
    - Pareto or heavy-tail mixtures.
  - Add workload knobs.
    - Group cardinality.
    - Group-key generation.
    - Mapping from workload/task semantics to group keys.
    - Group-size skew.
    - Tail heaviness.
    - Top-k gap.
    - Filter selectivity.
    - Correlation between group key and value.
    - Partition count and merge shape.
  - Add resource and performance measurement.
    - Update throughput.
    - Query latency.
    - Merge latency.
    - Memory or serialized state size.
  - Check whether AQPBMV2 execution modes reveal behavior that raw sketch benchmarks miss.

## Previous AQPBMV1

- AQPBMV1 is the existing raw-sketch benchmark layer.
  - Target: sketch primitives directly.
  - Input control: data shape and distribution.
  - Metrics: throughput, accuracy, and memory usage.
  - Role: provide primitive-level evidence for AQPBMV2.

## Future AQPBMV3

Placeholder.

## Future AQPBMV4

Placeholder.

## Future AQPBMV5

Placeholder.

## Existing Approximate Query Support

- Codex found approximate-query or sketch-backed functions in several systems.
  - This matches my impression that approximation support exists in practice.
  - This list is not exhaustive.
  - More systems and functions may need to be added later.

- Apache DataFusion:
  - `approx_distinct`
  - `approx_median`
  - `approx_percentile_cont`
  - `approx_percentile_cont_with_weight`
  - Approximate percentile functionality is described as using t-digest.
  - <https://datafusion.apache.org/user-guide/sql/aggregate_functions.html>

- Trino:
  - `approx_distinct`
  - `approx_most_frequent`
  - `approx_percentile`
  - `numeric_histogram`
  - HyperLogLog state functions such as `approx_set` and `merge`
  - <https://trino.io/docs/current/functions/aggregate.html>

- Spark SQL:
  - `approx_count_distinct`
  - `approx_percentile`
  - `percentile_approx`
  - `count_min_sketch`
  - HLL sketch functions
  - KLL sketch aggregate, merge, and query functions
  - <https://spark.apache.org/docs/latest/api/sql/index.html>

- Google BigQuery:
  - `APPROX_COUNT_DISTINCT`
  - `APPROX_QUANTILES`
  - `APPROX_TOP_COUNT`
  - `APPROX_TOP_SUM`
  - <https://docs.cloud.google.com/bigquery/docs/reference/standard-sql/approximate_aggregate_functions>

- ClickHouse:
  - Approximate distinct-count variants.
    - `uniq`
    - `uniqCombined`
    - `uniqHLL12`
    - `uniqTheta`
  - Quantile variants such as t-digest and GK-family functions.
  - <https://clickhouse.com/docs/sql-reference/aggregate-functions/reference>

- Snowflake:
  - HLL functionality
  - MinHash and similarity functionality
  - Approximate top-k functionality
  - Approximate percentile functionality
  - Accumulate, combine, and estimate style functions for some approximate states
  - <https://docs.snowflake.com/en/sql-reference/functions-aggregation>

- Apache Druid:
  - DataSketches Theta aggregators
  - DataSketches HLL aggregators
  - DataSketches Quantiles aggregators
  - Documentation also discusses older approximate histogram and cardinality implementations.
  - <https://druid.apache.org/docs/latest/querying/aggregations/>

- Apache DataSketches:
  - A production-quality sketch library rather than a complete AQP system.
  - Provides sketch implementations and adaptors for multiple systems.
    - Examples: Hive, Pig, PostgreSQL, BigQuery, and Druid.
  - Provides cross-language implementations and binary compatibility goals.
  - <https://datasketches.apache.org/>
  - <https://datasketches.apache.org/docs/Architecture/SketchesByComponent.html>

- Takeaway:
  - Approximate functionality is common.
  - The support is fragmented across several forms.
    - System-specific functions.
    - Sketch-state APIs.
    - UDFs and extensions.
    - Standalone sketch libraries.
  - This motivates AQPBMV2's middle layer.
    - Runnable approximate-function implementations.
    - Benchmark-owned execution modes.
