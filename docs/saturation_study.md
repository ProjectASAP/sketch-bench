# Saturation Study

How long a stream does each sketch need before its accuracy stops changing?
This study measures that length, N_sat, per sketch, config and data shape,
records the cost of the sketch at the largest N measured, and finds the stream
length N* past which an exact answer costs 10x (100x) more than the sketch.

It is stacked on PR #129 (`dev-milind-turboprom`) and does not change
`rqe-optimizer` or its atomic-cost table; it only adds a Pareto generator
(`--dataset pareto`, at `--dtype f64` or `i64`) and
`scripts/study_saturation.py`.

## Definition of N_sat

Accuracy is scored once, after the full stream, so each checkpoint N is a
separate `approxbench sketchbench --size N` run. For checkpoints
`N_1 < ... < N_m` (log-spaced, `--per-decade` per decade) the error at each
N is averaged over `--seeds` seeds, giving `e_1 .. e_m` with standard errors
`se_1 .. se_m`.

- plateau `P` = mean of the last `--plateau-tail` (T) values of `e`
- checkpoint i is *in band* when `|e_i - P| <= max(tolerance * |P|, 2 * se_i, 1e-9)`
  (`--tolerance`, default 0.10). The `2 * se_i` term keeps seed noise alone
  from breaking the plateau when the error itself is small; the absolute floor
  only matters for P = 0
- **N_sat** = `N_i` for the smallest i such that every checkpoint `i..m` is in band
- **not_saturated** when `m < T`, or when no checkpoint before the tail is in
  band (the tail agreeing with its own mean says nothing)

Because the band is on magnitude, direction does not matter: `precision_at_k`
rises toward 1 while the other metrics fall. N_sat is bounded by `--n-max`: a
sketch whose error is still moving at the top of the grid is reported
`not_saturated`, not "never saturates".

| family | sketches | data | error metric |
|---|---|---|---|
| frequency | `cms-fastpath-vector2d`, `countsketch-fastpath-vector2d` | Zipf(theta) over K keys, i64 | `are_top100` (falls back to `are_all` when fewer than 100 keys were seen) |
| topk | `cms-heap-topk-fastpath-vector2d` | Zipf(theta) over K keys, i64 | `precision_at_k` (k = 32) |
| cardinality | `hll` | Zipf(theta) over K keys, i64 | `relative_error` |
| quantile | `kll-percall`, `dd` | floor(Pareto(alpha, scale 1000)), i64 | `mean_rank_err` |

Zipf with `--zipf-s 0` is uniform over the K keys (rand_distr accepts s = 0),
so theta = 0 needs no separate dataset. The quantile streams are Pareto draws
with minimum 1000 floored to i64 (`--dtype i64 --pareto-scale 1000`: the
datagen renderer truncates, which is a floor for positive draws). i64 because
the polars exact quantile baseline is i64 only; scale 1000 keeps three
significant digits, so ties from flooring are rare. Rank error is scale-free,
so this changes nothing for KLL; for DDSketch the floor adds at most 0.1%
value error, below its alpha.

### Config grid

Each sketch's size knob steps 4x over two decades (theory predicts the plateau
as a power of it):

| sketch | configs | predicted plateau |
|---|---|---|
| CMS, CountSketch, CMS-heap top-k | cols 256, 1024, 4096, 16384 x rows 3, 5 | CMS ∝ 1/w, CountSketch ∝ 1/sqrt(w) |
| HLL (`lib`) | lg_k 12, 14, 16 | ∝ 1/sqrt(2^lg_k) |
| KLL | k 50, 200, 800 | ∝ ~1/k |
| DDSketch | alpha 0.005, 0.01, 0.02, 0.05 | ∝ alpha |

asap HLL is compiled at lg_k 12, 14 and 16 only and refuses 10; its nearest
valid value, 12, is already in the grid, so HLL has three configs (one and a
half decades of m). Every other value is accepted (Vector2D takes any
positive shape, KLL k in [8, 26602], DDSketch alpha in (0, 1)).
`--one-config` keeps the first config of each list (rows=3 cols=256, lg_k=12,
k=50, alpha=0.005).

