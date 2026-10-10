# Synthetic workload: AutoSketch vs. ASAP, cost by use

Cost by use: `w_cpu · AUC(CPU) + w_mem · AUC(memory)` (CPU only, in vCPU; Fargate prices, in $/hour). Latency: a plan's query latency (its longest chain, compaction then query, CPU elastic), and the median over its RQEs. AutoSketch's planning time is its search plus its measured benchmark.

## mixed + multi-grouping

104 RQEs on 11 streams; dropped: 0; sanity violations: 0. AutoSketch: 292 probes, 19 distinct (metric, config); measured benchmark 481 s at N = 1e+08 (paper rate 1.14e+03 s). AutoSketch latency 1.01e+03 ms; tightest feasible bound 490 ms.

| weights | method | cost (unbounded) | CPU (vCPU) | GiB | latency (ms) | median latency (ms) | planning (s) | cost at AutoSketch's latency |
|---|---|---|---|---|---|---|---|---|
| cpu | ASAP | 3.79 | 3.79 | 29.9 | 1.34e+04 | 62 | 1.05 | 5.93 |
| cpu | ASAP (no roll-ups) | 3.94 | 3.94 | 31.4 | 1.34e+04 | 51.1 | 1.01 | 6.07 |
| cpu | PerQuery-CostAware | 11.3 | 11.3 | 80.5 | 3.33e+03 | 59.9 | 0.832 | 14.2 |
| cpu | AutoSketch-Adapted | 3.72e+03 | 3.72e+03 | 5.63e+03 | 1.01e+03 | 10.3 | 481 | 3.72e+03 |
| fargate | ASAP | 0.245 | 3.85 | 19.9 | 2.68e+03 | 51.1 | 1.26 | 0.322 |
| fargate | ASAP (no roll-ups) | 0.257 | 4 | 21.3 | 2.68e+03 | 51.1 | 1.18 | 0.334 |
| fargate | PerQuery-CostAware | 0.799 | 11.5 | 74.4 | 3.26e+03 | 59.9 | 0.842 | 0.91 |
| fargate | AutoSketch-Adapted | 176 | 3.72e+03 | 5.63e+03 | 1.01e+03 | 10.3 | 481 | 176 |

Frontier points (latency ms, cost), by weights:

- cpu, ASAP: (490, 9.03), (649, 7.46), (813, 6.69), (980, 5.93), (1.08e+03, 5.87), (1.48e+03, 5.27), (2.68e+03, 3.85), (1.34e+04, 3.79)
- cpu, ASAP (no roll-ups): (490, 9.34), (649, 7.64), (813, 6.87), (980, 6.07), (1.08e+03, 6.01), (1.48e+03, 5.41), (2.68e+03, 4), (1.34e+04, 3.94)
- cpu, PerQuery-CostAware: (490, 19.7), (575, 17.5), (673, 16.2), (813, 15.3), (980, 14.2), (1.14e+03, 13.6), (1.34e+03, 13.1), (1.63e+03, 12.5), (1.96e+03, 12.4), (2.29e+03, 12), (2.68e+03, 11.6), (3.33e+03, 11.3)
- fargate, ASAP: (490, 0.444), (557, 0.385), (649, 0.382), (765, 0.353), (896, 0.322), (1.08e+03, 0.318), (1.3e+03, 0.315), (1.48e+03, 0.302), (2.29e+03, 0.274), (2.68e+03, 0.245)
- fargate, ASAP (no roll-ups): (490, 0.464), (557, 0.399), (649, 0.396), (765, 0.367), (896, 0.336), (959, 0.334), (1.08e+03, 0.33), (1.3e+03, 0.327), (1.48e+03, 0.314), (2.29e+03, 0.286), (2.68e+03, 0.257)
- fargate, PerQuery-CostAware: (490, 1.12), (575, 1.03), (673, 0.981), (813, 0.943), (959, 0.91), (1.14e+03, 0.88), (1.34e+03, 0.857), (1.63e+03, 0.839), (2.29e+03, 0.818), (2.68e+03, 0.803), (3.26e+03, 0.799)

Roll-up ablation (ASAP vs. ASAP without roll-ups):

