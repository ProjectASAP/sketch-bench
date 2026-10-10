# Version 2: cost by use under a batch latency SLA

Each method's cost by use (`w_cpu · AUC(CPU) + w_mem · AUC(memory)`) at each SLA; ASAP and PerQuery plan for the SLA (the cheapest plan whose batch latency, the longest chain, is at most it), AutoSketch's one plan ignores it.

## mixed: 50 RQEs

Sanity violations: 0. Tightest feasible SLA: ASAP 87.7 ms, PerQuery 87.7 ms. AutoSketch latency 1.01e+03 ms; planning time (search + measured benchmark) 212 s.

| weights | SLA (ms) | method | cost | latency (ms) | CPU (vCPU) | GiB | meets SLA |
|---|---|---|---|---|---|---|---|
| cpu | 100 | ASAP | 36.7 | 99.8 | 36.7 | 34.8 | yes |
| cpu | 100 | ASAP (no roll-ups) | 36.7 | 99.8 | 36.7 | 34.8 | yes |
| cpu | 100 | PerQuery-CostAware | 64.5 | 97.4 | 64.5 | 76.5 | yes |
| cpu | 100 | AutoSketch-Adapted | 3.72e+03 | 1.01e+03 | 3.72e+03 | 5.62e+03 | **no** |
| cpu | 300 | ASAP | 12.7 | 291 | 12.7 | 16.5 | yes |
| cpu | 300 | ASAP (no roll-ups) | 12.7 | 291 | 12.7 | 16.5 | yes |
| cpu | 300 | PerQuery-CostAware | 24.7 | 291 | 24.7 | 68.9 | yes |
| cpu | 300 | AutoSketch-Adapted | 3.72e+03 | 1.01e+03 | 3.72e+03 | 5.62e+03 | **no** |
| cpu | 1e+03 | ASAP | 5.67 | 980 | 5.67 | 30.7 | yes |
| cpu | 1e+03 | ASAP (no roll-ups) | 5.67 | 980 | 5.67 | 30.7 | yes |
| cpu | 1e+03 | PerQuery-CostAware | 13.5 | 980 | 13.5 | 157 | yes |
| cpu | 1e+03 | AutoSketch-Adapted | 3.72e+03 | 1.01e+03 | 3.72e+03 | 5.62e+03 | **no** |
| cpu | 3e+03 | ASAP | 3.6 | 2.68e+03 | 3.6 | 18.9 | yes |
| cpu | 3e+03 | ASAP (no roll-ups) | 3.6 | 2.68e+03 | 3.6 | 18.9 | yes |
| cpu | 3e+03 | PerQuery-CostAware | 10.8 | 2.68e+03 | 10.8 | 142 | yes |
| cpu | 3e+03 | AutoSketch-Adapted | 3.72e+03 | 1.01e+03 | 3.72e+03 | 5.62e+03 | yes |
| cpu | 1e+04 | ASAP | 3.6 | 2.68e+03 | 3.6 | 18.9 | yes |
| cpu | 1e+04 | ASAP (no roll-ups) | 3.6 | 2.68e+03 | 3.6 | 18.9 | yes |
| cpu | 1e+04 | PerQuery-CostAware | 10.5 | 3.33e+03 | 10.5 | 77 | yes |
| cpu | 1e+04 | AutoSketch-Adapted | 3.72e+03 | 1.01e+03 | 3.72e+03 | 5.62e+03 | yes |
| fargate | 100 | ASAP | 1.59 | 96.9 | 37.1 | 18.8 | yes |
| fargate | 100 | ASAP (no roll-ups) | 1.59 | 96.9 | 37.1 | 18.8 | yes |
| fargate | 100 | PerQuery-CostAware | 2.95 | 97.4 | 64.6 | 75.7 | yes |
| fargate | 100 | AutoSketch-Adapted | 176 | 1.01e+03 | 3.72e+03 | 5.62e+03 | **no** |
| fargate | 300 | ASAP | 0.586 | 291 | 12.7 | 16.5 | yes |
| fargate | 300 | ASAP (no roll-ups) | 0.586 | 291 | 12.7 | 16.5 | yes |
| fargate | 300 | PerQuery-CostAware | 1.3 | 291 | 24.7 | 68.2 | yes |
| fargate | 300 | AutoSketch-Adapted | 176 | 1.01e+03 | 3.72e+03 | 5.62e+03 | **no** |
| fargate | 1e+03 | ASAP | 0.307 | 896 | 5.69 | 17.2 | yes |
| fargate | 1e+03 | ASAP (no roll-ups) | 0.307 | 896 | 5.69 | 17.2 | yes |
| fargate | 1e+03 | PerQuery-CostAware | 0.864 | 896 | 13.7 | 69.7 | yes |
| fargate | 1e+03 | AutoSketch-Adapted | 176 | 1.01e+03 | 3.72e+03 | 5.62e+03 | **no** |
| fargate | 3e+03 | ASAP | 0.23 | 2.68e+03 | 3.6 | 18.9 | yes |
| fargate | 3e+03 | ASAP (no roll-ups) | 0.23 | 2.68e+03 | 3.6 | 18.9 | yes |
| fargate | 3e+03 | PerQuery-CostAware | 0.757 | 2.68e+03 | 10.9 | 70.9 | yes |
| fargate | 3e+03 | AutoSketch-Adapted | 176 | 1.01e+03 | 3.72e+03 | 5.62e+03 | yes |
| fargate | 1e+04 | ASAP | 0.23 | 2.68e+03 | 3.6 | 18.9 | yes |
| fargate | 1e+04 | ASAP (no roll-ups) | 0.23 | 2.68e+03 | 3.6 | 18.9 | yes |
| fargate | 1e+04 | PerQuery-CostAware | 0.752 | 3.26e+03 | 10.8 | 70.9 | yes |
| fargate | 1e+04 | AutoSketch-Adapted | 176 | 1.01e+03 | 3.72e+03 | 5.62e+03 | yes |

Roll-up ablation (ASAP vs. ASAP without roll-ups):