At the final N of each point one extra run
(`--operations insert,query,merge --metrics throughput,cpu,memory
--merge-shards 16 --runs 3 --warmup-runs 1 --flat`) records insert, merge and
query CPU seconds (user + sys) and `memory_bytes`. `BENCH_WARMUP_SECS`
defaults to 0, so these costs are indicative. The optimizer's costs come
from `--phase optimizer-cost`: each configuration once, serially, at Zipf
θ = 1.1 over 1e4 keys (Pareto `a` = 2 for quantiles) and 1e6 items, with 5
runs after 3 warm-ups and one accuracy pass, reduced by `approxbench
atomic-costs` to `rqe_atomic_costs.json` (#174). Top-k configs are measured at heaps 32, 128, 512 and
2048 (`TOPK_HEAPS`, `heap=`): a deployment merging `m` windows keeps `m · k`
and rqe-optimizer interpolates between them. The accuracy grid measures top-k
at the default heap, `k = 32`. Put it under the curves'
directory as `optimizer_cost/`, where `rqe-optimizer` reads it. The accuracy
grid also covers that shape (unless `--no-cost-shape`): θ = 1.1 and K = 1e4
join the grid as a full row and column (a = 2 joins the Pareto values), so
the grid stays a full cross, each cost-table row is a point on a curve, and
the synthetic evaluation, which uses the same data, reads an exact curve
rather than the worse neighbouring grid point.

`--cost-rows 3` times only the rows=3 configs of CMS, CountSketch and
CMS-heap top-k (all four cols); HLL, KLL and DDSketch are timed at every
config. The rows=5 points keep blank cost columns in `saturation.csv` and
get no `crossover.csv` row. Their cost is extrapolated: a Vector2D sketch
updates and reads one counter per row and stores rows x cols counters, so
insert CPU, query CPU and `memory_bytes` at rows=5 are ≈ 5/3 of rows=3 at
the same cols. For insert and query 5/3 is an upper bound: the fast path
hashes each item once, and CMS-heap top-k's heap update does not scale with
rows. Merge is a counter-wise add, also ≈ 5/3.

## Exact baseline and crossover N*

The cost phase also runs, at every checkpoint N, the exact polars baseline of
each family over the same distributions (every theta x K for Zipf, every
alpha for Pareto; frequency and top-k share one run):

| family | exact baseline (`--library polars`) | what it holds |
|---|---|---|
| frequency, topk | `cms`: `group_by(v).agg(len)` | the buffered stream, then a key→count table (grows with distinct keys seen, so with K) |
| cardinality | `hll`: `n_unique()` | the buffered stream |
| quantile | `kll-cdf`: 101-point quantile grid | the buffered stream |

Two serial runs per (baseline, distribution, N), 3 runs + 1 warmup each:
`--operations insert,query --metrics throughput,cpu,memory` and
`--operations prepare --metrics latency,cpu,memory` (prepare is the polars
pass and is only measurable under latency, where it is one timed call).
Exact CPU = insert + prepare + one query's CPU seconds (user + sys, all
polars threads); exact memory = `memory_bytes` of the prepare record.

Both sides are charged **one** query: the benchmark's query phase repeats the
query over a probe set (every key seen, 101 quantiles, or a repeated HLL
estimate) whose size differs between sketch and exact, so the per-query CPU
(phase CPU / operations) is used. This is "ingest N items, answer one query".

For each sketch point, with its cost run at the final N:

- sketch memory = its `memory_bytes`, taken as constant in N (it is fixed
  by the config except for KLL's log-N levels and DDSketch's bucket range);
- sketch CPU at N = insert CPU x N / N_final + one query's CPU (insert is a
  per-item cost, measured N-independent in the earlier run; a query is not a
  function of N);
- **N\*_mem(f)** = smallest checkpoint N **from which** exact memory stays
  >= f x sketch memory at every larger checkpoint, and **N\*_cpu(f)**
  likewise for CPU, for f = 10 and 100; `not_reached` when even the last
  checkpoint does not qualify. "From which it stays" rather than "first time":
  polars' fixed start-up cost (~30–50 ms) makes exact CPU look high at the
  smallest N and comparatively cheap further up.

`--phase crossover` recomputes `crossover.csv` from the cost phase's
`saturation_cost.jsonl` and `exact_cost.jsonl` without running anything.

`crossover.csv` has one row per point: `n_sat`, `sketch_memory_bytes`,
`sketch_cpu_secs` (insert + query at N_final) and the four N\* columns. Since
both costs grow linearly in N, the CPU ratio tends to a constant (per-item
exact cost / per-item sketch cost) and N\*_cpu may never be reached; the
memory ratio grows without bound, so N\*_mem always exists given a long
enough stream.

**Which memory metric.** Both rows report `memory_bytes`, a formula each row
computes over the structure it keeps: the sketch's counters or registers, and
for polars the buffered stream (`Vec` capacity x 8 bytes, so up to 2x N x 8
from doubling) plus the count table. `heap_bytes_peak` (the tracking
allocator) is not comparable here: it only exists with the `heap-track`
feature, which the default `approxbench` build does not enable, and it counts
allocations inside the timed pass only. A sketch allocates its table at build
time, outside the pass, so its peak would read near zero, while the polars
buffer grows inside the insert pass and polars' transient group_by/sort
buffers land in prepare. Using `memory_bytes` for both compares retained
state with retained state. It leaves out polars' transient working memory, so
exact memory is a lower bound and N\*_mem is conservative (the exact answer
really gets expensive a bit sooner).

## From data parameters to a configuration

`scripts/recommend_config.py` turns the worst case each query's data shows
into a recommended config per sketch family, writing `out/recommendations.csv`
(`dataset, query_id, range, family, config, est_error, target, meets_target,
n_sat, flags`, the cost columns `memory_bytes, insert_ns_per_item,
merge_us_per_fold, query_phase_us`, and the grid point used: `grid_param, grid_K, N,
shards`).

Inputs:

- ASAPQuery `asap-tools/dataset-analysis/results/skew_summary.csv`: per
  (dataset, query, range) `worst_theta_cms` (lowest θ of the weight a per-key
  counter sees), `worst_K`, `min_N`/`max_N` (items per evaluation),
  `worst_alpha_rank`/`worst_alpha_memory`, `tail_class` and `target_*`
  (defaults: `are_top100` ≤ 0.05, precision@k ≥ 0.95, rank error ≤ 0.01).
  The older per-window schema is read too (`lower`/`upper`, `K_win_max`,
  `rows_win_*`, default targets, finest window as the step).
- `out_grid_1e7/saturation_curve.csv` (error per N) and, for N above 1e7,
  `out_1e9/saturation_curve.csv` where it has the point; the two
  `saturation.csv` files for n_sat (the 1e9 one wins) and, once the cost
  phase has run, cost. Without cost the cost columns stay empty and configs
  are ordered by nominal size (rows·cols, k, 1/α); rows=5 without cost takes
  5/3 of rows=3.
- Optional `--merge-curves saturation_merge_curve.csv` (branch
  `merged-accuracy`).

