# Synthetic workload: AutoSketch vs. ASAP

## templates=all, shared=1, metrics=1, accuracy=p95

50 RQEs on 8 streams; sanity violations: 0. AutoSketch probes: 166, benchmark time ≥ 65.2 s.

| weights | SLA (ms) | RQEs | method | objective | CPU (vCPU) | GiB | deployments | Σ window/slide | max latency (ms) | planning (s) |
|---|---|---|---|---|---|---|---|---|---|---|
| cpu | 0.003 | 13 | ASAP | 462 | 462 | 0.0567 | 9 | 3.75e+03 | 0.00119 | 0.573 |
| cpu | 0.003 | 13 | PerQuery-CostAware | 462 | 462 | 0.0568 | 13 | 3.76e+03 | 0.00119 | 0.553 |
| cpu | 0.003 | 13 | AutoSketch-Adapted | 462 | 462 | 0.0567 | 13 | 3.76e+03 | 0.00113 | 0.002 |
| cpu | 0.01 | 22 | ASAP | 800 | 800 | 0.0624 | 16 | 4.75e+03 | 0.00901 | 0.609 |
| cpu | 0.01 | 22 | PerQuery-CostAware | 831 | 831 | 0.0629 | 22 | 4.85e+03 | 0.00901 | 0.57 |
| cpu | 0.01 | 22 | AutoSketch-Adapted | 1.72e+03 | 1.72e+03 | 0.0628 | 22 | 7.51e+03 | 0.00436 | 0.002 |
| cpu | 0.03 | 22 | ASAP | 229 | 229 | 0.0891 | 15 | 1.47e+03 | 0.0291 | 0.611 |
| cpu | 0.03 | 22 | PerQuery-CostAware | 248 | 248 | 0.0904 | 22 | 1.53e+03 | 0.0291 | 0.573 |
| cpu | 0.03 | 22 | AutoSketch-Adapted | 1.72e+03 | 1.72e+03 | 0.0628 | 22 | 7.51e+03 | 0.00436 | 0.002 |
| cpu | 0.1 | 22 | ASAP | 116 | 116 | 0.186 | 10 | 822 | 0.0838 | 0.61 |
| cpu | 0.1 | 22 | PerQuery-CostAware | 121 | 121 | 0.118 | 22 | 840 | 0.094 | 0.574 |
| cpu | 0.1 | 22 | AutoSketch-Adapted | 1.72e+03 | 1.72e+03 | 0.0628 | 22 | 7.51e+03 | 0.00436 | 0.002 |
| cpu | 0.3 | 22 | ASAP | 55 | 55 | 0.231 | 12 | 409 | 0.296 | 0.622 |
| cpu | 0.3 | 22 | PerQuery-CostAware | 68.9 | 68.9 | 0.287 | 22 | 511 | 0.296 | 0.576 |
| cpu | 0.3 | 22 | AutoSketch-Adapted | 1.72e+03 | 1.72e+03 | 0.0628 | 22 | 7.51e+03 | 0.00436 | 0.002 |
| cpu | 1.0 | 22 | ASAP | 33.1 | 33.1 | 0.274 | 8 | 258 | 0.98 | 0.622 |
| cpu | 1.0 | 22 | PerQuery-CostAware | 35.5 | 35.5 | 0.255 | 22 | 268 | 0.98 | 0.579 |
| cpu | 1.0 | 22 | AutoSketch-Adapted | 1.72e+03 | 1.72e+03 | 0.0628 | 22 | 7.51e+03 | 0.00436 | 0.002 |
| cpu | 3.0 | 42 | ASAP | 75.8 | 75.8 | 29.7 | 12 | 2e+03 | 2.94 | 0.721 |
| cpu | 3.0 | 42 | PerQuery-CostAware | 321 | 321 | 147 | 42 | 9.55e+03 | 2.94 | 0.627 |
| cpu | 3.0 | 42 | AutoSketch-Adapted | 2.15e+03 | 2.15e+03 | 58.9 | 42 | 1.69e+04 | 1.13 | 0.002 |
| cpu | 10.0 | 50 | ASAP | 377 | 377 | 18.1 | 18 | 1.71e+03 | 9.4 | 0.756 |
| cpu | 10.0 | 50 | PerQuery-CostAware | 528 | 528 | 65.7 | 50 | 4.33e+03 | 9.4 | 0.644 |
| cpu | 10.0 | 50 | AutoSketch-Adapted | 3.41e+03 | 3.41e+03 | 65 | 50 | 2.06e+04 | 4.36 | 0.002 |
| cpu | inf | 50 | ASAP | 3.32 | 3.32 | 42.8 | 8 | 12 | 1.21e+04 | 0.824 |
| cpu | inf | 50 | PerQuery-CostAware | 9.55 | 9.55 | 161 | 50 | 57 | 3.84e+03 | 0.69 |
| cpu | inf | 50 | AutoSketch-Adapted | 3.41e+03 | 3.41e+03 | 65 | 50 | 2.06e+04 | 4.36 | 0.002 |
| fargate | 0.003 | 13 | ASAP | 18.7 | 462 | 0.0567 | 9 | 3.75e+03 | 0.00119 | 0.575 |
| fargate | 0.003 | 13 | PerQuery-CostAware | 18.7 | 462 | 0.0568 | 13 | 3.76e+03 | 0.00119 | 0.554 |
| fargate | 0.003 | 13 | AutoSketch-Adapted | 18.7 | 462 | 0.0567 | 13 | 3.76e+03 | 0.00113 | 0.002 |
| fargate | 0.01 | 22 | ASAP | 32.4 | 800 | 0.0624 | 16 | 4.75e+03 | 0.00901 | 0.611 |
| fargate | 0.01 | 22 | PerQuery-CostAware | 33.7 | 831 | 0.0629 | 22 | 4.85e+03 | 0.00901 | 0.572 |
| fargate | 0.01 | 22 | AutoSketch-Adapted | 69.7 | 1.72e+03 | 0.0628 | 22 | 7.51e+03 | 0.00436 | 0.002 |
| fargate | 0.03 | 22 | ASAP | 9.27 | 229 | 0.0891 | 14 | 1.47e+03 | 0.0291 | 0.619 |
| fargate | 0.03 | 22 | PerQuery-CostAware | 10.1 | 248 | 0.0904 | 22 | 1.53e+03 | 0.0291 | 0.574 |
| fargate | 0.03 | 22 | AutoSketch-Adapted | 69.7 | 1.72e+03 | 0.0628 | 22 | 7.51e+03 | 0.00436 | 0.002 |
| fargate | 0.1 | 22 | ASAP | 4.72 | 116 | 0.186 | 10 | 822 | 0.0838 | 0.62 |
| fargate | 0.1 | 22 | PerQuery-CostAware | 4.92 | 121 | 0.118 | 22 | 840 | 0.094 | 0.576 |
| fargate | 0.1 | 22 | AutoSketch-Adapted | 69.7 | 1.72e+03 | 0.0628 | 22 | 7.51e+03 | 0.00436 | 0.002 |
| fargate | 0.3 | 22 | ASAP | 2.23 | 55 | 0.231 | 12 | 409 | 0.296 | 0.635 |
| fargate | 0.3 | 22 | PerQuery-CostAware | 2.79 | 68.9 | 0.287 | 22 | 511 | 0.296 | 0.579 |
| fargate | 0.3 | 22 | AutoSketch-Adapted | 69.7 | 1.72e+03 | 0.0628 | 22 | 7.51e+03 | 0.00436 | 0.002 |
| fargate | 1.0 | 22 | ASAP | 1.34 | 33.1 | 0.274 | 8 | 258 | 0.98 | 0.638 |
| fargate | 1.0 | 22 | PerQuery-CostAware | 1.44 | 35.5 | 0.255 | 22 | 268 | 0.98 | 0.582 |
| fargate | 1.0 | 22 | AutoSketch-Adapted | 69.7 | 1.72e+03 | 0.0628 | 22 | 7.51e+03 | 0.00436 | 0.002 |
| fargate | 3.0 | 42 | ASAP | 3.2 | 75.8 | 29.7 | 12 | 2e+03 | 2.94 | 0.752 |
| fargate | 3.0 | 42 | PerQuery-CostAware | 13.6 | 321 | 147 | 42 | 9.55e+03 | 2.94 | 0.632 |
| fargate | 3.0 | 42 | AutoSketch-Adapted | 87.4 | 2.15e+03 | 58.9 | 42 | 1.69e+04 | 1.13 | 0.002 |
| fargate | 10.0 | 50 | ASAP | 15.4 | 377 | 18.1 | 18 | 1.71e+03 | 9.4 | 0.77 |
| fargate | 10.0 | 50 | PerQuery-CostAware | 21.7 | 528 | 65.7 | 50 | 4.33e+03 | 9.4 | 0.651 |
| fargate | 10.0 | 50 | AutoSketch-Adapted | 138 | 3.41e+03 | 65 | 50 | 2.06e+04 | 4.36 | 0.002 |
| fargate | inf | 50 | ASAP | 0.221 | 3.35 | 19.1 | 8 | 14 | 2.94e+03 | 0.936 |
| fargate | inf | 50 | PerQuery-CostAware | 0.721 | 9.97 | 71.2 | 50 | 59 | 2.97e+03 | 0.696 |
| fargate | inf | 50 | AutoSketch-Adapted | 138 | 3.41e+03 | 65 | 50 | 2.06e+04 | 4.36 | 0.002 |