| weights | RQEs rolled up | deployments with | deployments without | cost with | cost without | saving |
|---|---|---|---|---|---|---|
| cpu | 42 | 14 | 30 | 3.79 | 3.94 | 3.6% |
| fargate | 42 | 14 | 30 | 0.245 | 0.257 | 4.8% |

## mixed + multi-grouping, m = 8

832 RQEs on 88 streams; dropped: 0; sanity violations: 0. AutoSketch: 2336 probes, 152 distinct (metric, config); measured benchmark 3.85e+03 s at N = 1e+08 (paper rate 9.12e+03 s). AutoSketch latency 1.01e+03 ms; tightest feasible bound 490 ms.

| weights | method | cost (unbounded) | CPU (vCPU) | GiB | latency (ms) | median latency (ms) | planning (s) | cost at AutoSketch's latency |
|---|---|---|---|---|---|---|---|---|
| cpu | ASAP | 30.4 | 30.4 | 239 | 1.34e+04 | 62 | 9.77 | 47.4 |
| cpu | ASAP (no roll-ups) | 31.5 | 31.5 | 251 | 1.34e+04 | 51.1 | 9.3 | 48.6 |
| cpu | PerQuery-CostAware | 90.3 | 90.3 | 644 | 3.33e+03 | 59.9 | 7.45 | 114 |
| cpu | AutoSketch-Adapted | 2.98e+04 | 2.98e+04 | 4.5e+04 | 1.01e+03 | 10.3 | 3.85e+03 | 2.98e+04 |
| fargate | ASAP | 1.96 | 30.8 | 159 | 2.68e+03 | 51.1 | 12 | 2.58 |
| fargate | ASAP (no roll-ups) | 2.05 | 32 | 170 | 2.68e+03 | 51.1 | 11.1 | 2.67 |
| fargate | PerQuery-CostAware | 6.39 | 92.3 | 595 | 3.26e+03 | 59.9 | 7.55 | 7.28 |
| fargate | AutoSketch-Adapted | 1.41e+03 | 2.98e+04 | 4.5e+04 | 1.01e+03 | 10.3 | 3.85e+03 | 1.41e+03 |

Frontier points (latency ms, cost), by weights:

- cpu, ASAP: (490, 72.2), (649, 59.7), (813, 53.6), (980, 47.4), (1.08e+03, 46.9), (1.48e+03, 42.2), (2.68e+03, 30.8), (1.34e+04, 30.4)
- cpu, ASAP (no roll-ups): (490, 74.7), (649, 61.1), (813, 55), (980, 48.6), (1.08e+03, 48.1), (1.48e+03, 43.3), (2.68e+03, 32), (1.34e+04, 31.5)
- cpu, PerQuery-CostAware: (490, 158), (575, 140), (673, 130), (813, 123), (980, 114), (1.14e+03, 109), (1.34e+03, 104), (1.63e+03, 99.9), (1.96e+03, 99.5), (2.29e+03, 95.9), (2.68e+03, 92.8), (3.33e+03, 90.3)
- fargate, ASAP: (490, 3.55), (557, 3.08), (649, 3.06), (765, 2.82), (896, 2.58), (1.08e+03, 2.54), (1.3e+03, 2.52), (1.48e+03, 2.42), (2.29e+03, 2.19), (2.68e+03, 1.96)
- fargate, ASAP (no roll-ups): (490, 3.71), (557, 3.19), (649, 3.17), (765, 2.94), (896, 2.69), (959, 2.67), (1.08e+03, 2.64), (1.3e+03, 2.62), (1.48e+03, 2.51), (2.29e+03, 2.29), (2.68e+03, 2.05)
- fargate, PerQuery-CostAware: (490, 9), (575, 8.25), (673, 7.85), (813, 7.54), (959, 7.28), (1.14e+03, 7.04), (1.34e+03, 6.85), (1.63e+03, 6.71), (2.29e+03, 6.54), (2.68e+03, 6.43), (3.26e+03, 6.39)

Roll-up ablation (ASAP vs. ASAP without roll-ups):

