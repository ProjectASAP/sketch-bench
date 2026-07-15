# Blog Outline: AQPBM

> Status: design doc for AQPBMV2.
> AQPBMV2 locates at the physical plan.
> AQPBMV2 monitors possible physical plans.
> 10 example queries on the ClickBench `hits` table with what the query is and what can be benchmarked around those queries.
> Open: build the running interpreter/toolkit; add interpreters beyond DataFusion; transcribe real engines' physical plans as reference baselines.
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
- [Example 9: top-10 search phrases by distinct users — a provably sound cheap plan](#example-9-top-10-search-phrases-by-distinct-users--a-provably-sound-cheap-plan)
- [Example 10: a fan-out join is safe for count-distinct but corrupts quantile](#example-10-a-fan-out-join-is-safe-for-count-distinct-but-corrupts-quantile)

## TL;DR

**The middle layer is the physical plan, and AQPBMV2 measures it.**
A sketch is a *leaf* — create / update / merge / finalize over one state — and an exact `HashSet` has the same shape, so leaves are interchangeable components, not the subject.
The *plan* is the subject: the graph around the leaves — how many states and keyed by what, how many passes, how partial states are partitioned and merged, which side of a join the aggregate sits on, how results compose.
A raw sketch benchmark tests one leaf over one stream; a system benchmark tests a whole engine; neither tests the plan.

**Why the plan is the interesting unit under approximation.**
Classic optimizers assume every physical plan of a query returns the same answer and differs only in cost; under approximation the answers differ, with different error — so there is no single optimal plan, only a (cost, error) frontier, and some rewrites (e.g. pushing an aggregate below a join) are sound for one leaf (idempotent HLL merge) and silently wrong for another (additive quantile merge).
The plan matters only when the answer is *not* already precomputable: a global `COUNT(DISTINCT)` is a stored sketch (nothing to benchmark), but the same aggregate per group is a live computation (the benchmark's territory).

**What the middle layer / toolkit must have:**

- an **IR** reusing a conventional plan representation — reuse a query engine's frontend for `SQL -> logical plan`, then build our own physical IR and interpreter (the Arroyo pattern), not the engine's executor;
- a **leaf plug-in** (`create/update/merge/finalize`) holding an exact or sketch state, plus **node annotations** the exact world never needed: merge algebra (idempotent / additive) and error metric (rank / value);
- an **interpreter** that pins the execution details determining the answer (partition topology, merge order for order-sensitive sketches) and reproduces them faithfully;
- **monitoring** reporting each plan on three tiers of portability:
  1. plan-intrinsic — total state size, passes, merges, error vs. an exact baseline;
  2. interpreter-affinity — parallel width and merge algebra → which interpreter *class* the plan favours (a directional hypothesis, to be validated, deletable);
  3. interpreter-dependent — wall-clock etc., with the interpreter an explicit dimension (>=2 interpreters that accept an injected plan, plan held fixed).

Today this is a problem definition and a measurement model, worked out through the ten example queries below on real data; the running toolkit is still preliminary.

## Table Header

The examples use Click-bench `hits` table (a real, anonymized web-analytics log, heavy-tailed) for one reason: a real table is better than something coming out of my mind.
Only the header is used.
The real data can be generated/synthetic.
Some columns may have different meaning from actual interpretation (I tried to look for documentation, but not a lot)(but it's not a problem, I just need the column to exist, not how the column is interpreted).

References (present so that no claim below is made off the top of our head)(and also a source that I can check):

- Canonical schema (the column list): ClickBench `create.sql` —
  `https://github.com/ClickHouse/ClickBench/blob/main/clickhouse/create.sql`.
- Dataset docs: ClickHouse example datasets, "Anonymized Web Analytics (Metrica)" —
  `https://clickhouse.com/docs/getting-started/example-datasets/metrica`.
  Note: this page lists the columns but gives **no per-column descriptions**.

The header below is a selected subset of the ~100 columns, annotated with (Claude) measured cardinality:

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
ResponseEndTiming  Int32      -- numeric timing column (ms), heavy-tailed; 74.6% are 0, 28,376 distinct among >0
SendTiming         Int32
ConnectTiming      Int32
```

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

### Example 9: top-10 search phrases by distinct users — a provably sound cheap plan

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
  - Caveat on the group key: `SearchPhrase` is raw text, so `"смотреть онлайн"` and `"смотреть онлайн "` are different groups.
    A real pipeline normalizes (lowercase, trim, stem) upstream.
    That is a data-cleaning step, identical for exact and approximate plans, and orthogonal to everything below.

- **Physical plan (from ClickHouse `EXPLAIN`):**

```text
Limit 10
 └─ Sorting (uniq(UserID) DESC, Limit 10)
     └─ Aggregating (Keys: SearchPhrase; Aggregates: uniq(UserID))
         └─ ReadFromMergeTree (Prewhere: notEmpty(SearchPhrase))
```

  - The plan keeps one count-distinct state **per phrase**, and `SearchPhrase` is a very high-cardinality key.
    Measured: 6,019,102 distinct non-empty phrases.
    One HLL per phrase is millions of 16KB states — not affordable — so the naive one-state-per-group plan is off the table, and a two-level plan is forced.

- **How approximation plays here — a composed plan that is cheap *and* provably correct:**
  - The two-level plan: first run a cheap frequency sketch on **hit counts** to shortlist candidate phrases, then build an HLL only for each candidate, then take the top-10 by HLL distinct-user estimate.
  - The reason this is sound is a hard bound: for any phrase, `distinct_users <= hits`.
    Hit count is therefore an **upper bound** on distinct-user count, so a shortlist "phrases with hits >= t" can never drop a phrase whose distinct-user count is >= t.
    The frequency prefilter yields a *safe superset*, never a lossy filter.
  - This gives a sound adaptive stopping rule (a threshold-algorithm argument): walk phrases in descending hit order, building HLLs; stop once the 10th-best HLL estimate so far is >= the hit count of the next un-examined phrase, because no un-examined phrase can then beat it.
  - Measured, the shortlist is tiny.
    The 10th-largest distinct-user count is 7,572, and only **17** of the 6,019,102 phrases have hits >= 7,572.
    So building ~17 HLLs (a few hundred to be safe) instead of 6 million answers the query exactly, with a correctness guarantee.

| rank by users | phrase | users | hits | rank by hits |
| --- | --- | --- | --- | --- |
| 1 | карелки | 23,673 | 70,263 | 1 |
| 5 | смотреть | 14,603 | 19,707 | 5 |
| 10 | комбинирование смотреть | 7,572 | 9,545 | 12 |

  - The ranking does reshuffle (the true #10 by users is #12 by hits, since the hits-per-user ratio varies ~2.6x across phrases), so the frequency order is *not* the final answer.
    But because it is a safe superset, the HLL pass over the shortlist recovers the exact top-10.

- **What the benchmark measures, and why it is AQP BM:**
  - This is the one positive composition in the set: a plan that no single sketch expresses, that is far cheaper than the naive plan, and that is **provably correct** rather than merely close.
  - The soundness rests entirely on a middle-layer fact — the `distinct_users <= hits` bound relating two different aggregates — plus the stopping rule built on it.
    The sketch API knows nothing about this bound; it only knows how to estimate one aggregate at a time.
  - So the benchmark measures the composed plan's cost against the naive plan, and, unlike every negative example here, certifies its answer as exact.
    A raw sketch benchmark cannot even state the plan, let alone its correctness argument.

### Example 10: a fan-out join is safe for count-distinct but corrupts quantile

```sql
-- cohort = users who EVER visited on mobile; then summarize ALL their pageviews
SELECT COUNT(DISTINCT h.UserID)            AS cohort_users,
       quantile(0.99)(h.ResponseEndTiming) AS p99
FROM   hits h
JOIN  (SELECT UserID FROM hits WHERE IsMobile = 1) m   -- a bag: one row per mobile pageview, NOT distinct
  ON   h.UserID = m.UserID
WHERE  h.ResponseEndTiming > 0;
```

Both sides come from `hits`; there is no invented table.
The right side is a cohort of users (those who ever went mobile), and the query reports how many such users there are and the p99 page-load time across all of their pageviews.

- **What it does, and who asks it:**
  - Cohort analytics: take the users who ever did X, then summarize all of their behaviour.
    Here X is "visited on mobile at least once".
  - This is the normal shape of a retention or segment query, and it is naturally a join of the fact table to a user cohort — both drawn from the same log.

- **The footgun that makes it interesting:**
  - The cohort is meant to be a *set* of users.
    The correct form is a semi-join: `WHERE h.UserID IN (SELECT UserID FROM hits WHERE IsMobile = 1)`, or `SELECT DISTINCT UserID`.
  - Written as `JOIN (SELECT UserID FROM hits WHERE IsMobile = 1)` without `DISTINCT`, the right side is a *bag*: one row per mobile pageview.
    So each `hits` row fans out by the number of mobile pageviews that user made.
  - Measured, that fan-out is real and non-uniform: the cohort is 2,208,170 users over 10,172,424 mobile pageviews, and per-user multiplicity is median 2, p99 47, max 2,489.

- **Physical plan (described):**

```text
Aggregating (Aggregates: uniq(h.UserID), quantile(0.99)(h.ResponseEndTiming))
 └─ Join (h.UserID = m.UserID)                       -- fan-out: the right side is a bag
    ├─ ReadFromMergeTree (default.hits)
    └─ ReadFromMergeTree (default.hits, Prewhere: IsMobile = 1)
```

  - The eager-aggregation rewrite (push the aggregate below the join, the reason mergeable sketches are attractive at all) would pre-compute one sketch per `UserID`, then fold each user's state in once per matching right row — i.e. fold it (that user's mobile-pageview count) times.

- **How approximation plays here — the same join is safe for one column and wrong for the other. Measured:**
  - `COUNT(DISTINCT h.UserID)` is 2,208,170 with or without the fan-out.
    A set does not care that a user's rows were duplicated; the duplicates collapse.
  - `quantile(0.99)(h.ResponseEndTiming)` moves from **29,095** (each pageview counted once) to **22,223** under the fan-out weighting — a **23.6%** error — because heavy-activity users are now counted dozens of times and drag the distribution down.
  - The sketch layer mirrors this exactly: HLL merge is idempotent (`merge(H, H) = H`), so folding a user's state more than once is harmless; KLL and t-digest merge is additive (`merge(K, K)` doubles the weight), so folding more than once corrupts the quantile.

- **What the benchmark measures, and why it is AQP BM:**
  - The same plan is **correct for the `COUNT(DISTINCT)` column and wrong for the `quantile` column**, in one query, over one join, on one table.
    The word `merge` is hiding two different algebras: an idempotent set union and an additive multiset union.
  - The current trait has exactly one `merge` signature, which cannot say which algebra it implements:

```rust
fn merge(&self, left: &mut Self::State, right: Self::State);
```

  - Sketches are attractive precisely because they are mergeable — pre-aggregate once, merge everywhere (the sketch-cube / rollup promise).
    This example is the governance rule on that promise: reuse-by-merge is safe under fan-out only for idempotent merges.
    The middle layer needs to carry the merge algebra so a planner knows which push-downs are sound; the sketch API alone does not expose it.
    Seeing this needs a fan-out join and two different aggregates side by side — which a raw sketch benchmark never has.

- Open questions this raises:
  - Should the interface expose the merge algebra (idempotent / additive) as a property, so a planner knows which push-downs are safe?
  - Theta sketches carry a richer algebra (union, intersection, difference).
    Are they the only count-distinct family whose merge composes with relational operators?
  - The exact baseline is not immune: an exact multiset quantile is corrupted by the same fan-out (29,095 -> 22,223).
    The corruption is a property of the aggregate's algebra, not of approximation — which is itself the point.
    The sketch merely inherits whichever algebra its state has.


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
- AQPBMV2 does not benchmark Spark / Trino / BigQuery / ClickHouse directly; those systems only show the functionality classes are real.
- Research bar: AQPBMV2 is a real contribution only if its plans reveal behavior raw sketch benchmarks miss; otherwise it is a useful toolkit but a weaker paper.

## AQPBMV2 Preliminary Result

`cargo run -q -p aqp-core --example aqpbmv2_functions` — a smoke test, not benchmark-grade evidence.

- **What runs**: all three functionality classes go through the one interface, three libraries each, against an exact baseline, under both `grouped_state` and `partitioned_merge`:
  - count distinct — DataSketches / `sketch_oxide` / `asap_sketchlib` HLL;
  - heavy hitters — DataSketches FrequentItems / `sketch_oxide` SpaceSaving / `asap_sketchlib` CMSHeap;
  - quantile — DataSketches TDigest / `sketch_oxide` TDigest / `asap_sketchlib` KLL.
- **Input**: a hand-written synthetic generator (rows assigned to groups by `idx % group_count`), not from a query planner or real schema — so these numbers are only about the toy groups. Count distinct: 50k rows / 32 groups / value cardinality 20k. Heavy hitters: 80k rows / 24 groups / top-3 ~45/20/15%. Quantile: 80k rows / 16 groups / p95.
- **Results** (mean / max relative error; heavy hitters report exact top-k recovery):

| functionality | DataSketches | `sketch_oxide` | `asap_sketchlib` |
| --- | --- | --- | --- |
| count distinct | 0.76% / 2.40% | 0.87% / 2.27% | 0.55% / 1.47% |
| heavy hitters | top-k exact | top-k exact | top-k exact |
| quantile | ~0 | 0.39% / 0.63% | 0 |

  - Every count-distinct and quantile group is within threshold; every heavy-hitter run returns the true top-k with none missing.
- **What it does / does not prove**: the wiring works and libraries compare under the same modes; it does **not** show one library is better, that AQPBMV2 is benchmark-grade, or that it reveals anything a raw sketch benchmark misses — the generated data is too easy.
- **Next**: replace toy generators with real workloads (Uniform / Zipf / Lognormal / Pareto; knobs for group cardinality, size skew, tail, top-k gap, filter selectivity, key–value correlation, partition/merge shape) and add resource/performance measurement (throughput, latencies, state size).

## Previous AQPBMV1

- AQPBMV1 is the existing raw-sketch benchmark layer.
  - Target: sketch primitives directly.
  - Input control: data shape and distribution.
  - Metrics: throughput, accuracy, and memory usage.
  - Role: provide primitive-level evidence for AQPBMV2.

## Existing Approximate Query Support

Approximate aggregates are shipped widely, in fragmented forms (system functions, sketch-state APIs, UDFs, standalone libraries) — which is what motivates a middle layer. Not exhaustive.

- **DataFusion**: `approx_distinct`, `approx_median`, `approx_percentile_cont[_with_weight]` (t-digest). <https://datafusion.apache.org/user-guide/sql/aggregate_functions.html>
- **Trino**: `approx_distinct`, `approx_most_frequent`, `approx_percentile`, `numeric_histogram`, HLL state functions (`approx_set`, `merge`). <https://trino.io/docs/current/functions/aggregate.html>
- **Spark SQL**: `approx_count_distinct`, `approx_percentile`, `count_min_sketch`, HLL and KLL sketch functions. <https://spark.apache.org/docs/latest/api/sql/index.html>
- **BigQuery**: `APPROX_COUNT_DISTINCT`, `APPROX_QUANTILES`, `APPROX_TOP_COUNT`, `APPROX_TOP_SUM`. <https://docs.cloud.google.com/bigquery/docs/reference/standard-sql/approximate_aggregate_functions>
- **ClickHouse**: `uniq` / `uniqCombined` / `uniqHLL12` / `uniqTheta` (+ `uniqThetaIntersect/Union/Not`); t-digest and GK quantiles. <https://clickhouse.com/docs/sql-reference/aggregate-functions/reference>
- **Snowflake**: HLL, MinHash/similarity, approximate top-k and percentile, accumulate/combine/estimate style. <https://docs.snowflake.com/en/sql-reference/functions-aggregation>
- **Druid**: DataSketches Theta / HLL / Quantiles aggregators. <https://druid.apache.org/docs/latest/querying/aggregations/>