Families: key queries → CMS, CountSketch, CMS-heap top-k; value queries →
KLL, DDSketch. No query is a count-distinct, so HLL is not a candidate.

Rounding (always toward the harder side, so estimates are conservative): θ
down to the grid, K up, α for rank error (`worst_alpha_rank`, steepest tail)
up, α for DDSketch cost (`worst_alpha_memory`) down. The error is read at
N = `max_N`, linear in log N between checkpoints; a merged window of a
CMS/CountSketch/DDSketch has the same error as one sketch over it. For top-k
and KLL, with `--merge-curves` the error comes from the merged curve at the
shard count nearest (in log scale) to m = range / step; without it the
single-sketch curve is optimistic. The smallest config meeting the target is
recommended; if none does, the best one.

Flags: `θ/K/alpha ... beyond grid` or `below grid` (clamped to the grid edge),
`extrapolated beyond N=...` (N above the largest measured N; the last error is
used), `N below grid`, `not saturated` (never saturated in the grid),
`not saturated at min_N` (min_N < n_sat), `light tail` (α not meaningful),
`single-sketch curve optimistic` / `merged curve at m=...`, `cost scaled from
rows=3`, `target not met`.

```bash
python3 scripts/recommend_config.py \
  --merge-curves ../sketch-bench-merged/out_merge/saturation_merge_curve.csv
python3 scripts/test_recommend_config.py
```

## Running

`--phase accuracy|cost|all` (default all) splits the study. `accuracy` runs
only the parallel accuracy runs and writes `saturation_curve.csv` and a
`saturation.csv` with blank cost columns. `cost` reads
`saturation_curve.csv` from `--out`, recomputes N_sat and the final error from
it, and runs only the serial cost runs (sketch at the final N, exact baseline
at every N), writing `saturation.csv` and `crossover.csv`; give it the same
grid arguments as the accuracy run (it exits if a point's curve is missing or
covers other sizes). Run the cost phase on an otherwise idle machine.

`--resume` (accuracy) keeps every point whose curve over the requested sizes
is already in `--out`'s `saturation_curve.csv`, writes those curves back
first, and runs only the remaining points; a point cut off mid-write is
dropped and rerun, and raw records are appended to
`saturation_accuracy.jsonl`. The grid run below was killed at 498 of 595
points and finished this way.

`--points-from FILE` restricts the grid to the points listed in a CSV with
`sketch,config,dist,param,cardinality` columns (a filtered `saturation.csv`
works as is); it exits if a listed point is outside the grid.

```sh
cargo build -p aqpbm-cli --release

# Full grid to 1e7 (the run below): accuracy now, cost later on a quiet machine.
python3 scripts/study_saturation.py --phase accuracy --n-max 1e7 --seeds 3 \
    --jobs 16 --out out_grid_1e7            # add --resume after an interruption
python3 scripts/study_saturation.py --phase cost --n-max 1e7 --seeds 3 \
    --cost-rows 3 --out out_grid_1e7

# 1e9 subset: the points listed in out_1e9/points.csv (see Results).
python3 scripts/study_saturation.py --phase accuracy --n-max 1e9 --seeds 3 \
    --jobs 8 --points-from out_1e9/points.csv --out out_1e9

python3 scripts/test_study_saturation.py

# Figures (need matplotlib, pandas, numpy).
python3 scripts/plot_saturation_curves.py out/saturation_curve.csv out/
python3 scripts/plot_saturation_curves.py out_1e9/saturation_curve.csv out_1e9/
python3 scripts/plot_saturation_cost.py out/cost_cpu_memory.png out/saturation_cost.jsonl
```

Restrict the grid with `--families`, `--thetas`, `--cardinalities`,
`--alphas`; `--one-config` keeps the first config of each sketch. `--jobs`
parallelises the accuracy runs only; the cost runs are always serial. Output
goes to `--out` (default `out/`): raw records in `saturation_accuracy.jsonl`,
`saturation_cost.jsonl` and `exact_cost.jsonl`, the per-N seed-mean errors in
`saturation_curve.csv`, one row per point in `saturation.csv`, and N\* per
point in `crossover.csv`. The plotting scripts draw one config per sketch
(rows=3 cols=1024, lg_k=12, k=200, alpha=0.01, the configs their bounds are
written for).

## Results (config grid, N up to 1e7)

`--phase accuracy --n-max 1e7 --seeds 3 --jobs 16 --out out_grid_1e7`: 595
points (504 Zipf points for the three Vector2D sketches, 63 HLL, 28
quantile) x 17 sizes x 3 seeds. 36 min wall on the shared 56-core machine
(31 min until it was killed at 498 points, 5 min for the resumed 97). Cost
columns are blank until the cost phase runs.

### N_sat per config