| weights | SLA (ms) | RQEs rolled up | deployments with | deployments without | cost with | cost without | saving |
|---|---|---|---|---|---|---|---|
| cpu | 100 | 0 | 14 | 14 | 36.7 | 36.7 | 0.0% |
| cpu | 300 | 0 | 13 | 13 | 12.7 | 12.7 | 0.0% |
| cpu | 1e+03 | 0 | 9 | 9 | 5.67 | 5.67 | 0.0% |
| cpu | 3e+03 | 0 | 8 | 8 | 3.6 | 3.6 | 0.0% |
| cpu | 1e+04 | 0 | 8 | 8 | 3.6 | 3.6 | 0.0% |
| fargate | 100 | 0 | 14 | 14 | 1.59 | 1.59 | 0.0% |
| fargate | 300 | 0 | 13 | 13 | 0.586 | 0.586 | 0.0% |
| fargate | 1e+03 | 0 | 9 | 9 | 0.307 | 0.307 | 0.0% |
| fargate | 3e+03 | 0 | 8 | 8 | 0.23 | 0.23 | 0.0% |
| fargate | 1e+04 | 0 | 8 | 8 | 0.23 | 0.23 | 0.0% |

## mixed, r = 8: 92 RQEs

Sanity violations: 0. Tightest feasible SLA: ASAP 87.7 ms, PerQuery 87.7 ms. AutoSketch latency 1.01e+03 ms; planning time (search + measured benchmark) 212 s.

| weights | SLA (ms) | method | cost | latency (ms) | CPU (vCPU) | GiB | meets SLA |
|---|---|---|---|---|---|---|---|
| cpu | 100 | ASAP | 36.7 | 99.8 | 36.7 | 34.8 | yes |
| cpu | 100 | ASAP (no roll-ups) | 36.7 | 99.8 | 36.7 | 34.8 | yes |
| cpu | 100 | PerQuery-CostAware | 80.8 | 97.4 | 80.8 | 107 | yes |
| cpu | 100 | AutoSketch-Adapted | 4.46e+03 | 1.01e+03 | 4.46e+03 | 5.86e+03 | **no** |
| cpu | 300 | ASAP | 12.7 | 291 | 12.7 | 16.5 | yes |
| cpu | 300 | ASAP (no roll-ups) | 12.7 | 291 | 12.7 | 16.5 | yes |
| cpu | 300 | PerQuery-CostAware | 34.8 | 291 | 34.8 | 99.4 | yes |
| cpu | 300 | AutoSketch-Adapted | 4.46e+03 | 1.01e+03 | 4.46e+03 | 5.86e+03 | **no** |
| cpu | 1e+03 | ASAP | 5.71 | 980 | 5.71 | 30.7 | yes |
| cpu | 1e+03 | ASAP (no roll-ups) | 5.71 | 980 | 5.71 | 30.7 | yes |
| cpu | 1e+03 | PerQuery-CostAware | 21.6 | 980 | 21.6 | 187 | yes |
| cpu | 1e+03 | AutoSketch-Adapted | 4.46e+03 | 1.01e+03 | 4.46e+03 | 5.86e+03 | **no** |
| cpu | 3e+03 | ASAP | 3.66 | 2.68e+03 | 3.66 | 18.9 | yes |
| cpu | 3e+03 | ASAP (no roll-ups) | 3.66 | 2.68e+03 | 3.66 | 18.9 | yes |
| cpu | 3e+03 | PerQuery-CostAware | 18.8 | 2.68e+03 | 18.8 | 172 | yes |
| cpu | 3e+03 | AutoSketch-Adapted | 4.46e+03 | 1.01e+03 | 4.46e+03 | 5.86e+03 | yes |
| cpu | 1e+04 | ASAP | 3.66 | 2.68e+03 | 3.66 | 18.9 | yes |
| cpu | 1e+04 | ASAP (no roll-ups) | 3.66 | 2.68e+03 | 3.66 | 18.9 | yes |
| cpu | 1e+04 | PerQuery-CostAware | 18.5 | 3.33e+03 | 18.5 | 107 | yes |
| cpu | 1e+04 | AutoSketch-Adapted | 4.46e+03 | 1.01e+03 | 4.46e+03 | 5.86e+03 | yes |
| fargate | 100 | ASAP | 1.59 | 96.9 | 37.1 | 18.8 | yes |
| fargate | 100 | ASAP (no roll-ups) | 1.59 | 96.9 | 37.1 | 18.8 | yes |
| fargate | 100 | PerQuery-CostAware | 3.69 | 97.4 | 81.3 | 88.7 | yes |
| fargate | 100 | AutoSketch-Adapted | 207 | 1.01e+03 | 4.46e+03 | 5.86e+03 | **no** |
| fargate | 300 | ASAP | 0.587 | 291 | 12.7 | 16.5 | yes |
| fargate | 300 | ASAP (no roll-ups) | 0.587 | 291 | 12.7 | 16.5 | yes |
| fargate | 300 | PerQuery-CostAware | 1.78 | 291 | 35 | 81.2 | yes |
| fargate | 300 | AutoSketch-Adapted | 207 | 1.01e+03 | 4.46e+03 | 5.86e+03 | **no** |
| fargate | 1e+03 | ASAP | 0.308 | 896 | 5.72 | 17.2 | yes |
| fargate | 1e+03 | ASAP (no roll-ups) | 0.308 | 896 | 5.72 | 17.2 | yes |
| fargate | 1e+03 | PerQuery-CostAware | 1.26 | 896 | 21.9 | 83 | yes |
| fargate | 1e+03 | AutoSketch-Adapted | 207 | 1.01e+03 | 4.46e+03 | 5.86e+03 | **no** |
| fargate | 3e+03 | ASAP | 0.232 | 2.68e+03 | 3.66 | 18.9 | yes |
| fargate | 3e+03 | ASAP (no roll-ups) | 0.232 | 2.68e+03 | 3.66 | 18.9 | yes |
| fargate | 3e+03 | PerQuery-CostAware | 1.14 | 2.68e+03 | 18.9 | 84.2 | yes |
| fargate | 3e+03 | AutoSketch-Adapted | 207 | 1.01e+03 | 4.46e+03 | 5.86e+03 | yes |
| fargate | 1e+04 | ASAP | 0.232 | 2.68e+03 | 3.66 | 18.9 | yes |
| fargate | 1e+04 | ASAP (no roll-ups) | 0.232 | 2.68e+03 | 3.66 | 18.9 | yes |
| fargate | 1e+04 | PerQuery-CostAware | 1.14 | 3.26e+03 | 18.8 | 84.2 | yes |
| fargate | 1e+04 | AutoSketch-Adapted | 207 | 1.01e+03 | 4.46e+03 | 5.86e+03 | yes |

Roll-up ablation (ASAP vs. ASAP without roll-ups):

