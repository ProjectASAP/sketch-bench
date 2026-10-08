# AutoSketch vs. ASAP on the trace workloads

## alibaba_v2022

51 RQEs; dropped as unservable: 0. Sanity violations: 0.

Weights cpu, 0.01 ms:

| method | objective | vs ASAP | CPU (vCPU) | GiB | deployments | Σ window/slide | max latency (ms) | planning (s) |
|---|---|---|---|---|---|---|---|---|
| ASAP | 0.173 | 1× | 0.173 | 0.000479 | 10 | 31 | 0.00759 | 0.0395 |
| PerQuery-CostAware | 0.189 | 1.1× | 0.189 | 0.000493 | 15 | 36 | 0.00759 | 0.065 |
| AutoSketch-Adapted | 1.17 | 6.76× | 1.17 | 0.000222 | 15 | 330 | 0.0024 | 0.000329 |

Weights cpu, 0.1 ms:

| method | objective | vs ASAP | CPU (vCPU) | GiB | deployments | Σ window/slide | max latency (ms) | planning (s) |
|---|---|---|---|---|---|---|---|---|
| ASAP | 0.0168 | 1× | 0.0168 | 0.000438 | 5 | 5 | 0.0616 | 0.0439 |
| PerQuery-CostAware | 0.0504 | 3× | 0.0504 | 0.000493 | 15 | 15 | 0.0616 | 0.0662 |
| AutoSketch-Adapted | 1.17 | 69.5× | 1.17 | 0.000222 | 15 | 330 | 0.0024 | 0.000329 |

Weights cpu, 1 ms:

| method | objective | vs ASAP | CPU (vCPU) | GiB | deployments | Σ window/slide | max latency (ms) | planning (s) |
|---|---|---|---|---|---|---|---|---|
| ASAP | 0.0168 | 1× | 0.0168 | 0.000438 | 5 | 5 | 0.0616 | 0.0433 |
| PerQuery-CostAware | 0.0504 | 3× | 0.0504 | 0.000493 | 15 | 15 | 0.0616 | 0.0657 |
| AutoSketch-Adapted | 1.17 | 69.5× | 1.17 | 0.000222 | 15 | 330 | 0.0024 | 0.000329 |

Weights cpu, 10 ms:

| method | objective | vs ASAP | CPU (vCPU) | GiB | deployments | Σ window/slide | max latency (ms) | planning (s) |
|---|---|---|---|---|---|---|---|---|
| ASAP | 0.993 | 1× | 0.993 | 0.15 | 8 | 71 | 7.23 | 0.044 |
| PerQuery-CostAware | 1.03 | 1.03× | 1.03 | 0.15 | 18 | 81 | 7.23 | 0.0709 |
| AutoSketch-Adapted | 2.14 | 2.16× | 2.14 | 0.149 | 18 | 396 | 7.23 | 0.000329 |

Weights cpu, no SLA:

| method | objective | vs ASAP | CPU (vCPU) | GiB | deployments | Σ window/slide | max latency (ms) | planning (s) |
|---|---|---|---|---|---|---|---|---|
| ASAP | 2.31 | 1× | 2.31 | 47.1 | 17 | 17 | 2.28e+04 | 0.0498 |
| PerQuery-CostAware | 4.48 | 1.94× | 4.48 | 53 | 51 | 51 | 2.28e+04 | 0.129 |
| AutoSketch-Adapted | 71.8 | 31.1× | 71.8 | 51.5 | 51 | 1.12e+03 | 887 | 0.000329 |

Weights fargate, 0.01 ms:

| method | objective | vs ASAP | CPU (vCPU) | GiB | deployments | Σ window/slide | max latency (ms) | planning (s) |
|---|---|---|---|---|---|---|---|---|
| ASAP | 0.00699 | 1× | 0.173 | 0.000422 | 10 | 30 | 0.00797 | 0.0409 |
| PerQuery-CostAware | 0.00767 | 1.1× | 0.189 | 0.000435 | 15 | 35 | 0.00797 | 0.0655 |
| AutoSketch-Adapted | 0.0473 | 6.76× | 1.17 | 0.000222 | 15 | 330 | 0.0024 | 0.000329 |

Weights fargate, 0.1 ms:

| method | objective | vs ASAP | CPU (vCPU) | GiB | deployments | Σ window/slide | max latency (ms) | planning (s) |
|---|---|---|---|---|---|---|---|---|
| ASAP | 0.000682 | 1× | 0.0168 | 0.000379 | 5 | 5 | 0.0616 | 0.0463 |
| PerQuery-CostAware | 0.00204 | 2.99× | 0.0504 | 0.000435 | 15 | 15 | 0.0616 | 0.0665 |
| AutoSketch-Adapted | 0.0473 | 69.3× | 1.17 | 0.000222 | 15 | 330 | 0.0024 | 0.000329 |