| weights | RQEs rolled up | deployments with | deployments without | cost with | cost without | saving |
|---|---|---|---|---|---|---|
| cpu | 336 | 112 | 240 | 30.4 | 31.5 | 3.6% |
| fargate | 336 | 112 | 240 | 1.96 | 2.05 | 4.8% |

## mixed + multi-grouping, m = 16

1664 RQEs on 176 streams; dropped: 0; sanity violations: 0. AutoSketch: 4672 probes, 304 distinct (metric, config); measured benchmark 7.7e+03 s at N = 1e+08 (paper rate 1.82e+04 s). AutoSketch latency 1.01e+03 ms; tightest feasible bound 490 ms.

| weights | method | cost (unbounded) | CPU (vCPU) | GiB | latency (ms) | median latency (ms) | planning (s) | cost at AutoSketch's latency |
|---|---|---|---|---|---|---|---|---|
| cpu | ASAP | 60.7 | 60.7 | 479 | 1.34e+04 | 62 | 18.1 | 94.9 |
| cpu | ASAP (no roll-ups) | 63 | 63 | 502 | 1.34e+04 | 51.1 | 17.1 | 97.2 |
| cpu | PerQuery-CostAware | 181 | 181 | 1.29e+03 | 3.33e+03 | 59.9 | 13.9 | 228 |
| cpu | AutoSketch-Adapted | 5.96e+04 | 5.96e+04 | 9e+04 | 1.01e+03 | 10.3 | 7.7e+03 | 5.96e+04 |
| fargate | ASAP | 3.91 | 61.7 | 318 | 2.68e+03 | 51.1 | 26.4 | 5.15 |
| fargate | ASAP (no roll-ups) | 4.11 | 64 | 341 | 2.68e+03 | 51.1 | 22.4 | 5.35 |
| fargate | PerQuery-CostAware | 12.8 | 185 | 1.19e+03 | 3.26e+03 | 59.9 | 14.4 | 14.6 |
| fargate | AutoSketch-Adapted | 2.81e+03 | 5.96e+04 | 9e+04 | 1.01e+03 | 10.3 | 7.7e+03 | 2.81e+03 |

Frontier points (latency ms, cost), by weights:

- cpu, ASAP: (490, 144), (649, 119), (813, 107), (980, 94.9), (1.08e+03, 93.9), (1.48e+03, 84.3), (2.68e+03, 61.7), (1.34e+04, 60.7)
- cpu, ASAP (no roll-ups): (490, 149), (649, 122), (813, 110), (980, 97.2), (1.08e+03, 96.2), (1.48e+03, 86.6), (2.68e+03, 64), (1.34e+04, 63)
- cpu, PerQuery-CostAware: (490, 315), (575, 280), (673, 260), (813, 245), (980, 228), (1.14e+03, 218), (1.34e+03, 209), (1.63e+03, 200), (1.96e+03, 199), (2.29e+03, 192), (2.68e+03, 186), (3.33e+03, 181)
- fargate, ASAP: (490, 7.1), (557, 6.16), (649, 6.12), (765, 5.65), (896, 5.15), (1.08e+03, 5.08), (1.3e+03, 5.04), (1.48e+03, 4.83), (2.29e+03, 4.38), (2.68e+03, 3.91)
- fargate, ASAP (no roll-ups): (490, 7.43), (557, 6.39), (649, 6.34), (765, 5.87), (896, 5.38), (959, 5.35), (1.08e+03, 5.28), (1.3e+03, 5.23), (1.48e+03, 5.03), (2.29e+03, 4.57), (2.68e+03, 4.11)
- fargate, PerQuery-CostAware: (490, 18), (575, 16.5), (673, 15.7), (813, 15.1), (959, 14.6), (1.14e+03, 14.1), (1.34e+03, 13.7), (1.63e+03, 13.4), (2.29e+03, 13.1), (2.68e+03, 12.9), (3.26e+03, 12.8)

Roll-up ablation (ASAP vs. ASAP without roll-ups):

| weights | RQEs rolled up | deployments with | deployments without | cost with | cost without | saving |
|---|---|---|---|---|---|---|
| cpu | 672 | 224 | 480 | 60.7 | 63 | 3.6% |
| fargate | 672 | 224 | 480 | 3.91 | 4.11 | 4.8% |

## mixed + multi-grouping, r = 8