| weights | SLA (ms) | RQEs rolled up | deployments with | deployments without | cost with | cost without | saving |
|---|---|---|---|---|---|---|---|
| cpu | 100 | 0 | 14 | 14 | 36.7 | 36.7 | 0.0% |
| cpu | 300 | 0 | 13 | 13 | 12.7 | 12.7 | 0.0% |
| cpu | 1e+03 | 0 | 9 | 9 | 5.71 | 5.71 | 0.0% |
| cpu | 3e+03 | 0 | 8 | 8 | 3.66 | 3.66 | 0.0% |
| cpu | 1e+04 | 0 | 8 | 8 | 3.66 | 3.66 | 0.0% |
| fargate | 100 | 0 | 14 | 14 | 1.59 | 1.59 | 0.0% |
| fargate | 300 | 0 | 13 | 13 | 0.587 | 0.587 | 0.0% |
| fargate | 1e+03 | 0 | 9 | 9 | 0.308 | 0.308 | 0.0% |
| fargate | 3e+03 | 0 | 8 | 8 | 0.232 | 0.232 | 0.0% |
| fargate | 1e+04 | 0 | 8 | 8 | 0.232 | 0.232 | 0.0% |

## mixed + multi-grouping: 104 RQEs

Sanity violations: 0. Tightest feasible SLA: ASAP 490 ms, PerQuery 490 ms. AutoSketch latency 1.01e+03 ms; planning time (search + measured benchmark) 481 s.

| weights | SLA (ms) | method | cost | latency (ms) | CPU (vCPU) | GiB | meets SLA |
|---|---|---|---|---|---|---|---|
| cpu | 100 | ASAP | infeasible (tightest 490 ms) |||||
| cpu | 100 | ASAP (no roll-ups) | infeasible (tightest 490 ms) |||||
| cpu | 100 | PerQuery-CostAware | infeasible (tightest 490 ms) |||||
| cpu | 100 | AutoSketch-Adapted | 3.72e+03 | 1.01e+03 | 3.72e+03 | 5.63e+03 | **no** |
| cpu | 300 | ASAP | infeasible (tightest 490 ms) |||||
| cpu | 300 | ASAP (no roll-ups) | infeasible (tightest 490 ms) |||||
| cpu | 300 | PerQuery-CostAware | infeasible (tightest 490 ms) |||||
| cpu | 300 | AutoSketch-Adapted | 3.72e+03 | 1.01e+03 | 3.72e+03 | 5.63e+03 | **no** |
| cpu | 1e+03 | ASAP | 5.93 | 980 | 5.93 | 31.7 | yes |
| cpu | 1e+03 | ASAP (no roll-ups) | 6.07 | 980 | 6.07 | 33.1 | yes |
| cpu | 1e+03 | PerQuery-CostAware | 14.2 | 980 | 14.2 | 161 | yes |
| cpu | 1e+03 | AutoSketch-Adapted | 3.72e+03 | 1.01e+03 | 3.72e+03 | 5.63e+03 | **no** |
| cpu | 3e+03 | ASAP | 3.85 | 2.68e+03 | 3.85 | 19.9 | yes |
| cpu | 3e+03 | ASAP (no roll-ups) | 4 | 2.68e+03 | 4 | 21.3 | yes |
| cpu | 3e+03 | PerQuery-CostAware | 11.6 | 2.68e+03 | 11.6 | 145 | yes |
| cpu | 3e+03 | AutoSketch-Adapted | 3.72e+03 | 1.01e+03 | 3.72e+03 | 5.63e+03 | yes |
| cpu | 1e+04 | ASAP | 3.85 | 2.68e+03 | 3.85 | 19.9 | yes |
| cpu | 1e+04 | ASAP (no roll-ups) | 4 | 2.68e+03 | 4 | 21.3 | yes |
| cpu | 1e+04 | PerQuery-CostAware | 11.3 | 3.33e+03 | 11.3 | 80.5 | yes |
| cpu | 1e+04 | AutoSketch-Adapted | 3.72e+03 | 1.01e+03 | 3.72e+03 | 5.63e+03 | yes |
| fargate | 100 | ASAP | infeasible (tightest 490 ms) |||||
| fargate | 100 | ASAP (no roll-ups) | infeasible (tightest 490 ms) |||||
| fargate | 100 | PerQuery-CostAware | infeasible (tightest 490 ms) |||||
| fargate | 100 | AutoSketch-Adapted | 176 | 1.01e+03 | 3.72e+03 | 5.63e+03 | **no** |
| fargate | 300 | ASAP | infeasible (tightest 490 ms) |||||
| fargate | 300 | ASAP (no roll-ups) | infeasible (tightest 490 ms) |||||
| fargate | 300 | PerQuery-CostAware | infeasible (tightest 490 ms) |||||
| fargate | 300 | AutoSketch-Adapted | 176 | 1.01e+03 | 3.72e+03 | 5.63e+03 | **no** |
| fargate | 1e+03 | ASAP | 0.322 | 896 | 5.94 | 18.3 | yes |
| fargate | 1e+03 | ASAP (no roll-ups) | 0.334 | 959 | 6.09 | 19.7 | yes |
| fargate | 1e+03 | PerQuery-CostAware | 0.91 | 959 | 14.4 | 73.2 | yes |
| fargate | 1e+03 | AutoSketch-Adapted | 176 | 1.01e+03 | 3.72e+03 | 5.63e+03 | **no** |
| fargate | 3e+03 | ASAP | 0.245 | 2.68e+03 | 3.85 | 19.9 | yes |
| fargate | 3e+03 | ASAP (no roll-ups) | 0.257 | 2.68e+03 | 4 | 21.3 | yes |
| fargate | 3e+03 | PerQuery-CostAware | 0.803 | 2.68e+03 | 11.7 | 74.4 | yes |
| fargate | 3e+03 | AutoSketch-Adapted | 176 | 1.01e+03 | 3.72e+03 | 5.63e+03 | yes |
| fargate | 1e+04 | ASAP | 0.245 | 2.68e+03 | 3.85 | 19.9 | yes |
| fargate | 1e+04 | ASAP (no roll-ups) | 0.257 | 2.68e+03 | 4 | 21.3 | yes |
| fargate | 1e+04 | PerQuery-CostAware | 0.799 | 3.26e+03 | 11.5 | 74.4 | yes |
| fargate | 1e+04 | AutoSketch-Adapted | 176 | 1.01e+03 | 3.72e+03 | 5.63e+03 | yes |

Roll-up ablation (ASAP vs. ASAP without roll-ups):