| sketch | config | not saturated by 1e7 | median N_sat | max N_sat |
|---|---|---|---|---|
| CMS | rows=3, cols 256 / 1024 / 4096 / 16384 | 2 / 2 / 4 / 4 of 21 | 1.8e4 / 1e5 / 1.8e5 / 3.2e5 | 5.6e5 / 1.8e6 / 1.8e6 / 1.8e6 |
| CMS | rows=5, same cols | 2 / 7 / 4 / 4 | 3.2e4 / 1e5 / 1.8e5 / 1e3 | 5.6e5 / 5.6e5 / 1.8e6 / 1.8e6 |
| CountSketch | rows=3 | 3 / 4 / 7 / 4 | 1.8e5 / 1e5 / 1e6 / 1e5 | 1e6 / 1.8e6 / 1.8e6 / 1.8e6 |
| CountSketch | rows=5 | 3 / 3 / 6 / 6 | 5.6e4 / 1.8e5 / 1e6 / 1e5 | 5.6e5 / 1.8e6 / 1.8e6 / 1.8e6 |
| CMS-heap top-k | rows=3 | 7 / 4 / 2 / 1 | 4.4e3 / 1e3 / 1.8e3 / 1.8e3 | 1.8e6 / 3.2e5 / 5.6e4 / 1.8e6 |
| CMS-heap top-k | rows=5 | 2 / 1 / 0 / 1 | 1.8e3 / 1.4e3 / 1.8e3 / 1.8e3 | 1.8e6 / 1e6 / 1.8e6 / 1e6 |
| HLL | lg_k 12 / 14 / 16 | 2 / 10 / 5 of 21 | 1e5 / 1e4 / 3.3e5 | 1.8e6 each |
| KLL | k 50 / 200 / 800 | 2 / 3 / 3 of 4 | 1e5 / 1.8e5 / 1.8e4 | |
| DDSketch | alpha 0.005 / 0.01 / 0.02 / 0.05 | 0 of 4 each | 1.4e4 / 2.1e3 / 1e3 / 1e3 | 3.2e5 / 3.2e3 / 1e3 / 1e3 |

108 of 595 points are not saturated by 1e7. For the Vector2D sketches they
sit at K >= 1e5 (CMS 25 of 29, CountSketch 33 of 36, top-k 16 of 18), and
at theta <= 0.5 (low skew: the error ∝ tail mass / w keeps changing while
new keys arrive; 17 CMS and 26 CountSketch points) or, for large w, at
theta >= 1.5, where the plateau is so small (1e-4 to 1e-6)
that the relative band is narrower than the remaining drift. For HLL and KLL
the unsaturated points are seed noise: with 3 seeds their tail wanders by
more than 10% of a plateau of 1e-3 to 1e-2 (the earlier run needed 10 seeds
for KLL). N_sat does not grow systematically with the config: a wider CMS
needs a somewhat longer stream before collisions settle (median 1.8e4 at
cols 256 to 3.2e5 at cols 16384, rows=3), but for the other sketches the
median moves by less than the checkpoint spacing's noise.

### Plateau vs config

The plateau is the seed-mean error at N = 1e7. The slope is a least-squares
fit of log(error) on log(knob) (w = cols, m = 2^lg_k, k, alpha), over the
configs with non-zero error; `*` marks a point not saturated by 1e7. For
top-k the fitted quantity is the miss rate 1 − precision_at_k. The rows are
a representative subset of the distributions.

| sketch | data | plateau at cols 256 / 1024 / 4096 / 16384 (rows=3) | slope rows=3 | slope rows=5 | theory |
|---|---|---|---|---|---|
| CMS | θ=0, K=1e7 | 4.8e+03* / 1.19e+03* / 294* / 71.7* | −1.01 | −1.01 | −1 |
| CMS | θ=0.5, K=1e7 | 162 / 40.2 / 9.89 / 2.38 | −1.01 | −1.02 | −1 |
| CMS | θ=1, K=1e5 | 1.21 / 0.237 / 0.0449 / 0.00619 | −1.26 | −1.31 | −1 |
| CMS | θ=1, K=1e7 | 2.12 / 0.464 / 0.101 / 0.0199 | −1.12 | −1.12 | −1 |
| CMS | θ=1.5, K=1e7 | 0.241 / 0.0288 / 0.00413 / 0.000429 | −1.51 | −1.55 | −1 |
| CMS | θ=2, K=1e7 | 0.0944 / 0.00472 / 0.000359* / 5.39e-06* | −2.30 | −2.33 | −1 |
| CountSketch | θ=0, K=1e7 | 9.46* / 4.9* / 2.64* / 1.6* | −0.43 | −0.42 | −0.5 |
| CountSketch | θ=0.5, K=1e7 | 0.806* / 0.42* / 0.223* / 0.102* | −0.49 | −0.51 | −0.5 |
| CountSketch | θ=1, K=1e5 | 0.265 / 0.0779 / 0.0182 / 0.00739 | −0.88 | −1.02 | −0.5 |
| CountSketch | θ=1, K=1e7 | 0.262 / 0.081 / 0.0183 / 0.00781 | −0.87 | −1.01 | −0.5 |
| CountSketch | θ=1.5, K=1e7 | 0.191 / 0.0257 / 0.00338 / 0.00256 | −1.08 | −1.48 | −0.5 |
| CountSketch | θ=2, K=1e7 | 0.171 / 0.0118 / 0.000819* / 0.00137 | −1.24 | −2.25 | −0.5 |
| top-k miss rate | θ=0, K=1e7 | 1 / 1 / 1 / 1* | 0.00 | 0.00 | — |
| top-k miss rate | θ=0.5, K=1e7 | 0.896 / 0.667* / 0.26* / 0.104 | −0.53 | −0.64 | — |
| top-k miss rate | θ=1, K=1e5 | 0.594* / 0.0312 / 0 / 0 | −2.12 | — | — |
| top-k miss rate | θ=1, K=1e7 | 0.75* / 0.281* / 0.0104 / 0 | −1.54 | — | — |
| top-k miss rate | θ=1.5, K=1e7 | 0.427* / 0.0521 / 0 / 0 | −1.52 | — | — |
| top-k miss rate | θ=2, K=1e7 | 0.125 / 0 / 0 / 0 | — | — | — |

