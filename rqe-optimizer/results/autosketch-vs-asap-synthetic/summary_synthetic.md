# Synthetic workload: AutoSketch vs. ASAP

## templates=all, shared=1, metrics=1, accuracy=p95

50 RQEs on 8 streams; sanity violations: 0. AutoSketch probes: 166, benchmark time ≥ 65.2 s.

| weights | SLA (ms) | RQEs | method | objective | CPU (vCPU) | GiB | deployments | Σ window/slide | max latency (ms) | planning (s) |
|---|---|---|---|---|---|---|---|---|---|---|
| cpu | 0.01 | 22 | ASAP | 800 | 800 | 0.0624 | 16 | 4.75e+03 | 0.00901 | 0.555 |
| cpu | 0.01 | 22 | PerQuery-CostAware | 831 | 831 | 0.0629 | 22 | 4.85e+03 | 0.00901 | 0.558 |
| cpu | 0.01 | 22 | AutoSketch-Adapted | 1.72e+03 | 1.72e+03 | 0.0628 | 22 | 7.51e+03 | 0.00436 | 0.00192 |
| cpu | 0.1 | 22 | ASAP | 116 | 116 | 0.186 | 10 | 822 | 0.0838 | 0.6 |
| cpu | 0.1 | 22 | PerQuery-CostAware | 121 | 121 | 0.118 | 22 | 840 | 0.094 | 0.561 |
| cpu | 0.1 | 22 | AutoSketch-Adapted | 1.72e+03 | 1.72e+03 | 0.0628 | 22 | 7.51e+03 | 0.00436 | 0.00192 |
| cpu | 1.0 | 22 | ASAP | 33.1 | 33.1 | 0.274 | 8 | 258 | 0.98 | 0.602 |
| cpu | 1.0 | 22 | PerQuery-CostAware | 35.5 | 35.5 | 0.255 | 22 | 268 | 0.98 | 0.567 |
| cpu | 1.0 | 22 | AutoSketch-Adapted | 1.72e+03 | 1.72e+03 | 0.0628 | 22 | 7.51e+03 | 0.00436 | 0.00192 |
| cpu | 10.0 | 50 | ASAP | 377 | 377 | 18.1 | 18 | 1.71e+03 | 9.4 | 0.73 |
| cpu | 10.0 | 50 | PerQuery-CostAware | 528 | 528 | 65.7 | 50 | 4.33e+03 | 9.4 | 0.633 |
| cpu | 10.0 | 50 | AutoSketch-Adapted | 3.41e+03 | 3.41e+03 | 65 | 50 | 2.06e+04 | 4.36 | 0.00192 |
| cpu | inf | 50 | ASAP | 3.32 | 3.32 | 42.8 | 8 | 12 | 1.21e+04 | 0.802 |
| cpu | inf | 50 | PerQuery-CostAware | 9.55 | 9.55 | 161 | 50 | 57 | 3.84e+03 | 0.67 |
| cpu | inf | 50 | AutoSketch-Adapted | 3.41e+03 | 3.41e+03 | 65 | 50 | 2.06e+04 | 4.36 | 0.00192 |
| fargate | 0.01 | 22 | ASAP | 32.4 | 800 | 0.0624 | 16 | 4.75e+03 | 0.00901 | 0.557 |
| fargate | 0.01 | 22 | PerQuery-CostAware | 33.7 | 831 | 0.0629 | 22 | 4.85e+03 | 0.00901 | 0.56 |
| fargate | 0.01 | 22 | AutoSketch-Adapted | 69.7 | 1.72e+03 | 0.0628 | 22 | 7.51e+03 | 0.00436 | 0.00192 |
| fargate | 0.1 | 22 | ASAP | 4.72 | 116 | 0.186 | 10 | 822 | 0.0838 | 0.61 |
| fargate | 0.1 | 22 | PerQuery-CostAware | 4.92 | 121 | 0.118 | 22 | 840 | 0.094 | 0.563 |
| fargate | 0.1 | 22 | AutoSketch-Adapted | 69.7 | 1.72e+03 | 0.0628 | 22 | 7.51e+03 | 0.00436 | 0.00192 |
| fargate | 1.0 | 22 | ASAP | 1.34 | 33.1 | 0.274 | 8 | 258 | 0.98 | 0.617 |
| fargate | 1.0 | 22 | PerQuery-CostAware | 1.44 | 35.5 | 0.255 | 22 | 268 | 0.98 | 0.568 |
| fargate | 1.0 | 22 | AutoSketch-Adapted | 69.7 | 1.72e+03 | 0.0628 | 22 | 7.51e+03 | 0.00436 | 0.00192 |
| fargate | 10.0 | 50 | ASAP | 15.4 | 377 | 18.1 | 18 | 1.71e+03 | 9.4 | 0.744 |
| fargate | 10.0 | 50 | PerQuery-CostAware | 21.7 | 528 | 65.7 | 50 | 4.33e+03 | 9.4 | 0.637 |
| fargate | 10.0 | 50 | AutoSketch-Adapted | 138 | 3.41e+03 | 65 | 50 | 2.06e+04 | 4.36 | 0.00192 |
| fargate | inf | 50 | ASAP | 0.221 | 3.35 | 19.1 | 8 | 14 | 2.94e+03 | 0.913 |
| fargate | inf | 50 | PerQuery-CostAware | 0.721 | 9.97 | 71.2 | 50 | 59 | 2.97e+03 | 0.683 |
| fargate | inf | 50 | AutoSketch-Adapted | 138 | 3.41e+03 | 65 | 50 | 2.06e+04 | 4.36 | 0.00192 |

