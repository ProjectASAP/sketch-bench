# AutoSketch vs. ASAP on the trace workloads

## alibaba_v2022

51 RQEs; dropped as unservable: 0. Sanity violations: 0.

Weights cpu, 0.003 ms:

| method | objective | CPU (vCPU) | GiB | deployments | Σ window/slide | max latency (ms) | planning (s) |
|---|---|---|---|---|---|---|---|
| ASAP | 0.998 | 0.998 | 0.000479 | 11 | 110 | 0.00279 | 0.0431 |
| PerQuery-CostAware | 1 | 1 | 0.000492 | 15 | 114 | 0.00279 | 0.07 |
| AutoSketch-Adapted | 1.17 | 1.17 | 0.000222 | 15 | 330 | 0.0024 | 0.0023 |

Weights cpu, 0.01 ms:

| method | objective | CPU (vCPU) | GiB | deployments | Σ window/slide | max latency (ms) | planning (s) |
|---|---|---|---|---|---|---|---|
| ASAP | 0.173 | 0.173 | 0.000479 | 10 | 31 | 0.00759 | 0.0437 |
| PerQuery-CostAware | 0.189 | 0.189 | 0.000493 | 15 | 36 | 0.00759 | 0.0708 |
| AutoSketch-Adapted | 1.17 | 1.17 | 0.000222 | 15 | 330 | 0.0024 | 0.0023 |

Weights cpu, 0.03 ms:

| method | objective | CPU (vCPU) | GiB | deployments | Σ window/slide | max latency (ms) | planning (s) |
|---|---|---|---|---|---|---|---|
| ASAP | 0.062 | 0.062 | 0.000207 | 6 | 8 | 0.0245 | 0.0461 |
| PerQuery-CostAware | 0.0808 | 0.0808 | 0.000262 | 15 | 17 | 0.0245 | 0.0714 |
| AutoSketch-Adapted | 1.17 | 1.17 | 0.000222 | 15 | 330 | 0.0024 | 0.0023 |

Weights cpu, 0.1 ms:

| method | objective | CPU (vCPU) | GiB | deployments | Σ window/slide | max latency (ms) | planning (s) |
|---|---|---|---|---|---|---|---|
| ASAP | 0.0168 | 0.0168 | 0.000438 | 5 | 5 | 0.0616 | 0.0444 |
| PerQuery-CostAware | 0.0504 | 0.0504 | 0.000493 | 15 | 15 | 0.0616 | 0.0718 |
| AutoSketch-Adapted | 1.17 | 1.17 | 0.000222 | 15 | 330 | 0.0024 | 0.0023 |

Weights cpu, 0.3 ms:

| method | objective | CPU (vCPU) | GiB | deployments | Σ window/slide | max latency (ms) | planning (s) |
|---|---|---|---|---|---|---|---|
| ASAP | 0.0168 | 0.0168 | 0.000438 | 5 | 5 | 0.0616 | 0.0438 |
| PerQuery-CostAware | 0.0504 | 0.0504 | 0.000493 | 15 | 15 | 0.0616 | 0.0719 |
| AutoSketch-Adapted | 1.17 | 1.17 | 0.000222 | 15 | 330 | 0.0024 | 0.0023 |

Weights cpu, 1 ms:

| method | objective | CPU (vCPU) | GiB | deployments | Σ window/slide | max latency (ms) | planning (s) |
|---|---|---|---|---|---|---|---|
| ASAP | 0.0168 | 0.0168 | 0.000438 | 5 | 5 | 0.0616 | 0.0435 |
| PerQuery-CostAware | 0.0504 | 0.0504 | 0.000493 | 15 | 15 | 0.0616 | 0.072 |
| AutoSketch-Adapted | 1.17 | 1.17 | 0.000222 | 15 | 330 | 0.0024 | 0.0023 |

Weights cpu, 3 ms:

| method | objective | CPU (vCPU) | GiB | deployments | Σ window/slide | max latency (ms) | planning (s) |
|---|---|---|---|---|---|---|---|
| ASAP | 0.0168 | 0.0168 | 0.000438 | 5 | 5 | 0.0616 | 0.0437 |
| PerQuery-CostAware | 0.0504 | 0.0504 | 0.000493 | 15 | 15 | 0.0616 | 0.0719 |
| AutoSketch-Adapted | 1.17 | 1.17 | 0.000222 | 15 | 330 | 0.0024 | 0.0023 |