| sketch | data | plateau, knob small → large | slope | theory |
|---|---|---|---|---|
| HLL, lg_k 12 / 14 / 16 | θ=0, K=1e7 | 0.0131 / 0.00522* / 0.00368 | −0.46 | −0.5 |
| HLL, lg_k 12 / 14 / 16 | θ=1, K=1e7 | 0.0116 / 0.00221* / 0.00213 | −0.61 | −0.5 |
| HLL, lg_k 12 / 14 / 16 | θ=2, K=1e7 | 0.0113* / 0.00415* / 0.00183 | −0.66 | −0.5 |
| KLL, k 50 / 200 / 800 | Pareto a=1.1 | 0.0118 / 0.00313* / 0.000688 | −1.03 | −0.97 |
| KLL, k 50 / 200 / 800 | Pareto a=1.5 | 0.0109 / 0.00336* / 0.00059* | −1.05 | −0.97 |
| KLL, k 50 / 200 / 800 | Pareto a=2 | 0.0143* / 0.00269* / 0.000566* | −1.16 | −0.97 |
| KLL, k 50 / 200 / 800 | Pareto a=3 | 0.011* / 0.0024 / 0.000418* | −1.18 | −0.97 |
| DDSketch, alpha 0.005 / 0.01 / 0.02 / 0.05 | Pareto a=1.1 | 0.00108 / 0.00273 / 0.00537 / 0.0134 | 1.08 | +1 |
| DDSketch, alpha 0.005 / 0.01 / 0.02 / 0.05 | Pareto a=1.5 | 0.00172 / 0.00355 / 0.00726 / 0.0181 | 1.02 | +1 |
| DDSketch, alpha 0.005 / 0.01 / 0.02 / 0.05 | Pareto a=2 | 0.00233 / 0.00492 / 0.00968 / 0.0241 | 1.01 | +1 |
| DDSketch, alpha 0.005 / 0.01 / 0.02 / 0.05 | Pareto a=3 | 0.00376 / 0.00735 / 0.0143 / 0.0361 | 0.98 | +1 |

- **CMS** follows 1/w exactly (slope −1.0) where collisions with the tail
  dominate the error (theta <= 0.5 at any K >= 1e5, including the
  unsaturated theta = 0 points). With skew it falls faster than 1/w (−1.1 at
  theta 1, −1.5 at 1.5, −2.3 at 2): the 1/w bound is for the mean colliding
  mass, but the min over rows discards rows where a heavy key collides, and
  that probability falls as (h/w)^rows. At K = 1e3 (not shown) cols >= K
  gives zero error, so fits there are steeper (−2 to −4) and not meaningful.
  rows=5 matches rows=3 where the tail dominates and is lower otherwise.
- **CountSketch** follows 1/sqrt(w) at low skew (−0.43 to −0.51 at
  theta <= 0.5, K = 1e7). With skew it steepens to about −theta (−0.87 at
  theta 1, −1.1 at 1.5): its error is governed by the residual L2 norm with
  the top ~w keys removed, which for Zipf(theta > 0.5) shrinks as
  w^(1/2 − theta), so error ∝ w^(−theta). rows=5 is steeper still (median
  of 5 rows).
- **HLL** at K = 1e7 fits −0.46 to −0.66 against −0.5, within the noise of
  3 seeds of |relative error| over three configs. At K <= 1e5 the larger
  m reach HLL's linear-counting (small-range) regime, where the error drops
  to ~1e-4 or 0, so slopes there (−1 to −1.7) measure the regime change,
  not the asymptotic 1/sqrt(m).
- **KLL** fits −1.03 to −1.18 against the −0.97 of its 2.296/k^0.9723
  bound: about 1/k, the noisier values at alpha 2 and 3 coming from 3-seed
  plateaus.
- **DDSketch** fits 0.98 to 1.08: rank error ∝ alpha, on every Pareto tail.
- **CMS-heap top-k** has no power law to compare: its miss rate is 1 at
  theta = 0 (no heavy hitters to find), falls roughly as w^−0.5 at
  theta = 0.5 and as w^−1.5 to w^−2 at theta >= 1 until it hits 0
  (cols >= 4096 finds all top 32 for theta >= 1 at K = 1e5).

## Results (1e9 subset)

`--phase accuracy --n-max 1e9 --seeds 3 --jobs 8 --points-from
out_1e9/points.csv --out out_1e9`: 185 points x 25 sizes (1e3 to 1e9, four
per decade) x 3 seeds, 18.4 h wall on the shared machine. The points are all
108 grid points that were not saturated by 1e7, plus every point of the
plotted configs (rows=3 cols=1024, lg_k=12, k=200, alpha=0.01) so their
figures (`out_1e9/error_vs_N__*.png`) cover N up to 1e9. Error values at
N <= 1e7 match the grid run exactly for every sketch except KLL, whose
compaction coin flips are not seeded (seed means differ by up to 2x at small
N and a few percent at the plateau).

Unsaturated at 1e7 → status at 1e9 (N_sat recomputed on the 25-point curve):

| sketch | unsaturated at 1e7 | saturated by 1e9 | still not saturated |
|---|---|---|---|
| CMS | 29 | 18 | 11 |
| CountSketch | 36 | 25 | 11 |
| CMS-heap top-k | 18 | 11 | 7 |
| HLL | 17 | 10 | 7 |
| KLL | 8 | 3 | 5 |
| DDSketch | 0 | — | — |
| total | 108 | 67 | 41 |

What stays unsaturated, and why:

- **CMS and CountSketch, theta = 0, K = 1e7 (all 8 configs of each; 16 of
  the 22).** Error keeps
  rising: x5.5 for CMS and x3.1 to x3.9 for CountSketch from 1e7 to 1e9. With
  uniform data over 1e7 keys, N = 1e9 is only 100 items per key, so the
  sketch's additive error (∝ N/w for CMS, ∝ ||f||2/sqrt(w) for CountSketch)
  still grows faster than the true top-100 counts. At K = 1e5 the same
  configs saturate by 5.6e7 (about 500 items per key), with the plateau 1.3x
  the 1e7 error. Uniform frequency points only saturate at N >> K. The other
  6 are 3 theta = 2 points at cols 16384 with errors near 2e-5, where a few
  collisions move the mean, and 3 theta = 0 points at K = 1e3 or 1e5 (1 CMS,
  2 CountSketch).
- **CMS-heap top-k, K = 1e7, theta 0.5 to 1.5, cols 256 and 1024.** The miss rate
  halves from 1e7 to 1e9 (0.25 → 0.125 at theta 1) and is still falling:
  more items separate the true top 32 from the tail. This is the one sketch
  whose error improves with N rather than approaching a plateau from below.
- **HLL at lg_k 14 and 16, theta >= 1.2, K = 1e7; theta = 2 everywhere.**
  HLL error depends only on the distinct count seen, which for skewed Zipf
  keeps growing until N covers the tail; the 3-seed curves are noisy enough
  that the ±10% band rarely holds for three consecutive sizes. At lg_k 12 the
  error settles at 0.021 once all 1e7 distinct values are seen (theta <= 1).
  The theta = 2 points at lg_k 14 (0.011 to 0.021 at 1e9, above lg_k 12) are
  not explained by the bound and need more seeds to trust.
- **KLL.** Errors from 1e7 to 1e9 move by 0.8x to 1.4x with no trend; the
  3-seed plateau is within noise of the band edge, so N_sat flips in both
  directions (k=200 alpha 3 was saturated at 1e7 and is not at 1e9). KLL is
  effectively flat by 1e6; its N_sat needs 10 seeds (as in the earlier run).

The longer curve also moves 1e7-saturated verdicts: of the 77 extra points,
55 keep the same N_sat, 13 move it to another size <= 1e7, 7 move it past
1e7 (mostly HLL lg_k 12 at K = 1e7, where the distinct count is still
growing), and 2 lose it (one top-k, one KLL). N_sat from a curve that ends
at 1e7 is therefore a lower bound when K is large or the error metric is
noisy; for CMS/CountSketch with theta >= 0.5 and DDSketch it is stable.

## Results (earlier reduced run)

This run predates the config grid and the i64 Pareto data: quantile streams
were Pareto(alpha, scale 1) at f64, and the configs were the then-first ones.
Reduced grid: `--one-config --n-max 1e7`, 3 seeds for the Zipf sketches and
10 seeds for the quantile sketches (`--families quantile --seeds 10`, which
KLL needed to separate its plateau from seed noise). Cost runs were serial on
an otherwise idle 56-core machine. Each cell is `N_sat (error at N = 1e7)`.

| sketch (config) | metric | theta | K=1e3 | K=1e5 | K=1e7 |
|---|---|---|---|---|---|
| cms-fastpath-vector2d (rows=3 cols=1024) | are_top100 | 0.0 | 3.2e5 (0.254) | not sat. (65.9) | not sat. (1.19e+03) |
| cms-fastpath-vector2d (rows=3 cols=1024) | are_top100 | 0.5 | 5623 (0.0627) | 1e5 (3.63) | 5.6e5 (40.2) |
| cms-fastpath-vector2d (rows=3 cols=1024) | are_top100 | 0.8 | 1e5 (0.031) | 3.2e4 (0.666) | 1e5 (2.3) |
| cms-fastpath-vector2d (rows=3 cols=1024) | are_top100 | 1.0 | 1.8e4 (0.0194) | 5.6e4 (0.237) | 3.2e4 (0.464) |
| cms-fastpath-vector2d (rows=3 cols=1024) | are_top100 | 1.2 | 5623 (0.0121) | 5.6e4 (0.0924) | 5.6e4 (0.126) |
| cms-fastpath-vector2d (rows=3 cols=1024) | are_top100 | 1.5 | 5.6e4 (0.00615) | 1.8e5 (0.0266) | 1.8e5 (0.0288) |
| cms-fastpath-vector2d (rows=3 cols=1024) | are_top100 | 2.0 | 3.2e5 (0.00184) | 1.8e6 (0.00472) | 1.8e6 (0.00472) |
| countsketch-fastpath-vector2d (rows=3 cols=1024) | are_top100 | 0.0 | not sat. (0.342) | not sat. (2.33) | not sat. (4.9) |
| countsketch-fastpath-vector2d (rows=3 cols=1024) | are_top100 | 0.5 | 3.2e4 (0.157) | 1e5 (0.348) | not sat. (0.42) |
| countsketch-fastpath-vector2d (rows=3 cols=1024) | are_top100 | 0.8 | 1778 (0.0855) | 1e5 (0.133) | 1e6 (0.135) |
| countsketch-fastpath-vector2d (rows=3 cols=1024) | are_top100 | 1.0 | 3162 (0.0586) | 5.6e4 (0.0779) | 5.6e4 (0.081) |
| countsketch-fastpath-vector2d (rows=3 cols=1024) | are_top100 | 1.2 | 1000 (0.0404) | 1e5 (0.0495) | 1e5 (0.0492) |
| countsketch-fastpath-vector2d (rows=3 cols=1024) | are_top100 | 1.5 | 1.8e5 (0.0235) | 3.2e5 (0.0265) | 5.6e5 (0.0257) |
| countsketch-fastpath-vector2d (rows=3 cols=1024) | are_top100 | 2.0 | 3162 (0.0106) | 1e5 (0.0115) | 1.8e6 (0.0118) |
| cms-heap-topk-fastpath-vector2d (rows=3 cols=1024) | precision_at_k | 0.0 | not sat. (0.104) | 1.8e4 (0) | 1000 (0) |
| cms-heap-topk-fastpath-vector2d (rows=3 cols=1024) | precision_at_k | 0.5 | 1000 (0.823) | 5.6e4 (0.531) | not sat. (0.333) |
| cms-heap-topk-fastpath-vector2d (rows=3 cols=1024) | precision_at_k | 0.8 | 1000 (1) | 5623 (0.917) | not sat. (0.688) |
| cms-heap-topk-fastpath-vector2d (rows=3 cols=1024) | precision_at_k | 1.0 | 1000 (1) | 1778 (0.969) | not sat. (0.719) |
| cms-heap-topk-fastpath-vector2d (rows=3 cols=1024) | precision_at_k | 1.2 | 1000 (1) | 1000 (0.969) | 3.2e5 (0.865) |
| cms-heap-topk-fastpath-vector2d (rows=3 cols=1024) | precision_at_k | 1.5 | 1000 (1) | 1000 (0.979) | 1000 (0.948) |
| cms-heap-topk-fastpath-vector2d (rows=3 cols=1024) | precision_at_k | 2.0 | 1778 (1) | 3162 (0.99) | 3162 (1) |
| hll (lg_k=12) | relative_error | 0.0 | 1000 (0.011) | 3.2e5 (0.0114) | 3.2e5 (0.0131) |
| hll (lg_k=12) | relative_error | 0.5 | 1000 (0.011) | 1e5 (0.0114) | 1e6 (0.00814) |
| hll (lg_k=12) | relative_error | 0.8 | 1778 (0.011) | 1.8e4 (0.0114) | 1e6 (0.014) |
| hll (lg_k=12) | relative_error | 1.0 | 3.2e4 (0.011) | 1e6 (0.0114) | 3162 (0.0116) |
| hll (lg_k=12) | relative_error | 1.2 | 1778 (0.011) | 1e6 (0.0101) | not sat. (0.0109) |
| hll (lg_k=12) | relative_error | 1.5 | 3.2e4 (0.011) | 1.8e5 (0.0135) | 5.6e5 (0.0234) |
| hll (lg_k=12) | relative_error | 2.0 | 1.8e6 (0.011) | 5.6e4 (0.00604) | not sat. (0.0113) |