## templates=dashboard, shared=1, metrics=1, accuracy=p95

21 RQEs on 4 streams; sanity violations: 0. AutoSketch probes: 85, benchmark time ≥ 48.8 s.

| weights | SLA (ms) | RQEs | method | objective | CPU (vCPU) | GiB | deployments | Σ window/slide | max latency (ms) | planning (s) |
|---|---|---|---|---|---|---|---|---|---|---|
| cpu | 0.01 | 9 | ASAP | 224 | 224 | 0.00505 | 7 | 701 | 0.00843 | 0.05 |
| cpu | 0.01 | 9 | PerQuery-CostAware | 224 | 224 | 0.00512 | 9 | 703 | 0.00843 | 0.227 |
| cpu | 0.01 | 9 | AutoSketch-Adapted | 653 | 653 | 0.00505 | 9 | 1.95e+03 | 0.00436 | 0.000845 |
| cpu | 0.1 | 9 | ASAP | 15.3 | 15.3 | 0.00686 | 4 | 55 | 0.0838 | 0.0519 |
| cpu | 0.1 | 9 | PerQuery-CostAware | 16.7 | 16.7 | 0.00629 | 9 | 61 | 0.094 | 0.228 |
| cpu | 0.1 | 9 | AutoSketch-Adapted | 653 | 653 | 0.00505 | 9 | 1.95e+03 | 0.00436 | 0.000845 |
| cpu | 1.0 | 9 | ASAP | 1.68 | 1.68 | 0.00863 | 3 | 9 | 0.98 | 0.0529 |
| cpu | 1.0 | 9 | PerQuery-CostAware | 2.77 | 2.77 | 0.00917 | 9 | 14 | 0.98 | 0.229 |
| cpu | 1.0 | 9 | AutoSketch-Adapted | 653 | 653 | 0.00505 | 9 | 1.95e+03 | 0.00436 | 0.000845 |
| cpu | 10.0 | 21 | ASAP | 29.4 | 29.4 | 11.8 | 7 | 630 | 9.4 | 0.146 |
| cpu | 10.0 | 21 | PerQuery-CostAware | 87.8 | 87.8 | 35.4 | 21 | 1.88e+03 | 9.4 | 0.256 |
| cpu | 10.0 | 21 | AutoSketch-Adapted | 911 | 911 | 35.3 | 21 | 7.58e+03 | 1.13 | 0.000845 |
| cpu | inf | 21 | ASAP | 0.767 | 0.767 | 22.6 | 4 | 8 | 1.53e+03 | 0.16 |
| cpu | inf | 21 | PerQuery-CostAware | 2.51 | 2.51 | 88 | 21 | 24 | 3.84e+03 | 0.272 |
| cpu | inf | 21 | AutoSketch-Adapted | 911 | 911 | 35.3 | 21 | 7.58e+03 | 1.13 | 0.000845 |
| fargate | 0.01 | 9 | ASAP | 9.06 | 224 | 0.00505 | 7 | 701 | 0.00843 | 0.0502 |
| fargate | 0.01 | 9 | PerQuery-CostAware | 9.06 | 224 | 0.00512 | 9 | 703 | 0.00843 | 0.227 |
| fargate | 0.01 | 9 | AutoSketch-Adapted | 26.4 | 653 | 0.00505 | 9 | 1.95e+03 | 0.00436 | 0.000845 |
| fargate | 0.1 | 9 | ASAP | 0.618 | 15.3 | 0.00686 | 4 | 55 | 0.0838 | 0.0596 |
| fargate | 0.1 | 9 | PerQuery-CostAware | 0.677 | 16.7 | 0.00629 | 9 | 61 | 0.094 | 0.229 |
| fargate | 0.1 | 9 | AutoSketch-Adapted | 26.4 | 653 | 0.00505 | 9 | 1.95e+03 | 0.00436 | 0.000845 |
| fargate | 1.0 | 9 | ASAP | 0.0679 | 1.68 | 0.00863 | 3 | 9 | 0.98 | 0.0616 |
| fargate | 1.0 | 9 | PerQuery-CostAware | 0.112 | 2.77 | 0.00917 | 9 | 14 | 0.98 | 0.23 |
| fargate | 1.0 | 9 | AutoSketch-Adapted | 26.4 | 653 | 0.00505 | 9 | 1.95e+03 | 0.00436 | 0.000845 |
| fargate | 10.0 | 21 | ASAP | 1.24 | 29.4 | 11.8 | 7 | 630 | 9.4 | 0.147 |
| fargate | 10.0 | 21 | PerQuery-CostAware | 3.71 | 87.8 | 35.4 | 21 | 1.88e+03 | 9.4 | 0.258 |
| fargate | 10.0 | 21 | AutoSketch-Adapted | 37 | 911 | 35.3 | 21 | 7.58e+03 | 1.13 | 0.000845 |
| fargate | inf | 21 | ASAP | 0.072 | 0.776 | 9.11 | 4 | 6 | 1.98e+03 | 0.171 |
| fargate | inf | 21 | PerQuery-CostAware | 0.265 | 2.65 | 35.4 | 21 | 24 | 2.97e+03 | 0.275 |
| fargate | inf | 21 | AutoSketch-Adapted | 37 | 911 | 35.3 | 21 | 7.58e+03 | 1.13 | 0.000845 |