Weights fargate, 1 ms:

| method | objective | vs ASAP | CPU (vCPU) | GiB | deployments | Σ window/slide | max latency (ms) | planning (s) |
|---|---|---|---|---|---|---|---|---|
| ASAP | 0.000682 | 1× | 0.0168 | 0.000379 | 5 | 5 | 0.0616 | 0.0457 |
| PerQuery-CostAware | 0.00204 | 2.99× | 0.0504 | 0.000435 | 15 | 15 | 0.0616 | 0.0664 |
| AutoSketch-Adapted | 0.0473 | 69.3× | 1.17 | 0.000222 | 15 | 330 | 0.0024 | 0.000329 |

Weights fargate, 10 ms:

| method | objective | vs ASAP | CPU (vCPU) | GiB | deployments | Σ window/slide | max latency (ms) | planning (s) |
|---|---|---|---|---|---|---|---|---|
| ASAP | 0.0409 | 1× | 0.993 | 0.15 | 8 | 71 | 7.23 | 0.0491 |
| PerQuery-CostAware | 0.0422 | 1.03× | 1.03 | 0.15 | 18 | 81 | 7.23 | 0.0717 |
| AutoSketch-Adapted | 0.0875 | 2.14× | 2.14 | 0.149 | 18 | 396 | 7.23 | 0.000329 |

Weights fargate, no SLA:

| method | objective | vs ASAP | CPU (vCPU) | GiB | deployments | Σ window/slide | max latency (ms) | planning (s) |
|---|---|---|---|---|---|---|---|---|
| ASAP | 0.303 | 1× | 2.31 | 47.1 | 17 | 17 | 2.28e+04 | 0.0543 |
| PerQuery-CostAware | 0.417 | 1.38× | 4.48 | 53 | 51 | 51 | 2.28e+04 | 0.131 |
| AutoSketch-Adapted | 3.14 | 10.4× | 71.8 | 51.5 | 51 | 1.12e+03 | 887 | 0.000329 |

## google_2011

27 RQEs; dropped as unservable: 0. Sanity violations: 0.

Weights cpu, 0.01 ms:

| method | objective | vs ASAP | CPU (vCPU) | GiB | deployments | Σ window/slide | max latency (ms) | planning (s) |
|---|---|---|---|---|---|---|---|---|
| ASAP | 0.00107 | 1× | 0.00107 | 7.16e-05 | 5 | 7 | 0.00991 | 0.0272 |
| PerQuery-CostAware | 0.00174 | 1.63× | 0.00174 | 8.86e-05 | 12 | 14 | 0.00991 | 0.0409 |
| AutoSketch-Adapted | 0.00636 | 5.96× | 0.00636 | 5.2e-05 | 12 | 56 | 0.00439 | 0.000193 |

Weights cpu, 0.1 ms:

| method | objective | vs ASAP | CPU (vCPU) | GiB | deployments | Σ window/slide | max latency (ms) | planning (s) |
|---|---|---|---|---|---|---|---|---|
| ASAP | 0.000442 | 1× | 0.000442 | 6.9e-05 | 4 | 4 | 0.0246 | 0.0292 |
| PerQuery-CostAware | 0.00133 | 3× | 0.00133 | 8.86e-05 | 12 | 12 | 0.0246 | 0.0409 |
| AutoSketch-Adapted | 0.00636 | 14.4× | 0.00636 | 5.2e-05 | 12 | 56 | 0.00439 | 0.000193 |

Weights cpu, 1 ms:

| method | objective | vs ASAP | CPU (vCPU) | GiB | deployments | Σ window/slide | max latency (ms) | planning (s) |
|---|---|---|---|---|---|---|---|---|
| ASAP | 0.00191 | 1× | 0.00191 | 0.0029 | 8 | 11 | 0.827 | 0.0304 |
| PerQuery-CostAware | 0.00321 | 1.68× | 0.00321 | 0.00326 | 18 | 21 | 0.827 | 0.0526 |
| AutoSketch-Adapted | 0.0122 | 6.39× | 0.0122 | 0.00305 | 18 | 84 | 0.366 | 0.000193 |

Weights cpu, 10 ms:

| method | objective | vs ASAP | CPU (vCPU) | GiB | deployments | Σ window/slide | max latency (ms) | planning (s) |
|---|---|---|---|---|---|---|---|---|
| ASAP | 0.0163 | 1× | 0.0163 | 0.0464 | 12 | 17 | 9.18 | 0.0319 |
| PerQuery-CostAware | 0.0218 | 1.33× | 0.0218 | 0.0525 | 27 | 32 | 9.18 | 0.0651 |
| AutoSketch-Adapted | 0.0645 | 3.96× | 0.0645 | 0.0496 | 27 | 126 | 5 | 0.000193 |

Weights cpu, no SLA:

| method | objective | vs ASAP | CPU (vCPU) | GiB | deployments | Σ window/slide | max latency (ms) | planning (s) |
|---|---|---|---|---|---|---|---|---|
| ASAP | 0.00484 | 1× | 0.00484 | 0.041 | 9 | 9 | 28 | 0.0315 |
| PerQuery-CostAware | 0.014 | 2.9× | 0.014 | 0.0525 | 27 | 27 | 28 | 0.0654 |
| AutoSketch-Adapted | 0.0645 | 13.3× | 0.0645 | 0.0496 | 27 | 126 | 5 | 0.000193 |

Weights fargate, 0.01 ms:

| method | objective | vs ASAP | CPU (vCPU) | GiB | deployments | Σ window/slide | max latency (ms) | planning (s) |
|---|---|---|---|---|---|---|---|---|
| ASAP | 4.35e-05 | 1× | 0.00107 | 7.16e-05 | 5 | 7 | 0.00991 | 0.0285 |
| PerQuery-CostAware | 7.1e-05 | 1.63× | 0.00174 | 8.86e-05 | 12 | 14 | 0.00991 | 0.041 |
| AutoSketch-Adapted | 0.000258 | 5.93× | 0.00636 | 5.2e-05 | 12 | 56 | 0.00439 | 0.000193 |

Weights fargate, 0.1 ms:

| method | objective | vs ASAP | CPU (vCPU) | GiB | deployments | Σ window/slide | max latency (ms) | planning (s) |
|---|---|---|---|---|---|---|---|---|
| ASAP | 1.82e-05 | 1× | 0.000442 | 6.9e-05 | 4 | 4 | 0.0246 | 0.0303 |
| PerQuery-CostAware | 5.41e-05 | 2.97× | 0.00133 | 8.86e-05 | 12 | 12 | 0.0246 | 0.0412 |
| AutoSketch-Adapted | 0.000258 | 14.1× | 0.00636 | 5.2e-05 | 12 | 56 | 0.00439 | 0.000193 |

Weights fargate, 1 ms:

| method | objective | vs ASAP | CPU (vCPU) | GiB | deployments | Σ window/slide | max latency (ms) | planning (s) |
|---|---|---|---|---|---|---|---|---|
| ASAP | 9.01e-05 | 1× | 0.00191 | 0.0029 | 8 | 11 | 0.827 | 0.0316 |
| PerQuery-CostAware | 0.000144 | 1.6× | 0.00321 | 0.00326 | 18 | 21 | 0.827 | 0.0508 |
| AutoSketch-Adapted | 0.000507 | 5.63× | 0.0122 | 0.00305 | 18 | 84 | 0.366 | 0.000193 |

Weights fargate, 10 ms:

| method | objective | vs ASAP | CPU (vCPU) | GiB | deployments | Σ window/slide | max latency (ms) | planning (s) |
|---|---|---|---|---|---|---|---|---|
| ASAP | 0.000867 | 1× | 0.0163 | 0.0464 | 12 | 17 | 9.18 | 0.033 |
| PerQuery-CostAware | 0.00111 | 1.29× | 0.0218 | 0.0525 | 27 | 32 | 9.18 | 0.0659 |
| AutoSketch-Adapted | 0.00283 | 3.27× | 0.0645 | 0.0496 | 27 | 126 | 5 | 0.000193 |

Weights fargate, no SLA:

| method | objective | vs ASAP | CPU (vCPU) | GiB | deployments | Σ window/slide | max latency (ms) | planning (s) |
|---|---|---|---|---|---|---|---|---|
| ASAP | 0.000378 | 1× | 0.00484 | 0.041 | 9 | 9 | 28 | 0.0328 |
| PerQuery-CostAware | 0.000801 | 2.12× | 0.014 | 0.0525 | 27 | 27 | 28 | 0.0661 |
| AutoSketch-Adapted | 0.00283 | 7.49× | 0.0645 | 0.0496 | 27 | 126 | 5 | 0.000193 |
