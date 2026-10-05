# Saturation curves at the synthetic workload's label cardinalities

The synthetic PromQL workload of the AutoSketch vs. ASAP planner evaluation
(ProjectASAP/ASAPQuery#777, plan §6) varies `card(label_0)` over 1e1–1e6.
The saturation grid of #130 measured K ∈ {1e3, 1e5, 1e7} only. These runs
add K ∈ {10, 100, 1e4, 1e6} for the frequency and top-k families, by
measurement rather than interpolation in K. Quantile (Pareto) curves do not
depend on K and come from #130/#131 unchanged.

## Results (accuracy phase)

- **K ≤ 100 saturates early:** every point reaches `N_sat` between 1e3 and 6e4.
- **Large K with low skew may never saturate:** at K = 1e4 and 1e6 with
  θ ≤ 0.5, many CMS, CountSketch and top-k points are still moving at
  N = 1e8 ("never" below: 35 of 240 points at K ≥ 1e4). Their `N_sat` is a
  lower bound, and consumers flag them.
- **Top-k merging loses precision at large K:** merged/single precision@k
  at N = 1e7 drops to 0.50 (rows=3 cols=16384, θ = 0, K = 1e4) and to
  0.82–0.90 at K = 1e6 with θ ≤ 1.5. CMS and CountSketch merge exactly (#131),
  so they have no merge runs here.

`N_sat` uses #130's rule (tolerance 0.10, plateau tail 3, 3 seeds). "never" =
not saturated by the largest N.

| sketch | config | θ=0 K=1e+01 | θ=0 K=1e+02 | θ=0 K=1e+04 | θ=0 K=1e+06 | θ=0.5 K=1e+01 | θ=0.5 K=1e+02 | θ=0.5 K=1e+04 | θ=0.5 K=1e+06 | θ=1 K=1e+01 | θ=1 K=1e+02 | θ=1 K=1e+04 | θ=1 K=1e+06 | θ=1.5 K=1e+01 | θ=1.5 K=1e+02 | θ=1.5 K=1e+04 | θ=1.5 K=1e+06 | θ=2 K=1e+01 | θ=2 K=1e+02 | θ=2 K=1e+04 | θ=2 K=1e+06 |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| cms-fastpath-vector2d | rows=3 cols=1024 | 1e+03 | 1e+03 | 6e+06 | never | 1e+03 | 1e+03 | 6e+04 | 3e+05 | 1e+03 | 1e+03 | 2e+04 | 6e+04 | 1e+03 | 1e+03 | 2e+05 | 1e+05 | 1e+03 | 1e+03 | 3e+06 | 3e+06 |
| cms-fastpath-vector2d | rows=3 cols=16384 | 1e+03 | 1e+03 | never | never | 1e+03 | 1e+03 | 3e+04 | 6e+05 | 1e+03 | 1e+03 | 6e+04 | 2e+05 | 1e+03 | 1e+03 | 2e+06 | 2e+07 | 1e+03 | 1e+03 | 3e+06 | 6e+06 |
| cms-fastpath-vector2d | rows=3 cols=256 | 1e+03 | 3e+03 | 6e+06 | never | 1e+03 | 1e+03 | 3e+04 | 3e+05 | 1e+03 | 3e+04 | 6e+03 | 1e+04 | 1e+03 | 1e+04 | 2e+04 | 1e+04 | 1e+03 | 6e+04 | 3e+05 | 3e+04 |
| cms-fastpath-vector2d | rows=3 cols=4096 | 1e+03 | 1e+03 | never | never | 1e+03 | 1e+03 | 6e+04 | 3e+05 | 1e+03 | 1e+03 | 1e+04 | 1e+05 | 1e+03 | 1e+03 | 2e+06 | 1e+06 | 1e+03 | 1e+03 | 6e+06 | 2e+07 |
| cms-fastpath-vector2d | rows=5 cols=1024 | 1e+03 | 1e+03 | 6e+06 | never | 1e+03 | 1e+03 | 6e+04 | 3e+05 | 1e+03 | 1e+03 | 6e+04 | 6e+04 | 1e+03 | 1e+03 | 3e+05 | 3e+05 | 1e+03 | 1e+03 | 1e+07 | 1e+07 |
| cms-fastpath-vector2d | rows=5 cols=16384 | 1e+03 | 1e+03 | never | never | 1e+03 | 1e+03 | 3e+06 | 1e+06 | 1e+03 | 1e+03 | 6e+04 | 2e+06 | 1e+03 | 1e+03 | 2e+07 | 2e+07 | 1e+03 | 1e+03 | 1e+07 | never |
| cms-fastpath-vector2d | rows=5 cols=256 | 1e+03 | 1e+03 | 6e+06 | never | 1e+03 | 1e+03 | 3e+04 | 3e+05 | 1e+03 | 1e+03 | 1e+04 | 2e+04 | 1e+03 | 1e+03 | 6e+04 | 6e+04 | 1e+03 | 1e+03 | 6e+05 | 2e+05 |
| cms-fastpath-vector2d | rows=5 cols=4096 | 1e+03 | 1e+03 | 3e+06 | never | 1e+03 | 1e+03 | 2e+05 | 3e+05 | 1e+03 | 1e+03 | 3e+05 | 2e+05 | 1e+03 | 1e+03 | 6e+06 | 3e+06 | 1e+03 | 1e+03 | never | never |
| cms-heap-topk-fastpath-vector2d | rows=3 cols=1024 | 1e+03 | 1e+03 | never | 3e+03 | 1e+03 | 1e+03 | 1e+04 | never | 1e+03 | 1e+03 | 1e+03 | 2e+07 | 1e+03 | 1e+03 | 1e+03 | 1e+03 | 1e+03 | 1e+03 | 2e+03 | 2e+03 |
| cms-heap-topk-fastpath-vector2d | rows=3 cols=16384 | 1e+03 | 1e+03 | 3e+06 | 6e+04 | 1e+03 | 1e+03 | 6e+03 | 1e+05 | 1e+03 | 1e+03 | 1e+03 | 1e+03 | 1e+03 | 1e+03 | 1e+03 | 1e+03 | 1e+03 | 1e+03 | 2e+03 | 3e+03 |
| cms-heap-topk-fastpath-vector2d | rows=3 cols=256 | 1e+03 | 1e+03 | never | 1e+03 | 1e+03 | 1e+03 | 1e+06 | 1e+07 | 1e+03 | 1e+03 | 6e+05 | never | 1e+03 | 1e+03 | 3e+06 | never | 1e+03 | 1e+03 | 2e+07 | 2e+07 |
| cms-heap-topk-fastpath-vector2d | rows=3 cols=4096 | 1e+03 | 1e+03 | never | 2e+04 | 1e+03 | 1e+03 | 6e+03 | 2e+05 | 1e+03 | 1e+03 | 1e+03 | 1e+03 | 1e+03 | 1e+03 | 1e+03 | 1e+03 | 1e+03 | 1e+03 | 2e+03 | 3e+03 |
| cms-heap-topk-fastpath-vector2d | rows=5 cols=1024 | 1e+03 | 1e+03 | never | 1e+03 | 1e+03 | 1e+03 | 6e+03 | 6e+05 | 1e+03 | 1e+03 | 1e+03 | 1e+03 | 1e+03 | 1e+03 | 1e+03 | 1e+03 | 1e+03 | 1e+03 | 2e+03 | 3e+03 |
| cms-heap-topk-fastpath-vector2d | rows=5 cols=16384 | 1e+03 | 1e+03 | 6e+06 | 3e+05 | 1e+03 | 1e+03 | 6e+03 | 6e+04 | 1e+03 | 1e+03 | 1e+03 | 1e+03 | 1e+03 | 1e+03 | 1e+03 | 1e+03 | 1e+03 | 1e+03 | 2e+03 | 3e+03 |
| cms-heap-topk-fastpath-vector2d | rows=5 cols=256 | 1e+03 | 1e+03 | never | 1e+03 | 1e+03 | 1e+03 | 1e+06 | 6e+06 | 1e+03 | 1e+03 | 1e+03 | 1e+07 | 1e+03 | 1e+03 | 1e+03 | 1e+03 | 1e+03 | 1e+03 | 2e+03 | 3e+03 |
| cms-heap-topk-fastpath-vector2d | rows=5 cols=4096 | 1e+03 | 1e+03 | never | 2e+04 | 1e+03 | 1e+03 | 2e+03 | 2e+05 | 1e+03 | 1e+03 | 1e+03 | 1e+03 | 1e+03 | 1e+03 | 1e+03 | 1e+03 | 1e+03 | 1e+03 | 2e+03 | 3e+03 |
| countsketch-fastpath-vector2d | rows=3 cols=1024 | 1e+03 | 1e+03 | 6e+06 | never | 1e+03 | 1e+03 | 1e+05 | 1e+06 | 1e+03 | 1e+03 | 3e+04 | 2e+05 | 1e+03 | 1e+03 | 2e+04 | 3e+05 | 1e+03 | 1e+03 | 3e+06 | 3e+06 |
| countsketch-fastpath-vector2d | rows=3 cols=16384 | 1e+03 | 1e+03 | 2e+06 | never | 1e+03 | 1e+03 | 3e+05 | 3e+06 | 1e+03 | 1e+03 | 6e+05 | 6e+05 | 1e+03 | 1e+03 | 1e+05 | 6e+03 | 1e+03 | 1e+03 | 6e+06 | 3e+06 |
| countsketch-fastpath-vector2d | rows=3 cols=256 | 1e+03 | 1e+03 | 1e+07 | never | 1e+03 | 1e+03 | 2e+05 | 2e+06 | 1e+03 | 1e+03 | 6e+04 | 3e+05 | 1e+03 | 2e+03 | 2e+04 | 1e+05 | 1e+03 | 1e+04 | 6e+03 | 2e+05 |
| countsketch-fastpath-vector2d | rows=3 cols=4096 | 1e+03 | 1e+03 | never | never | 1e+03 | 1e+03 | 2e+05 | 3e+06 | 1e+03 | 1e+03 | 3e+05 | 1e+06 | 1e+03 | 1e+03 | 2e+05 | 3e+06 | 1e+03 | 1e+03 | 1e+06 | 3e+06 |
| countsketch-fastpath-vector2d | rows=5 cols=1024 | 1e+03 | 1e+03 | 2e+06 | never | 1e+03 | 1e+03 | 6e+03 | 2e+06 | 1e+03 | 1e+03 | 6e+04 | 1e+05 | 1e+03 | 1e+03 | 1e+05 | 3e+05 | 1e+03 | 1e+03 | 3e+05 | 3e+06 |
| countsketch-fastpath-vector2d | rows=5 cols=16384 | 1e+03 | 1e+03 | 3e+06 | never | 1e+03 | 1e+03 | 3e+05 | 2e+06 | 1e+03 | 1e+03 | 2e+04 | 3e+06 | 1e+03 | 1e+03 | 3e+05 | 2e+07 | 1e+03 | 1e+03 | never | 2e+07 |
| countsketch-fastpath-vector2d | rows=5 cols=256 | 1e+03 | 1e+03 | 2e+06 | never | 1e+03 | 1e+03 | 1e+04 | 6e+06 | 1e+03 | 3e+04 | 6e+04 | 2e+05 | 1e+03 | 1e+03 | 3e+04 | 6e+04 | 1e+03 | 1e+04 | 2e+05 | 6e+05 |
| countsketch-fastpath-vector2d | rows=5 cols=4096 | 1e+03 | 1e+03 | never | never | 1e+03 | 1e+03 | 2e+04 | 6e+06 | 1e+03 | 1e+03 | 2e+05 | 2e+06 | 1e+03 | 1e+03 | 6e+04 | 3e+06 | 1e+03 | 1e+03 | never | 3e+06 |

Points that never saturate by the largest N, per sketch and K:

| sketch | K | never saturated / points |
|---|---|---|
| cms-fastpath-vector2d | 1e+01 | 0 / 40 |
| cms-fastpath-vector2d | 1e+02 | 0 / 40 |
| cms-fastpath-vector2d | 1e+04 | 4 / 40 |
| cms-fastpath-vector2d | 1e+06 | 10 / 40 |
| cms-heap-topk-fastpath-vector2d | 1e+01 | 0 / 40 |
| cms-heap-topk-fastpath-vector2d | 1e+02 | 0 / 40 |
| cms-heap-topk-fastpath-vector2d | 1e+04 | 6 / 40 |
| cms-heap-topk-fastpath-vector2d | 1e+06 | 3 / 40 |
| countsketch-fastpath-vector2d | 1e+01 | 0 / 40 |
| countsketch-fastpath-vector2d | 1e+02 | 0 / 40 |
| countsketch-fastpath-vector2d | 1e+04 | 4 / 40 |
| countsketch-fastpath-vector2d | 1e+06 | 8 / 40 |

Top-k precision@k merged / single at N = 1e+07 (1.00 = no loss):

| config | θ | K | m=4 | m=16 | m=64 |
|---|---|---|---|---|---|
| rows=3 cols=1024 | 0 | 1e+01 | 1.00 | 1.00 | 1.00 |
| rows=3 cols=1024 | 0 | 1e+02 | 1.00 | 0.97 | 0.97 |
| rows=3 cols=1024 | 0 | 1e+04 | nan | nan | nan |
| rows=3 cols=1024 | 0 | 1e+06 | nan | nan | nan |
| rows=3 cols=1024 | 0.5 | 1e+01 | 1.00 | 1.00 | 1.00 |
| rows=3 cols=1024 | 0.5 | 1e+02 | 1.00 | 1.00 | 1.00 |
| rows=3 cols=1024 | 0.5 | 1e+04 | 1.00 | 1.00 | 1.00 |
| rows=3 cols=1024 | 0.5 | 1e+06 | 0.91 | 0.82 | 0.91 |
| rows=3 cols=1024 | 1 | 1e+01 | 1.00 | 1.00 | 1.00 |
| rows=3 cols=1024 | 1 | 1e+02 | 1.00 | 1.00 | 1.00 |
| rows=3 cols=1024 | 1 | 1e+04 | 1.00 | 1.00 | 1.00 |
| rows=3 cols=1024 | 1 | 1e+06 | 0.92 | 0.93 | 0.92 |
| rows=3 cols=1024 | 1.5 | 1e+01 | 1.00 | 1.00 | 1.00 |
| rows=3 cols=1024 | 1.5 | 1e+02 | 1.00 | 1.00 | 1.00 |
| rows=3 cols=1024 | 1.5 | 1e+04 | 1.00 | 1.00 | 1.00 |
| rows=3 cols=1024 | 1.5 | 1e+06 | 1.00 | 1.00 | 1.00 |
| rows=3 cols=1024 | 2 | 1e+01 | 1.00 | 1.00 | 1.00 |
| rows=3 cols=1024 | 2 | 1e+02 | 1.00 | 1.00 | 1.00 |
| rows=3 cols=1024 | 2 | 1e+04 | 1.00 | 1.00 | 1.00 |
| rows=3 cols=1024 | 2 | 1e+06 | 0.99 | 0.99 | 1.00 |
| rows=3 cols=16384 | 0 | 1e+01 | 1.00 | 1.00 | 1.00 |
| rows=3 cols=16384 | 0 | 1e+02 | 1.00 | 0.97 | 0.97 |
| rows=3 cols=16384 | 0 | 1e+04 | 0.50 | 1.00 | 0.50 |
| rows=3 cols=16384 | 0 | 1e+06 | nan | nan | nan |
| rows=3 cols=16384 | 0.5 | 1e+01 | 1.00 | 1.00 | 1.00 |
| rows=3 cols=16384 | 0.5 | 1e+02 | 1.00 | 1.00 | 1.00 |
| rows=3 cols=16384 | 0.5 | 1e+04 | 1.00 | 1.00 | 1.00 |
| rows=3 cols=16384 | 0.5 | 1e+06 | 1.00 | 1.01 | 1.00 |
| rows=3 cols=16384 | 1 | 1e+01 | 1.00 | 1.00 | 1.00 |
| rows=3 cols=16384 | 1 | 1e+02 | 1.00 | 1.00 | 1.00 |
| rows=3 cols=16384 | 1 | 1e+04 | 1.00 | 1.00 | 1.00 |
| rows=3 cols=16384 | 1 | 1e+06 | 1.00 | 1.00 | 1.00 |
| rows=3 cols=16384 | 1.5 | 1e+01 | 1.00 | 1.00 | 1.00 |
| rows=3 cols=16384 | 1.5 | 1e+02 | 1.00 | 1.00 | 1.00 |
| rows=3 cols=16384 | 1.5 | 1e+04 | 1.00 | 1.00 | 1.00 |
| rows=3 cols=16384 | 1.5 | 1e+06 | 1.00 | 1.00 | 1.00 |
| rows=3 cols=16384 | 2 | 1e+01 | 1.00 | 1.00 | 1.00 |
| rows=3 cols=16384 | 2 | 1e+02 | 1.00 | 1.00 | 1.00 |
| rows=3 cols=16384 | 2 | 1e+04 | 1.00 | 1.00 | 1.00 |
| rows=3 cols=16384 | 2 | 1e+06 | 1.00 | 1.00 | 1.00 |
| rows=3 cols=256 | 0 | 1e+01 | 1.00 | 1.00 | 1.00 |
| rows=3 cols=256 | 0 | 1e+02 | 1.00 | 0.99 | 0.99 |
| rows=3 cols=256 | 0 | 1e+04 | 1.00 | 1.00 | 1.00 |
| rows=3 cols=256 | 0 | 1e+06 | nan | nan | nan |
| rows=3 cols=256 | 0.5 | 1e+01 | 1.00 | 1.00 | 1.00 |
| rows=3 cols=256 | 0.5 | 1e+02 | 1.00 | 1.00 | 1.00 |
| rows=3 cols=256 | 0.5 | 1e+04 | 1.00 | 1.00 | 1.00 |
| rows=3 cols=256 | 0.5 | 1e+06 | 0.82 | 0.82 | 1.00 |
| rows=3 cols=256 | 1 | 1e+01 | 1.00 | 1.00 | 1.00 |
| rows=3 cols=256 | 1 | 1e+02 | 1.00 | 1.00 | 1.00 |
| rows=3 cols=256 | 1 | 1e+04 | 0.98 | 0.98 | 0.98 |
| rows=3 cols=256 | 1 | 1e+06 | 0.89 | 0.85 | 0.89 |
| rows=3 cols=256 | 1.5 | 1e+01 | 1.00 | 1.00 | 1.00 |
| rows=3 cols=256 | 1.5 | 1e+02 | 1.00 | 1.00 | 1.00 |
| rows=3 cols=256 | 1.5 | 1e+04 | 0.98 | 1.00 | 0.98 |
| rows=3 cols=256 | 1.5 | 1e+06 | 0.96 | 0.92 | 0.90 |
| rows=3 cols=256 | 2 | 1e+01 | 1.00 | 1.00 | 1.00 |
| rows=3 cols=256 | 2 | 1e+02 | 1.00 | 1.00 | 1.00 |
| rows=3 cols=256 | 2 | 1e+04 | 0.96 | 0.94 | 0.96 |
| rows=3 cols=256 | 2 | 1e+06 | 0.98 | 1.00 | 0.99 |
| rows=3 cols=4096 | 0 | 1e+01 | 1.00 | 1.00 | 1.00 |
| rows=3 cols=4096 | 0 | 1e+02 | 1.00 | 0.97 | 0.97 |
| rows=3 cols=4096 | 0 | 1e+04 | nan | nan | nan |
| rows=3 cols=4096 | 0 | 1e+06 | nan | nan | nan |
| rows=3 cols=4096 | 0.5 | 1e+01 | 1.00 | 1.00 | 1.00 |
| rows=3 cols=4096 | 0.5 | 1e+02 | 1.00 | 1.00 | 1.00 |
| rows=3 cols=4096 | 0.5 | 1e+04 | 1.00 | 1.00 | 1.00 |
| rows=3 cols=4096 | 0.5 | 1e+06 | 0.97 | 0.99 | 1.00 |
| rows=3 cols=4096 | 1 | 1e+01 | 1.00 | 1.00 | 1.00 |
| rows=3 cols=4096 | 1 | 1e+02 | 1.00 | 1.00 | 1.00 |
| rows=3 cols=4096 | 1 | 1e+04 | 1.00 | 1.00 | 1.00 |
| rows=3 cols=4096 | 1 | 1e+06 | 0.99 | 0.99 | 0.98 |
| rows=3 cols=4096 | 1.5 | 1e+01 | 1.00 | 1.00 | 1.00 |
| rows=3 cols=4096 | 1.5 | 1e+02 | 1.00 | 1.00 | 1.00 |
| rows=3 cols=4096 | 1.5 | 1e+04 | 1.00 | 1.00 | 1.00 |
| rows=3 cols=4096 | 1.5 | 1e+06 | 1.00 | 1.00 | 1.00 |
| rows=3 cols=4096 | 2 | 1e+01 | 1.00 | 1.00 | 1.00 |
| rows=3 cols=4096 | 2 | 1e+02 | 1.00 | 1.00 | 1.00 |
| rows=3 cols=4096 | 2 | 1e+04 | 1.00 | 1.00 | 1.00 |
| rows=3 cols=4096 | 2 | 1e+06 | 1.00 | 1.00 | 1.00 |
| rows=5 cols=1024 | 0 | 1e+01 | 1.00 | 1.00 | 1.00 |
| rows=5 cols=1024 | 0 | 1e+02 | 1.00 | 0.97 | 0.97 |
| rows=5 cols=1024 | 0 | 1e+04 | nan | nan | nan |
| rows=5 cols=1024 | 0 | 1e+06 | nan | nan | nan |
| rows=5 cols=1024 | 0.5 | 1e+01 | 1.00 | 1.00 | 1.00 |
| rows=5 cols=1024 | 0.5 | 1e+02 | 1.00 | 1.00 | 1.00 |
| rows=5 cols=1024 | 0.5 | 1e+04 | 1.00 | 1.00 | 1.00 |
| rows=5 cols=1024 | 0.5 | 1e+06 | 1.03 | 1.15 | 1.15 |
| rows=5 cols=1024 | 1 | 1e+01 | 1.00 | 1.00 | 1.00 |
| rows=5 cols=1024 | 1 | 1e+02 | 1.00 | 1.00 | 1.00 |
| rows=5 cols=1024 | 1 | 1e+04 | 1.00 | 1.00 | 1.00 |
| rows=5 cols=1024 | 1 | 1e+06 | 1.00 | 1.00 | 1.00 |
| rows=5 cols=1024 | 1.5 | 1e+01 | 1.00 | 1.00 | 1.00 |
| rows=5 cols=1024 | 1.5 | 1e+02 | 1.00 | 1.00 | 1.00 |
| rows=5 cols=1024 | 1.5 | 1e+04 | 1.00 | 1.00 | 1.00 |
| rows=5 cols=1024 | 1.5 | 1e+06 | 1.00 | 1.00 | 1.00 |
| rows=5 cols=1024 | 2 | 1e+01 | 1.00 | 1.00 | 1.00 |
| rows=5 cols=1024 | 2 | 1e+02 | 1.00 | 1.00 | 1.00 |
| rows=5 cols=1024 | 2 | 1e+04 | 1.00 | 1.00 | 1.00 |
| rows=5 cols=1024 | 2 | 1e+06 | 1.00 | 1.00 | 1.00 |
| rows=5 cols=16384 | 0 | 1e+01 | 1.00 | 1.00 | 1.00 |
| rows=5 cols=16384 | 0 | 1e+02 | 1.00 | 0.97 | 0.97 |
| rows=5 cols=16384 | 0 | 1e+04 | 1.00 | 1.00 | 1.00 |
| rows=5 cols=16384 | 0 | 1e+06 | nan | nan | nan |
| rows=5 cols=16384 | 0.5 | 1e+01 | 1.00 | 1.00 | 1.00 |
| rows=5 cols=16384 | 0.5 | 1e+02 | 1.00 | 1.00 | 1.00 |
| rows=5 cols=16384 | 0.5 | 1e+04 | 1.00 | 1.00 | 1.00 |
| rows=5 cols=16384 | 0.5 | 1e+06 | 1.00 | 1.00 | 1.00 |
| rows=5 cols=16384 | 1 | 1e+01 | 1.00 | 1.00 | 1.00 |
| rows=5 cols=16384 | 1 | 1e+02 | 1.00 | 1.00 | 1.00 |
| rows=5 cols=16384 | 1 | 1e+04 | 1.00 | 1.00 | 1.00 |
| rows=5 cols=16384 | 1 | 1e+06 | 1.00 | 1.00 | 1.00 |
| rows=5 cols=16384 | 1.5 | 1e+01 | 1.00 | 1.00 | 1.00 |
| rows=5 cols=16384 | 1.5 | 1e+02 | 1.00 | 1.00 | 1.00 |
| rows=5 cols=16384 | 1.5 | 1e+04 | 1.00 | 1.00 | 1.00 |
| rows=5 cols=16384 | 1.5 | 1e+06 | 1.00 | 1.00 | 1.00 |
| rows=5 cols=16384 | 2 | 1e+01 | 1.00 | 1.00 | 1.00 |
| rows=5 cols=16384 | 2 | 1e+02 | 1.00 | 1.00 | 1.00 |
| rows=5 cols=16384 | 2 | 1e+04 | 1.00 | 1.00 | 1.00 |
| rows=5 cols=16384 | 2 | 1e+06 | 1.00 | 1.00 | 1.00 |
| rows=5 cols=256 | 0 | 1e+01 | 1.00 | 1.00 | 1.00 |
| rows=5 cols=256 | 0 | 1e+02 | 1.00 | 0.97 | 0.97 |
| rows=5 cols=256 | 0 | 1e+04 | 1.00 | 1.00 | 1.00 |
| rows=5 cols=256 | 0 | 1e+06 | nan | nan | nan |
| rows=5 cols=256 | 0.5 | 1e+01 | 1.00 | 1.00 | 1.00 |
| rows=5 cols=256 | 0.5 | 1e+02 | 1.00 | 1.00 | 1.00 |
| rows=5 cols=256 | 0.5 | 1e+04 | 1.00 | 1.00 | 1.00 |
| rows=5 cols=256 | 0.5 | 1e+06 | 0.94 | 1.00 | 1.00 |
| rows=5 cols=256 | 1 | 1e+01 | 1.00 | 1.00 | 1.00 |
| rows=5 cols=256 | 1 | 1e+02 | 1.00 | 1.00 | 1.00 |
| rows=5 cols=256 | 1 | 1e+04 | 1.00 | 1.00 | 1.00 |
| rows=5 cols=256 | 1 | 1e+06 | 0.99 | 0.96 | 0.99 |
| rows=5 cols=256 | 1.5 | 1e+01 | 1.00 | 1.00 | 1.00 |
| rows=5 cols=256 | 1.5 | 1e+02 | 1.00 | 1.00 | 1.00 |
| rows=5 cols=256 | 1.5 | 1e+04 | 1.00 | 1.00 | 1.00 |
| rows=5 cols=256 | 1.5 | 1e+06 | 1.00 | 1.00 | 1.01 |
| rows=5 cols=256 | 2 | 1e+01 | 1.00 | 1.00 | 1.00 |
| rows=5 cols=256 | 2 | 1e+02 | 1.00 | 1.00 | 1.00 |
| rows=5 cols=256 | 2 | 1e+04 | 1.00 | 1.00 | 1.00 |
| rows=5 cols=256 | 2 | 1e+06 | 1.00 | 1.00 | 1.00 |
| rows=5 cols=4096 | 0 | 1e+01 | 1.00 | 1.00 | 1.00 |
| rows=5 cols=4096 | 0 | 1e+02 | 1.00 | 0.97 | 0.97 |
| rows=5 cols=4096 | 0 | 1e+04 | 1.00 | 1.00 | 1.00 |
| rows=5 cols=4096 | 0 | 1e+06 | nan | nan | nan |
| rows=5 cols=4096 | 0.5 | 1e+01 | 1.00 | 1.00 | 1.00 |
| rows=5 cols=4096 | 0.5 | 1e+02 | 1.00 | 1.00 | 1.00 |
| rows=5 cols=4096 | 0.5 | 1e+04 | 1.00 | 1.00 | 1.00 |
| rows=5 cols=4096 | 0.5 | 1e+06 | 1.00 | 0.99 | 1.00 |
| rows=5 cols=4096 | 1 | 1e+01 | 1.00 | 1.00 | 1.00 |
| rows=5 cols=4096 | 1 | 1e+02 | 1.00 | 1.00 | 1.00 |
| rows=5 cols=4096 | 1 | 1e+04 | 1.00 | 1.00 | 1.00 |
| rows=5 cols=4096 | 1 | 1e+06 | 1.00 | 1.00 | 1.00 |
| rows=5 cols=4096 | 1.5 | 1e+01 | 1.00 | 1.00 | 1.00 |
| rows=5 cols=4096 | 1.5 | 1e+02 | 1.00 | 1.00 | 1.00 |
| rows=5 cols=4096 | 1.5 | 1e+04 | 1.00 | 1.00 | 1.00 |
| rows=5 cols=4096 | 1.5 | 1e+06 | 1.00 | 1.00 | 1.00 |
| rows=5 cols=4096 | 2 | 1e+01 | 1.00 | 1.00 | 1.00 |
| rows=5 cols=4096 | 2 | 1e+02 | 1.00 | 1.00 | 1.00 |
| rows=5 cols=4096 | 2 | 1e+04 | 1.00 | 1.00 | 1.00 |
| rows=5 cols=4096 | 2 | 1e+06 | 1.00 | 1.00 | 1.00 |

11 points have single-sketch precision 0 (no ratio, shown as nan).

Largest losses: rows=3 cols=16384 θ=0 K=1e+04: 0.50, rows=3 cols=1024 θ=0.5 K=1e+06: 0.82, rows=3 cols=256 θ=0.5 K=1e+06: 0.82, rows=3 cols=256 θ=1 K=1e+06: 0.85, rows=3 cols=256 θ=1.5 K=1e+06: 0.90

## Cost phase

`saturation.csv`'s cost columns (insert, merge and query CPU, memory) are
filled for all 240 rows=3 points; rows=5 configs are scaled from rows=3 by
consumers. Every cost was measured **serially** (one approxbench process at
a time) on clnode109, by two runs:

| Points | Stream length N | How |
|---|---|---|
| 178: all 80 CMS, all 80 CountSketch, 18 top-k | 1e8 | `study_saturation.py --phase cost` |
| 62 top-k (K = 10, 100, 1e4, 1e6; 15–16 each) | 1e7 | `complete_saturation_costs.py --jobs 1 --n-max 1e7 --normalize-n 1e8` |

**Normalization.** Readers divide `insert_cpu_secs` by the point's last
curve N (1e8). The 1e7 points' insert CPU is therefore written scaled by
1e8 / 1e7, so insert CPU per item equals each record's own CPU divided by
its own N. Merge (16 shards folded into one) and the whole query phase are
written as measured.

**Why two stream lengths.** The serial 1e8 cost phase measured 178 points,
then was stopped: its remaining top-k points took 30–50 minutes each, so the
62 would have taken 25–50 hours. They were first measured 12 at a time
(below), which failed a consistency check, then re-measured serially at
N = 1e7 after this validation (`scripts/check_cost_size.py`): five points
with serial 1e8 records were re-measured at 1e7, and the 1e7 / 1e8 ratios
are:

| sketch | config | θ | K | insert per item small/full N | merge per fold small/full N | query phase small/full N | per query small/full N |
|---|---|---|---|---|---|---|---|
| cms-fastpath-vector2d | rows=3 cols=1024 | 1 | 10000 | 1.003 | 7.259 | 1.011 | 1.012 |
| countsketch-fastpath-vector2d | rows=3 cols=4096 | 0.5 | 1000000 | 1.005 | 0.933 | 1.504 | 1.507 |
| cms-heap-topk-fastpath-vector2d | rows=3 cols=256 | 0 | 100 | 1.010 | 0.967 | 0.923 | 0.920 |
| cms-heap-topk-fastpath-vector2d | rows=3 cols=256 | 1.5 | 1000000 | 1.035 | 1.051 | 1.000 | 0.935 |
| cms-heap-topk-fastpath-vector2d | rows=3 cols=256 | 2 | 100 | 0.992 | 0.988 | 1.067 | 0.941 |

Largest deviation from the full-N measurement: 625.9%

Acceptance for the top-k rerun: insert and merge within 5% of 1e8, the
query phase within 10%. The three top-k points measured insert 1.010 / 1.035
/ 0.992, merge 0.967 / 1.051 / 0.988 (1.051 is 5.1%, accepted as measured),
and query phase 0.923 / 1.000 / 1.067. The CMS merge (7.3×; a µs-scale merge,
1.9 vs 13.7 µs per fold) and the CountSketch query phase at K = 1e6 (1.5×)
fail, but neither sketch is in the 1e7 rerun set: all CMS and CountSketch
points are 1e8 measurements.

**Discarded parallel records.** The 62 top-k points were first measured 12
at a time (`complete_saturation_costs.py --jobs 12`). Re-measuring 12
serially measured points as one 12-way batch (`scripts/check_parallel_costs.py`)
showed parallel load shifts CPU time by more than 10%:

| sketch | config | θ | K | insert parallel/serial | merge parallel/serial | query parallel/serial |
|---|---|---|---|---|---|---|
| cms-fastpath-vector2d | rows=3 cols=256 | 0 | 10 | 1.196 | 1.185 | 1.000 |
| cms-fastpath-vector2d | rows=3 cols=1024 | 1 | 10000 | 1.179 | 1.059 | 1.109 |
| cms-fastpath-vector2d | rows=3 cols=4096 | 1.5 | 100 | 1.172 | 1.011 | 1.000 |
| cms-fastpath-vector2d | rows=3 cols=16384 | 2 | 1000000 | 1.198 | 1.063 | 1.104 |
| countsketch-fastpath-vector2d | rows=3 cols=256 | 0.5 | 1000000 | 1.015 | 0.042 | 1.039 |
| countsketch-fastpath-vector2d | rows=3 cols=1024 | 1 | 100 | 1.106 | 0.906 | 1.000 |
| countsketch-fastpath-vector2d | rows=3 cols=4096 | 2 | 10 | 1.173 | 1.047 | 1.400 |
| countsketch-fastpath-vector2d | rows=3 cols=16384 | 0 | 10000 | 1.187 | 0.977 | 1.063 |
| cms-heap-topk-fastpath-vector2d | rows=3 cols=256 | 0 | 10 | 1.012 | 1.032 | 1.143 |
| cms-heap-topk-fastpath-vector2d | rows=3 cols=256 | 0.5 | 1000000 | 1.073 | 1.072 | 1.315 |
| cms-heap-topk-fastpath-vector2d | rows=3 cols=256 | 1 | 10000 | 1.026 | 0.979 | 1.000 |
| cms-heap-topk-fastpath-vector2d | rows=3 cols=256 | 2 | 100 | 0.961 | 1.003 | 0.933 |

Largest deviation from serial: 95.8%

Those records are kept, unused, in the raw
`saturation_cost.parallel-62-included.jsonl` (not committed). One extra top-k
point (θ = 2, K = 1e4) also has a serial 1e8 record, from the stopped serial
rerun, kept in `saturation_cost.serial-1e8-179.jsonl`. For consistency it was
re-measured at 1e7 with the other 61.

## Files

| File | SHA-256 |
|---|---|
| `saturation.csv` (accuracy and cost) | `152327593e47b11501b8c8c24ec721fbf0c20634f7917a2f04e6ef78518529fd` |
| `saturation_curve.csv` | `789ab79bcd727e4d7805ff70f9c0440a5bab1e48721d1f4b04cd456ca2e74f70` |
| `saturation_merge_curve.csv` (top-k, m ∈ {1, 4, 16, 64}, N ≤ 1e7) | `3f5e3857fdd5d0f66a60bcaa7ae78f1d20361a4d9469c2b60b68b13061d3e57a` |

The raw per-run JSONL (about 60 MB) is not committed.

## Machine

CloudLab clnode109 (node0.zz-y-318434.softmeasure-pg0.clemson.cloudlab.us):
Intel Xeon E5-2683 v3 @ 2.00 GHz, 2 sockets × 14 cores × 2 threads, 56 CPUs.
sketch-bench at a0b8b42 (#131's head, identical in content to main's
bd644fe), approxbench release build. Accuracy phase from 2026-10-05 03:24 UTC
to 05:45 UTC; serial 1e8 cost phase 05:45–13:31 (178 points); serial 1e7
cost run 20:37–22:45 (62 points).

## Commands

```sh
cargo build -p aqpbm-cli --release
G="--families frequency,topk --thetas 0,0.5,1.0,1.5,2.0 --cardinalities 10,100,10000,1000000 --n-max 1e8 --seeds 3"
# accuracy grid (single sketch)
python3 scripts/study_saturation.py --phase accuracy $G --jobs 32 --out OUT/grid
# top-k after merging m shards (N <= 1e7)
python3 scripts/study_saturation.py --phase accuracy --families topk \
    --thetas 0,0.5,1.0,1.5,2.0 --cardinalities 10,100,10000,1000000 \
    --n-max 1e7 --seeds 3 --merge-shards-list 1,4,16,64 --jobs 14 --out OUT/merge_topk
# cost (rows=3; rows=5 is scaled by consumers): serial phase at N = 1e8
# (stopped after 178 points), then the rest serially at N = 1e7
python3 scripts/study_saturation.py --phase cost $G --cost-rows 3 --out OUT/grid
python3 scripts/complete_saturation_costs.py OUT/grid --jobs 1 --n-max 1e7 --normalize-n 1e8
# checks: 12-way parallel vs serial, and N = 1e7 vs 1e8 (keys on stdin)
python3 scripts/check_parallel_costs.py OUT/grid/saturation_cost.serial-178.jsonl OUT/parallel.jsonl --jobs 12 < keys
python3 scripts/check_cost_size.py OUT/grid/saturation_cost.serial-178.jsonl OUT/size.jsonl --n-max 1e7 < keys
# this README's tables
python3 scripts/summarize_synthetic_curves.py OUT/grid OUT/merge_topk
```
