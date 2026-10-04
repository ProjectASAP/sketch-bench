# Saturation study: conclusions and why

This page has two parts:

- **Conclusions** across three studies:
  - Task 1–2: worst-case data parameters from real traces (ProjectASAP/ASAPQuery#746).
  - Task 3: the sketch saturation and cost study (this PR).
  - Accuracy after merging (#131).
- **The math** behind three of the results.

Method, tables and figures: [`saturation_study.md`](saturation_study.md) and
[`figures/saturation/`](figures/saturation/).

Notation:

| Symbol | Meaning |
|---|---|
| θ | Zipf skew of the keys |
| K | number of distinct keys |
| N | number of items inserted |
| a | Pareto tail index of the values |
| α_dd | DDSketch's relative-accuracy parameter (`alpha=` in configs) |
| k | KLL size parameter |
| w | CMS / CountSketch width (cols) |
| m | HLL register count (2^lg_k) when talking about HLL; otherwise the number of shards merged |

## Conclusions

### Real data (tasks 1–2)

1. **Real group-by keys are skewed, with θ ≈ 1.0–1.5.** This covers Google user / job / priority and Alibaba service / msname / um / dm. Only per-machine or per-node keys, where one key is one series, are near-uniform (θ ≈ 0–0.3).
2. **The worst case is much worse than the average, and longer samples expose it.** For Google `by (user)`, the lower θ bound is 1.22 on a 1.4 h sample but 1.10 on a 7-day sample with 5-minute evaluations. Fitting on a short sample, or on average behaviour, undersizes the sketch.
3. **Instant and range queries see different distributions.** An instant query counts series per group; a range query counts samples per group. For MSRTMCR `by (msname)`, θ is 0.83 for the instant query and 1.44 for 5m/1h ranges, and the instant query needs a 4× larger CMS. Fits must follow the query's semantics.
4. **K per window depends on the time scale.** CallGraph `by (service)` has about 40K keys per minute and 2.77M over 6 h. Google `by (user)` stays at about 500–700.
5. **Most value tails are lognormal or light; few are truly heavy.** The clear heavy tails are call latency `rt` (a ≈ 2) and a few BOOM series. Power law vs lognormal is often undecidable, so the analysis reports a tail class (`light` / `power_law` / `lognormal` / `heavy_inconclusive`) rather than a pass/fail power-law test.

### Sketches (task 3)

6. **No sketch's theoretical error bound depends on N.** Error that still moves with N comes from two small-N effects:
   - the ground truth itself is noisy when N is small;
   - while N < K, new tail keys keep arriving.

   Past N_sat, error is set by the config alone, so a profile from a small window extrapolates to larger windows.
7. **N_sat is mostly 1e3–1e6, with two exceptions:**
   - With uniform keys and large K, CMS and CountSketch never saturate: at K = 1e7 the error is still rising at 1e9.
   - With large K, top-k precision keeps improving up to 1e9.

   So an N_sat measured only to 1e7 is a lower bound when K is large.
8. **Error scales with config size as theory predicts, so configs can be interpolated:**

   | Sketch | Error scales as |
   |---|---|
   | CMS | 1/w (steeper with skew) |
   | CountSketch | 1/√w (about w^(−θ) with skew) |
   | HLL | 1/√m |
   | KLL | 1/k |
   | DDSketch | α_dd |

9. **The data distribution decides which sketch to use:**
   - Low θ is the worst case for CMS and top-k, so size them from the lower θ bound.
   - At low skew and large K, CountSketch beats CMS at the same memory. For CallGraph, CountSketch needs 48 KB where CMS needs 192 KB.
   - Top-k is unreliable for θ ≤ 0.5.
   - KLL's error does not depend on the distribution (why: section 1 below).
   - DDSketch's rank error grows with a while its memory shrinks with a. Size the error from the upper a bound and the memory from the lower a bound (why: section 2 below).
10. **Merging (#131):**
    - CMS, CountSketch, HLL and DDSketch merge exactly.
    - KLL's error rises after merging, more for larger k: about 1.0–1.1× at k = 50/200 and 1.12–1.32× at k = 800 at N = 1e7, and up to 3–4× at N = 1e4–1e5 for k = 800 merged from 4 shards.
    - Top-k is unchanged in the median, but loses up to 40% precision at large K, for every width.

    So single-sketch curves are optimistic for top-k and KLL on merged windows (why: section 3 below).
11. **Cost:**
    - Per-item insert CPU is flat in N and in the distribution: CMS ≈ 22 ns, CountSketch ≈ 26 ns, HLL ≈ 3.4 ns, KLL ≈ 25 ns, DDSketch ≈ 16 ns.
    - The exception is CMS-heap top-k, which grows from about 100 ns at θ = 0 to 1,750 ns at θ = 2.
    - Memory is fixed by the config, except DDSketch's, which grows with heavier tails.
12. **Sketch vs exact computation.** The exact baseline buffers the samples and computes at query time.
    - **Memory:** sketches save 10× after a few thousand items and 100× after 1e4–1e6 items, depending on sketch size.
    - **CPU:**
      - CMS and CountSketch are about 13× cheaper, HLL about 40×; neither reaches 100×.
      - Quantile sketches pass 100× from 1e5–1e6 items, because exact computation sorts.
      - Top-k at high skew is slower per item than exact; its benefit is memory only.
    - Real windows hold millions of items, so sketches save at least one to two orders of magnitude of memory.
13. **Worst-case parameters → configuration is automated** in `scripts/recommend_config.py`. Examples:

    | Query | Recommendation |
    |---|---|
    | Google `by (user)` | CMS 3×1024 (12 KB) |
    | CallGraph `by (service)` | CountSketch 3×4096 (48 KB) |
    | MSRTMCR, instant query | 192 KB |
    | p99 latency | DDSketch α_dd = 0.01 (3–5 KB) |

**Limits:**
- Some recommendations extrapolate beyond N = 1e7.
- Merged accuracy covers every top-k and KLL config, but only up to N = 1e7 and m = 64 shards.
- The exact baseline buffers the stream. A streaming hash-map counter would shrink the frequency sketches' memory advantage.

## Why: the math

### 1. KLL's rank error does not depend on the value distribution

**KLL only compares items.** Each compactor sorts its buffer and keeps the odd- or even-indexed half, chosen by a coin flip; the kept half moves up one level. Every step depends only on the relative order of items and on the coins, never on their values.

**So a strictly increasing transform g leaves the sketch unchanged.** Replace every x by g(x) and every comparison, sort and discard happens the same way. The item returned for a quantile is g of the item that would have been returned before.

**Every continuous distribution therefore behaves like the uniform one.** Let the X_i be i.i.d. with continuous CDF F, and take g = F. Then U_i = F(X_i) ~ Uniform(0, 1), and the sketch state on X equals the sketch state on U.

**Rank error is defined on ranks**, err(q) = |rank(x̂) − qN| / N, and a monotone transform preserves ranks. So the rank error on any continuous F has exactly the same distribution as on uniform data. It depends only on k, N and the coins.

**The bound has no F in it:** with probability at least 1 − δ, err ≤ ε with ε = O((1/k)·√log(1/δ)). DataSketches' empirical form is ε ≈ 2.296 / k^0.9723, which is 1.33% at k = 200.

**Measured:** at k = 200 the mean rank error is about 3×10⁻³ for all four Pareto tails (a = 1.1, 1.5, 2, 3).

**Assumptions:**
- *No ties.* Discrete data creates ties; we floor Pareto values with scale 1000, so ties are rare.
- *Non-adversarial arrival order.* i.i.d. order is exchangeable.

KLL's memory, O(k·log(N/k)), is also distribution-free.

### 2. DDSketch: rank error ∝ a, memory ∝ ln N / a

**How DDSketch works.** It puts x > 0 into bucket ⌈log_γ x⌉ with γ = (1 + α_dd) / (1 − α_dd), and answers a quantile with its bucket's representative x̂. This guarantees |x̂ − x_q| ≤ α_dd·x_q, a bound on the *value* error.

**From value error to rank error.** Near x_q, rank changes at rate N·f(x_q), where f is the density. So, normalized by N:

  rank error(q) ≈ f(x_q)·|x̂ − x_q| ≤ α_dd·x_q·f(x_q).

For Pareto(a, x_m), F(x) = 1 − (x_m/x)^a and f(x) = a·x_m^a·x^(−a−1). Therefore

  **x·f(x) = a·(x_m/x)^a = a·(1 − F(x)) = a·(1 − q)**,

so rank error(q) ≤ α_dd·a·(1 − q). Averaged over the 101-point grid q ∈ [0, 1]:

  **mean rank error ≤ α_dd·a / 2.**

Intuition: a larger a means a lighter tail. More of the data sits near x_m, and a bucket whose width is proportional to x then spans more ranks.

Measured at α_dd = 0.01 (bound in parentheses):

| a | 1.1 | 1.5 | 2 | 3 |
|---|---|---|---|---|
| mean rank error | 2.4e-3 (≤ 5.5e-3) | 3.8e-3 (≤ 7.5e-3) | 4.9e-3 (≤ 1.0e-2) | 7.2e-3 (≤ 1.5e-2) |

The fitted log-log slope against α_dd is 0.98–1.08, as the bound predicts.

**Memory: the bucket count follows the value range.**

  buckets ≈ log_γ(x_max / x_min) = ln(x_max / x_min) / ln γ ≈ ln(x_max / x_min) / (2·α_dd).

- **Minimum:** x_min ≈ x_m.
- **Maximum:** the maximum of N Pareto draws has P(max ≤ x) = (1 − (x_m/x)^a)^N, so typically x_max ≈ x_m·N^(1/a).

Substituting:

  **buckets ≈ ln N / (2·a·α_dd).**

At N = 1e7 and α_dd = 0.01 this predicts about 733 buckets at a = 1.1 and 269 at a = 3, a ratio of 2.7. The measured memory is 6.3 KB vs 2.3 KB, also 2.7.

**Which bound to use for sizing:**

| Concern | Dependence on a | Worst case | Use |
|---|---|---|---|
| Rank error | ∝ a | largest a (lightest tail) | upper a bound |
| Memory | ∝ ln N / a | smallest a (heaviest tail) | lower a bound |

Memory also grows like ln N, so a longer window needs slightly more of it.

### 3. Why merging is exact for CMS / HLL / DDSketch but not for KLL (or top-k)

**CMS, CountSketch, HLL and DDSketch:** the sketch state is a deterministic function of the multiset of items, and merging is associative.
- CMS and CountSketch merge by adding counters (with the same hash functions).
- HLL takes the register-wise max.
- DDSketch adds bucket counts.

Merging m shards therefore produces exactly the sketch you'd get from feeding all N items into one. Measured: 0 of 1,479 cells differ.

**KLL: randomized compaction.**

Where its error comes from: at level h, each item stands for 2^h inputs. A compaction at level h moves any rank by at most 2^h, with a random ± sign of mean 0. The total error is a sum of independent zero-mean terms:

  err(x) = Σ_compactions ±2^h·𝟙[the compaction affects x],
  **Var(err) ≈ Σ_h C_h·(2^h)²**,

where C_h is the number of compactions at level h. KLL's geometrically shrinking capacities keep this sum ≲ (N/k)², i.e. normalized error ≈ 1/k. Most of the variance comes from the top levels, where 2^h is largest.

What merging adds:
1. Each shard ran its own compactions while it held only N/m items, and all of those stay in the result. A single stream might have done them later, at a lower level, or not at all.
2. Concatenating the shards' compactors overfills the levels, especially the top ones, so the merged sketch must compact again there. Those extra compactions carry the largest weights 2^h.

So the merged C_h is larger at the top levels, and the variance Σ C_h·4^h grows by a constant factor. The order of the error does not change: KLL's ε guarantee holds under any merge order, with a somewhat larger constant. More shards (larger m = S/x) mean more extra high-level compactions. The full grid (10 seeds) matches this and adds a dependence on k: about 1.0–1.1× at k = 50/200 but 1.12–1.32× at k = 800 at N = 1e7. With a large k, each shard stays exact (uncompacted) longer, so merging introduces compactions the single stream would have done at lower levels or not at all. That shows up most at intermediate N: up to 3–4× at N = 1e4–1e5 for k = 800 with m = 4, converging by 1e6–1e7.

**Top-k: a different and larger effect.** Each shard's heap keeps only that shard's top 32 keys. A key that is heavy globally but never in any one shard's top 32 is lost at merge time. That is lost information, not added noise. Across all 8 configs the median precision is unchanged, but at large K it drops by up to 40%, and widening the sketch does not remove the loss.

**What this means for configuration:**
- For KLL on merged windows, use the merged curves. The recommender now does this: p99 latency over 5m windows moves from k = 50 to k = 200 to stay under the 1% target.
- For top-k, use the merged curves (#131), not the single-sketch ones.
- KLL's compaction randomness is unseeded. The merge grid uses 10 seeds for that reason.
