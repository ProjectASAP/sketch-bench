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

## Cost phase: pending

Pending: 62 points running in parallel, plus a serial-vs-parallel
consistency check.

The serial cost phase (`study_saturation.py --phase cost`) measured 178 of
the 240 rows=3 points (all CMS and CountSketch, and 18 top-k), then was
stopped: its remaining top-k points took 10–20 minutes each. The other 62
top-k points are being measured with `scripts/complete_saturation_costs.py
--jobs 12`, which runs the cost phase's exact approxbench command and appends
to the same file. Because CPU timing can shift under parallel load,
`scripts/check_parallel_costs.py` will re-measure 12 serially measured points
(4 CMS, 4 CountSketch, 4 top-k, covering every K and θ) as one 12-way batch,
and compare insert/merge/query CPU against the serial values. If they differ
by more than about 10%, the 62 points will be re-run serially. This section
and `saturation.csv`'s cost columns will be filled in when that finishes,
listing which points were measured in parallel. Until then the cost columns
of `saturation.csv` are empty.

## Files

| File | SHA-256 |
|---|---|
| `saturation.csv` (accuracy only; cost columns pending) | `72677a2a074f109e160b76821f360318cb3faffe8a8fcada5b1f0cd82e51f544` |
| `saturation_curve.csv` | `789ab79bcd727e4d7805ff70f9c0440a5bab1e48721d1f4b04cd456ca2e74f70` |
| `saturation_merge_curve.csv` (top-k, m ∈ {1, 4, 16, 64}, N ≤ 1e7) | `3f5e3857fdd5d0f66a60bcaa7ae78f1d20361a4d9469c2b60b68b13061d3e57a` |

The raw per-run JSONL (32 MB) is not committed.

## Machine

CloudLab clnode109 (node0.zz-y-318434.softmeasure-pg0.clemson.cloudlab.us):
Intel Xeon E5-2683 v3 @ 2.00 GHz, 2 sockets × 14 cores × 2 threads, 56 CPUs.
sketch-bench at a0b8b42 (#131's head, identical in content to main's
bd644fe), approxbench release build. Accuracy phase from 2026-10-05 03:24 UTC
to 05:45 UTC.

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
# cost (rows=3; rows=5 is scaled by consumers): serial phase, then the remainder
python3 scripts/study_saturation.py --phase cost $G --cost-rows 3 --out OUT/grid
python3 scripts/complete_saturation_costs.py OUT/grid --jobs 12
# this README's tables
python3 scripts/summarize_synthetic_curves.py OUT/grid OUT/merge_topk
```