| weights | SLA (ms) | RQEs rolled up | deployments with | deployments without | cost with | cost without | saving |
|---|---|---|---|---|---|---|---|
| cpu | 1e+03 | 42 | 15 | 31 | 5.93 | 6.07 | 2.4% |
| cpu | 3e+03 | 42 | 14 | 30 | 3.85 | 4 | 3.6% |
| cpu | 1e+04 | 42 | 14 | 30 | 3.85 | 4 | 3.6% |
| fargate | 1e+03 | 42 | 15 | 31 | 0.322 | 0.334 | 3.7% |
| fargate | 3e+03 | 42 | 14 | 30 | 0.245 | 0.257 | 4.8% |
| fargate | 1e+04 | 42 | 14 | 30 | 0.245 | 0.257 | 4.8% |

## mixed + multi-grouping, r = 8: 200 RQEs

Sanity violations: 0. Tightest feasible SLA: ASAP 490 ms, PerQuery 490 ms. AutoSketch latency 1.01e+03 ms; planning time (search + measured benchmark) 481 s.

| weights | SLA (ms) | method | cost | latency (ms) | CPU (vCPU) | GiB | meets SLA |
|---|---|---|---|---|---|---|---|
| cpu | 100 | ASAP | infeasible (tightest 490 ms) |||||
| cpu | 100 | ASAP (no roll-ups) | infeasible (tightest 490 ms) |||||
| cpu | 100 | PerQuery-CostAware | infeasible (tightest 490 ms) |||||
| cpu | 100 | AutoSketch-Adapted | 4.46e+03 | 1.01e+03 | 4.46e+03 | 5.86e+03 | **no** |
| cpu | 300 | ASAP | infeasible (tightest 490 ms) |||||
| cpu | 300 | ASAP (no roll-ups) | infeasible (tightest 490 ms) |||||
| cpu | 300 | PerQuery-CostAware | infeasible (tightest 490 ms) |||||
| cpu | 300 | AutoSketch-Adapted | 4.46e+03 | 1.01e+03 | 4.46e+03 | 5.86e+03 | **no** |
| cpu | 1e+03 | ASAP | 5.98 | 980 | 5.98 | 31.7 | yes |
| cpu | 1e+03 | ASAP (no roll-ups) | 6.13 | 980 | 6.13 | 33.1 | yes |
| cpu | 1e+03 | PerQuery-CostAware | 23.1 | 980 | 23.1 | 192 | yes |
| cpu | 1e+03 | AutoSketch-Adapted | 4.46e+03 | 1.01e+03 | 4.46e+03 | 5.86e+03 | **no** |
| cpu | 3e+03 | ASAP | 3.94 | 2.68e+03 | 3.94 | 19.9 | yes |
| cpu | 3e+03 | ASAP (no roll-ups) | 4.08 | 2.68e+03 | 4.08 | 21.3 | yes |
| cpu | 3e+03 | PerQuery-CostAware | 20.3 | 2.68e+03 | 20.3 | 176 | yes |
| cpu | 3e+03 | AutoSketch-Adapted | 4.46e+03 | 1.01e+03 | 4.46e+03 | 5.86e+03 | yes |
| cpu | 1e+04 | ASAP | 3.94 | 2.68e+03 | 3.94 | 19.9 | yes |
| cpu | 1e+04 | ASAP (no roll-ups) | 4.08 | 2.68e+03 | 4.08 | 21.3 | yes |
| cpu | 1e+04 | PerQuery-CostAware | 19.9 | 3.33e+03 | 19.9 | 112 | yes |
| cpu | 1e+04 | AutoSketch-Adapted | 4.46e+03 | 1.01e+03 | 4.46e+03 | 5.86e+03 | yes |
| fargate | 100 | ASAP | infeasible (tightest 490 ms) |||||
| fargate | 100 | ASAP (no roll-ups) | infeasible (tightest 490 ms) |||||
| fargate | 100 | PerQuery-CostAware | infeasible (tightest 490 ms) |||||
| fargate | 100 | AutoSketch-Adapted | 207 | 1.01e+03 | 4.46e+03 | 5.86e+03 | **no** |
| fargate | 300 | ASAP | infeasible (tightest 490 ms) |||||
| fargate | 300 | ASAP (no roll-ups) | infeasible (tightest 490 ms) |||||
| fargate | 300 | PerQuery-CostAware | infeasible (tightest 490 ms) |||||
| fargate | 300 | AutoSketch-Adapted | 207 | 1.01e+03 | 4.46e+03 | 5.86e+03 | **no** |
| fargate | 1e+03 | ASAP | 0.324 | 896 | 5.99 | 18.3 | yes |
| fargate | 1e+03 | ASAP (no roll-ups) | 0.336 | 959 | 6.14 | 19.7 | yes |
| fargate | 1e+03 | PerQuery-CostAware | 1.33 | 959 | 23.3 | 87.4 | yes |
| fargate | 1e+03 | AutoSketch-Adapted | 207 | 1.01e+03 | 4.46e+03 | 5.86e+03 | **no** |
| fargate | 3e+03 | ASAP | 0.248 | 2.68e+03 | 3.94 | 19.9 | yes |
| fargate | 3e+03 | ASAP (no roll-ups) | 0.26 | 2.68e+03 | 4.08 | 21.3 | yes |
| fargate | 3e+03 | PerQuery-CostAware | 1.22 | 2.68e+03 | 20.4 | 88.7 | yes |
| fargate | 3e+03 | AutoSketch-Adapted | 207 | 1.01e+03 | 4.46e+03 | 5.86e+03 | yes |
| fargate | 1e+04 | ASAP | 0.248 | 2.68e+03 | 3.94 | 19.9 | yes |
| fargate | 1e+04 | ASAP (no roll-ups) | 0.26 | 2.68e+03 | 4.08 | 21.3 | yes |
| fargate | 1e+04 | PerQuery-CostAware | 1.22 | 3.26e+03 | 20.3 | 88.7 | yes |
| fargate | 1e+04 | AutoSketch-Adapted | 207 | 1.01e+03 | 4.46e+03 | 5.86e+03 | yes |

Roll-up ablation (ASAP vs. ASAP without roll-ups):

| weights | SLA (ms) | RQEs rolled up | deployments with | deployments without | cost with | cost without | saving |
|---|---|---|---|---|---|---|---|
| cpu | 1e+03 | 84 | 15 | 31 | 5.98 | 6.13 | 2.4% |
| cpu | 3e+03 | 84 | 14 | 30 | 3.94 | 4.08 | 3.5% |
| cpu | 1e+04 | 84 | 14 | 30 | 3.94 | 4.08 | 3.5% |
| fargate | 1e+03 | 84 | 15 | 31 | 0.324 | 0.336 | 3.6% |
| fargate | 3e+03 | 84 | 14 | 30 | 0.248 | 0.26 | 4.7% |
| fargate | 1e+04 | 84 | 14 | 30 | 0.248 | 0.26 | 4.7% |