200 RQEs on 11 streams; dropped: 0; sanity violations: 0. AutoSketch: 548 probes, 19 distinct (metric, config); measured benchmark 481 s at N = 1e+08 (paper rate 1.14e+03 s). AutoSketch latency 1.01e+03 ms; tightest feasible bound 490 ms.

| weights | method | cost (unbounded) | CPU (vCPU) | GiB | latency (ms) | median latency (ms) | planning (s) | cost at AutoSketch's latency |
|---|---|---|---|---|---|---|---|---|
| cpu | ASAP | 3.94 | 3.94 | 19.9 | 2.68e+03 | 58.6 | 2.44 | 5.98 |
| cpu | ASAP (no roll-ups) | 4.08 | 4.08 | 21.3 | 2.68e+03 | 51.1 | 2.36 | 6.13 |
| cpu | PerQuery-CostAware | 19.9 | 19.9 | 112 | 3.33e+03 | 34.8 | 1.62 | 23.1 |
| cpu | AutoSketch-Adapted | 4.46e+03 | 4.46e+03 | 5.86e+03 | 1.01e+03 | 5.33 | 481 | 4.46e+03 |
| fargate | ASAP | 0.248 | 3.94 | 19.9 | 2.68e+03 | 58.6 | 3.1 | 0.324 |
| fargate | ASAP (no roll-ups) | 0.26 | 4.08 | 21.3 | 2.68e+03 | 51.1 | 2.78 | 0.336 |
| fargate | PerQuery-CostAware | 1.22 | 20.3 | 88.7 | 3.26e+03 | 34.8 | 1.64 | 1.33 |
| fargate | AutoSketch-Adapted | 207 | 4.46e+03 | 5.86e+03 | 1.01e+03 | 5.33 | 481 | 207 |

Frontier points (latency ms, cost), by weights:

- cpu, ASAP: (490, 9.06), (557, 7.58), (649, 7.51), (765, 6.77), (896, 5.99), (980, 5.98), (1.08e+03, 5.92), (1.3e+03, 5.87), (1.48e+03, 5.34), (2.29e+03, 4.65), (2.68e+03, 3.94)
- cpu, ASAP (no roll-ups): (490, 9.37), (557, 7.76), (649, 7.68), (765, 6.95), (896, 6.17), (980, 6.13), (1.08e+03, 6.07), (1.3e+03, 6.01), (1.48e+03, 5.48), (2.29e+03, 4.79), (2.68e+03, 4.08)
- cpu, PerQuery-CostAware: (490, 29.5), (575, 26.5), (673, 25.1), (813, 24.2), (980, 23.1), (1.14e+03, 22.4), (1.34e+03, 21.7), (1.63e+03, 21.1), (1.96e+03, 21.1), (2.29e+03, 20.6), (2.68e+03, 20.3), (3.33e+03, 19.9)
- fargate, ASAP: (490, 0.445), (557, 0.387), (649, 0.384), (765, 0.355), (896, 0.324), (1.08e+03, 0.32), (1.3e+03, 0.317), (1.48e+03, 0.305), (2.29e+03, 0.277), (2.68e+03, 0.248)
- fargate, ASAP (no roll-ups): (490, 0.466), (557, 0.401), (649, 0.398), (765, 0.369), (896, 0.338), (959, 0.336), (1.08e+03, 0.332), (1.3e+03, 0.329), (1.48e+03, 0.317), (2.29e+03, 0.289), (2.68e+03, 0.26)
- fargate, PerQuery-CostAware: (490, 1.58), (575, 1.46), (673, 1.41), (813, 1.37), (959, 1.33), (1.14e+03, 1.3), (1.34e+03, 1.27), (1.63e+03, 1.26), (2.29e+03, 1.23), (2.68e+03, 1.22), (3.26e+03, 1.22)

Roll-up ablation (ASAP vs. ASAP without roll-ups):

| weights | RQEs rolled up | deployments with | deployments without | cost with | cost without | saving |
|---|---|---|---|---|---|---|
| cpu | 84 | 14 | 30 | 3.94 | 4.08 | 3.5% |
| fargate | 84 | 14 | 30 | 0.248 | 0.26 | 4.7% |