## templates=dashboard, shared=1, metrics=1, accuracy=p95

21 RQEs on 4 streams; sanity violations: 0. AutoSketch probes: 85, benchmark time ≥ 48.8 s.

| weights | SLA (ms) | RQEs | method | objective | CPU (vCPU) | GiB | deployments | Σ window/slide | max latency (ms) | planning (s) |
|---|---|---|---|---|---|---|---|---|---|---|
| cpu | 0.003 | 5 | ASAP | 9.27 | 9.27 | 0.00119 | 3 | 76 | 0.00119 | 0.0488 |
| cpu | 0.003 | 5 | PerQuery-CostAware | 9.33 | 9.33 | 0.00126 | 5 | 78 | 0.00119 | 0.221 |
| cpu | 0.003 | 5 | AutoSketch-Adapted | 9.38 | 9.38 | 0.0012 | 5 | 78 | 0.00113 | 0.000876 |
| cpu | 0.01 | 9 | ASAP | 224 | 224 | 0.00505 | 7 | 701 | 0.00843 | 0.0527 |
| cpu | 0.01 | 9 | PerQuery-CostAware | 224 | 224 | 0.00512 | 9 | 703 | 0.00843 | 0.229 |
| cpu | 0.01 | 9 | AutoSketch-Adapted | 653 | 653 | 0.00505 | 9 | 1.95e+03 | 0.00436 | 0.000876 |
| cpu | 0.03 | 9 | ASAP | 46 | 46 | 0.00489 | 5 | 151 | 0.0289 | 0.0538 |
| cpu | 0.03 | 9 | PerQuery-CostAware | 57.4 | 57.4 | 0.00572 | 9 | 186 | 0.0289 | 0.229 |
| cpu | 0.03 | 9 | AutoSketch-Adapted | 653 | 653 | 0.00505 | 9 | 1.95e+03 | 0.00436 | 0.000876 |
| cpu | 0.1 | 9 | ASAP | 15.3 | 15.3 | 0.00686 | 4 | 55 | 0.0838 | 0.0529 |
| cpu | 0.1 | 9 | PerQuery-CostAware | 16.7 | 16.7 | 0.00629 | 9 | 61 | 0.094 | 0.23 |
| cpu | 0.1 | 9 | AutoSketch-Adapted | 653 | 653 | 0.00505 | 9 | 1.95e+03 | 0.00436 | 0.000876 |
| cpu | 0.3 | 9 | ASAP | 5.41 | 5.41 | 0.00831 | 5 | 25 | 0.296 | 0.0522 |
| cpu | 0.3 | 9 | PerQuery-CostAware | 6.85 | 6.85 | 0.00923 | 9 | 31 | 0.296 | 0.23 |
| cpu | 0.3 | 9 | AutoSketch-Adapted | 653 | 653 | 0.00505 | 9 | 1.95e+03 | 0.00436 | 0.000876 |
| cpu | 1.0 | 9 | ASAP | 1.68 | 1.68 | 0.00863 | 3 | 9 | 0.98 | 0.0522 |
| cpu | 1.0 | 9 | PerQuery-CostAware | 2.77 | 2.77 | 0.00917 | 9 | 14 | 0.98 | 0.231 |
| cpu | 1.0 | 9 | AutoSketch-Adapted | 653 | 653 | 0.00505 | 9 | 1.95e+03 | 0.00436 | 0.000876 |
| cpu | 3.0 | 21 | ASAP | 60.3 | 60.3 | 29.3 | 7 | 1.88e+03 | 2.94 | 0.143 |
| cpu | 3.0 | 21 | PerQuery-CostAware | 181 | 181 | 87.8 | 21 | 5.64e+03 | 2.94 | 0.258 |
| cpu | 3.0 | 21 | AutoSketch-Adapted | 911 | 911 | 35.3 | 21 | 7.58e+03 | 1.13 | 0.000876 |
| cpu | 10.0 | 21 | ASAP | 29.4 | 29.4 | 11.8 | 7 | 630 | 9.4 | 0.141 |
| cpu | 10.0 | 21 | PerQuery-CostAware | 87.8 | 87.8 | 35.4 | 21 | 1.88e+03 | 9.4 | 0.259 |
| cpu | 10.0 | 21 | AutoSketch-Adapted | 911 | 911 | 35.3 | 21 | 7.58e+03 | 1.13 | 0.000876 |
| cpu | inf | 21 | ASAP | 0.767 | 0.767 | 22.6 | 4 | 8 | 1.53e+03 | 0.165 |
| cpu | inf | 21 | PerQuery-CostAware | 2.51 | 2.51 | 88 | 21 | 24 | 3.84e+03 | 0.275 |
| cpu | inf | 21 | AutoSketch-Adapted | 911 | 911 | 35.3 | 21 | 7.58e+03 | 1.13 | 0.000876 |
| fargate | 0.003 | 5 | ASAP | 0.375 | 9.27 | 0.00119 | 3 | 76 | 0.00119 | 0.0488 |
| fargate | 0.003 | 5 | PerQuery-CostAware | 0.378 | 9.33 | 0.00126 | 5 | 78 | 0.00119 | 0.221 |
| fargate | 0.003 | 5 | AutoSketch-Adapted | 0.38 | 9.38 | 0.0012 | 5 | 78 | 0.00113 | 0.000876 |
| fargate | 0.01 | 9 | ASAP | 9.06 | 224 | 0.00505 | 7 | 701 | 0.00843 | 0.0531 |
| fargate | 0.01 | 9 | PerQuery-CostAware | 9.06 | 224 | 0.00512 | 9 | 703 | 0.00843 | 0.229 |
| fargate | 0.01 | 9 | AutoSketch-Adapted | 26.4 | 653 | 0.00505 | 9 | 1.95e+03 | 0.00436 | 0.000876 |
| fargate | 0.03 | 9 | ASAP | 1.86 | 46 | 0.00489 | 5 | 151 | 0.0289 | 0.0588 |
| fargate | 0.03 | 9 | PerQuery-CostAware | 2.32 | 57.4 | 0.00572 | 9 | 186 | 0.0289 | 0.23 |
| fargate | 0.03 | 9 | AutoSketch-Adapted | 26.4 | 653 | 0.00505 | 9 | 1.95e+03 | 0.00436 | 0.000876 |
| fargate | 0.1 | 9 | ASAP | 0.618 | 15.3 | 0.00686 | 4 | 55 | 0.0838 | 0.0606 |
| fargate | 0.1 | 9 | PerQuery-CostAware | 0.677 | 16.7 | 0.00629 | 9 | 61 | 0.094 | 0.231 |
| fargate | 0.1 | 9 | AutoSketch-Adapted | 26.4 | 653 | 0.00505 | 9 | 1.95e+03 | 0.00436 | 0.000876 |
| fargate | 0.3 | 9 | ASAP | 0.219 | 5.41 | 0.00831 | 5 | 25 | 0.296 | 0.0611 |
| fargate | 0.3 | 9 | PerQuery-CostAware | 0.277 | 6.85 | 0.00923 | 9 | 31 | 0.296 | 0.231 |
| fargate | 0.3 | 9 | AutoSketch-Adapted | 26.4 | 653 | 0.00505 | 9 | 1.95e+03 | 0.00436 | 0.000876 |
| fargate | 1.0 | 9 | ASAP | 0.0679 | 1.68 | 0.00863 | 3 | 9 | 0.98 | 0.0611 |
| fargate | 1.0 | 9 | PerQuery-CostAware | 0.112 | 2.77 | 0.00917 | 9 | 14 | 0.98 | 0.232 |
| fargate | 1.0 | 9 | AutoSketch-Adapted | 26.4 | 653 | 0.00505 | 9 | 1.95e+03 | 0.00436 | 0.000876 |
| fargate | 3.0 | 21 | ASAP | 2.57 | 60.3 | 29.3 | 7 | 1.88e+03 | 2.94 | 0.155 |
| fargate | 3.0 | 21 | PerQuery-CostAware | 7.71 | 181 | 87.8 | 21 | 5.64e+03 | 2.94 | 0.259 |
| fargate | 3.0 | 21 | AutoSketch-Adapted | 37 | 911 | 35.3 | 21 | 7.58e+03 | 1.13 | 0.000876 |
| fargate | 10.0 | 21 | ASAP | 1.24 | 29.4 | 11.8 | 7 | 630 | 9.4 | 0.143 |
| fargate | 10.0 | 21 | PerQuery-CostAware | 3.71 | 87.8 | 35.4 | 21 | 1.88e+03 | 9.4 | 0.26 |
| fargate | 10.0 | 21 | AutoSketch-Adapted | 37 | 911 | 35.3 | 21 | 7.58e+03 | 1.13 | 0.000876 |
| fargate | inf | 21 | ASAP | 0.072 | 0.776 | 9.11 | 4 | 6 | 1.98e+03 | 0.175 |
| fargate | inf | 21 | PerQuery-CostAware | 0.265 | 2.65 | 35.4 | 21 | 24 | 2.97e+03 | 0.278 |
| fargate | inf | 21 | AutoSketch-Adapted | 37 | 911 | 35.3 | 21 | 7.58e+03 | 1.13 | 0.000876 |