| sketch (config) | metric | alpha=1.1 | alpha=1.5 | alpha=2.0 | alpha=3.0 |
|---|---|---|---|---|---|
| kll-percall (k=200) | mean_rank_err | 1.8e6 (0.00318) | 1.8e5 (0.00345) | 1.8e6 (0.00321) | 1e6 (0.00338) |
| dd (alpha=0.01) | mean_rank_err | 3162 (0.00257) | 1000 (0.00378) | 1000 (0.00485) | 1000 (0.00721) |

### Figures

Committed under `docs/figures/saturation/`: `reduced_run/` (error vs N per
sketch with theoretical bounds, the serial cost figure, and a README that
explains every figure) and `grid_1e9/` (the same plots from the 1e9 subset).

### Reading the curves

`plot_saturation_curves.py` draws one panel per distribution with the
theoretical bound. None of the bounds depends on N; where the empirical
error still moves with N it is a finite-N effect (noisy ground truth at small
N, or a tail that keeps filling in while N is below K).

| sketch | bound (this config) | empirical behaviour |
|---|---|---|
| CMS `are_top100` | mean over top 100 of (e/w)/p_i | rises then plateaus; plateau falls steeply with theta (worst case = low theta); theta = 0 with K >= 1e5 never saturates by 1e7 |
| CountSketch `are_top100` | mean over top 100 of sqrt(3/w)·‖p‖₂/p_i | hump then decline to plateau (the 1/N term of ‖f‖₂ fades); far below CMS at low theta; needs N of order 1/Σp² |
| CMS-heap top-k `precision_at_k` | lower bound: share of top 32 with p_i − p_33 > e/w | ≈ 1 for theta >= 0.8 and K <= 1e5; ill-defined at theta = 0; at K = 1e7 precision falls once N approaches K |
| HLL `relative_error` | 2 × 1.04/sqrt(m) = 3.25% | depends on the distinct count seen, not N; constant once every key has been seen |
| KLL `mean_rank_err` | 2.296/k^0.9723 = 1.33% | flat at ≈ 3e-3 from ≈ 5e3 items, independent of alpha; the rule's N_sat is conservative because compaction noise exceeds 2 SE |
| DDSketch `mean_rank_err` | α_dd · a / 2 (derived from the value-error guarantee) | flat by 1e3–1e4; grows with a (lighter tail, larger rank error); memory shrinks with a |

Serial per-operation cost at N = 1e7: insert is N- and distribution-independent
for every sketch (CMS 27 ns, CountSketch 31 ns, HLL 4 ns, KLL 32 ns, DDSketch
20–27 ns per item) except CMS-heap top-k, which grows from ≈ 100 ns at theta = 0
to ≈ 2,100 ns at theta = 2. DDSketch memory falls from 6.3 KB (alpha = 1.1) to
2.3 KB (alpha = 3); the others are fixed by their config.

### Grid cost and crossover N*

