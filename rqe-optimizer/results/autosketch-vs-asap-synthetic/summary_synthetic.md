# Synthetic workload: AutoSketch vs. ASAP

## templates=all, shared=1, accuracy=p95

50 RQEs on 8 streams; sanity violations: 0. AutoSketch probes: 158, benchmark time ≥ 56.6 s.

| weights | SLA (ms) | RQEs | method | objective | vs ASAP | CPU (vCPU) | GiB | deployments | Σ window/slide | max latency (ms) | planning (s) |
|---|---|---|---|---|---|---|---|---|---|---|---|
| cpu | 0.01 | 22 | ASAP | 800 | 1× | 800 | 0.0624 | 16 | 4.75e+03 | 0.00901 | 0.456 |
| cpu | 0.01 | 22 | PerQuery-CostAware | 831 | 1.04× | 831 | 0.0629 | 22 | 4.85e+03 | 0.00901 | 0.499 |
| cpu | 0.01 | 22 | AutoSketch-Adapted | 1.72e+03 | 2.15× | 1.72e+03 | 0.0628 | 22 | 7.51e+03 | 0.00436 | 0.000928 |
| cpu | 0.1 | 22 | ASAP | 116 | 1× | 116 | 0.186 | 10 | 822 | 0.0838 | 0.507 |
| cpu | 0.1 | 22 | PerQuery-CostAware | 121 | 1.04× | 121 | 0.118 | 22 | 840 | 0.094 | 0.503 |
| cpu | 0.1 | 22 | AutoSketch-Adapted | 1.72e+03 | 14.8× | 1.72e+03 | 0.0628 | 22 | 7.51e+03 | 0.00436 | 0.000928 |
| cpu | 1.0 | 22 | ASAP | 33.1 | 1× | 33.1 | 0.274 | 8 | 258 | 0.98 | 0.502 |
| cpu | 1.0 | 22 | PerQuery-CostAware | 35.5 | 1.07× | 35.5 | 0.255 | 22 | 268 | 0.98 | 0.507 |
| cpu | 1.0 | 22 | AutoSketch-Adapted | 1.72e+03 | 52× | 1.72e+03 | 0.0628 | 22 | 7.51e+03 | 0.00436 | 0.000928 |
| cpu | 10.0 | 50 | ASAP | 377 | 1× | 377 | 18.1 | 18 | 1.71e+03 | 9.4 | 0.623 |
| cpu | 10.0 | 50 | PerQuery-CostAware | 528 | 1.4× | 528 | 65.7 | 50 | 4.33e+03 | 9.4 | 0.572 |
| cpu | 10.0 | 50 | AutoSketch-Adapted | 3.41e+03 | 9.04× | 3.41e+03 | 65 | 50 | 2.06e+04 | 4.36 | 0.000928 |
| cpu | inf | 50 | ASAP | 3.32 | 1× | 3.32 | 42.8 | 8 | 12 | 1.21e+04 | 0.698 |
| cpu | inf | 50 | PerQuery-CostAware | 9.55 | 2.88× | 9.55 | 161 | 50 | 57 | 3.84e+03 | 0.616 |
| cpu | inf | 50 | AutoSketch-Adapted | 3.41e+03 | 1.03e+03× | 3.41e+03 | 65 | 50 | 2.06e+04 | 4.36 | 0.000928 |
| fargate | 0.01 | 22 | ASAP | 32.4 | 1× | 800 | 0.0624 | 16 | 4.75e+03 | 0.00901 | 0.458 |
| fargate | 0.01 | 22 | PerQuery-CostAware | 33.7 | 1.04× | 831 | 0.0629 | 22 | 4.85e+03 | 0.00901 | 0.503 |
| fargate | 0.01 | 22 | AutoSketch-Adapted | 69.7 | 2.15× | 1.72e+03 | 0.0628 | 22 | 7.51e+03 | 0.00436 | 0.000928 |
| fargate | 0.1 | 22 | ASAP | 4.72 | 1× | 116 | 0.186 | 10 | 822 | 0.0838 | 0.517 |
| fargate | 0.1 | 22 | PerQuery-CostAware | 4.92 | 1.04× | 121 | 0.118 | 22 | 840 | 0.094 | 0.505 |
| fargate | 0.1 | 22 | AutoSketch-Adapted | 69.7 | 14.8× | 1.72e+03 | 0.0628 | 22 | 7.51e+03 | 0.00436 | 0.000928 |
| fargate | 1.0 | 22 | ASAP | 1.34 | 1× | 33.1 | 0.274 | 8 | 258 | 0.98 | 0.517 |
| fargate | 1.0 | 22 | PerQuery-CostAware | 1.44 | 1.07× | 35.5 | 0.255 | 22 | 268 | 0.98 | 0.51 |
| fargate | 1.0 | 22 | AutoSketch-Adapted | 69.7 | 51.9× | 1.72e+03 | 0.0628 | 22 | 7.51e+03 | 0.00436 | 0.000928 |
| fargate | 10.0 | 50 | ASAP | 15.4 | 1× | 377 | 18.1 | 18 | 1.71e+03 | 9.4 | 0.638 |
| fargate | 10.0 | 50 | PerQuery-CostAware | 21.7 | 1.41× | 528 | 65.7 | 50 | 4.33e+03 | 9.4 | 0.578 |
| fargate | 10.0 | 50 | AutoSketch-Adapted | 138 | 9.01× | 3.41e+03 | 65 | 50 | 2.06e+04 | 4.36 | 0.000928 |
| fargate | inf | 50 | ASAP | 0.221 | 1× | 3.35 | 19.1 | 8 | 14 | 2.94e+03 | 0.81 |
| fargate | inf | 50 | PerQuery-CostAware | 0.721 | 3.26× | 9.97 | 71.2 | 50 | 59 | 2.97e+03 | 0.624 |
| fargate | inf | 50 | AutoSketch-Adapted | 138 | 627× | 3.41e+03 | 65 | 50 | 2.06e+04 | 4.36 | 0.000928 |