## templates=dashboard, shared=1, metrics=16, accuracy=p95

336 RQEs on 64 streams; sanity violations: 0. AutoSketch probes: 1360, benchmark time ≥ 780 s.

| weights | SLA (ms) | RQEs | method | objective | CPU (vCPU) | GiB | deployments | Σ window/slide | max latency (ms) | planning (s) |
|---|---|---|---|---|---|---|---|---|---|---|
| cpu | 0.003 | 80 | ASAP | 148 | 148 | 0.0191 | 48 | 1.22e+03 | 0.00119 | 0.576 |
| cpu | 0.003 | 80 | PerQuery-CostAware | 149 | 149 | 0.0201 | 80 | 1.25e+03 | 0.00119 | 3.37 |
| cpu | 0.003 | 80 | AutoSketch-Adapted | 150 | 150 | 0.0192 | 80 | 1.25e+03 | 0.00113 | 0.0132 |
| cpu | 0.01 | 144 | ASAP | 3.58e+03 | 3.58e+03 | 0.0809 | 112 | 1.12e+04 | 0.00843 | 0.657 |
| cpu | 0.01 | 144 | PerQuery-CostAware | 3.58e+03 | 3.58e+03 | 0.0818 | 144 | 1.12e+04 | 0.00843 | 3.5 |
| cpu | 0.01 | 144 | AutoSketch-Adapted | 1.04e+04 | 1.04e+04 | 0.0808 | 144 | 3.12e+04 | 0.00436 | 0.0132 |
| cpu | 0.03 | 144 | ASAP | 736 | 736 | 0.0782 | 80 | 2.42e+03 | 0.0289 | 0.683 |
| cpu | 0.03 | 144 | PerQuery-CostAware | 918 | 918 | 0.0915 | 144 | 2.98e+03 | 0.0289 | 3.51 |
| cpu | 0.03 | 144 | AutoSketch-Adapted | 1.04e+04 | 1.04e+04 | 0.0808 | 144 | 3.12e+04 | 0.00436 | 0.0132 |
| cpu | 0.1 | 144 | ASAP | 244 | 244 | 0.11 | 79 | 880 | 0.0838 | 0.715 |
| cpu | 0.1 | 144 | PerQuery-CostAware | 267 | 267 | 0.101 | 144 | 976 | 0.094 | 3.52 |
| cpu | 0.1 | 144 | AutoSketch-Adapted | 1.04e+04 | 1.04e+04 | 0.0808 | 144 | 3.12e+04 | 0.00436 | 0.0132 |
| cpu | 0.3 | 144 | ASAP | 86.5 | 86.5 | 0.134 | 80 | 400 | 0.296 | 0.736 |
| cpu | 0.3 | 144 | PerQuery-CostAware | 110 | 110 | 0.148 | 144 | 496 | 0.296 | 3.53 |
| cpu | 0.3 | 144 | AutoSketch-Adapted | 1.04e+04 | 1.04e+04 | 0.0808 | 144 | 3.12e+04 | 0.00436 | 0.0132 |
| cpu | 1.0 | 144 | ASAP | 26.8 | 26.8 | 0.138 | 48 | 144 | 0.98 | 0.764 |
| cpu | 1.0 | 144 | PerQuery-CostAware | 44.4 | 44.4 | 0.147 | 144 | 224 | 0.98 | 3.54 |
| cpu | 1.0 | 144 | AutoSketch-Adapted | 1.04e+04 | 1.04e+04 | 0.0808 | 144 | 3.12e+04 | 0.00436 | 0.0132 |
| cpu | 3.0 | 336 | ASAP | 965 | 965 | 468 | 112 | 3.01e+04 | 2.94 | 2.16 |
| cpu | 3.0 | 336 | PerQuery-CostAware | 2.89e+03 | 2.89e+03 | 1.4e+03 | 336 | 9.02e+04 | 2.94 | 3.96 |
| cpu | 3.0 | 336 | AutoSketch-Adapted | 1.46e+04 | 1.46e+04 | 565 | 336 | 1.21e+05 | 1.13 | 0.0132 |
| cpu | 10.0 | 336 | ASAP | 470 | 470 | 190 | 112 | 1.01e+04 | 9.4 | 2.17 |
| cpu | 10.0 | 336 | PerQuery-CostAware | 1.4e+03 | 1.4e+03 | 566 | 336 | 3.02e+04 | 9.4 | 3.94 |
| cpu | 10.0 | 336 | AutoSketch-Adapted | 1.46e+04 | 1.46e+04 | 565 | 336 | 1.21e+05 | 1.13 | 0.0132 |
| cpu | inf | 336 | ASAP | 12.3 | 12.3 | 362 | 64 | 128 | 1.53e+03 | 2.65 |
| cpu | inf | 336 | PerQuery-CostAware | 40.1 | 40.1 | 1.41e+03 | 336 | 384 | 3.84e+03 | 4.21 |
| cpu | inf | 336 | AutoSketch-Adapted | 1.46e+04 | 1.46e+04 | 565 | 336 | 1.21e+05 | 1.13 | 0.0132 |
| fargate | 0.003 | 80 | ASAP | 6.01 | 148 | 0.0191 | 48 | 1.22e+03 | 0.00119 | 0.578 |
| fargate | 0.003 | 80 | PerQuery-CostAware | 6.05 | 149 | 0.0201 | 80 | 1.25e+03 | 0.00119 | 3.37 |
| fargate | 0.003 | 80 | AutoSketch-Adapted | 6.08 | 150 | 0.0192 | 80 | 1.25e+03 | 0.00113 | 0.0132 |
| fargate | 0.01 | 144 | ASAP | 145 | 3.58e+03 | 0.0809 | 112 | 1.12e+04 | 0.00843 | 0.664 |
| fargate | 0.01 | 144 | PerQuery-CostAware | 145 | 3.58e+03 | 0.0818 | 144 | 1.12e+04 | 0.00843 | 3.51 |
| fargate | 0.01 | 144 | AutoSketch-Adapted | 423 | 1.04e+04 | 0.0808 | 144 | 3.12e+04 | 0.00436 | 0.0132 |
| fargate | 0.03 | 144 | ASAP | 29.8 | 736 | 0.0782 | 80 | 2.42e+03 | 0.0289 | 0.697 |
| fargate | 0.03 | 144 | PerQuery-CostAware | 37.2 | 918 | 0.0915 | 144 | 2.98e+03 | 0.0289 | 3.51 |
| fargate | 0.03 | 144 | AutoSketch-Adapted | 423 | 1.04e+04 | 0.0808 | 144 | 3.12e+04 | 0.00436 | 0.0132 |
| fargate | 0.1 | 144 | ASAP | 9.89 | 244 | 0.11 | 64 | 880 | 0.0838 | 0.757 |
| fargate | 0.1 | 144 | PerQuery-CostAware | 10.8 | 267 | 0.101 | 144 | 976 | 0.094 | 3.53 |
| fargate | 0.1 | 144 | AutoSketch-Adapted | 423 | 1.04e+04 | 0.0808 | 144 | 3.12e+04 | 0.00436 | 0.0132 |
| fargate | 0.3 | 144 | ASAP | 3.51 | 86.5 | 0.133 | 80 | 400 | 0.296 | 0.777 |
| fargate | 0.3 | 144 | PerQuery-CostAware | 4.44 | 110 | 0.148 | 144 | 496 | 0.296 | 3.54 |
| fargate | 0.3 | 144 | AutoSketch-Adapted | 423 | 1.04e+04 | 0.0808 | 144 | 3.12e+04 | 0.00436 | 0.0132 |
| fargate | 1.0 | 144 | ASAP | 1.09 | 26.8 | 0.138 | 48 | 144 | 0.98 | 0.82 |
| fargate | 1.0 | 144 | PerQuery-CostAware | 1.8 | 44.4 | 0.147 | 144 | 224 | 0.98 | 3.54 |
| fargate | 1.0 | 144 | AutoSketch-Adapted | 423 | 1.04e+04 | 0.0808 | 144 | 3.12e+04 | 0.00436 | 0.0132 |
| fargate | 3.0 | 336 | ASAP | 41.2 | 965 | 468 | 112 | 3.01e+04 | 2.94 | 2.21 |
| fargate | 3.0 | 336 | PerQuery-CostAware | 123 | 2.89e+03 | 1.4e+03 | 336 | 9.02e+04 | 2.94 | 3.99 |
| fargate | 3.0 | 336 | AutoSketch-Adapted | 593 | 1.46e+04 | 565 | 336 | 1.21e+05 | 1.13 | 0.0132 |
| fargate | 10.0 | 336 | ASAP | 19.9 | 470 | 190 | 112 | 1.01e+04 | 9.4 | 2.22 |
| fargate | 10.0 | 336 | PerQuery-CostAware | 59.4 | 1.4e+03 | 566 | 336 | 3.02e+04 | 9.4 | 4 |
| fargate | 10.0 | 336 | AutoSketch-Adapted | 593 | 1.46e+04 | 565 | 336 | 1.21e+05 | 1.13 | 0.0132 |
| fargate | inf | 336 | ASAP | 1.15 | 12.4 | 146 | 64 | 96 | 1.98e+03 | 2.89 |
| fargate | inf | 336 | PerQuery-CostAware | 4.24 | 42.4 | 566 | 336 | 384 | 2.97e+03 | 4.26 |
| fargate | inf | 336 | AutoSketch-Adapted | 593 | 1.46e+04 | 565 | 336 | 1.21e+05 | 1.13 | 0.0132 |