## mixed

50 RQEs on 8 streams; dropped: 0; sanity violations: 0. AutoSketch: 166 probes, 10 distinct (metric, config); measured benchmark 212 s at N = 1e+08 (paper rate 600 s). AutoSketch latency 1.01e+03 ms; tightest feasible bound 87.7 ms.

| weights | method | cost (unbounded) | CPU (vCPU) | GiB | latency (ms) | median latency (ms) | planning (s) | cost at AutoSketch's latency |
|---|---|---|---|---|---|---|---|---|
| cpu | ASAP | 3.54 | 3.54 | 28.9 | 1.34e+04 | 97.4 | 0.722 | 5.67 |
| cpu | ASAP (no roll-ups) | 3.54 | 3.54 | 28.9 | 1.34e+04 | 97.4 | 0.721 | 5.67 |
| cpu | PerQuery-CostAware | 10.5 | 10.5 | 77 | 3.33e+03 | 97.4 | 0.638 | 13.5 |
| cpu | AutoSketch-Adapted | 3.72e+03 | 3.72e+03 | 5.62e+03 | 1.01e+03 | 1.22 | 212 | 3.72e+03 |
| fargate | ASAP | 0.23 | 3.6 | 18.9 | 2.68e+03 | 51.1 | 0.779 | 0.307 |
| fargate | ASAP (no roll-ups) | 0.23 | 3.6 | 18.9 | 2.68e+03 | 51.1 | 0.779 | 0.307 |
| fargate | PerQuery-CostAware | 0.752 | 10.8 | 70.9 | 3.26e+03 | 97.4 | 0.646 | 0.864 |
| fargate | AutoSketch-Adapted | 176 | 3.72e+03 | 5.62e+03 | 1.01e+03 | 1.22 | 212 | 176 |

Frontier points (latency ms, cost), by weights:

- cpu, ASAP: (87.7, 47.4), (138, 24.1), (206, 16.2), (342, 11.7), (541, 7.28), (813, 6.44), (980, 5.67), (1.3e+03, 5.55), (1.48e+03, 5.02), (2.68e+03, 3.6), (1.34e+04, 3.54)
- cpu, ASAP (no roll-ups): (87.7, 47.4), (138, 24.1), (206, 16.2), (342, 11.7), (541, 7.28), (813, 6.44), (980, 5.67), (1.3e+03, 5.55), (1.48e+03, 5.02), (2.68e+03, 3.6), (1.34e+04, 3.54)
- cpu, PerQuery-CostAware: (87.7, 78.3), (118, 51.5), (164, 36.6), (235, 28.4), (324, 23.3), (452, 19.2), (575, 16.7), (829, 14), (980, 13.5), (1.14e+03, 12.9), (1.63e+03, 11.7), (2.29e+03, 11.2), (3.33e+03, 10.5)
- fargate, ASAP: (87.7, 2.01), (115, 1.25), (163, 0.847), (219, 0.715), (291, 0.586), (386, 0.494), (541, 0.37), (765, 0.338), (896, 0.307), (1.3e+03, 0.3), (1.48e+03, 0.287), (2.68e+03, 0.23)
- fargate, ASAP (no roll-ups): (87.7, 2.01), (115, 1.25), (163, 0.847), (219, 0.715), (291, 0.586), (386, 0.494), (541, 0.37), (765, 0.338), (896, 0.307), (1.3e+03, 0.3), (1.48e+03, 0.287), (2.68e+03, 0.23)
- fargate, PerQuery-CostAware: (87.7, 3.52), (118, 2.4), (164, 1.79), (233, 1.49), (323, 1.24), (452, 1.08), (575, 0.981), (829, 0.879), (896, 0.864), (1.14e+03, 0.833), (1.63e+03, 0.792), (2.29e+03, 0.771), (3.26e+03, 0.752)

Roll-up ablation (ASAP vs. ASAP without roll-ups):

| weights | RQEs rolled up | deployments with | deployments without | cost with | cost without | saving |
|---|---|---|---|---|---|---|
| cpu | 0 | 8 | 8 | 3.54 | 3.54 | 0.0% |
| fargate | 0 | 8 | 8 | 0.23 | 0.23 | 0.0% |