## templates=dashboard, shared=1, accuracy=p95

21 RQEs on 4 streams; sanity violations: 0. AutoSketch probes: 83, benchmark time ≥ 40.2 s.

| weights | SLA (ms) | RQEs | method | objective | vs ASAP | CPU (vCPU) | GiB | deployments | Σ window/slide | max latency (ms) | planning (s) |
|---|---|---|---|---|---|---|---|---|---|---|---|
| cpu | 0.01 | 9 | ASAP | 224 | 1× | 224 | 0.00505 | 7 | 701 | 0.00843 | 0.0406 |
| cpu | 0.01 | 9 | PerQuery-CostAware | 224 | 1× | 224 | 0.00512 | 9 | 703 | 0.00843 | 0.214 |
| cpu | 0.01 | 9 | AutoSketch-Adapted | 653 | 2.92× | 653 | 0.00505 | 9 | 1.95e+03 | 0.00436 | 0.000394 |
| cpu | 0.1 | 9 | ASAP | 15.3 | 1× | 15.3 | 0.00686 | 4 | 55 | 0.0838 | 0.043 |
| cpu | 0.1 | 9 | PerQuery-CostAware | 16.7 | 1.09× | 16.7 | 0.00629 | 9 | 61 | 0.094 | 0.215 |
| cpu | 0.1 | 9 | AutoSketch-Adapted | 653 | 42.8× | 653 | 0.00505 | 9 | 1.95e+03 | 0.00436 | 0.000394 |
| cpu | 1.0 | 9 | ASAP | 1.68 | 1× | 1.68 | 0.00863 | 3 | 9 | 0.98 | 0.0438 |
| cpu | 1.0 | 9 | PerQuery-CostAware | 2.77 | 1.65× | 2.77 | 0.00917 | 9 | 14 | 0.98 | 0.216 |
| cpu | 1.0 | 9 | AutoSketch-Adapted | 653 | 389× | 653 | 0.00505 | 9 | 1.95e+03 | 0.00436 | 0.000394 |
| cpu | 10.0 | 21 | ASAP | 29.4 | 1× | 29.4 | 11.8 | 7 | 630 | 9.4 | 0.135 |
| cpu | 10.0 | 21 | PerQuery-CostAware | 87.8 | 2.99× | 87.8 | 35.4 | 21 | 1.88e+03 | 9.4 | 0.243 |
| cpu | 10.0 | 21 | AutoSketch-Adapted | 911 | 31× | 911 | 35.3 | 21 | 7.58e+03 | 1.13 | 0.000394 |
| cpu | inf | 21 | ASAP | 0.767 | 1× | 0.767 | 22.6 | 4 | 8 | 1.53e+03 | 0.154 |
| cpu | inf | 21 | PerQuery-CostAware | 2.51 | 3.27× | 2.51 | 88 | 21 | 24 | 3.84e+03 | 0.259 |
| cpu | inf | 21 | AutoSketch-Adapted | 911 | 1.19e+03× | 911 | 35.3 | 21 | 7.58e+03 | 1.13 | 0.000394 |
| fargate | 0.01 | 9 | ASAP | 9.06 | 1× | 224 | 0.00505 | 7 | 701 | 0.00843 | 0.0407 |
| fargate | 0.01 | 9 | PerQuery-CostAware | 9.06 | 1× | 224 | 0.00512 | 9 | 703 | 0.00843 | 0.214 |
| fargate | 0.01 | 9 | AutoSketch-Adapted | 26.4 | 2.92× | 653 | 0.00505 | 9 | 1.95e+03 | 0.00436 | 0.000394 |
| fargate | 0.1 | 9 | ASAP | 0.618 | 1× | 15.3 | 0.00686 | 4 | 55 | 0.0838 | 0.0509 |
| fargate | 0.1 | 9 | PerQuery-CostAware | 0.677 | 1.09× | 16.7 | 0.00629 | 9 | 61 | 0.094 | 0.216 |
| fargate | 0.1 | 9 | AutoSketch-Adapted | 26.4 | 42.8× | 653 | 0.00505 | 9 | 1.95e+03 | 0.00436 | 0.000394 |
| fargate | 1.0 | 9 | ASAP | 0.0679 | 1× | 1.68 | 0.00863 | 3 | 9 | 0.98 | 0.0524 |
| fargate | 1.0 | 9 | PerQuery-CostAware | 0.112 | 1.65× | 2.77 | 0.00917 | 9 | 14 | 0.98 | 0.217 |
| fargate | 1.0 | 9 | AutoSketch-Adapted | 26.4 | 389× | 653 | 0.00505 | 9 | 1.95e+03 | 0.00436 | 0.000394 |
| fargate | 10.0 | 21 | ASAP | 1.24 | 1× | 29.4 | 11.8 | 7 | 630 | 9.4 | 0.136 |
| fargate | 10.0 | 21 | PerQuery-CostAware | 3.71 | 2.99× | 87.8 | 35.4 | 21 | 1.88e+03 | 9.4 | 0.244 |
| fargate | 10.0 | 21 | AutoSketch-Adapted | 37 | 29.8× | 911 | 35.3 | 21 | 7.58e+03 | 1.13 | 0.000394 |
| fargate | inf | 21 | ASAP | 0.072 | 1× | 0.776 | 9.11 | 4 | 6 | 1.98e+03 | 0.164 |
| fargate | inf | 21 | PerQuery-CostAware | 0.265 | 3.68× | 2.65 | 35.4 | 21 | 24 | 2.97e+03 | 0.262 |
| fargate | inf | 21 | AutoSketch-Adapted | 37 | 515× | 911 | 35.3 | 21 | 7.58e+03 | 1.13 | 0.000394 |