`--phase cost --n-max 1e7 --seeds 3 --cost-rows 3` ran serially on a
separate idle 56-core machine (load 1.0, ≈ 3 h; 343 sketch points plus the
exact baselines). Figures and CSVs: `docs/figures/saturation/grid_cost/`.

Per-item insert CPU is flat in N and in the distribution for every sketch
(CMS ≈ 22 ns, CountSketch ≈ 26 ns, HLL ≈ 3.4 ns, KLL ≈ 25 ns, DDSketch
≈ 16 ns) except CMS-heap top-k, which rises from ≈ 100 ns at theta 0 to
≈ 1,750 ns at theta 2. DDSketch memory falls with the Pareto index (6.3 KB at
alpha 1.1 to 2.3 KB at alpha 3, alpha_dd = 0.01).

Crossover at theta = 1, K = 1e5 (frequency, top-k, HLL) and Pareto alpha 1.5
(quantile):

| sketch (config) | memory | N\*_mem 10x | N\*_mem 100x | N\*_cpu 10x | N\*_cpu 100x |
|---|---|---|---|---|---|
| CMS / CountSketch 3x256 | 3 KB | 1.8e3 | 1.8e4 | 1e3 | not reached |
| CMS / CountSketch 3x1024 | 12 KB | 5.6e3 | 1e5 | 1e3 | not reached |
| CMS / CountSketch 3x16384 | 192 KB | 1.8e5 | 3.2e6 | 1e3 | not reached |
| CMS-heap top-k (any cols) | 3–192 KB | as CMS | as CMS | not reached | not reached |
| HLL lg_k 12 / 14 / 16 | 4 / 16 / 64 KB | 5.6e3 / 1.8e4 / 1e5 | 5.6e4 / 1.8e5 / 5.6e5 | 3.2e3 / 1e4 / 3.2e4 | not reached |
| KLL k 50 / 200 / 800 | 1.6 / 6.4 / 26 KB | 1.8e3 / 5.6e3 / 1.8e4 | 1.8e4 / 1e5 / 3.2e5 | 1e3 | 1e6 |
| DDSketch alpha 0.01 | 4.6 KB | 5.6e3 | 5.6e4 | 1e3 | 1.8e5 |

- **Memory**: the exact baseline buffers the stream (~13 bytes/item at 1e7,
  independent of K), so a sketch saves 10x after a few thousand items and
  100x after 1e4–1e6, scaling with its own size.
- **CPU**: exact frequency cost (buffer + polars group-by) is ≈ 300 ns/item
  and exact cardinality (buffer + n_unique) ≈ 145 ns/item at 1e7, so
  CMS/CountSketch (≈ 25 ns) stay ≈ 13x cheaper at every N and HLL (≈ 3.4 ns)
  ≈ 40x once its O(m) estimate is amortised, but neither reaches 100x; quantile exact (sort) is superlinear, so
  KLL and DDSketch pass 100x at 1e6 and 1.8e5. CMS-heap top-k at theta >= 1 is
  slower per item than the exact baseline (≈ 650 ns vs ≈ 300 ns), so its
  benefit is memory only.
- The exact baseline here is "store the samples, compute at query time"
  (what a TSDB holding raw samples does); a streaming exact counter (hash map,
  O(K) memory) would move the frequency memory crossover to N where the
  table of K keys outgrows the sketch.

### Not covered yet

- **Accuracy after merging** is now measured (`--operations merge --metrics
  accuracy --merge-shards m`, or `--merge-shards-list 1,4,16,64` here, into
  `saturation_merge_curve.csv`): the N-item stream is split into m contiguous
  shards, one sketch per shard is folded into one, and it is scored with the
  query's comparator against the whole-stream truth. A bounded run (first
  config per sketch, N up to 1e7, 3 seeds, theta 0.5/1.0/1.5, K 1e3/1e5/1e7,
  alpha 1.1/2) found:
  - CMS, CountSketch, HLL and DDSketch: merged error is identical to the
    single sketch at every N, point and shard count (0 of 1479 cells
    differ), so their saturation curves apply unchanged to merged windows.
  - KLL (k=50): merged mean rank error drifts but stays within about 2 seed
    SE of the single sketch; at N=1e7 it is 0.0115/0.0115/0.0104/0.0121
    (alpha 1.1) and 0.0121/0.0124/0.0119/0.0141 (alpha 2) for m=1/4/16/64,
    so only 64 shards shows a (5-16%) increase.
  - Top-k heap (rows=3 cols=256): with few keys (K=1000) every heavy key
    survives in every shard's heap and precision@k is unchanged; with
    K=1e5 or 1e7 and theta 1.0/1.5, merged precision@k at N=1e7 drops by
    0.03-0.09 (8-25% relative), e.g. theta 1.5, K=1e5: 0.60 -> 0.55/0.51/0.52
    for m=4/16/64 (theta 0.5 is at low precision and within seed noise), so
    the single-sketch curve overstates merged top-k quality.
  - Full grids (all top-k configs, KLL k 50/200/800 and DDSketch with 10
    seeds; `docs/figures/saturation/merge_grid/`): DDSketch stays exact;
    top-k's median merged/single precision is 1.00 but drops to 0.60 at large
    K for every width; KLL's merged error grows with k (≈ 1.0–1.1x at
    k = 50/200, 1.12–1.32x at k = 800 at N = 1e7, and up to 3–4x at
    N = 1e4–1e5 for k = 800, m = 4). `recommend_config.py --merge-curves`
    now covers every top-k and KLL config.
- **HLL lg_k 10.** asap_sketchlib has register types for lg_k 12, 14, 16
  (and 18 for the bucket list) only, so the HLL grid starts at 12.

