# Saturation Study

How long a stream does each sketch need before its accuracy stops changing?
This study measures that length, N_sat, per sketch, config and data shape, and
records the cost of the sketch at the largest N measured.

It is stacked on PR #129 (`dev-milind-turboprom`) and does not change
`rqe-optimizer` or its atomic-cost table; it only adds a Pareto generator
(`--dataset pareto`) and `scripts/study_saturation.py`.

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
| quantile | `kll-percall`, `dd` | Pareto(alpha), scale 1, f64 | `mean_rank_err` |

Zipf with `--zipf-s 0` is uniform over the K keys (rand_distr accepts s = 0),
so theta = 0 needs no separate dataset.

At the final N of each point one extra run
(`--operations insert,query,merge --metrics throughput,cpu,memory
--merge-shards 16 --runs 3 --warmup-runs 1 --flat`) records insert, merge and
query CPU seconds (user + sys) and `memory_bytes`. `BENCH_WARMUP_SECS`
defaults to 0, so these costs are indicative;
`scripts/export_rqe_optimizer_costs.sh` remains the source of optimizer costs.

## Running

```sh
cargo build -p aqpbm-cli --release

# Full grid: all configs, N = 1e3..1e8, 5 seeds.
python3 scripts/study_saturation.py --jobs 24

# Reduced grid (the run below).
python3 scripts/study_saturation.py --one-config --n-max 1e7 --seeds 3 --jobs 24

python3 scripts/test_study_saturation.py

# Figures (need matplotlib, pandas, numpy).
python3 scripts/plot_saturation_curves.py out/saturation_curve.csv out/
python3 scripts/plot_saturation_cost.py out/cost_cpu_memory.png out/saturation_cost.jsonl
```

Restrict the grid with `--families`, `--thetas`, `--cardinalities`,
`--alphas`; `--one-config` keeps the first config of each sketch. `--jobs`
parallelises the accuracy runs only; the cost runs are always serial. Output
goes to `--out` (default `out/`): raw records in `saturation_accuracy.jsonl`
and `saturation_cost.jsonl`, the per-N seed-mean errors in
`saturation_curve.csv`, and one row per point in `saturation.csv`.

## Results (reduced run)

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

### Not covered yet

- **Accuracy after merging.** Every curve is one sketch fed N items. For CMS,
  CountSketch, HLL and DDSketch a merge of m shards equals one sketch over
  the union, so the curves apply to merged windows. For KLL (merge depth) and
  the top-k heap (a globally heavy key can miss every shard's heap) they are
  only a lower bound on merged error; measuring that is the next PR.
- **Configuration scaling.** One config per sketch was run; the theoretical
  plateau scaling (CMS ∝ 1/w, CountSketch ∝ 1/sqrt(w), HLL ∝ 1/sqrt(m),
  KLL ∝ 1/k) is not yet checked against the full grid.