## mixed, m = 8

400 RQEs on 64 streams; dropped: 0; sanity violations: 0. AutoSketch: 1328 probes, 80 distinct (metric, config); measured benchmark 1.7e+03 s at N = 1e+08 (paper rate 4.8e+03 s). AutoSketch latency 1.01e+03 ms; tightest feasible bound 87.7 ms.

| weights | method | cost (unbounded) | CPU (vCPU) | GiB | latency (ms) | median latency (ms) | planning (s) | cost at AutoSketch's latency |
|---|---|---|---|---|---|---|---|---|
| cpu | ASAP | 28.3 | 28.3 | 231 | 1.34e+04 | 97.4 | 6.34 | 45.4 |
| cpu | ASAP (no roll-ups) | 28.3 | 28.3 | 231 | 1.34e+04 | 97.4 | 6.35 | 45.4 |
| cpu | PerQuery-CostAware | 84.1 | 84.1 | 616 | 3.33e+03 | 97.4 | 5.44 | 108 |
| cpu | AutoSketch-Adapted | 2.97e+04 | 2.97e+04 | 4.5e+04 | 1.01e+03 | 1.22 | 1.7e+03 | 2.97e+04 |
| fargate | ASAP | 1.84 | 28.8 | 151 | 2.68e+03 | 51.1 | 7.45 | 2.46 |
| fargate | ASAP (no roll-ups) | 1.84 | 28.8 | 151 | 2.68e+03 | 51.1 | 7.46 | 2.46 |
| fargate | PerQuery-CostAware | 6.02 | 86.2 | 568 | 3.26e+03 | 97.4 | 5.52 | 6.91 |
| fargate | AutoSketch-Adapted | 1.4e+03 | 2.97e+04 | 4.5e+04 | 1.01e+03 | 1.22 | 1.7e+03 | 1.4e+03 |

Frontier points (latency ms, cost), by weights:

- cpu, ASAP: (87.7, 379), (138, 193), (206, 129), (342, 93.8), (541, 58.2), (813, 51.5), (980, 45.4), (1.3e+03, 44.4), (1.48e+03, 40.1), (2.68e+03, 28.8), (1.34e+04, 28.3)
- cpu, ASAP (no roll-ups): (87.7, 379), (138, 193), (206, 129), (342, 93.8), (541, 58.2), (813, 51.5), (980, 45.4), (1.3e+03, 44.4), (1.48e+03, 40.1), (2.68e+03, 28.8), (1.34e+04, 28.3)
- cpu, PerQuery-CostAware: (87.7, 626), (118, 412), (164, 293), (235, 227), (324, 186), (452, 154), (575, 133), (829, 112), (980, 108), (1.14e+03, 103), (1.63e+03, 93.8), (2.29e+03, 89.7), (3.33e+03, 84.1)
- fargate, ASAP: (87.7, 16.1), (115, 10), (163, 6.78), (219, 5.72), (291, 4.69), (386, 3.95), (541, 2.96), (765, 2.71), (896, 2.46), (1.3e+03, 2.4), (1.48e+03, 2.3), (2.68e+03, 1.84)
- fargate, ASAP (no roll-ups): (87.7, 16.1), (115, 10), (163, 6.78), (219, 5.72), (291, 4.69), (386, 3.95), (541, 2.96), (765, 2.71), (896, 2.46), (1.3e+03, 2.4), (1.48e+03, 2.3), (2.68e+03, 1.84)
- fargate, PerQuery-CostAware: (87.7, 28.1), (118, 19.2), (164, 14.3), (233, 11.9), (323, 9.95), (452, 8.66), (575, 7.85), (829, 7.03), (896, 6.91), (1.14e+03, 6.66), (1.63e+03, 6.34), (2.29e+03, 6.17), (3.26e+03, 6.02)

Roll-up ablation (ASAP vs. ASAP without roll-ups):

| weights | RQEs rolled up | deployments with | deployments without | cost with | cost without | saving |
|---|---|---|---|---|---|---|
| cpu | 0 | 64 | 64 | 28.3 | 28.3 | 0.0% |
| fargate | 0 | 64 | 64 | 1.84 | 1.84 | 0.0% |