## templates=dashboard, shared=1, metrics=16, accuracy=p95

336 RQEs on 64 streams; sanity violations: 0. AutoSketch probes: 1360, benchmark time ≥ 780 s.

| weights | SLA (ms) | RQEs | method | objective | CPU (vCPU) | GiB | deployments | Σ window/slide | max latency (ms) | planning (s) |
|---|---|---|---|---|---|---|---|---|---|---|
| cpu | 0.01 | 144 | ASAP | 3.58e+03 | 3.58e+03 | 0.0809 | 112 | 1.12e+04 | 0.00843 | 0.616 |
| cpu | 0.01 | 144 | PerQuery-CostAware | 3.58e+03 | 3.58e+03 | 0.0818 | 144 | 1.12e+04 | 0.00843 | 3.51 |
| cpu | 0.01 | 144 | AutoSketch-Adapted | 1.04e+04 | 1.04e+04 | 0.0808 | 144 | 3.12e+04 | 0.00436 | 0.0142 |
| cpu | 0.1 | 144 | ASAP | 244 | 244 | 0.11 | 79 | 880 | 0.0838 | 0.71 |
| cpu | 0.1 | 144 | PerQuery-CostAware | 267 | 267 | 0.101 | 144 | 976 | 0.094 | 3.53 |
| cpu | 0.1 | 144 | AutoSketch-Adapted | 1.04e+04 | 1.04e+04 | 0.0808 | 144 | 3.12e+04 | 0.00436 | 0.0142 |
| cpu | 1.0 | 144 | ASAP | 26.8 | 26.8 | 0.138 | 48 | 144 | 0.98 | 0.763 |
| cpu | 1.0 | 144 | PerQuery-CostAware | 44.4 | 44.4 | 0.147 | 144 | 224 | 0.98 | 3.55 |
| cpu | 1.0 | 144 | AutoSketch-Adapted | 1.04e+04 | 1.04e+04 | 0.0808 | 144 | 3.12e+04 | 0.00436 | 0.0142 |
| cpu | 10.0 | 336 | ASAP | 470 | 470 | 190 | 112 | 1.01e+04 | 9.4 | 2.18 |
| cpu | 10.0 | 336 | PerQuery-CostAware | 1.4e+03 | 1.4e+03 | 566 | 336 | 3.02e+04 | 9.4 | 3.98 |
| cpu | 10.0 | 336 | AutoSketch-Adapted | 1.46e+04 | 1.46e+04 | 565 | 336 | 1.21e+05 | 1.13 | 0.0142 |
| cpu | inf | 336 | ASAP | 12.3 | 12.3 | 362 | 64 | 128 | 1.53e+03 | 2.63 |
| cpu | inf | 336 | PerQuery-CostAware | 40.1 | 40.1 | 1.41e+03 | 336 | 384 | 3.84e+03 | 4.21 |
| cpu | inf | 336 | AutoSketch-Adapted | 1.46e+04 | 1.46e+04 | 565 | 336 | 1.21e+05 | 1.13 | 0.0142 |
| fargate | 0.01 | 144 | ASAP | 145 | 3.58e+03 | 0.0809 | 112 | 1.12e+04 | 0.00843 | 0.622 |
| fargate | 0.01 | 144 | PerQuery-CostAware | 145 | 3.58e+03 | 0.0818 | 144 | 1.12e+04 | 0.00843 | 3.51 |
| fargate | 0.01 | 144 | AutoSketch-Adapted | 423 | 1.04e+04 | 0.0808 | 144 | 3.12e+04 | 0.00436 | 0.0142 |
| fargate | 0.1 | 144 | ASAP | 9.89 | 244 | 0.11 | 64 | 880 | 0.0838 | 0.751 |
| fargate | 0.1 | 144 | PerQuery-CostAware | 10.8 | 267 | 0.101 | 144 | 976 | 0.094 | 3.54 |
| fargate | 0.1 | 144 | AutoSketch-Adapted | 423 | 1.04e+04 | 0.0808 | 144 | 3.12e+04 | 0.00436 | 0.0142 |
| fargate | 1.0 | 144 | ASAP | 1.09 | 26.8 | 0.138 | 48 | 144 | 0.98 | 0.821 |
| fargate | 1.0 | 144 | PerQuery-CostAware | 1.8 | 44.4 | 0.147 | 144 | 224 | 0.98 | 3.55 |
| fargate | 1.0 | 144 | AutoSketch-Adapted | 423 | 1.04e+04 | 0.0808 | 144 | 3.12e+04 | 0.00436 | 0.0142 |
| fargate | 10.0 | 336 | ASAP | 19.9 | 470 | 190 | 112 | 1.01e+04 | 9.4 | 2.23 |
| fargate | 10.0 | 336 | PerQuery-CostAware | 59.4 | 1.4e+03 | 566 | 336 | 3.02e+04 | 9.4 | 3.98 |
| fargate | 10.0 | 336 | AutoSketch-Adapted | 593 | 1.46e+04 | 565 | 336 | 1.21e+05 | 1.13 | 0.0142 |
| fargate | inf | 336 | ASAP | 1.15 | 12.4 | 146 | 64 | 96 | 1.98e+03 | 2.87 |
| fargate | inf | 336 | PerQuery-CostAware | 4.24 | 42.4 | 566 | 336 | 384 | 2.97e+03 | 4.27 |
| fargate | inf | 336 | AutoSketch-Adapted | 593 | 1.46e+04 | 565 | 336 | 1.21e+05 | 1.13 | 0.0142 |