## mixed, m = 8: 400 RQEs

Sanity violations: 0. Tightest feasible SLA: ASAP 87.7 ms, PerQuery 87.7 ms. AutoSketch latency 1.01e+03 ms; planning time (search + measured benchmark) 1.7e+03 s.

| weights | SLA (ms) | method | cost | latency (ms) | CPU (vCPU) | GiB | meets SLA |
|---|---|---|---|---|---|---|---|
| cpu | 100 | ASAP | 294 | 99.8 | 294 | 278 | yes |
| cpu | 100 | ASAP (no roll-ups) | 294 | 99.8 | 294 | 278 | yes |
| cpu | 100 | PerQuery-CostAware | 516 | 97.4 | 516 | 612 | yes |
| cpu | 100 | AutoSketch-Adapted | 2.97e+04 | 1.01e+03 | 2.97e+04 | 4.5e+04 | **no** |
| cpu | 300 | ASAP | 101 | 291 | 101 | 132 | yes |
| cpu | 300 | ASAP (no roll-ups) | 101 | 291 | 101 | 132 | yes |
| cpu | 300 | PerQuery-CostAware | 198 | 291 | 198 | 551 | yes |
| cpu | 300 | AutoSketch-Adapted | 2.97e+04 | 1.01e+03 | 2.97e+04 | 4.5e+04 | **no** |
| cpu | 1e+03 | ASAP | 45.4 | 980 | 45.4 | 245 | yes |
| cpu | 1e+03 | ASAP (no roll-ups) | 45.4 | 980 | 45.4 | 245 | yes |
| cpu | 1e+03 | PerQuery-CostAware | 108 | 980 | 108 | 1.26e+03 | yes |
| cpu | 1e+03 | AutoSketch-Adapted | 2.97e+04 | 1.01e+03 | 2.97e+04 | 4.5e+04 | **no** |
| cpu | 3e+03 | ASAP | 28.8 | 2.68e+03 | 28.8 | 151 | yes |
| cpu | 3e+03 | ASAP (no roll-ups) | 28.8 | 2.68e+03 | 28.8 | 151 | yes |
| cpu | 3e+03 | PerQuery-CostAware | 86.7 | 2.68e+03 | 86.7 | 1.13e+03 | yes |
| cpu | 3e+03 | AutoSketch-Adapted | 2.97e+04 | 1.01e+03 | 2.97e+04 | 4.5e+04 | yes |
| cpu | 1e+04 | ASAP | 28.8 | 2.68e+03 | 28.8 | 151 | yes |
| cpu | 1e+04 | ASAP (no roll-ups) | 28.8 | 2.68e+03 | 28.8 | 151 | yes |
| cpu | 1e+04 | PerQuery-CostAware | 84.1 | 3.33e+03 | 84.1 | 616 | yes |
| cpu | 1e+04 | AutoSketch-Adapted | 2.97e+04 | 1.01e+03 | 2.97e+04 | 4.5e+04 | yes |
| fargate | 100 | ASAP | 12.7 | 96.9 | 297 | 151 | yes |
| fargate | 100 | ASAP (no roll-ups) | 12.7 | 96.9 | 297 | 151 | yes |
| fargate | 100 | PerQuery-CostAware | 23.6 | 97.4 | 516 | 606 | yes |
| fargate | 100 | AutoSketch-Adapted | 1.4e+03 | 1.01e+03 | 2.97e+04 | 4.5e+04 | **no** |
| fargate | 300 | ASAP | 4.69 | 291 | 101 | 132 | yes |
| fargate | 300 | ASAP (no roll-ups) | 4.69 | 291 | 101 | 132 | yes |
| fargate | 300 | PerQuery-CostAware | 10.4 | 291 | 198 | 545 | yes |
| fargate | 300 | AutoSketch-Adapted | 1.4e+03 | 1.01e+03 | 2.97e+04 | 4.5e+04 | **no** |
| fargate | 1e+03 | ASAP | 2.46 | 896 | 45.5 | 138 | yes |
| fargate | 1e+03 | ASAP (no roll-ups) | 2.46 | 896 | 45.5 | 138 | yes |
| fargate | 1e+03 | PerQuery-CostAware | 6.91 | 896 | 109 | 558 | yes |
| fargate | 1e+03 | AutoSketch-Adapted | 1.4e+03 | 1.01e+03 | 2.97e+04 | 4.5e+04 | **no** |
| fargate | 3e+03 | ASAP | 1.84 | 2.68e+03 | 28.8 | 151 | yes |
| fargate | 3e+03 | ASAP (no roll-ups) | 1.84 | 2.68e+03 | 28.8 | 151 | yes |
| fargate | 3e+03 | PerQuery-CostAware | 6.06 | 2.68e+03 | 87.2 | 568 | yes |
| fargate | 3e+03 | AutoSketch-Adapted | 1.4e+03 | 1.01e+03 | 2.97e+04 | 4.5e+04 | yes |
| fargate | 1e+04 | ASAP | 1.84 | 2.68e+03 | 28.8 | 151 | yes |
| fargate | 1e+04 | ASAP (no roll-ups) | 1.84 | 2.68e+03 | 28.8 | 151 | yes |
| fargate | 1e+04 | PerQuery-CostAware | 6.02 | 3.26e+03 | 86.2 | 568 | yes |
| fargate | 1e+04 | AutoSketch-Adapted | 1.4e+03 | 1.01e+03 | 2.97e+04 | 4.5e+04 | yes |

Roll-up ablation (ASAP vs. ASAP without roll-ups):

| weights | SLA (ms) | RQEs rolled up | deployments with | deployments without | cost with | cost without | saving |
|---|---|---|---|---|---|---|---|
| cpu | 100 | 0 | 112 | 112 | 294 | 294 | 0.0% |
| cpu | 300 | 0 | 104 | 104 | 101 | 101 | 0.0% |
| cpu | 1e+03 | 0 | 72 | 72 | 45.4 | 45.4 | 0.0% |
| cpu | 3e+03 | 0 | 64 | 64 | 28.8 | 28.8 | 0.0% |
| cpu | 1e+04 | 0 | 64 | 64 | 28.8 | 28.8 | 0.0% |
| fargate | 100 | 0 | 112 | 112 | 12.7 | 12.7 | 0.0% |
| fargate | 300 | 0 | 104 | 104 | 4.69 | 4.69 | 0.0% |
| fargate | 1e+03 | 0 | 72 | 72 | 2.46 | 2.46 | 0.0% |
| fargate | 3e+03 | 0 | 64 | 64 | 1.84 | 1.84 | 0.0% |
| fargate | 1e+04 | 0 | 64 | 64 | 1.84 | 1.84 | 0.0% |