## templates=dashboard, shared=1, metrics=8, accuracy=p95

168 RQEs on 32 streams; sanity violations: 0. AutoSketch probes: 680, benchmark time ≥ 390 s.

| weights | SLA (ms) | RQEs | method | objective | CPU (vCPU) | GiB | deployments | Σ window/slide | max latency (ms) | planning (s) |
|---|---|---|---|---|---|---|---|---|---|---|
| cpu | 0.003 | 40 | ASAP | 74.2 | 74.2 | 0.00955 | 24 | 608 | 0.00119 | 0.296 |
| cpu | 0.003 | 40 | PerQuery-CostAware | 74.7 | 74.7 | 0.01 | 40 | 624 | 0.00119 | 1.73 |
| cpu | 0.003 | 40 | AutoSketch-Adapted | 75 | 75 | 0.0096 | 40 | 624 | 0.00113 | 0.00674 |
| cpu | 0.01 | 72 | ASAP | 1.79e+03 | 1.79e+03 | 0.0404 | 56 | 5.61e+03 | 0.00843 | 0.335 |
| cpu | 0.01 | 72 | PerQuery-CostAware | 1.79e+03 | 1.79e+03 | 0.0409 | 72 | 5.62e+03 | 0.00843 | 1.79 |
| cpu | 0.01 | 72 | AutoSketch-Adapted | 5.22e+03 | 5.22e+03 | 0.0404 | 72 | 1.56e+04 | 0.00436 | 0.00674 |
| cpu | 0.03 | 72 | ASAP | 368 | 368 | 0.0391 | 40 | 1.21e+03 | 0.0289 | 0.346 |
| cpu | 0.03 | 72 | PerQuery-CostAware | 459 | 459 | 0.0458 | 72 | 1.49e+03 | 0.0289 | 1.8 |
| cpu | 0.03 | 72 | AutoSketch-Adapted | 5.22e+03 | 5.22e+03 | 0.0404 | 72 | 1.56e+04 | 0.00436 | 0.00674 |
| cpu | 0.1 | 72 | ASAP | 122 | 122 | 0.0551 | 40 | 440 | 0.0838 | 0.356 |
| cpu | 0.1 | 72 | PerQuery-CostAware | 134 | 134 | 0.0504 | 72 | 488 | 0.094 | 1.81 |
| cpu | 0.1 | 72 | AutoSketch-Adapted | 5.22e+03 | 5.22e+03 | 0.0404 | 72 | 1.56e+04 | 0.00436 | 0.00674 |
| cpu | 0.3 | 72 | ASAP | 43.3 | 43.3 | 0.0664 | 40 | 200 | 0.296 | 0.371 |
| cpu | 0.3 | 72 | PerQuery-CostAware | 54.8 | 54.8 | 0.0739 | 72 | 248 | 0.296 | 1.81 |
| cpu | 0.3 | 72 | AutoSketch-Adapted | 5.22e+03 | 5.22e+03 | 0.0404 | 72 | 1.56e+04 | 0.00436 | 0.00674 |
| cpu | 1.0 | 72 | ASAP | 13.4 | 13.4 | 0.069 | 24 | 72 | 0.98 | 0.378 |
| cpu | 1.0 | 72 | PerQuery-CostAware | 22.2 | 22.2 | 0.0733 | 72 | 112 | 0.98 | 1.82 |
| cpu | 1.0 | 72 | AutoSketch-Adapted | 5.22e+03 | 5.22e+03 | 0.0404 | 72 | 1.56e+04 | 0.00436 | 0.00674 |
| cpu | 3.0 | 168 | ASAP | 483 | 483 | 234 | 56 | 1.5e+04 | 2.94 | 1.08 |
| cpu | 3.0 | 168 | PerQuery-CostAware | 1.45e+03 | 1.45e+03 | 702 | 168 | 4.51e+04 | 2.94 | 2.03 |
| cpu | 3.0 | 168 | AutoSketch-Adapted | 7.28e+03 | 7.28e+03 | 282 | 168 | 6.06e+04 | 1.13 | 0.00674 |
| cpu | 10.0 | 168 | ASAP | 235 | 235 | 94.8 | 56 | 5.04e+03 | 9.4 | 1.08 |
| cpu | 10.0 | 168 | PerQuery-CostAware | 702 | 702 | 283 | 168 | 1.51e+04 | 9.4 | 2.04 |
| cpu | 10.0 | 168 | AutoSketch-Adapted | 7.28e+03 | 7.28e+03 | 282 | 168 | 6.06e+04 | 1.13 | 0.00674 |
| cpu | inf | 168 | ASAP | 6.14 | 6.14 | 181 | 32 | 64 | 1.53e+03 | 1.28 |
| cpu | inf | 168 | PerQuery-CostAware | 20.1 | 20.1 | 704 | 168 | 192 | 3.84e+03 | 2.15 |
| cpu | inf | 168 | AutoSketch-Adapted | 7.28e+03 | 7.28e+03 | 282 | 168 | 6.06e+04 | 1.13 | 0.00674 |
| fargate | 0.003 | 40 | ASAP | 3 | 74.2 | 0.00955 | 24 | 608 | 0.00119 | 0.298 |
| fargate | 0.003 | 40 | PerQuery-CostAware | 3.02 | 74.7 | 0.01 | 40 | 624 | 0.00119 | 1.73 |
| fargate | 0.003 | 40 | AutoSketch-Adapted | 3.04 | 75 | 0.0096 | 40 | 624 | 0.00113 | 0.00674 |
| fargate | 0.01 | 72 | ASAP | 72.5 | 1.79e+03 | 0.0404 | 56 | 5.61e+03 | 0.00843 | 0.338 |
| fargate | 0.01 | 72 | PerQuery-CostAware | 72.5 | 1.79e+03 | 0.0409 | 72 | 5.62e+03 | 0.00843 | 1.8 |
| fargate | 0.01 | 72 | AutoSketch-Adapted | 211 | 5.22e+03 | 0.0404 | 72 | 1.56e+04 | 0.00436 | 0.00674 |
| fargate | 0.03 | 72 | ASAP | 14.9 | 368 | 0.0391 | 40 | 1.21e+03 | 0.0289 | 0.355 |
| fargate | 0.03 | 72 | PerQuery-CostAware | 18.6 | 459 | 0.0458 | 72 | 1.49e+03 | 0.0289 | 1.8 |
| fargate | 0.03 | 72 | AutoSketch-Adapted | 211 | 5.22e+03 | 0.0404 | 72 | 1.56e+04 | 0.00436 | 0.00674 |
| fargate | 0.1 | 72 | ASAP | 4.95 | 122 | 0.0549 | 32 | 440 | 0.0838 | 0.385 |
| fargate | 0.1 | 72 | PerQuery-CostAware | 5.41 | 134 | 0.0504 | 72 | 488 | 0.094 | 1.81 |
| fargate | 0.1 | 72 | AutoSketch-Adapted | 211 | 5.22e+03 | 0.0404 | 72 | 1.56e+04 | 0.00436 | 0.00674 |
| fargate | 0.3 | 72 | ASAP | 1.75 | 43.3 | 0.0664 | 40 | 200 | 0.296 | 0.396 |
| fargate | 0.3 | 72 | PerQuery-CostAware | 2.22 | 54.8 | 0.0739 | 72 | 248 | 0.296 | 1.82 |
| fargate | 0.3 | 72 | AutoSketch-Adapted | 211 | 5.22e+03 | 0.0404 | 72 | 1.56e+04 | 0.00436 | 0.00674 |
| fargate | 1.0 | 72 | ASAP | 0.543 | 13.4 | 0.069 | 24 | 72 | 0.98 | 0.407 |
| fargate | 1.0 | 72 | PerQuery-CostAware | 0.899 | 22.2 | 0.0733 | 72 | 112 | 0.98 | 1.82 |
| fargate | 1.0 | 72 | AutoSketch-Adapted | 211 | 5.22e+03 | 0.0404 | 72 | 1.56e+04 | 0.00436 | 0.00674 |
| fargate | 3.0 | 168 | ASAP | 20.6 | 483 | 234 | 56 | 1.5e+04 | 2.94 | 1.12 |
| fargate | 3.0 | 168 | PerQuery-CostAware | 61.7 | 1.45e+03 | 702 | 168 | 4.51e+04 | 2.94 | 2.04 |
| fargate | 3.0 | 168 | AutoSketch-Adapted | 296 | 7.28e+03 | 282 | 168 | 6.06e+04 | 1.13 | 0.00674 |
| fargate | 10.0 | 168 | ASAP | 9.95 | 235 | 94.8 | 56 | 5.04e+03 | 9.4 | 1.11 |
| fargate | 10.0 | 168 | PerQuery-CostAware | 29.7 | 702 | 283 | 168 | 1.51e+04 | 9.4 | 2.05 |
| fargate | 10.0 | 168 | AutoSketch-Adapted | 296 | 7.28e+03 | 282 | 168 | 6.06e+04 | 1.13 | 0.00674 |
| fargate | inf | 168 | ASAP | 0.576 | 6.2 | 72.9 | 32 | 48 | 1.98e+03 | 1.68 |
| fargate | inf | 168 | PerQuery-CostAware | 2.12 | 21.2 | 283 | 168 | 192 | 2.97e+03 | 2.18 |
| fargate | inf | 168 | AutoSketch-Adapted | 296 | 7.28e+03 | 282 | 168 | 6.06e+04 | 1.13 | 0.00674 |