## templates=dashboard, shared=1, metrics=8, accuracy=p95

168 RQEs on 32 streams; sanity violations: 0. AutoSketch probes: 680, benchmark time ≥ 390 s.

| weights | SLA (ms) | RQEs | method | objective | CPU (vCPU) | GiB | deployments | Σ window/slide | max latency (ms) | planning (s) |
|---|---|---|---|---|---|---|---|---|---|---|
| cpu | 0.01 | 72 | ASAP | 1.79e+03 | 1.79e+03 | 0.0404 | 56 | 5.61e+03 | 0.00843 | 0.304 |
| cpu | 0.01 | 72 | PerQuery-CostAware | 1.79e+03 | 1.79e+03 | 0.0409 | 72 | 5.62e+03 | 0.00843 | 1.75 |
| cpu | 0.01 | 72 | AutoSketch-Adapted | 5.22e+03 | 5.22e+03 | 0.0404 | 72 | 1.56e+04 | 0.00436 | 0.00664 |
| cpu | 0.1 | 72 | ASAP | 122 | 122 | 0.0551 | 40 | 440 | 0.0838 | 0.349 |
| cpu | 0.1 | 72 | PerQuery-CostAware | 134 | 134 | 0.0504 | 72 | 488 | 0.094 | 1.77 |
| cpu | 0.1 | 72 | AutoSketch-Adapted | 5.22e+03 | 5.22e+03 | 0.0404 | 72 | 1.56e+04 | 0.00436 | 0.00664 |
| cpu | 1.0 | 72 | ASAP | 13.4 | 13.4 | 0.069 | 24 | 72 | 0.98 | 0.367 |
| cpu | 1.0 | 72 | PerQuery-CostAware | 22.2 | 22.2 | 0.0733 | 72 | 112 | 0.98 | 1.77 |
| cpu | 1.0 | 72 | AutoSketch-Adapted | 5.22e+03 | 5.22e+03 | 0.0404 | 72 | 1.56e+04 | 0.00436 | 0.00664 |
| cpu | 10.0 | 168 | ASAP | 235 | 235 | 94.8 | 56 | 5.04e+03 | 9.4 | 1.07 |
| cpu | 10.0 | 168 | PerQuery-CostAware | 702 | 702 | 283 | 168 | 1.51e+04 | 9.4 | 1.99 |
| cpu | 10.0 | 168 | AutoSketch-Adapted | 7.28e+03 | 7.28e+03 | 282 | 168 | 6.06e+04 | 1.13 | 0.00664 |
| cpu | inf | 168 | ASAP | 6.14 | 6.14 | 181 | 32 | 64 | 1.53e+03 | 1.26 |
| cpu | inf | 168 | PerQuery-CostAware | 20.1 | 20.1 | 704 | 168 | 192 | 3.84e+03 | 2.12 |
| cpu | inf | 168 | AutoSketch-Adapted | 7.28e+03 | 7.28e+03 | 282 | 168 | 6.06e+04 | 1.13 | 0.00664 |
| fargate | 0.01 | 72 | ASAP | 72.5 | 1.79e+03 | 0.0404 | 56 | 5.61e+03 | 0.00843 | 0.307 |
| fargate | 0.01 | 72 | PerQuery-CostAware | 72.5 | 1.79e+03 | 0.0409 | 72 | 5.62e+03 | 0.00843 | 1.76 |
| fargate | 0.01 | 72 | AutoSketch-Adapted | 211 | 5.22e+03 | 0.0404 | 72 | 1.56e+04 | 0.00436 | 0.00664 |
| fargate | 0.1 | 72 | ASAP | 4.95 | 122 | 0.0549 | 32 | 440 | 0.0838 | 0.376 |
| fargate | 0.1 | 72 | PerQuery-CostAware | 5.41 | 134 | 0.0504 | 72 | 488 | 0.094 | 1.77 |
| fargate | 0.1 | 72 | AutoSketch-Adapted | 211 | 5.22e+03 | 0.0404 | 72 | 1.56e+04 | 0.00436 | 0.00664 |
| fargate | 1.0 | 72 | ASAP | 0.543 | 13.4 | 0.069 | 24 | 72 | 0.98 | 0.395 |
| fargate | 1.0 | 72 | PerQuery-CostAware | 0.899 | 22.2 | 0.0733 | 72 | 112 | 0.98 | 1.78 |
| fargate | 1.0 | 72 | AutoSketch-Adapted | 211 | 5.22e+03 | 0.0404 | 72 | 1.56e+04 | 0.00436 | 0.00664 |
| fargate | 10.0 | 168 | ASAP | 9.95 | 235 | 94.8 | 56 | 5.04e+03 | 9.4 | 1.1 |
| fargate | 10.0 | 168 | PerQuery-CostAware | 29.7 | 702 | 283 | 168 | 1.51e+04 | 9.4 | 2.01 |
| fargate | 10.0 | 168 | AutoSketch-Adapted | 296 | 7.28e+03 | 282 | 168 | 6.06e+04 | 1.13 | 0.00664 |
| fargate | inf | 168 | ASAP | 0.576 | 6.2 | 72.9 | 32 | 48 | 1.98e+03 | 1.65 |
| fargate | inf | 168 | PerQuery-CostAware | 2.12 | 21.2 | 283 | 168 | 192 | 2.97e+03 | 2.13 |
| fargate | inf | 168 | AutoSketch-Adapted | 296 | 7.28e+03 | 282 | 168 | 6.06e+04 | 1.13 | 0.00664 |