## mixed, m = 16: 800 RQEs

Sanity violations: 0. Tightest feasible SLA: ASAP 87.7 ms, PerQuery 87.7 ms. AutoSketch latency 1.01e+03 ms; planning time (search + measured benchmark) 3.39e+03 s.

| weights | SLA (ms) | method | cost | latency (ms) | CPU (vCPU) | GiB | meets SLA |
|---|---|---|---|---|---|---|---|
| cpu | 100 | ASAP | 588 | 99.8 | 588 | 556 | yes |
| cpu | 100 | ASAP (no roll-ups) | 588 | 99.8 | 588 | 556 | yes |
| cpu | 100 | PerQuery-CostAware | 1.03e+03 | 97.4 | 1.03e+03 | 1.22e+03 | yes |
| cpu | 100 | AutoSketch-Adapted | 5.95e+04 | 1.01e+03 | 5.95e+04 | 9e+04 | **no** |
| cpu | 300 | ASAP | 203 | 291 | 203 | 264 | yes |
| cpu | 300 | ASAP (no roll-ups) | 203 | 291 | 203 | 264 | yes |
| cpu | 300 | PerQuery-CostAware | 395 | 291 | 395 | 1.1e+03 | yes |
| cpu | 300 | AutoSketch-Adapted | 5.95e+04 | 1.01e+03 | 5.95e+04 | 9e+04 | **no** |
| cpu | 1e+03 | ASAP | 90.8 | 980 | 90.8 | 491 | yes |
| cpu | 1e+03 | ASAP (no roll-ups) | 90.8 | 980 | 90.8 | 491 | yes |
| cpu | 1e+03 | PerQuery-CostAware | 215 | 980 | 215 | 2.52e+03 | yes |
| cpu | 1e+03 | AutoSketch-Adapted | 5.95e+04 | 1.01e+03 | 5.95e+04 | 9e+04 | **no** |
| cpu | 3e+03 | ASAP | 57.6 | 2.68e+03 | 57.6 | 302 | yes |
| cpu | 3e+03 | ASAP (no roll-ups) | 57.6 | 2.68e+03 | 57.6 | 302 | yes |
| cpu | 3e+03 | PerQuery-CostAware | 173 | 2.68e+03 | 173 | 2.27e+03 | yes |
| cpu | 3e+03 | AutoSketch-Adapted | 5.95e+04 | 1.01e+03 | 5.95e+04 | 9e+04 | yes |
| cpu | 1e+04 | ASAP | 57.6 | 2.68e+03 | 57.6 | 302 | yes |
| cpu | 1e+04 | ASAP (no roll-ups) | 57.6 | 2.68e+03 | 57.6 | 302 | yes |
| cpu | 1e+04 | PerQuery-CostAware | 168 | 3.33e+03 | 168 | 1.23e+03 | yes |
| cpu | 1e+04 | AutoSketch-Adapted | 5.95e+04 | 1.01e+03 | 5.95e+04 | 9e+04 | yes |
| fargate | 100 | ASAP | 25.4 | 96.9 | 593 | 301 | yes |
| fargate | 100 | ASAP (no roll-ups) | 25.4 | 96.9 | 593 | 301 | yes |
| fargate | 100 | PerQuery-CostAware | 47.2 | 97.4 | 1.03e+03 | 1.21e+03 | yes |
| fargate | 100 | AutoSketch-Adapted | 2.81e+03 | 1.01e+03 | 5.95e+04 | 9e+04 | **no** |
| fargate | 300 | ASAP | 9.38 | 291 | 203 | 264 | yes |
| fargate | 300 | ASAP (no roll-ups) | 9.38 | 291 | 203 | 264 | yes |
| fargate | 300 | PerQuery-CostAware | 20.9 | 291 | 395 | 1.09e+03 | yes |
| fargate | 300 | AutoSketch-Adapted | 2.81e+03 | 1.01e+03 | 5.95e+04 | 9e+04 | **no** |
| fargate | 1e+03 | ASAP | 4.91 | 896 | 91 | 276 | yes |
| fargate | 1e+03 | ASAP (no roll-ups) | 4.91 | 896 | 91 | 276 | yes |
| fargate | 1e+03 | PerQuery-CostAware | 13.8 | 896 | 219 | 1.12e+03 | yes |
| fargate | 1e+03 | AutoSketch-Adapted | 2.81e+03 | 1.01e+03 | 5.95e+04 | 9e+04 | **no** |
| fargate | 3e+03 | ASAP | 3.68 | 2.68e+03 | 57.6 | 302 | yes |
| fargate | 3e+03 | ASAP (no roll-ups) | 3.68 | 2.68e+03 | 57.6 | 302 | yes |
| fargate | 3e+03 | PerQuery-CostAware | 12.1 | 2.68e+03 | 174 | 1.14e+03 | yes |
| fargate | 3e+03 | AutoSketch-Adapted | 2.81e+03 | 1.01e+03 | 5.95e+04 | 9e+04 | yes |
| fargate | 1e+04 | ASAP | 3.68 | 2.68e+03 | 57.6 | 302 | yes |
| fargate | 1e+04 | ASAP (no roll-ups) | 3.68 | 2.68e+03 | 57.6 | 302 | yes |
| fargate | 1e+04 | PerQuery-CostAware | 12 | 3.26e+03 | 172 | 1.14e+03 | yes |
| fargate | 1e+04 | AutoSketch-Adapted | 2.81e+03 | 1.01e+03 | 5.95e+04 | 9e+04 | yes |

Roll-up ablation (ASAP vs. ASAP without roll-ups):