## mixed, m = 16

800 RQEs on 128 streams; dropped: 0; sanity violations: 0. AutoSketch: 2656 probes, 160 distinct (metric, config); measured benchmark 3.39e+03 s at N = 1e+08 (paper rate 9.6e+03 s). AutoSketch latency 1.01e+03 ms; tightest feasible bound 87.7 ms.

| weights | method | cost (unbounded) | CPU (vCPU) | GiB | latency (ms) | median latency (ms) | planning (s) | cost at AutoSketch's latency |
|---|---|---|---|---|---|---|---|---|
| cpu | ASAP | 56.6 | 56.6 | 463 | 1.34e+04 | 97.4 | 13.7 | 90.8 |
| cpu | ASAP (no roll-ups) | 56.6 | 56.6 | 463 | 1.34e+04 | 97.4 | 13.7 | 90.8 |
| cpu | PerQuery-CostAware | 168 | 168 | 1.23e+03 | 3.33e+03 | 97.4 | 11.4 | 215 |
| cpu | AutoSketch-Adapted | 5.95e+04 | 5.95e+04 | 9e+04 | 1.01e+03 | 1.22 | 3.39e+03 | 5.95e+04 |
| fargate | ASAP | 3.68 | 57.6 | 302 | 2.68e+03 | 51.1 | 16.9 | 4.91 |
| fargate | ASAP (no roll-ups) | 3.68 | 57.6 | 302 | 2.68e+03 | 51.1 | 16.8 | 4.91 |
| fargate | PerQuery-CostAware | 12 | 172 | 1.14e+03 | 3.26e+03 | 97.4 | 11.7 | 13.8 |
| fargate | AutoSketch-Adapted | 2.81e+03 | 5.95e+04 | 9e+04 | 1.01e+03 | 1.22 | 3.39e+03 | 2.81e+03 |

Frontier points (latency ms, cost), by weights:

- cpu, ASAP: (87.7, 758), (138, 386), (206, 259), (342, 188), (541, 116), (813, 103), (980, 90.8), (1.3e+03, 88.8), (1.48e+03, 80.3), (2.68e+03, 57.6), (1.34e+04, 56.6)
- cpu, ASAP (no roll-ups): (87.7, 758), (138, 386), (206, 259), (342, 188), (541, 116), (813, 103), (980, 90.8), (1.3e+03, 88.8), (1.48e+03, 80.3), (2.68e+03, 57.6), (1.34e+04, 56.6)
- cpu, PerQuery-CostAware: (87.7, 1.25e+03), (118, 824), (164, 586), (235, 454), (324, 372), (452, 308), (575, 267), (829, 224), (980, 215), (1.14e+03, 206), (1.63e+03, 188), (2.29e+03, 179), (3.33e+03, 168)
- fargate, ASAP: (87.7, 32.2), (115, 20), (163, 13.6), (219, 11.4), (291, 9.38), (386, 7.91), (541, 5.93), (765, 5.41), (896, 4.91), (1.3e+03, 4.8), (1.48e+03, 4.59), (2.68e+03, 3.68)
- fargate, ASAP (no roll-ups): (87.7, 32.2), (115, 20), (163, 13.6), (219, 11.4), (291, 9.38), (386, 7.91), (541, 5.93), (765, 5.41), (896, 4.91), (1.3e+03, 4.8), (1.48e+03, 4.59), (2.68e+03, 3.68)
- fargate, PerQuery-CostAware: (87.7, 56.3), (118, 38.5), (164, 28.7), (233, 23.8), (323, 19.9), (452, 17.3), (575, 15.7), (829, 14.1), (896, 13.8), (1.14e+03, 13.3), (1.63e+03, 12.7), (2.29e+03, 12.3), (3.26e+03, 12)

Roll-up ablation (ASAP vs. ASAP without roll-ups):

| weights | RQEs rolled up | deployments with | deployments without | cost with | cost without | saving |
|---|---|---|---|---|---|---|
| cpu | 0 | 128 | 128 | 56.6 | 56.6 | 0.0% |
| fargate | 0 | 128 | 128 | 3.68 | 3.68 | 0.0% |

## mixed, r = 8