## templates=dashboard, shared=8, metrics=1, accuracy=p95

52 RQEs on 4 streams; sanity violations: 0. AutoSketch probes: 206, benchmark time ≥ 48.8 s.

| weights | SLA (ms) | RQEs | method | objective | CPU (vCPU) | GiB | deployments | Σ window/slide | max latency (ms) | planning (s) |
|---|---|---|---|---|---|---|---|---|---|---|
| cpu | 0.003 | 8 | ASAP | 55.5 | 55.5 | 0.00685 | 3 | 451 | 0.00119 | 0.17 |
| cpu | 0.003 | 8 | PerQuery-CostAware | 59.2 | 59.2 | 0.00741 | 8 | 483 | 0.00119 | 0.622 |
| cpu | 0.003 | 8 | AutoSketch-Adapted | 59.3 | 59.3 | 0.00735 | 8 | 483 | 0.00113 | 0.00135 |
| cpu | 0.01 | 19 | ASAP | 1.34e+03 | 1.34e+03 | 0.0299 | 7 | 4.2e+03 | 0.00843 | 0.194 |
| cpu | 0.01 | 19 | PerQuery-CostAware | 1.6e+03 | 1.6e+03 | 0.035 | 19 | 4.96e+03 | 0.00843 | 0.644 |
| cpu | 0.01 | 19 | AutoSketch-Adapted | 4.67e+03 | 4.67e+03 | 0.0349 | 19 | 1.39e+04 | 0.00436 | 0.00135 |
| cpu | 0.03 | 19 | ASAP | 276 | 276 | 0.0287 | 5 | 901 | 0.0289 | 0.196 |
| cpu | 0.03 | 19 | PerQuery-CostAware | 406 | 406 | 0.0386 | 19 | 1.29e+03 | 0.0289 | 0.645 |
| cpu | 0.03 | 19 | AutoSketch-Adapted | 4.67e+03 | 4.67e+03 | 0.0349 | 19 | 1.39e+04 | 0.00436 | 0.00135 |
| cpu | 0.1 | 19 | ASAP | 88.3 | 88.3 | 0.0404 | 4 | 316 | 0.0838 | 0.201 |
| cpu | 0.1 | 19 | PerQuery-CostAware | 116 | 116 | 0.0428 | 19 | 403 | 0.094 | 0.647 |
| cpu | 0.1 | 19 | AutoSketch-Adapted | 4.67e+03 | 4.67e+03 | 0.0349 | 19 | 1.39e+04 | 0.00436 | 0.00135 |
| cpu | 0.3 | 19 | ASAP | 26.5 | 26.5 | 0.0454 | 4 | 107 | 0.296 | 0.227 |
| cpu | 0.3 | 19 | PerQuery-CostAware | 41.9 | 41.9 | 0.0636 | 19 | 167 | 0.296 | 0.649 |
| cpu | 0.3 | 19 | AutoSketch-Adapted | 4.67e+03 | 4.67e+03 | 0.0349 | 19 | 1.39e+04 | 0.00436 | 0.00135 |
| cpu | 1.0 | 19 | ASAP | 9.9 | 9.9 | 0.0505 | 3 | 49 | 0.98 | 0.273 |
| cpu | 1.0 | 19 | PerQuery-CostAware | 15.8 | 15.8 | 0.0613 | 19 | 70 | 0.98 | 0.649 |
| cpu | 1.0 | 19 | AutoSketch-Adapted | 4.67e+03 | 4.67e+03 | 0.0349 | 19 | 1.39e+04 | 0.00436 | 0.00135 |
| cpu | 3.0 | 52 | ASAP | 361 | 361 | 175 | 7 | 1.13e+04 | 2.94 | 0.691 |
| cpu | 3.0 | 52 | PerQuery-CostAware | 1.29e+03 | 1.29e+03 | 628 | 52 | 4.04e+04 | 2.94 | 0.72 |
| cpu | 3.0 | 52 | AutoSketch-Adapted | 6.52e+03 | 6.52e+03 | 253 | 52 | 5.42e+04 | 1.13 | 0.00135 |
| cpu | 10.0 | 52 | ASAP | 174 | 174 | 70.7 | 7 | 3.76e+03 | 9.4 | 0.7 |
| cpu | 10.0 | 52 | PerQuery-CostAware | 622 | 622 | 253 | 52 | 1.35e+04 | 9.4 | 0.722 |
| cpu | 10.0 | 52 | AutoSketch-Adapted | 6.52e+03 | 6.52e+03 | 253 | 52 | 5.42e+04 | 1.13 | 0.00135 |
| cpu | inf | 52 | ASAP | 2.35 | 2.35 | 136 | 4 | 33 | 1.53e+03 | 0.881 |
| cpu | inf | 52 | PerQuery-CostAware | 9.9 | 9.9 | 629 | 52 | 109 | 3.84e+03 | 0.783 |
| cpu | inf | 52 | AutoSketch-Adapted | 6.52e+03 | 6.52e+03 | 253 | 52 | 5.42e+04 | 1.13 | 0.00135 |
| fargate | 0.003 | 8 | ASAP | 2.25 | 55.5 | 0.00685 | 3 | 451 | 0.00119 | 0.176 |
| fargate | 0.003 | 8 | PerQuery-CostAware | 2.4 | 59.2 | 0.00741 | 8 | 483 | 0.00119 | 0.623 |
| fargate | 0.003 | 8 | AutoSketch-Adapted | 2.4 | 59.3 | 0.00735 | 8 | 483 | 0.00113 | 0.00135 |
| fargate | 0.01 | 19 | ASAP | 54.4 | 1.34e+03 | 0.0299 | 7 | 4.2e+03 | 0.00843 | 0.201 |
| fargate | 0.01 | 19 | PerQuery-CostAware | 64.7 | 1.6e+03 | 0.035 | 19 | 4.96e+03 | 0.00843 | 0.645 |
| fargate | 0.01 | 19 | AutoSketch-Adapted | 189 | 4.67e+03 | 0.0349 | 19 | 1.39e+04 | 0.00436 | 0.00135 |
| fargate | 0.03 | 19 | ASAP | 11.2 | 276 | 0.0287 | 5 | 901 | 0.0289 | 0.204 |
| fargate | 0.03 | 19 | PerQuery-CostAware | 16.4 | 406 | 0.0386 | 19 | 1.29e+03 | 0.0289 | 0.647 |
| fargate | 0.03 | 19 | AutoSketch-Adapted | 189 | 4.67e+03 | 0.0349 | 19 | 1.39e+04 | 0.00436 | 0.00135 |
| fargate | 0.1 | 19 | ASAP | 3.58 | 88.3 | 0.0404 | 4 | 316 | 0.0838 | 0.227 |
| fargate | 0.1 | 19 | PerQuery-CostAware | 4.7 | 116 | 0.0428 | 19 | 403 | 0.094 | 0.649 |
| fargate | 0.1 | 19 | AutoSketch-Adapted | 189 | 4.67e+03 | 0.0349 | 19 | 1.39e+04 | 0.00436 | 0.00135 |
| fargate | 0.3 | 19 | ASAP | 1.07 | 26.5 | 0.0454 | 4 | 107 | 0.296 | 0.234 |
| fargate | 0.3 | 19 | PerQuery-CostAware | 1.7 | 41.9 | 0.0636 | 19 | 167 | 0.296 | 0.651 |
| fargate | 0.3 | 19 | AutoSketch-Adapted | 189 | 4.67e+03 | 0.0349 | 19 | 1.39e+04 | 0.00436 | 0.00135 |
| fargate | 1.0 | 19 | ASAP | 0.401 | 9.9 | 0.0505 | 3 | 49 | 0.98 | 0.262 |
| fargate | 1.0 | 19 | PerQuery-CostAware | 0.642 | 15.8 | 0.0613 | 19 | 70 | 0.98 | 0.651 |
| fargate | 1.0 | 19 | AutoSketch-Adapted | 189 | 4.67e+03 | 0.0349 | 19 | 1.39e+04 | 0.00436 | 0.00135 |
| fargate | 3.0 | 52 | ASAP | 15.4 | 361 | 175 | 7 | 1.13e+04 | 2.94 | 0.719 |
| fargate | 3.0 | 52 | PerQuery-CostAware | 55 | 1.29e+03 | 628 | 52 | 4.04e+04 | 2.94 | 0.724 |
| fargate | 3.0 | 52 | AutoSketch-Adapted | 265 | 6.52e+03 | 253 | 52 | 5.42e+04 | 1.13 | 0.00135 |
| fargate | 10.0 | 52 | ASAP | 7.35 | 174 | 70.7 | 7 | 3.76e+03 | 9.4 | 0.758 |
| fargate | 10.0 | 52 | PerQuery-CostAware | 26.3 | 622 | 253 | 52 | 1.35e+04 | 9.4 | 0.727 |
| fargate | 10.0 | 52 | AutoSketch-Adapted | 265 | 6.52e+03 | 253 | 52 | 5.42e+04 | 1.13 | 0.00135 |
| fargate | inf | 52 | ASAP | 0.342 | 2.44 | 54.8 | 4 | 21 | 1.98e+03 | 1.32 |
| fargate | inf | 52 | PerQuery-CostAware | 1.55 | 10.4 | 253 | 52 | 91 | 3.96e+03 | 0.793 |
| fargate | inf | 52 | AutoSketch-Adapted | 265 | 6.52e+03 | 253 | 52 | 5.42e+04 | 1.13 | 0.00135 |