## templates=dashboard, shared=8, metrics=1, accuracy=p95

52 RQEs on 4 streams; sanity violations: 0. AutoSketch probes: 206, benchmark time ≥ 48.8 s.

| weights | SLA (ms) | RQEs | method | objective | CPU (vCPU) | GiB | deployments | Σ window/slide | max latency (ms) | planning (s) |
|---|---|---|---|---|---|---|---|---|---|---|
| cpu | 0.01 | 19 | ASAP | 1.34e+03 | 1.34e+03 | 0.0299 | 7 | 4.2e+03 | 0.00843 | 0.188 |
| cpu | 0.01 | 19 | PerQuery-CostAware | 1.6e+03 | 1.6e+03 | 0.035 | 19 | 4.96e+03 | 0.00843 | 0.65 |
| cpu | 0.01 | 19 | AutoSketch-Adapted | 4.67e+03 | 4.67e+03 | 0.0349 | 19 | 1.39e+04 | 0.00436 | 0.00143 |
| cpu | 0.1 | 19 | ASAP | 88.3 | 88.3 | 0.0404 | 4 | 316 | 0.0838 | 0.207 |
| cpu | 0.1 | 19 | PerQuery-CostAware | 116 | 116 | 0.0428 | 19 | 403 | 0.094 | 0.653 |
| cpu | 0.1 | 19 | AutoSketch-Adapted | 4.67e+03 | 4.67e+03 | 0.0349 | 19 | 1.39e+04 | 0.00436 | 0.00143 |
| cpu | 1.0 | 19 | ASAP | 9.9 | 9.9 | 0.0505 | 3 | 49 | 0.98 | 0.265 |
| cpu | 1.0 | 19 | PerQuery-CostAware | 15.8 | 15.8 | 0.0613 | 19 | 70 | 0.98 | 0.655 |
| cpu | 1.0 | 19 | AutoSketch-Adapted | 4.67e+03 | 4.67e+03 | 0.0349 | 19 | 1.39e+04 | 0.00436 | 0.00143 |
| cpu | 10.0 | 52 | ASAP | 174 | 174 | 70.7 | 7 | 3.76e+03 | 9.4 | 0.651 |
| cpu | 10.0 | 52 | PerQuery-CostAware | 622 | 622 | 253 | 52 | 1.35e+04 | 9.4 | 0.733 |
| cpu | 10.0 | 52 | AutoSketch-Adapted | 6.52e+03 | 6.52e+03 | 253 | 52 | 5.42e+04 | 1.13 | 0.00143 |
| cpu | inf | 52 | ASAP | 2.35 | 2.35 | 136 | 4 | 33 | 1.53e+03 | 0.878 |
| cpu | inf | 52 | PerQuery-CostAware | 9.9 | 9.9 | 629 | 52 | 109 | 3.84e+03 | 0.789 |
| cpu | inf | 52 | AutoSketch-Adapted | 6.52e+03 | 6.52e+03 | 253 | 52 | 5.42e+04 | 1.13 | 0.00143 |
| fargate | 0.01 | 19 | ASAP | 54.4 | 1.34e+03 | 0.0299 | 7 | 4.2e+03 | 0.00843 | 0.195 |
| fargate | 0.01 | 19 | PerQuery-CostAware | 64.7 | 1.6e+03 | 0.035 | 19 | 4.96e+03 | 0.00843 | 0.651 |
| fargate | 0.01 | 19 | AutoSketch-Adapted | 189 | 4.67e+03 | 0.0349 | 19 | 1.39e+04 | 0.00436 | 0.00143 |
| fargate | 0.1 | 19 | ASAP | 3.58 | 88.3 | 0.0404 | 4 | 316 | 0.0838 | 0.234 |
| fargate | 0.1 | 19 | PerQuery-CostAware | 4.7 | 116 | 0.0428 | 19 | 403 | 0.094 | 0.654 |
| fargate | 0.1 | 19 | AutoSketch-Adapted | 189 | 4.67e+03 | 0.0349 | 19 | 1.39e+04 | 0.00436 | 0.00143 |
| fargate | 1.0 | 19 | ASAP | 0.401 | 9.9 | 0.0505 | 3 | 49 | 0.98 | 0.254 |
| fargate | 1.0 | 19 | PerQuery-CostAware | 0.642 | 15.8 | 0.0613 | 19 | 70 | 0.98 | 0.658 |
| fargate | 1.0 | 19 | AutoSketch-Adapted | 189 | 4.67e+03 | 0.0349 | 19 | 1.39e+04 | 0.00436 | 0.00143 |
| fargate | 10.0 | 52 | ASAP | 7.35 | 174 | 70.7 | 7 | 3.76e+03 | 9.4 | 0.711 |
| fargate | 10.0 | 52 | PerQuery-CostAware | 26.3 | 622 | 253 | 52 | 1.35e+04 | 9.4 | 0.738 |
| fargate | 10.0 | 52 | AutoSketch-Adapted | 265 | 6.52e+03 | 253 | 52 | 5.42e+04 | 1.13 | 0.00143 |
| fargate | inf | 52 | ASAP | 0.342 | 2.44 | 54.8 | 4 | 21 | 1.98e+03 | 1.31 |
| fargate | inf | 52 | PerQuery-CostAware | 1.55 | 10.4 | 253 | 52 | 91 | 3.96e+03 | 0.8 |
| fargate | inf | 52 | AutoSketch-Adapted | 265 | 6.52e+03 | 253 | 52 | 5.42e+04 | 1.13 | 0.00143 |