| weights | SLA (ms) | RQEs rolled up | deployments with | deployments without | cost with | cost without | saving |
|---|---|---|---|---|---|---|---|
| cpu | 100 | 0 | 224 | 224 | 588 | 588 | 0.0% |
| cpu | 300 | 0 | 208 | 208 | 203 | 203 | 0.0% |
| cpu | 1e+03 | 0 | 144 | 144 | 90.8 | 90.8 | 0.0% |
| cpu | 3e+03 | 0 | 128 | 128 | 57.6 | 57.6 | 0.0% |
| cpu | 1e+04 | 0 | 128 | 128 | 57.6 | 57.6 | 0.0% |
| fargate | 100 | 0 | 224 | 224 | 25.4 | 25.4 | 0.0% |
| fargate | 300 | 0 | 208 | 208 | 9.38 | 9.38 | 0.0% |
| fargate | 1e+03 | 0 | 144 | 144 | 4.91 | 4.91 | 0.0% |
| fargate | 3e+03 | 0 | 128 | 128 | 3.68 | 3.68 | 0.0% |
| fargate | 1e+04 | 0 | 128 | 128 | 3.68 | 3.68 | 0.0% |

## mixed + multi-grouping, m = 8: 832 RQEs

Sanity violations: 0. Tightest feasible SLA: ASAP 490 ms, PerQuery 490 ms. AutoSketch latency 1.01e+03 ms; planning time (search + measured benchmark) 3.85e+03 s.

| weights | SLA (ms) | method | cost | latency (ms) | CPU (vCPU) | GiB | meets SLA |
|---|---|---|---|---|---|---|---|
| cpu | 100 | ASAP | infeasible (tightest 490 ms) |||||
| cpu | 100 | ASAP (no roll-ups) | infeasible (tightest 490 ms) |||||
| cpu | 100 | PerQuery-CostAware | infeasible (tightest 490 ms) |||||
| cpu | 100 | AutoSketch-Adapted | 2.98e+04 | 1.01e+03 | 2.98e+04 | 4.5e+04 | **no** |
| cpu | 300 | ASAP | infeasible (tightest 490 ms) |||||
| cpu | 300 | ASAP (no roll-ups) | infeasible (tightest 490 ms) |||||
| cpu | 300 | PerQuery-CostAware | infeasible (tightest 490 ms) |||||
| cpu | 300 | AutoSketch-Adapted | 2.98e+04 | 1.01e+03 | 2.98e+04 | 4.5e+04 | **no** |
| cpu | 1e+03 | ASAP | 47.4 | 980 | 47.4 | 253 | yes |
| cpu | 1e+03 | ASAP (no roll-ups) | 48.6 | 980 | 48.6 | 265 | yes |
| cpu | 1e+03 | PerQuery-CostAware | 114 | 980 | 114 | 1.29e+03 | yes |
| cpu | 1e+03 | AutoSketch-Adapted | 2.98e+04 | 1.01e+03 | 2.98e+04 | 4.5e+04 | **no** |
| cpu | 3e+03 | ASAP | 30.8 | 2.68e+03 | 30.8 | 159 | yes |
| cpu | 3e+03 | ASAP (no roll-ups) | 32 | 2.68e+03 | 32 | 170 | yes |
| cpu | 3e+03 | PerQuery-CostAware | 92.8 | 2.68e+03 | 92.8 | 1.16e+03 | yes |
| cpu | 3e+03 | AutoSketch-Adapted | 2.98e+04 | 1.01e+03 | 2.98e+04 | 4.5e+04 | yes |
| cpu | 1e+04 | ASAP | 30.8 | 2.68e+03 | 30.8 | 159 | yes |
| cpu | 1e+04 | ASAP (no roll-ups) | 32 | 2.68e+03 | 32 | 170 | yes |
| cpu | 1e+04 | PerQuery-CostAware | 90.3 | 3.33e+03 | 90.3 | 644 | yes |
| cpu | 1e+04 | AutoSketch-Adapted | 2.98e+04 | 1.01e+03 | 2.98e+04 | 4.5e+04 | yes |
| fargate | 100 | ASAP | infeasible (tightest 490 ms) |||||
| fargate | 100 | ASAP (no roll-ups) | infeasible (tightest 490 ms) |||||
| fargate | 100 | PerQuery-CostAware | infeasible (tightest 490 ms) |||||
| fargate | 100 | AutoSketch-Adapted | 1.41e+03 | 1.01e+03 | 2.98e+04 | 4.5e+04 | **no** |
| fargate | 300 | ASAP | infeasible (tightest 490 ms) |||||
| fargate | 300 | ASAP (no roll-ups) | infeasible (tightest 490 ms) |||||
| fargate | 300 | PerQuery-CostAware | infeasible (tightest 490 ms) |||||
| fargate | 300 | AutoSketch-Adapted | 1.41e+03 | 1.01e+03 | 2.98e+04 | 4.5e+04 | **no** |
| fargate | 1e+03 | ASAP | 2.58 | 896 | 47.6 | 146 | yes |
| fargate | 1e+03 | ASAP (no roll-ups) | 2.67 | 959 | 48.7 | 158 | yes |
| fargate | 1e+03 | PerQuery-CostAware | 7.28 | 959 | 115 | 585 | yes |
| fargate | 1e+03 | AutoSketch-Adapted | 1.41e+03 | 1.01e+03 | 2.98e+04 | 4.5e+04 | **no** |
| fargate | 3e+03 | ASAP | 1.96 | 2.68e+03 | 30.8 | 159 | yes |
| fargate | 3e+03 | ASAP (no roll-ups) | 2.05 | 2.68e+03 | 32 | 170 | yes |
| fargate | 3e+03 | PerQuery-CostAware | 6.43 | 2.68e+03 | 93.3 | 595 | yes |
| fargate | 3e+03 | AutoSketch-Adapted | 1.41e+03 | 1.01e+03 | 2.98e+04 | 4.5e+04 | yes |
| fargate | 1e+04 | ASAP | 1.96 | 2.68e+03 | 30.8 | 159 | yes |
| fargate | 1e+04 | ASAP (no roll-ups) | 2.05 | 2.68e+03 | 32 | 170 | yes |
| fargate | 1e+04 | PerQuery-CostAware | 6.39 | 3.26e+03 | 92.3 | 595 | yes |
| fargate | 1e+04 | AutoSketch-Adapted | 1.41e+03 | 1.01e+03 | 2.98e+04 | 4.5e+04 | yes |

Roll-up ablation (ASAP vs. ASAP without roll-ups):

| weights | SLA (ms) | RQEs rolled up | deployments with | deployments without | cost with | cost without | saving |
|---|---|---|---|---|---|---|---|
| cpu | 1e+03 | 336 | 120 | 248 | 47.4 | 48.6 | 2.4% |
| cpu | 3e+03 | 336 | 112 | 240 | 30.8 | 32 | 3.6% |
| cpu | 1e+04 | 336 | 112 | 240 | 30.8 | 32 | 3.6% |
| fargate | 1e+03 | 336 | 120 | 248 | 2.58 | 2.67 | 3.7% |
| fargate | 3e+03 | 336 | 112 | 240 | 1.96 | 2.05 | 4.8% |
| fargate | 1e+04 | 336 | 112 | 240 | 1.96 | 2.05 | 4.8% |