Weights cpu, 10 ms:

| method | objective | CPU (vCPU) | GiB | deployments | Σ window/slide | max latency (ms) | planning (s) |
|---|---|---|---|---|---|---|---|
| ASAP | 0.993 | 0.993 | 0.15 | 8 | 71 | 7.23 | 0.0451 |
| PerQuery-CostAware | 1.03 | 1.03 | 0.15 | 18 | 81 | 7.23 | 0.0776 |
| AutoSketch-Adapted | 2.14 | 2.14 | 0.149 | 18 | 396 | 7.23 | 0.0023 |

Weights cpu, no SLA:

| method | objective | CPU (vCPU) | GiB | deployments | Σ window/slide | max latency (ms) | planning (s) |
|---|---|---|---|---|---|---|---|
| ASAP | 2.31 | 2.31 | 47.1 | 17 | 17 | 2.28e+04 | 0.0508 |
| PerQuery-CostAware | 4.48 | 4.48 | 53 | 51 | 51 | 2.28e+04 | 0.135 |
| AutoSketch-Adapted | 71.8 | 71.8 | 51.5 | 51 | 1.12e+03 | 887 | 0.0023 |

Weights fargate, 0.003 ms:

| method | objective | CPU (vCPU) | GiB | deployments | Σ window/slide | max latency (ms) | planning (s) |
|---|---|---|---|---|---|---|---|
| ASAP | 0.0404 | 0.998 | 0.000479 | 11 | 110 | 0.00279 | 0.0452 |
| PerQuery-CostAware | 0.0405 | 1 | 0.000492 | 15 | 114 | 0.00279 | 0.0706 |
| AutoSketch-Adapted | 0.0473 | 1.17 | 0.000222 | 15 | 330 | 0.0024 | 0.0023 |

Weights fargate, 0.01 ms:

| method | objective | CPU (vCPU) | GiB | deployments | Σ window/slide | max latency (ms) | planning (s) |
|---|---|---|---|---|---|---|---|
| ASAP | 0.00699 | 0.173 | 0.000422 | 10 | 30 | 0.00797 | 0.0452 |
| PerQuery-CostAware | 0.00767 | 0.189 | 0.000435 | 15 | 35 | 0.00797 | 0.0713 |
| AutoSketch-Adapted | 0.0473 | 1.17 | 0.000222 | 15 | 330 | 0.0024 | 0.0023 |

Weights fargate, 0.03 ms:

| method | objective | CPU (vCPU) | GiB | deployments | Σ window/slide | max latency (ms) | planning (s) |
|---|---|---|---|---|---|---|---|
| ASAP | 0.00251 | 0.062 | 0.000207 | 6 | 8 | 0.0245 | 0.0485 |
| PerQuery-CostAware | 0.00327 | 0.0808 | 0.000262 | 15 | 17 | 0.0245 | 0.0723 |
| AutoSketch-Adapted | 0.0473 | 1.17 | 0.000222 | 15 | 330 | 0.0024 | 0.0023 |

Weights fargate, 0.1 ms:

| method | objective | CPU (vCPU) | GiB | deployments | Σ window/slide | max latency (ms) | planning (s) |
|---|---|---|---|---|---|---|---|
| ASAP | 0.000682 | 0.0168 | 0.000379 | 5 | 5 | 0.0616 | 0.047 |
| PerQuery-CostAware | 0.00204 | 0.0504 | 0.000435 | 15 | 15 | 0.0616 | 0.0725 |
| AutoSketch-Adapted | 0.0473 | 1.17 | 0.000222 | 15 | 330 | 0.0024 | 0.0023 |

Weights fargate, 0.3 ms:

| method | objective | CPU (vCPU) | GiB | deployments | Σ window/slide | max latency (ms) | planning (s) |
|---|---|---|---|---|---|---|---|
| ASAP | 0.000682 | 0.0168 | 0.000379 | 5 | 5 | 0.0616 | 0.0463 |
| PerQuery-CostAware | 0.00204 | 0.0504 | 0.000435 | 15 | 15 | 0.0616 | 0.0743 |
| AutoSketch-Adapted | 0.0473 | 1.17 | 0.000222 | 15 | 330 | 0.0024 | 0.0023 |

Weights fargate, 1 ms:

| method | objective | CPU (vCPU) | GiB | deployments | Σ window/slide | max latency (ms) | planning (s) |
|---|---|---|---|---|---|---|---|
| ASAP | 0.000682 | 0.0168 | 0.000379 | 5 | 5 | 0.0616 | 0.046 |
| PerQuery-CostAware | 0.00204 | 0.0504 | 0.000435 | 15 | 15 | 0.0616 | 0.0726 |
| AutoSketch-Adapted | 0.0473 | 1.17 | 0.000222 | 15 | 330 | 0.0024 | 0.0023 |

Weights fargate, 3 ms:

| method | objective | CPU (vCPU) | GiB | deployments | Σ window/slide | max latency (ms) | planning (s) |
|---|---|---|---|---|---|---|---|
| ASAP | 0.000682 | 0.0168 | 0.000379 | 5 | 5 | 0.0616 | 0.0461 |
| PerQuery-CostAware | 0.00204 | 0.0504 | 0.000435 | 15 | 15 | 0.0616 | 0.0727 |
| AutoSketch-Adapted | 0.0473 | 1.17 | 0.000222 | 15 | 330 | 0.0024 | 0.0023 |

Weights fargate, 10 ms:

| method | objective | CPU (vCPU) | GiB | deployments | Σ window/slide | max latency (ms) | planning (s) |
|---|---|---|---|---|---|---|---|
| ASAP | 0.0409 | 0.993 | 0.15 | 8 | 71 | 7.23 | 0.0499 |
| PerQuery-CostAware | 0.0422 | 1.03 | 0.15 | 18 | 81 | 7.23 | 0.0797 |
| AutoSketch-Adapted | 0.0875 | 2.14 | 0.149 | 18 | 396 | 7.23 | 0.0023 |

Weights fargate, no SLA:

| method | objective | CPU (vCPU) | GiB | deployments | Σ window/slide | max latency (ms) | planning (s) |
|---|---|---|---|---|---|---|---|
| ASAP | 0.303 | 2.31 | 47.1 | 17 | 17 | 2.28e+04 | 0.0553 |
| PerQuery-CostAware | 0.417 | 4.48 | 53 | 51 | 51 | 2.28e+04 | 0.137 |
| AutoSketch-Adapted | 3.14 | 71.8 | 51.5 | 51 | 1.12e+03 | 887 | 0.0023 |

## google_2011

27 RQEs; dropped as unservable: 0. Sanity violations: 0.

Weights cpu, 0.003 ms:

| method | objective | CPU (vCPU) | GiB | deployments | Σ window/slide | max latency (ms) | planning (s) |
|---|---|---|---|---|---|---|---|
| ASAP | 0.00112 | 0.00112 | 5.76e-05 | 6 | 11 | 0.00293 | 0.0284 |
| PerQuery-CostAware | 0.00135 | 0.00135 | 6.48e-05 | 9 | 14 | 0.00293 | 0.0346 |
| AutoSketch-Adapted | 0.00345 | 0.00345 | 2.95e-05 | 9 | 42 | 0.0016 | 0.0012 |

Weights cpu, 0.01 ms:

| method | objective | CPU (vCPU) | GiB | deployments | Σ window/slide | max latency (ms) | planning (s) |
|---|---|---|---|---|---|---|---|
| ASAP | 0.00107 | 0.00107 | 7.16e-05 | 5 | 7 | 0.00991 | 0.0292 |
| PerQuery-CostAware | 0.00174 | 0.00174 | 8.86e-05 | 12 | 14 | 0.00991 | 0.0397 |
| AutoSketch-Adapted | 0.00636 | 0.00636 | 5.2e-05 | 12 | 56 | 0.00439 | 0.0012 |

Weights cpu, 0.03 ms:

| method | objective | CPU (vCPU) | GiB | deployments | Σ window/slide | max latency (ms) | planning (s) |
|---|---|---|---|---|---|---|---|
| ASAP | 0.000442 | 0.000442 | 6.9e-05 | 4 | 4 | 0.0246 | 0.0292 |
| PerQuery-CostAware | 0.00133 | 0.00133 | 8.86e-05 | 12 | 12 | 0.0246 | 0.0396 |
| AutoSketch-Adapted | 0.00636 | 0.00636 | 5.2e-05 | 12 | 56 | 0.00439 | 0.0012 |

Weights cpu, 0.1 ms:

| method | objective | CPU (vCPU) | GiB | deployments | Σ window/slide | max latency (ms) | planning (s) |
|---|---|---|---|---|---|---|---|
| ASAP | 0.000442 | 0.000442 | 6.9e-05 | 4 | 4 | 0.0246 | 0.0293 |
| PerQuery-CostAware | 0.00133 | 0.00133 | 8.86e-05 | 12 | 12 | 0.0246 | 0.0399 |
| AutoSketch-Adapted | 0.00636 | 0.00636 | 5.2e-05 | 12 | 56 | 0.00439 | 0.0012 |

Weights cpu, 0.3 ms:

| method | objective | CPU (vCPU) | GiB | deployments | Σ window/slide | max latency (ms) | planning (s) |
|---|---|---|---|---|---|---|---|
| ASAP | 0.00315 | 0.00315 | 0.00106 | 6 | 17 | 0.219 | 0.0295 |
| PerQuery-CostAware | 0.00424 | 0.00424 | 0.00121 | 15 | 26 | 0.219 | 0.0444 |
| AutoSketch-Adapted | 0.00927 | 0.00927 | 0.00117 | 15 | 70 | 0.219 | 0.0012 |

Weights cpu, 1 ms:

| method | objective | CPU (vCPU) | GiB | deployments | Σ window/slide | max latency (ms) | planning (s) |
|---|---|---|---|---|---|---|---|
| ASAP | 0.00191 | 0.00191 | 0.0029 | 8 | 11 | 0.827 | 0.0302 |
| PerQuery-CostAware | 0.00321 | 0.00321 | 0.00326 | 18 | 21 | 0.827 | 0.0495 |
| AutoSketch-Adapted | 0.0122 | 0.0122 | 0.00305 | 18 | 84 | 0.366 | 0.0012 |

Weights cpu, 3 ms:

| method | objective | CPU (vCPU) | GiB | deployments | Σ window/slide | max latency (ms) | planning (s) |
|---|---|---|---|---|---|---|---|
| ASAP | 0.0183 | 0.0183 | 0.0216 | 10 | 26 | 2.57 | 0.0312 |
| PerQuery-CostAware | 0.0216 | 0.0216 | 0.0247 | 24 | 40 | 2.57 | 0.0612 |
| AutoSketch-Adapted | 0.0357 | 0.0357 | 0.024 | 24 | 112 | 2.27 | 0.0012 |

Weights cpu, 10 ms:

| method | objective | CPU (vCPU) | GiB | deployments | Σ window/slide | max latency (ms) | planning (s) |
|---|---|---|---|---|---|---|---|
| ASAP | 0.0163 | 0.0163 | 0.0464 | 12 | 17 | 9.18 | 0.0318 |
| PerQuery-CostAware | 0.0218 | 0.0218 | 0.0525 | 27 | 32 | 9.18 | 0.0642 |
| AutoSketch-Adapted | 0.0645 | 0.0645 | 0.0496 | 27 | 126 | 5 | 0.0012 |

Weights cpu, no SLA:

| method | objective | CPU (vCPU) | GiB | deployments | Σ window/slide | max latency (ms) | planning (s) |
|---|---|---|---|---|---|---|---|
| ASAP | 0.00484 | 0.00484 | 0.041 | 9 | 9 | 28 | 0.0325 |
| PerQuery-CostAware | 0.014 | 0.014 | 0.0525 | 27 | 27 | 28 | 0.0641 |
| AutoSketch-Adapted | 0.0645 | 0.0645 | 0.0496 | 27 | 126 | 5 | 0.0012 |

Weights fargate, 0.003 ms:

| method | objective | CPU (vCPU) | GiB | deployments | Σ window/slide | max latency (ms) | planning (s) |
|---|---|---|---|---|---|---|---|
| ASAP | 4.56e-05 | 0.00112 | 5.76e-05 | 6 | 11 | 0.00293 | 0.0283 |
| PerQuery-CostAware | 5.51e-05 | 0.00135 | 6.48e-05 | 9 | 14 | 0.00293 | 0.0348 |
| AutoSketch-Adapted | 0.00014 | 0.00345 | 2.95e-05 | 9 | 42 | 0.0016 | 0.0012 |

Weights fargate, 0.01 ms:

| method | objective | CPU (vCPU) | GiB | deployments | Σ window/slide | max latency (ms) | planning (s) |
|---|---|---|---|---|---|---|---|
| ASAP | 4.35e-05 | 0.00107 | 7.16e-05 | 5 | 7 | 0.00991 | 0.0304 |
| PerQuery-CostAware | 7.1e-05 | 0.00174 | 8.86e-05 | 12 | 14 | 0.00991 | 0.0398 |
| AutoSketch-Adapted | 0.000258 | 0.00636 | 5.2e-05 | 12 | 56 | 0.00439 | 0.0012 |