92 RQEs on 8 streams; dropped: 0; sanity violations: 0. AutoSketch: 296 probes, 10 distinct (metric, config); measured benchmark 212 s at N = 1e+08 (paper rate 600 s). AutoSketch latency 1.01e+03 ms; tightest feasible bound 87.7 ms.

| weights | method | cost (unbounded) | CPU (vCPU) | GiB | latency (ms) | median latency (ms) | planning (s) | cost at AutoSketch's latency |
|---|---|---|---|---|---|---|---|---|
| cpu | ASAP | 3.66 | 3.66 | 18.9 | 2.68e+03 | 58.6 | 1.64 | 5.71 |
| cpu | ASAP (no roll-ups) | 3.66 | 3.66 | 18.9 | 2.68e+03 | 58.6 | 1.64 | 5.71 |
| cpu | PerQuery-CostAware | 18.5 | 18.5 | 107 | 3.33e+03 | 61.5 | 1.23 | 21.6 |
| cpu | AutoSketch-Adapted | 4.46e+03 | 4.46e+03 | 5.86e+03 | 1.01e+03 | 1.21 | 212 | 4.46e+03 |
| fargate | ASAP | 0.232 | 3.66 | 18.9 | 2.68e+03 | 58.6 | 2 | 0.308 |
| fargate | ASAP (no roll-ups) | 0.232 | 3.66 | 18.9 | 2.68e+03 | 58.6 | 2.02 | 0.308 |
| fargate | PerQuery-CostAware | 1.14 | 18.8 | 84.2 | 3.26e+03 | 51.1 | 1.24 | 1.26 |
| fargate | AutoSketch-Adapted | 207 | 4.46e+03 | 5.86e+03 | 1.01e+03 | 1.21 | 212 | 207 |

Frontier points (latency ms, cost), by weights:

- cpu, ASAP: (87.7, 47.4), (115, 29), (163, 19.1), (219, 15.9), (291, 12.7), (389, 10.3), (541, 7.3), (765, 6.5), (980, 5.71), (1.3e+03, 5.59), (1.48e+03, 5.06), (2.68e+03, 3.66)
- cpu, ASAP (no roll-ups): (87.7, 47.4), (115, 29), (163, 19.1), (219, 15.9), (291, 12.7), (389, 10.3), (541, 7.3), (765, 6.5), (980, 5.71), (1.3e+03, 5.59), (1.48e+03, 5.06), (2.68e+03, 3.66)
- cpu, PerQuery-CostAware: (87.7, 95.7), (118, 66), (164, 48.4), (235, 38.9), (324, 32.9), (452, 28.7), (575, 25), (829, 22.2), (980, 21.6), (1.14e+03, 21), (1.63e+03, 19.7), (2.29e+03, 19.2), (3.33e+03, 18.5)
- fargate, ASAP: (87.7, 2.01), (115, 1.25), (163, 0.848), (219, 0.715), (291, 0.587), (386, 0.495), (541, 0.371), (765, 0.339), (896, 0.308), (1.3e+03, 0.301), (1.48e+03, 0.289), (2.68e+03, 0.232)
- fargate, ASAP (no roll-ups): (87.7, 2.01), (115, 1.25), (163, 0.848), (219, 0.715), (291, 0.587), (386, 0.495), (541, 0.371), (765, 0.339), (896, 0.308), (1.3e+03, 0.301), (1.48e+03, 0.289), (2.68e+03, 0.232)
- fargate, PerQuery-CostAware: (87.7, 4.28), (118, 3.05), (164, 2.33), (233, 1.97), (323, 1.69), (452, 1.52), (575, 1.38), (829, 1.27), (896, 1.26), (1.14e+03, 1.23), (1.63e+03, 1.18), (2.29e+03, 1.16), (3.26e+03, 1.14)

Roll-up ablation (ASAP vs. ASAP without roll-ups):

| weights | RQEs rolled up | deployments with | deployments without | cost with | cost without | saving |
|---|---|---|---|---|---|---|
| cpu | 0 | 8 | 8 | 3.66 | 3.66 | 0.0% |
| fargate | 0 | 8 | 8 | 0.232 | 0.232 | 0.0% |