## templates=dashboard, shared=64, accuracy=p95

57 RQEs on 4 streams; sanity violations: 0. AutoSketch probes: 219, benchmark time ≥ 40.2 s.

| weights | SLA (ms) | RQEs | method | objective | vs ASAP | CPU (vCPU) | GiB | deployments | Σ window/slide | max latency (ms) | planning (s) |
|---|---|---|---|---|---|---|---|---|---|---|---|
| cpu | 0.01 | 21 | ASAP | 1.34e+03 | 1× | 1.34e+03 | 0.0299 | 7 | 4.2e+03 | 0.00843 | 0.171 |
| cpu | 0.01 | 21 | PerQuery-CostAware | 1.61e+03 | 1.2× | 1.61e+03 | 0.0361 | 21 | 5.04e+03 | 0.00843 | 0.638 |
| cpu | 0.01 | 21 | AutoSketch-Adapted | 4.7e+03 | 3.5× | 4.7e+03 | 0.036 | 21 | 1.4e+04 | 0.00436 | 0.000954 |
| cpu | 0.1 | 21 | ASAP | 88.3 | 1× | 88.3 | 0.0405 | 4 | 316 | 0.0838 | 0.212 |
| cpu | 0.1 | 21 | PerQuery-CostAware | 118 | 1.34× | 118 | 0.0447 | 21 | 417 | 0.094 | 0.644 |
| cpu | 0.1 | 21 | AutoSketch-Adapted | 4.7e+03 | 53.2× | 4.7e+03 | 0.036 | 21 | 1.4e+04 | 0.00436 | 0.000954 |
| cpu | 1.0 | 21 | ASAP | 9.9 | 1× | 9.9 | 0.0506 | 3 | 49 | 0.98 | 0.267 |
| cpu | 1.0 | 21 | PerQuery-CostAware | 16.7 | 1.69× | 16.7 | 0.0655 | 21 | 75 | 0.98 | 0.644 |
| cpu | 1.0 | 21 | AutoSketch-Adapted | 4.7e+03 | 475× | 4.7e+03 | 0.036 | 21 | 1.4e+04 | 0.00436 | 0.000954 |
| cpu | 10.0 | 57 | ASAP | 174 | 1× | 174 | 70.7 | 7 | 3.76e+03 | 9.4 | 0.709 |
| cpu | 10.0 | 57 | PerQuery-CostAware | 626 | 3.6× | 626 | 254 | 57 | 1.35e+04 | 9.4 | 0.723 |
| cpu | 10.0 | 57 | AutoSketch-Adapted | 6.56e+03 | 37.7× | 6.56e+03 | 254 | 57 | 5.45e+04 | 1.13 | 0.000954 |
| cpu | inf | 57 | ASAP | 2.37 | 1× | 2.37 | 136 | 4 | 33 | 1.53e+03 | 0.917 |
| cpu | inf | 57 | PerQuery-CostAware | 10.5 | 4.43× | 10.5 | 632 | 57 | 114 | 3.84e+03 | 0.785 |
| cpu | inf | 57 | AutoSketch-Adapted | 6.56e+03 | 2.77e+03× | 6.56e+03 | 254 | 57 | 5.45e+04 | 1.13 | 0.000954 |
| fargate | 0.01 | 21 | ASAP | 54.4 | 1× | 1.34e+03 | 0.0299 | 7 | 4.2e+03 | 0.00843 | 0.176 |
| fargate | 0.01 | 21 | PerQuery-CostAware | 65.2 | 1.2× | 1.61e+03 | 0.0361 | 21 | 5.04e+03 | 0.00843 | 0.639 |
| fargate | 0.01 | 21 | AutoSketch-Adapted | 190 | 3.5× | 4.7e+03 | 0.036 | 21 | 1.4e+04 | 0.00436 | 0.000954 |
| fargate | 0.1 | 21 | ASAP | 3.58 | 1× | 88.3 | 0.0405 | 4 | 316 | 0.0838 | 0.227 |
| fargate | 0.1 | 21 | PerQuery-CostAware | 4.79 | 1.34× | 118 | 0.0447 | 21 | 417 | 0.094 | 0.643 |
| fargate | 0.1 | 21 | AutoSketch-Adapted | 190 | 53.2× | 4.7e+03 | 0.036 | 21 | 1.4e+04 | 0.00436 | 0.000954 |
| fargate | 1.0 | 21 | ASAP | 0.401 | 1× | 9.9 | 0.0506 | 3 | 49 | 0.98 | 0.251 |
| fargate | 1.0 | 21 | PerQuery-CostAware | 0.676 | 1.69× | 16.7 | 0.0655 | 21 | 75 | 0.98 | 0.646 |
| fargate | 1.0 | 21 | AutoSketch-Adapted | 190 | 474× | 4.7e+03 | 0.036 | 21 | 1.4e+04 | 0.00436 | 0.000954 |
| fargate | 10.0 | 57 | ASAP | 7.35 | 1× | 174 | 70.7 | 7 | 3.76e+03 | 9.4 | 0.789 |
| fargate | 10.0 | 57 | PerQuery-CostAware | 26.5 | 3.6× | 626 | 254 | 57 | 1.35e+04 | 9.4 | 0.729 |
| fargate | 10.0 | 57 | AutoSketch-Adapted | 267 | 36.3× | 6.56e+03 | 254 | 57 | 5.45e+04 | 1.13 | 0.000954 |
| fargate | inf | 57 | ASAP | 0.343 | 1× | 2.45 | 54.8 | 4 | 21 | 1.98e+03 | 1.26 |
| fargate | inf | 57 | PerQuery-CostAware | 1.58 | 4.6× | 11 | 255 | 57 | 96 | 3.96e+03 | 0.797 |
| fargate | inf | 57 | AutoSketch-Adapted | 267 | 777× | 6.56e+03 | 254 | 57 | 5.45e+04 | 1.13 | 0.000954 |