Weights fargate, 0.03 ms:

| method | objective | CPU (vCPU) | GiB | deployments | Σ window/slide | max latency (ms) | planning (s) |
|---|---|---|---|---|---|---|---|
| ASAP | 1.82e-05 | 0.000442 | 6.9e-05 | 4 | 4 | 0.0246 | 0.0304 |
| PerQuery-CostAware | 5.41e-05 | 0.00133 | 8.86e-05 | 12 | 12 | 0.0246 | 0.0399 |
| AutoSketch-Adapted | 0.000258 | 0.00636 | 5.2e-05 | 12 | 56 | 0.00439 | 0.0012 |

Weights fargate, 0.1 ms:

| method | objective | CPU (vCPU) | GiB | deployments | Σ window/slide | max latency (ms) | planning (s) |
|---|---|---|---|---|---|---|---|
| ASAP | 1.82e-05 | 0.000442 | 6.9e-05 | 4 | 4 | 0.0246 | 0.0304 |
| PerQuery-CostAware | 5.41e-05 | 0.00133 | 8.86e-05 | 12 | 12 | 0.0246 | 0.04 |
| AutoSketch-Adapted | 0.000258 | 0.00636 | 5.2e-05 | 12 | 56 | 0.00439 | 0.0012 |

Weights fargate, 0.3 ms:

| method | objective | CPU (vCPU) | GiB | deployments | Σ window/slide | max latency (ms) | planning (s) |
|---|---|---|---|---|---|---|---|
| ASAP | 0.000132 | 0.00315 | 0.00106 | 6 | 17 | 0.219 | 0.031 |
| PerQuery-CostAware | 0.000177 | 0.00424 | 0.00121 | 15 | 26 | 0.219 | 0.0448 |
| AutoSketch-Adapted | 0.000381 | 0.00927 | 0.00117 | 15 | 70 | 0.219 | 0.0012 |

Weights fargate, 1 ms:

| method | objective | CPU (vCPU) | GiB | deployments | Σ window/slide | max latency (ms) | planning (s) |
|---|---|---|---|---|---|---|---|
| ASAP | 9.01e-05 | 0.00191 | 0.0029 | 8 | 11 | 0.827 | 0.0314 |
| PerQuery-CostAware | 0.000144 | 0.00321 | 0.00326 | 18 | 21 | 0.827 | 0.0499 |
| AutoSketch-Adapted | 0.000507 | 0.0122 | 0.00305 | 18 | 84 | 0.366 | 0.0012 |

Weights fargate, 3 ms:

| method | objective | CPU (vCPU) | GiB | deployments | Σ window/slide | max latency (ms) | planning (s) |
|---|---|---|---|---|---|---|---|
| ASAP | 0.000835 | 0.0183 | 0.0216 | 10 | 26 | 2.57 | 0.0326 |
| PerQuery-CostAware | 0.000987 | 0.0216 | 0.0247 | 24 | 40 | 2.57 | 0.0597 |
| AutoSketch-Adapted | 0.00155 | 0.0357 | 0.024 | 24 | 112 | 2.27 | 0.0012 |

Weights fargate, 10 ms:

| method | objective | CPU (vCPU) | GiB | deployments | Σ window/slide | max latency (ms) | planning (s) |
|---|---|---|---|---|---|---|---|
| ASAP | 0.000867 | 0.0163 | 0.0464 | 12 | 17 | 9.18 | 0.0332 |
| PerQuery-CostAware | 0.00111 | 0.0218 | 0.0525 | 27 | 32 | 9.18 | 0.065 |
| AutoSketch-Adapted | 0.00283 | 0.0645 | 0.0496 | 27 | 126 | 5 | 0.0012 |

Weights fargate, no SLA:

| method | objective | CPU (vCPU) | GiB | deployments | Σ window/slide | max latency (ms) | planning (s) |
|---|---|---|---|---|---|---|---|
| ASAP | 0.000378 | 0.00484 | 0.041 | 9 | 9 | 28 | 0.0339 |
| PerQuery-CostAware | 0.000801 | 0.014 | 0.0525 | 27 | 27 | 28 | 0.065 |
| AutoSketch-Adapted | 0.00283 | 0.0645 | 0.0496 | 27 | 126 | 5 | 0.0012 |