## mixed + multi-grouping, m = 16: 1664 RQEs

Sanity violations: 0. Tightest feasible SLA: ASAP 490 ms, PerQuery 490 ms. AutoSketch latency 1.01e+03 ms; planning time (search + measured benchmark) 7.7e+03 s.

| weights | SLA (ms) | method | cost | latency (ms) | CPU (vCPU) | GiB | meets SLA |
|---|---|---|---|---|---|---|---|
| cpu | 100 | ASAP | infeasible (tightest 490 ms) |||||
| cpu | 100 | ASAP (no roll-ups) | infeasible (tightest 490 ms) |||||
| cpu | 100 | PerQuery-CostAware | infeasible (tightest 490 ms) |||||
| cpu | 100 | AutoSketch-Adapted | 5.96e+04 | 1.01e+03 | 5.96e+04 | 9e+04 | **no** |
| cpu | 300 | ASAP | infeasible (tightest 490 ms) |||||
| cpu | 300 | ASAP (no roll-ups) | infeasible (tightest 490 ms) |||||
| cpu | 300 | PerQuery-CostAware | infeasible (tightest 490 ms) |||||
| cpu | 300 | AutoSketch-Adapted | 5.96e+04 | 1.01e+03 | 5.96e+04 | 9e+04 | **no** |
| cpu | 1e+03 | ASAP | 94.9 | 980 | 94.9 | 507 | yes |
| cpu | 1e+03 | ASAP (no roll-ups) | 97.2 | 980 | 97.2 | 530 | yes |
| cpu | 1e+03 | PerQuery-CostAware | 228 | 980 | 228 | 2.57e+03 | yes |
| cpu | 1e+03 | AutoSketch-Adapted | 5.96e+04 | 1.01e+03 | 5.96e+04 | 9e+04 | **no** |
| cpu | 3e+03 | ASAP | 61.7 | 2.68e+03 | 61.7 | 318 | yes |
| cpu | 3e+03 | ASAP (no roll-ups) | 64 | 2.68e+03 | 64 | 341 | yes |
| cpu | 3e+03 | PerQuery-CostAware | 186 | 2.68e+03 | 186 | 2.32e+03 | yes |
| cpu | 3e+03 | AutoSketch-Adapted | 5.96e+04 | 1.01e+03 | 5.96e+04 | 9e+04 | yes |
| cpu | 1e+04 | ASAP | 61.7 | 2.68e+03 | 61.7 | 318 | yes |
| cpu | 1e+04 | ASAP (no roll-ups) | 64 | 2.68e+03 | 64 | 341 | yes |
| cpu | 1e+04 | PerQuery-CostAware | 181 | 3.33e+03 | 181 | 1.29e+03 | yes |
| cpu | 1e+04 | AutoSketch-Adapted | 5.96e+04 | 1.01e+03 | 5.96e+04 | 9e+04 | yes |
| fargate | 100 | ASAP | infeasible (tightest 490 ms) |||||
| fargate | 100 | ASAP (no roll-ups) | infeasible (tightest 490 ms) |||||
| fargate | 100 | PerQuery-CostAware | infeasible (tightest 490 ms) |||||
| fargate | 100 | AutoSketch-Adapted | 2.81e+03 | 1.01e+03 | 5.96e+04 | 9e+04 | **no** |
| fargate | 300 | ASAP | infeasible (tightest 490 ms) |||||
| fargate | 300 | ASAP (no roll-ups) | infeasible (tightest 490 ms) |||||
| fargate | 300 | PerQuery-CostAware | infeasible (tightest 490 ms) |||||
| fargate | 300 | AutoSketch-Adapted | 2.81e+03 | 1.01e+03 | 5.96e+04 | 9e+04 | **no** |
| fargate | 1e+03 | ASAP | 5.15 | 896 | 95.1 | 292 | yes |
| fargate | 1e+03 | ASAP (no roll-ups) | 5.35 | 959 | 97.4 | 315 | yes |
| fargate | 1e+03 | PerQuery-CostAware | 14.6 | 959 | 231 | 1.17e+03 | yes |
| fargate | 1e+03 | AutoSketch-Adapted | 2.81e+03 | 1.01e+03 | 5.96e+04 | 9e+04 | **no** |
| fargate | 3e+03 | ASAP | 3.91 | 2.68e+03 | 61.7 | 318 | yes |
| fargate | 3e+03 | ASAP (no roll-ups) | 4.11 | 2.68e+03 | 64 | 341 | yes |
| fargate | 3e+03 | PerQuery-CostAware | 12.9 | 2.68e+03 | 187 | 1.19e+03 | yes |
| fargate | 3e+03 | AutoSketch-Adapted | 2.81e+03 | 1.01e+03 | 5.96e+04 | 9e+04 | yes |
| fargate | 1e+04 | ASAP | 3.91 | 2.68e+03 | 61.7 | 318 | yes |
| fargate | 1e+04 | ASAP (no roll-ups) | 4.11 | 2.68e+03 | 64 | 341 | yes |
| fargate | 1e+04 | PerQuery-CostAware | 12.8 | 3.26e+03 | 185 | 1.19e+03 | yes |
| fargate | 1e+04 | AutoSketch-Adapted | 2.81e+03 | 1.01e+03 | 5.96e+04 | 9e+04 | yes |

Roll-up ablation (ASAP vs. ASAP without roll-ups):

| weights | SLA (ms) | RQEs rolled up | deployments with | deployments without | cost with | cost without | saving |
|---|---|---|---|---|---|---|---|
| cpu | 1e+03 | 672 | 240 | 496 | 94.9 | 97.2 | 2.4% |
| cpu | 3e+03 | 672 | 224 | 480 | 61.7 | 64 | 3.6% |
| cpu | 1e+04 | 672 | 224 | 480 | 61.7 | 64 | 3.6% |
| fargate | 1e+03 | 672 | 240 | 496 | 5.15 | 5.35 | 3.7% |
| fargate | 3e+03 | 672 | 224 | 480 | 3.91 | 4.11 | 4.8% |
| fargate | 1e+04 | 672 | 224 | 480 | 3.91 | 4.11 | 4.8% |