## templates=dashboard, shared=8, accuracy=p95

52 RQEs on 4 streams; sanity violations: 0. AutoSketch probes: 201, benchmark time ≥ 40.2 s.

| weights | SLA (ms) | RQEs | method | objective | vs ASAP | CPU (vCPU) | GiB | deployments | Σ window/slide | max latency (ms) | planning (s) |
|---|---|---|---|---|---|---|---|---|---|---|---|
| cpu | 0.01 | 19 | ASAP | 1.34e+03 | 1× | 1.34e+03 | 0.0299 | 7 | 4.2e+03 | 0.00843 | 0.148 |
| cpu | 0.01 | 19 | PerQuery-CostAware | 1.6e+03 | 1.19× | 1.6e+03 | 0.035 | 19 | 4.96e+03 | 0.00843 | 0.615 |
| cpu | 0.01 | 19 | AutoSketch-Adapted | 4.67e+03 | 3.48× | 4.67e+03 | 0.0349 | 19 | 1.39e+04 | 0.00436 | 0.000857 |
| cpu | 0.1 | 19 | ASAP | 88.3 | 1× | 88.3 | 0.0404 | 4 | 316 | 0.0838 | 0.165 |
| cpu | 0.1 | 19 | PerQuery-CostAware | 116 | 1.31× | 116 | 0.0428 | 19 | 403 | 0.094 | 0.619 |
| cpu | 0.1 | 19 | AutoSketch-Adapted | 4.67e+03 | 52.9× | 4.67e+03 | 0.0349 | 19 | 1.39e+04 | 0.00436 | 0.000857 |
| cpu | 1.0 | 19 | ASAP | 9.9 | 1× | 9.9 | 0.0505 | 3 | 49 | 0.98 | 0.229 |
| cpu | 1.0 | 19 | PerQuery-CostAware | 15.8 | 1.6× | 15.8 | 0.0613 | 19 | 70 | 0.98 | 0.621 |
| cpu | 1.0 | 19 | AutoSketch-Adapted | 4.67e+03 | 472× | 4.67e+03 | 0.0349 | 19 | 1.39e+04 | 0.00436 | 0.000857 |
| cpu | 10.0 | 52 | ASAP | 174 | 1× | 174 | 70.7 | 7 | 3.76e+03 | 9.4 | 0.658 |
| cpu | 10.0 | 52 | PerQuery-CostAware | 622 | 3.58× | 622 | 253 | 52 | 1.35e+04 | 9.4 | 0.694 |
| cpu | 10.0 | 52 | AutoSketch-Adapted | 6.52e+03 | 37.5× | 6.52e+03 | 253 | 52 | 5.42e+04 | 1.13 | 0.000857 |
| cpu | inf | 52 | ASAP | 2.35 | 1× | 2.35 | 136 | 4 | 33 | 1.53e+03 | 0.836 |
| cpu | inf | 52 | PerQuery-CostAware | 9.9 | 4.21× | 9.9 | 629 | 52 | 109 | 3.84e+03 | 0.754 |
| cpu | inf | 52 | AutoSketch-Adapted | 6.52e+03 | 2.77e+03× | 6.52e+03 | 253 | 52 | 5.42e+04 | 1.13 | 0.000857 |
| fargate | 0.01 | 19 | ASAP | 54.4 | 1× | 1.34e+03 | 0.0299 | 7 | 4.2e+03 | 0.00843 | 0.155 |
| fargate | 0.01 | 19 | PerQuery-CostAware | 64.7 | 1.19× | 1.6e+03 | 0.035 | 19 | 4.96e+03 | 0.00843 | 0.618 |
| fargate | 0.01 | 19 | AutoSketch-Adapted | 189 | 3.48× | 4.67e+03 | 0.0349 | 19 | 1.39e+04 | 0.00436 | 0.000857 |
| fargate | 0.1 | 19 | ASAP | 3.58 | 1× | 88.3 | 0.0404 | 4 | 316 | 0.0838 | 0.194 |
| fargate | 0.1 | 19 | PerQuery-CostAware | 4.7 | 1.31× | 116 | 0.0428 | 19 | 403 | 0.094 | 0.62 |
| fargate | 0.1 | 19 | AutoSketch-Adapted | 189 | 52.9× | 4.67e+03 | 0.0349 | 19 | 1.39e+04 | 0.00436 | 0.000857 |
| fargate | 1.0 | 19 | ASAP | 0.401 | 1× | 9.9 | 0.0505 | 3 | 49 | 0.98 | 0.218 |
| fargate | 1.0 | 19 | PerQuery-CostAware | 0.642 | 1.6× | 15.8 | 0.0613 | 19 | 70 | 0.98 | 0.622 |
| fargate | 1.0 | 19 | AutoSketch-Adapted | 189 | 472× | 4.67e+03 | 0.0349 | 19 | 1.39e+04 | 0.00436 | 0.000857 |
| fargate | 10.0 | 52 | ASAP | 7.35 | 1× | 174 | 70.7 | 7 | 3.76e+03 | 9.4 | 0.718 |
| fargate | 10.0 | 52 | PerQuery-CostAware | 26.3 | 3.58× | 622 | 253 | 52 | 1.35e+04 | 9.4 | 0.7 |
| fargate | 10.0 | 52 | AutoSketch-Adapted | 265 | 36.1× | 6.52e+03 | 253 | 52 | 5.42e+04 | 1.13 | 0.000857 |
| fargate | inf | 52 | ASAP | 0.342 | 1× | 2.44 | 54.8 | 4 | 21 | 1.98e+03 | 1.27 |
| fargate | inf | 52 | PerQuery-CostAware | 1.55 | 4.52× | 10.4 | 253 | 52 | 91 | 3.96e+03 | 0.765 |
| fargate | inf | 52 | AutoSketch-Adapted | 265 | 774× | 6.52e+03 | 253 | 52 | 5.42e+04 | 1.13 | 0.000857 |
