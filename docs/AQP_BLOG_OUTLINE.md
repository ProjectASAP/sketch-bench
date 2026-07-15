# Blog Outline: AQPBM

> Status: design doc for AQPBMV2.
> AQPBMV2 locates at the physical plan.
> AQPBMV2 monitors possible physical plans.
> 10 example queries on the ClickBench `hits` table with what the query is and what can be benchmarked around those queries.
> Open: build the running interpreter/toolkit; add interpreters beyond DataFusion; transcribe real engines' physical plans as reference baselines.
>
> Goal: explain the problem scope of AQPBM and the contribution of the current AQPBMV2 design.

## Concrete Problem Definition

AQPBM is trying to understand how much speedup and memory reduction can be gained, and how much accuracy needs to lose, from introducing approximation into queries.

### Components

#### Data Source

- Synthetic data generation under different distribution.
- Effect of order of data is unknown.
- Real data set

#### Queries

- Real queries
- Hand-made representative queries

#### Physical Layer / Execution Plan

- multiple possible physical layer for each query in previous section
  - **Notice**: the physical layer is **not** automatically translated from queries in previous section
    - if the translation is automatic, **that's great**s (i.e., ASAPController)
    - after all, hand written physical layer is fine

#### Monitor

- Performance: throughput/latency/wall clock/etc.
- Accuracy: relative error/rank error (when applicable)/etc.
- Resource: storage overhead/memory usage/etc.
- **Interpreter of Physical Layer**: if this is avoidable, definitely avoid this; how to monitor previous metrics, I have no idea at this moment

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
<!-- - [Example 10: a fan-out join is safe for count-distinct but corrupts quantile](#example-10-a-fan-out-join-is-safe-for-count-distinct-but-corrupts-quantile) -->

## TL;DR

**The middle layer is around the physical plan, and AQPBMV2 measures it.**

- Sketch or exact data structures are interchangeable.
- How sketch or exact data structures together serve some functionality is the middle layer.
- A physical plan is a good fit: already optimized, but not executed.

**Why the plan matters under approximation.**

- Different physical plans of one query may give answers with different error.
- It's just how different sketches are composed together and how error builds up.

**What the toolkit must have:**

- an **IR**: physical plan + our annotation/metrics + interpreter (possible, likely to be necessary)
- benchmark tool: how the **IR** actually performs, if it is executed "somehow"

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

#### One line explanation

Count the distinct users in each (region, operating system) cell.

#### Who asks it

- a normal web-analytics question.

#### Physical plan (from ClickHouse `EXPLAIN`)

```text
Output: RegionID, OS, uniqExact(UserID)

Aggregating
│  Keys: RegionID, OS
│  Aggregates: uniqExact(UserID)
│  Skip merging: 0
└──ReadFromMergeTree (default.hits)
      Read type: Default
      Parts: 3 | Granules: 12323
      Output: RegionID, OS, UserID
```

#### How approximation can play

- Option: one HLL per `(RegionID, OS)` group, updated with every `UserID` in the group, finalized to one cardinality per group.
  - But the group sizes decide whether that is a good idea.
  Measured on the real table:
    - Group count: 63,467.
    - Median rows per group: 9.
      Groups with fewer than 100 rows: 52,709 (83%).
    - Largest group: 9,724,473 rows, 1,429,492 distinct users (`RegionID=229, OS=44`).
- A better option: exact state for small groups, sketch state for large ones.
- Another better option: exact state for all groups, and bumps to sketch when the exact state grows.

#### What the benchmark can measure

- Run all-exact, all-HLL, and the size-aware hybrid through the same plan, and report per-group memory and error.
- Overall query throughput.


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

#### One line explanation

Drop the fastest 25% and the slowest 25% of page loads, then report the 5 regions with the most unique users among what is left.

#### Who asks it

- "Which regions have the biggest typical audience"
  - trimming both tails drops bot/cache hits and timeouts, a common de-noising pattern.

#### Physical plan (from ClickHouse `EXPLAIN`)

```text
Output: RegionID, uniqExact(UserID)

Limit (preliminary LIMIT)
│  Limit 5
│  Offset 0
└──Sorting (Sorting for ORDER BY)
   │  Sort description: uniqExact(UserID) DESC
   │  Limit 5
   └──Aggregating
      │  Keys: RegionID
      │  Aggregates: uniqExact(UserID)
      │  Skip merging: 0
      └──ReadFromMergeTree (default.hits)
            Read type: Default
            Parts: 3 | Granules: 12323
            Output: RegionID, UserID
            Prewhere filter
            Prewhere filter column:  ResponseEndTiming <= 231.25 AND ResponseEndTiming >= 15.
```

#### How approximation can play

- quantile can benefit from sketch
- cardinality can benefit from sketch
- but I don't have idea how to composite together yet

#### What the benchmark can measure

- End-to-end cost of the composition and whether the top-5 ranking survives approximation.

### Example 3: top-5 most active users per region

```sql
-- 3a: top 5 users by hit count, per region
SELECT RegionID, UserID, count() AS c FROM hits GROUP BY RegionID, UserID
--     ... keep the top 5 rows per RegionID

-- 3b: identical shape, different target column
SELECT RegionID, URL, count() AS c FROM hits GROUP BY RegionID, URL
--     ... keep the top 5 rows per RegionID
```

#### One line explanation

For each region, report the 5 most frequent values of a column — users in 3a, URLs in 3b.

#### Who asks it

- Top-k users per region: bot/abuse detection, power-user identification.
- Top-k URLs per region: bread-and-butter web analytics. Same query shape, only the target column differs.

#### Physical plan (from ClickHouse `EXPLAIN`)

```text
Output: RegionID, topK(5)(UserID)

Aggregating
│  Keys: RegionID
│  Aggregates: topK(5)(UserID)
│  Skip merging: 0
└──ReadFromMergeTree (default.hits)
      Read type: Default
      Parts: 3 | Granules: 12323
      Output: RegionID, UserID
```

#### How approximation can play

- ClickHouse's `topK` is *already* a heavy-hitter sketch (SpaceSaving)

#### What the benchmark can measure

- the data distribution can affect the accuracy

### Example 4: how many users are on mobile

```sql
SELECT IsMobile, COUNT(DISTINCT UserID) FROM hits GROUP BY IsMobile;
```

#### One line explanation

Split users into mobile vs. non-mobile and count the distinct users on each side.

#### Who asks it

- "How many unique users are on mobile" — a headline metric on any product dashboard.

#### Physical plan (from ClickHouse `EXPLAIN`)

```text
Output: IsMobile, uniqExact(UserID)

Aggregating
│  Keys: IsMobile
│  Aggregates: uniqExact(UserID)
│  Skip merging: 0
└──ReadFromMergeTree (default.hits)
      Read type: Default
      Parts: 3 | Granules: 12323
      Output: IsMobile, UserID
```

#### How approximation can play

- HLL or other sketch for approximate distince

#### What the benchmark can measure

- just another normal query, nothing special, not rely on AQPBMV2

### Example 5: which regions have the most mobile users

```sql
SELECT RegionID, COUNT(DISTINCT UserID) AS mobile_users
FROM   hits
WHERE  MobilePhone != 0
GROUP BY RegionID
ORDER BY mobile_users DESC
LIMIT 5;
```

#### One line explanation

Among mobile visits only, report the 5 regions with the most unique users.

#### Who asks it

- "Where is my mobile audience biggest"
- `MobilePhone` is a vendor/brand id, not a phone number. Measured, `MobilePhone = 0` covers 92,775,562 rows (**92.78%**), so `MobilePhone != 0` keeps only 7.22% of rows.

#### Physical plan (from ClickHouse `EXPLAIN`)

```text
Output: RegionID, uniqExact(UserID)

Limit (preliminary LIMIT)
│  Limit 5
│  Offset 0
└──Sorting (Sorting for ORDER BY)
   │  Sort description: uniqExact(UserID) DESC
   │  Limit 5
   └──Aggregating
      │  Keys: RegionID
      │  Aggregates: uniqExact(UserID)
      │  Skip merging: 0
      └──ReadFromMergeTree (default.hits)
            Read type: Default
            Parts: 3 | Granules: 12323
            Output: RegionID, UserID
            Prewhere filter
            Prewhere filter column:  MobilePhone != 0
```

#### How approximation can play

- Top-K and cardinality can be benefitted from approximation

#### What the benchmark can measure

- how is this different from example 2?

| | Example 5 | Example 2 |
| --- | --- | --- |
| predicate | `MobilePhone != 0` | `ResponseEndTiming BETWEEN lo AND hi` |
| known before the scan? | yes — a constant | no — `lo`/`hi` do not exist until a quantile state is finalized |
| passes required | one | two, or one with state fan-out |

- then, what can benchmark show?

### Example 6: median event time per site

```sql
SELECT CounterID, quantile(0.5)(EventTime) AS median_t
FROM   hits
GROUP BY CounterID;
```

#### One line explanation

For each site, find the median moment of its traffic during the month.

#### Who asks it

- "When during the month was this site's traffic centred" — a real site-analytics question (e.g. spotting activity clustered around a launch).

#### Physical plan (from ClickHouse `EXPLAIN`)

```text
Output: CounterID, quantile(0.5)(EventTime)

Aggregating
│  Keys: CounterID
│  Aggregates: quantile(0.5)(EventTime)
│  Skip merging: 0
└──ReadFromMergeTree (default.hits)
      Read type: Default
      Parts: 3 | Granules: 12323
      Output: CounterID, EventTime
```

#### How approximation can play

- quantile sketch

#### What the benchmark can measure

- the quantile sketch have different behavior

### Example 7: p99 latency per site

```sql
SELECT CounterID, quantile(0.99)(ResponseEndTiming) AS p99
FROM   hits
WHERE  ResponseEndTiming > 0
GROUP BY CounterID;
```

#### One line explanation

For each site, estimate the 99th-percentile page-load time.

#### Who asks it

- latency dashboard

#### Physical plan (from ClickHouse `EXPLAIN`)

```text
Output: CounterID, quantile(0.99)(ResponseEndTiming)

Aggregating
│  Keys: CounterID
│  Aggregates: quantile(0.99)(ResponseEndTiming)
│  Skip merging: 0
└──ReadFromMergeTree (default.hits)
      Read type: Default
      Parts: 3 | Granules: 12323
      Output: ResponseEndTiming, CounterID
      Prewhere filter
      Prewhere filter column:  ResponseEndTiming > 0
```

#### How approximation can play

- One quantile state per site; candidates differ in what they guarantee.
- KLL guarantees **rank error**, no promise on value; DDSketch guarantees **relative value error**, no promise on rank.
- On this right-skewed latency a 1% rank error is harmless at p50 but blows up to ~**92%** *value* error at p99 (the tail is sparse)

#### What the benchmark can measure

- same as example 6
- but, the data shape can matter

### Example 8: audience overlap

```sql
-- distinct users who visited BOTH site A and site B
SELECT COUNT(DISTINCT UserID) FROM (
    SELECT UserID FROM hits WHERE CounterID = 199550
    INTERSECT
    SELECT UserID FROM hits WHERE CounterID = 105857
);
```

#### One line explanation

Count the distinct users who visited **both** site A and site B.

#### Who asks it

- Audience overlap between two properties

#### Physical plan (from ClickHouse `EXPLAIN`)

```text
Output: uniqExact(UserID)

Aggregating
│  Keys:
│  Aggregates: uniqExact(UserID)
│  Skip merging: 0
└──IntersectOrExcept
   ├──ReadFromMergeTree (default.hits)
   │     Read type: Default
   │     Parts: 2 | Granules: 875
   │     Output: UserID
   │     Prewhere filter
   │     Prewhere filter column:  CounterID = 199550
   └──ReadFromMergeTree (default.hits)
         Read type: Default
         Parts: 2 | Granules: 708
         Output: UserID
         Prewhere filter
         Prewhere filter column:  CounterID = 105857
```

#### How approximation can play

- HLL has **union** (registers max-ed) but **no intersection**
  - one route is inclusion-exclusion `|A∩B| = |A|+|B|-|A∪B|`
- Measured on the two largest sites: each HLL is accurate to ~1%, but the intersection (only 2.1% of the union) comes out **58.5%** off (needs manual check the data)
- Theta has a *native* intersection (~2x better) but is still **27.8%** off (needs manual check the data)

#### What the benchmark can measure

- End-to-end intersection error vs single sketch error and HLL-inclusion-exclusion vs Theta-native

### Example 9: top-10 search phrases by distinct users
```sql
SELECT SearchPhrase, COUNT(DISTINCT UserID) AS users
FROM   hits
WHERE  SearchPhrase != ''
GROUP BY SearchPhrase
ORDER BY users DESC
LIMIT 10;
```

#### One line explanation

Report the 10 search phrases with the most distinct users.

#### Who asks it

- "What are people searching for", ranked by **unique users** not raw hits, so one bot hammering a phrase does not dominate

#### Physical plan (from ClickHouse `EXPLAIN`)

```text
Output: SearchPhrase, uniqExact(UserID)

Limit (preliminary LIMIT)
│  Limit 10
│  Offset 0
└──Sorting (Sorting for ORDER BY)
   │  Sort description: uniqExact(UserID) DESC
   │  Limit 10
   └──Aggregating
      │  Keys: SearchPhrase
      │  Aggregates: uniqExact(UserID)
      │  Skip merging: 0
      └──ReadFromMergeTree (default.hits)
            Read type: Default
            Parts: 3 | Granules: 12323
            Output: SearchPhrase, UserID
            Prewhere filter
            Prewhere filter column:  notEmpty(SearchPhrase)
```

#### How approximation can play

- cardinality sketch and top-k sketch
- but when to create the sketch is a question
  - creating a hll for each SearchPhrase is not good

#### What the benchmark can measure

- sketch composition methods may affect the result

<!-- ### Example 10: a fan-out join is safe for count-distinct but corrupts quantile

```sql
-- cohort = users who EVER visited on mobile; then summarize ALL their pageviews
SELECT COUNT(DISTINCT h.UserID)            AS cohort_users,
       quantile(0.99)(h.ResponseEndTiming) AS p99
FROM   hits h
JOIN  (SELECT UserID FROM hits WHERE IsMobile = 1) m   -- a bag: one row per mobile pageview, NOT distinct
  ON   h.UserID = m.UserID
WHERE  h.ResponseEndTiming > 0;
```

#### One line explanation

For users who ever visited on mobile, count them and report the p99 of all their page loads. Both sides come from `hits` — the fact table joined to the mobile-user cohort.

#### Who asks it

- Cohort analytics: pick users by one behaviour, then summarize all their activity.
- Footgun: the cohort should be a *set*, but `JOIN (SELECT UserID ... WHERE IsMobile=1)` without `DISTINCT` is a *bag* — one row per mobile page-view — so each user's rows get duplicated by how many mobile visits they made (measured: up to 2,489× for one user).

#### Physical plan (from ClickHouse `EXPLAIN`)

```text
Output: uniqExact(UserID), quantile(0.99)(ResponseEndTiming)

Aggregating
│  Keys:
│  Aggregates: uniqExact(UserID), quantile(0.99)(ResponseEndTiming)
│  Skip merging: 0
└──Join (JOIN FillRightFirst)
   │  h ⋈ m
   │  Type: inner | Strictness: all | Algorithm: SpillingHashJoin(ConcurrentHashJoin)
   │  Join conditions: UserID = UserID
   │  Output:
   │    Left:  UserID, ResponseEndTiming
   │    Right: Empty
   ├──ReadFromMergeTree (default.hits)
   │     Read type: Default
   │     Parts: 3 | Granules: 12323
   │     Output: UserID, ResponseEndTiming
   │     Prewhere filter
   │     Prewhere filter column:  ResponseEndTiming > 0
   │     Runtime filters: RF1(UserID, UserID from default.hits)
   └──BuildRuntimeFilter (Build runtime join filter on UserID)
      │  Filter id: RF1
      │  Source table: default.hits
      └──ReadFromMergeTree (default.hits)
            Read type: Default
            Parts: 3 | Granules: 12323
            Output: UserID
            Prewhere filter
            Prewhere filter column:  IsMobile = 1
```

#### How approximation can play

- The duplication hits the two aggregates differently. `COUNT(DISTINCT)` ignores it (a set drops duplicates — 2,208,170 either way); `quantile(0.99)` is corrupted, 29,095 → 22,223 (**23.6%** off), because heavy users now count many times.
- Sketches inherit this: HLL merge is idempotent, so double-counting a state is harmless; a quantile sketch's merge is additive, so double-counting corrupts it.

#### What the benchmark can measure

- One plan is correct for `COUNT(DISTINCT)` and wrong for `quantile` — so the middle layer must know each state's merge algebra (idempotent vs additive) to tell which plan rewrites are safe. -->

<!-- 
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
- **Next**: replace toy generators with real workloads (Uniform / Zipf / Lognormal / Pareto; knobs for group cardinality, size skew, tail, top-k gap, filter selectivity, key–value correlation, partition/merge shape) and add resource/performance measurement (throughput, latencies, state size). -->

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
