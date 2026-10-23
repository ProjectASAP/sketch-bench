# How to read the error-vs-N figures

These figures come from the **reduced run** in `docs/saturation_study.md`: one
config per sketch (CMS/CountSketch/top-k rows=3 cols=1024, HLL lg_k=12,
KLL k=200, DDSketch alpha=0.01), N up to 1e7, 3 seeds (10 seeds for KLL and
DDSketch), Pareto values at f64. `cost_cpu_memory.png` (see `README_cost.md`)
is the serial cost of the same run. The config-grid / 1e9 figures are in
`../grid_1e9/`.

Each figure has one sketch at one config. Each panel shows one data distribution:
- **Rows** are the number of distinct keys, K = 1K / 100K / 10M.
- **Columns** are the Zipf skew θ = 0 … 2. For quantile sketches the columns are the Pareto tail index α instead.

What the marks mean:
- The x-axis is the number of items inserted, N, on a log scale.
- The blue line is the mean error over 3 seeds, and the band is ±2 standard errors.
- The dotted gray line is the theoretical bound, whose value is printed in the panel title.
- The dashed orange line is N_saturation: the first N after which every later error stays within max(10% of the plateau, 2 SE) of the mean of the last three points.

Every panel has its own y-scale.

**Common theme.** None of the six theoretical bounds depends on N. For every sketch, the error normalized by N (or by the key's frequency) is bounded by a quantity that depends only on the sketch configuration and on the distribution. That distribution is θ and K, or α. Where the empirical error still moves with N, the cause is one of two finite-N effects:
- **Sampling noise in the ground truth.** When N is small, the "true" ranks and counts are themselves noisy.
- **The tail filling in.** While N ≲ K, new keys keep arriving, and they add collision mass the sketch has not seen yet.

N_saturation measures how long these effects last.

Caveat on N_sat: the rule compares against the mean of the last three points. When a curve is still trending at N = 10⁷, the marker can land too early. Read the curve, not just the marker, and treat "not saturated" as "not by 10⁷", not as "never".

---

## CMS (rows=3, cols=1024) · `are_top100`

**Metric.** The mean relative error |f̂ − f| / f over the true top-100 keys. Lower is better.

**Bound.** CMS guarantees f̂ − f ≤ εN with ε = e/w (probability ≥ 1 − e⁻³). Since f_i ≈ N·p_i, key i's relative error is ≤ ε/p_i, and the bound is the mean of that over the top 100. It is **N-independent** and depends on θ and K.

**What the panels show:**
1. **Rise, then plateau.** CMS only ever over-counts, and the over-count comes from all the other keys hashed into the same cell. At small N few keys have arrived, so there are few collisions. The error climbs as the other keys fill the cells, then flattens once the collision mass grows in proportion to N. N_sat marks that point, typically 10⁴–10⁶.
2. **The plateau falls steeply with θ.**
   - K = 100K: about 3.6 at θ = 0.5, 0.25 at θ = 1, and about 0.005 at θ = 2.
   - With more skew, the top-100 keys hold more of the mass, so the same collision noise is small relative to their counts.
   - **For CMS, the worst case is the lower θ bound.**
3. **At θ = 0 with K ≥ 100K the error never saturates.** It reaches 70 (K = 100K) and 1,200 (K = 10M) at N = 10⁷ and is still rising. Under a uniform distribution the true top-100 counts only grow once N ≫ K, while the collision noise grows with N from the start. The sketch cannot saturate before N is several times K.
4. **At high θ the error is ≈ 0 at small N and rises late.** For example, N_sat ≈ 2×10⁶ at θ = 2. Very few distinct keys have appeared, so nothing collides until the rare tail keys start arriving.
5. **The bound is very loose at high θ.** It is 14.8 at θ = 2 against a measured 0.005, because it is set by the rank-100 key's tiny p_i. Use it as a worst-case ceiling, not as a predictor.

**Takeaway for configuration.** Size w for the lowest θ the workload can show. Also make sure the per-window N is well past N_sat. For near-uniform keys with large K, CMS with w = 1024 is unusable.

---

## CountSketch (rows=3, cols=1024) · `are_top100`

**Metric.** Same as CMS.

**Bound.** |f̂ − f| ≤ ε·‖f‖₂ with ε = √(3/w). The relative bound is ε·‖p‖₂/p_i, averaged over the top 100. It is **N-independent** and depends on θ and K.

**What the panels show:**
1. **A different shape from CMS: a hump, then decline to the plateau.** This is most visible at K = 10M, θ = 0.5–1.
   - CountSketch uses signed counters, so collisions cancel on average. Its error scales with the L2 norm of the counts, not their sum.
   - With finite N, E[‖f‖₂²] ≈ N²·Σp² + N. The relative error therefore behaves like √(Σp² + 1/N)/p_i.
   - While 1/N dominates Σp², which happens at small N and large K, the error **falls as more data arrives**. It settles on the N-independent plateau once N ≫ 1/Σp².
   - The initial rise at the smallest N is the same few-keys-seen effect as CMS.
2. **θ = 0 grows with N and never saturates.** The error is 0.35 at K = 1K, 2.4 at 100K and 5 at 10M by N = 10⁷. Under a uniform distribution the top-100 counts stay near the Poisson noise level until N ≫ K, while ‖f‖₂ grows like √N.
3. **The plateau is much lower than CMS for low θ.** At θ = 0.5, K = 100K it is about 0.33 against CMS's 3.6. This is the L2-vs-L1 advantage on flat distributions. For θ ≥ 1.5 both sketches sit near 0.01–0.03.
4. **Measured values stay well below the bound** everywhere (the bound is off-scale at θ = 2).

**Takeaway.** For low-skew, high-cardinality keys, CountSketch beats CMS at the same memory. It also needs a *minimum* N (of order 1/Σp²) before its error drops. A short window x can therefore hurt CountSketch in a way it does not hurt CMS.

---

## CMS-heap top-k (rows=3, cols=1024, heap = 32) · `precision@32`

**Metric.** The share of the 32 reported keys that are in the true top 32. **Higher is better.**

**Bound (lower bound).** The share of true top-32 keys with p_i − p₃₃ > e/w. Such a key cannot be pushed below the 33rd key by CMS over-estimation. The bound is **N-independent**, because both the gap and the CMS error scale with N. The curves should stay above it, and they all do.

**What the panels show:**
1. **θ = 0: precision ≈ 0 and the bound is 0.** Under a uniform distribution the "true top 32" is just random fluctuation, far below the CMS noise. Top-k is ill-defined here; the sketch is not at fault. The 0.6 at K = 1K and small N comes from ties among keys seen once or twice.
2. **θ ≥ 0.8 with K ≤ 100K: precision ≈ 1 almost immediately** (N_sat ≈ 10³–10⁴). The heads stand far above rank 33.
3. **θ = 0.5, or K = 10M: precision rises with N.** For example, θ = 0.5, K = 10M goes from 0 below 10⁵ to 0.33 at 10⁷ and is not saturated. When N ≪ K most keys have been seen 0–1 times, so the true ranking is itself noise, and the head only emerges with more data.
4. **K = 10M, θ = 0.8–1.2: precision rises, then falls after N ≈ 10⁶** (θ = 1: 0.97 → 0.72). The 10M-key tail only fills in as N approaches K. Its growing collision mass inflates tail keys' estimates, and those tail keys evict the true rank 25–32 keys, whose gap to rank 33 is small. These curves are not saturated by 10⁷, even where an N_sat marker is drawn (θ = 1.2).
5. **θ ≥ 1.5: ≈ 1 everywhere.** The lower bound (0.41–0.56) is loose.

**Takeaway.**
- Use the lower θ bound.
- Top-k with θ ≤ 0.5 is not meaningful.
- For large K near θ ≈ 1, precision degrades as the per-window N approaches K. Either widen w or shorten the window.
- Separately, insert CPU rises steeply with θ: about 150 → 2,100 ns/item (see the cost figure).

---

## HLL (lg_k = 12, m = 4096) · `relative_error`

**Metric.** |estimate − true distinct count| / true distinct count.

**Bound.** Standard error σ = 1.04/√m = 1.63%, so the dotted line is at 2σ = 3.25% (about 95% coverage). It is **independent of N**. HLL's error depends only on the number of distinct keys D seen so far. Below about 2.5m ≈ 10K distinct keys, HLL switches to linear counting, which is more accurate.

**What the panels show:**
1. **K = 1K: the error becomes exactly constant (≈ 1.1%, band collapses to 0) once all 1,000 keys have been seen.** HLL is deterministic in the key set, and every seed eventually inserts the same set. More N changes nothing.
2. **K ≥ 100K: the error wanders within the 2σ band at every N.** D keeps growing, and each new D gives a different, essentially random estimate error of about σ. There is no real trend with N. The N_sat markers here mostly reflect noise, not saturation.
3. **θ = 2: the error is 0 at small N.** Only a handful of distinct keys have appeared, and linear counting is exact.
4. All points are within the 3.25% bound except one point, which touches it. That is expected at 95% coverage.

**Takeaway.** For HLL, "N_saturation" should be read as "the distinct count has stopped growing". The accuracy is set by lg_k alone, and the question for configuration is only the D per window (memory is fixed at 4 KB here).

---

## KLL (k = 200) · `mean_rank_err` (10 seeds)

**Metric.** Mean over the 101 quantiles 0.00…1.00 of |rank(estimate) − q·N| / N.

**Bound.** Normalized rank error ε ≈ 2.296/k^0.9723 = 1.33% (DataSketches formula, 99% confidence, single quantile) — **N-independent** (the error is already normalized by N) and **distribution-independent** (KLL is comparison-based).

**What the panels show.**
1. **Lower at the smallest N** (few compactions), then **flat at ≈ 3×10⁻³ from about N ≈ 5×10³** — roughly 4× below the bound.
2. **No dependence on α**: all four columns sit at the same level, as theory predicts — the tail shape does not matter to KLL's rank error.
3. **The N_sat markers (2×10⁵ – 2×10⁶) are later than the eye would place them (≈ 5×10³).** Even with 10 seeds the curve wiggles ±15% around the plateau — KLL's compaction randomness at each N is larger than 2 SE of the seed mean — so the "every later point in band" rule keeps getting broken until near the end. For KLL, read the plateau from the curve; the rule's N_sat is conservative.

**Takeaway.** KLL's accuracy is set by k and is reached after a few thousand items; Pareto α affects neither its error nor (see the cost figure) its memory or CPU.

---

## DDSketch (α_dd = 0.01) · `mean_rank_err` (10 seeds)

**Metric.** Same as KLL. Note DDSketch's own guarantee is on **value** error (relative error ≤ α_dd), not rank error.

**Bound (derived here, not quoted from a paper).** A relative value error α_dd at value x shifts rank by ≈ pdf(x)·α_dd·x; for Pareto(a), pdf(x)·x = a·(1 − q), so averaged over the quantile grid: mean rank error ≤ α_dd·a/2. **N-independent**, grows with a.

**What the panels show.**
1. **Error falls slightly with N and is flat by 10³–10⁴**; N_sat = 10³ – 3×10³, the earliest of all sketches. The 10-seed band is very tight.
2. **Error increases with α**: 2.4e-3 (α = 1.1), 3.8e-3 (1.5), 4.9e-3 (2), 7.2e-3 (3), tracking the bound's ∝ a. A lighter tail packs more mass near the minimum, so the same relative value error spans more ranks. **For DDSketch rank error, heavier tails are easier.**
3. **Memory goes the other way** (cost figure): 6.3 KB at α = 1.1 down to 2.3 KB at α = 3 — a heavier tail spans a wider value range and needs more buckets.
4. Measured values are ≈ 2× below the bound everywhere.

**Takeaway.** DDSketch saturates almost immediately. For configuration, use the *upper* α bound (lightest tail) from task 2 for rank error and the *lower* α bound (heaviest tail) for memory.
