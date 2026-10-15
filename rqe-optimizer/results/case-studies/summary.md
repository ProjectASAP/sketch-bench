## synthetic/case/anomaly/base=0/tp95

| SLA (ms) | asap vCPU (MVs) | perquery vCPU (MVs) | autosketch vCPU (MVs) | ASAP latency ms |
|---|---|---|---|---|
| none | 0.114 (3) | 0.226 (6) | 1.96 (6) | 3.31 |
| 0.3 | 0.377 (6) | 0.377 (6) | 1.96 (6) | 0.297 |
| 0.5 | 0.339 (4) | 0.377 (6) | 1.96 (6) | 0.412 |
| 1.0 | 0.301 (4) | 0.377 (6) | 1.96 (6) | 0.825 |
| 2.0 | 0.151 (4) | 0.226 (6) | 1.96 (6) | 1.11 |
| 3.0 | 0.151 (4) | 0.226 (6) | 1.96 (6) | 1.11 |

ASAP plan at SLA none:
- dd(alpha=0.1) by (service,region,status) window 15s slide 15s: p99_by_service[5m] (merges 20), p99_by_region[5m] (merges 20), p99_by_service,region[5m] (merges 20), p99_by_service,region,status[5m] (merges 20)
- dd(alpha=0.1) by (service,endpoint) window 60s slide 60s: p99_by_service,endpoint[5m] (merges 5)
- dd(alpha=0.1) by (region,endpoint) window 300s slide 300s: p99_by_region,endpoint[5m] (merges 1)

ASAP plan at SLA 0.3:
- dd(alpha=0.1) by (service) window 15s slide 15s: p99_by_service[5m] (merges 20)
- dd(alpha=0.1) by (region) window 15s slide 15s: p99_by_region[5m] (merges 20)
- dd(alpha=0.1) by (service,region) window 60s slide 60s: p99_by_service,region[5m] (merges 5)
- dd(alpha=0.1) by (service,endpoint) window 300s slide 60s: p99_by_service,endpoint[5m] (merges 1)
- dd(alpha=0.1) by (service,region,status) window 300s slide 300s: p99_by_service,region,status[5m] (merges 1)
- dd(alpha=0.1) by (region,endpoint) window 300s slide 300s: p99_by_region,endpoint[5m] (merges 1)

ASAP plan at SLA 0.5:
- dd(alpha=0.1) by (service,region) window 30s slide 15s: p99_by_service[5m] (merges 10), p99_by_region[5m] (merges 10), p99_by_service,region[5m] (merges 10)
- dd(alpha=0.1) by (service,endpoint) window 300s slide 60s: p99_by_service,endpoint[5m] (merges 1)
- dd(alpha=0.1) by (service,region,status) window 300s slide 300s: p99_by_service,region,status[5m] (merges 1)
- dd(alpha=0.1) by (region,endpoint) window 300s slide 300s: p99_by_region,endpoint[5m] (merges 1)

ASAP plan at SLA 1.0:
- dd(alpha=0.1) by (service,region) window 15s slide 15s: p99_by_service[5m] (merges 20), p99_by_region[5m] (merges 20), p99_by_service,region[5m] (merges 20)
- dd(alpha=0.1) by (service,endpoint) window 300s slide 60s: p99_by_service,endpoint[5m] (merges 1)
- dd(alpha=0.1) by (service,region,status) window 300s slide 300s: p99_by_service,region,status[5m] (merges 1)
- dd(alpha=0.1) by (region,endpoint) window 300s slide 300s: p99_by_region,endpoint[5m] (merges 1)

ASAP plan at SLA 2.0:
- dd(alpha=0.1) by (service,region) window 15s slide 15s: p99_by_service[5m] (merges 20), p99_by_region[5m] (merges 20), p99_by_service,region[5m] (merges 20)
- dd(alpha=0.1) by (service,endpoint) window 60s slide 60s: p99_by_service,endpoint[5m] (merges 5)
- dd(alpha=0.1) by (service,region,status) window 300s slide 300s: p99_by_service,region,status[5m] (merges 1)
- dd(alpha=0.1) by (region,endpoint) window 300s slide 300s: p99_by_region,endpoint[5m] (merges 1)

## synthetic/case/anomaly/drilldown_endpoints=100/tp95

| SLA (ms) | asap vCPU (MVs) | perquery vCPU (MVs) | autosketch vCPU (MVs) | ASAP latency ms |
|---|---|---|---|---|
| none | 0.0776 (2) | 0.264 (7) | 2 (7) | 82.5 |
| 3.0 | infeasible | infeasible | 2 (7) |  |
| 10.0 | 0.151 (4) | 0.264 (7) | 2 (7) | 4.75 |
| 30.0 | 0.113 (3) | 0.264 (7) | 2 (7) | 16.4 |
| 100.0 | 0.0776 (2) | 0.264 (7) | 2 (7) | 82.5 |

ASAP plan at SLA none:
- dd(alpha=0.1) by (service,region) window 15s slide 15s: p99_by_service[5m] (merges 20), p99_by_region[5m] (merges 20), p99_by_service,region[5m] (merges 20)
- dd(alpha=0.1) by (region,service,endpoint,status) window 60s slide 60s: p99_by_service,endpoint[5m] (merges 5), p99_by_service,region,status[5m] (merges 5), p99_by_region,endpoint[5m] (merges 5), p99_by_region,service,endpoint,status[5m] (merges 5)

ASAP plan at SLA 10.0:
- dd(alpha=0.1) by (service,region,status) window 15s slide 15s: p99_by_service[5m] (merges 20), p99_by_region[5m] (merges 20), p99_by_service,region[5m] (merges 20), p99_by_service,region,status[5m] (merges 20)
- dd(alpha=0.1) by (service,endpoint) window 60s slide 60s: p99_by_service,endpoint[5m] (merges 5)
- dd(alpha=0.1) by (region,endpoint) window 300s slide 300s: p99_by_region,endpoint[5m] (merges 1)
- dd(alpha=0.1) by (region,service,endpoint,status) window 300s slide 300s: p99_by_region,service,endpoint,status[5m] (merges 1)

ASAP plan at SLA 30.0:
- dd(alpha=0.1) by (service,region) window 15s slide 15s: p99_by_service[5m] (merges 20), p99_by_region[5m] (merges 20), p99_by_service,region[5m] (merges 20)
- dd(alpha=0.1) by (service,endpoint) window 60s slide 60s: p99_by_service,endpoint[5m] (merges 5)
- dd(alpha=0.1) by (region,service,endpoint,status) window 300s slide 300s: p99_by_service,region,status[5m] (merges 1), p99_by_region,endpoint[5m] (merges 1), p99_by_region,service,endpoint,status[5m] (merges 1)

## synthetic/case/anomaly/drilldown_endpoints=10/tp95

| SLA (ms) | asap vCPU (MVs) | perquery vCPU (MVs) | autosketch vCPU (MVs) | ASAP latency ms |
|---|---|---|---|---|
| none | 0.0435 (1) | 0.264 (7) | 2 (7) | 33.1 |
| 3.0 | 0.113 (3) | 0.264 (7) | 2 (7) | 1.54 |
| 10.0 | 0.0757 (2) | 0.264 (7) | 2 (7) | 8.19 |
| 30.0 | 0.0757 (2) | 0.264 (7) | 2 (7) | 8.19 |
| 100.0 | 0.0435 (1) | 0.264 (7) | 2 (7) | 33.1 |

ASAP plan at SLA none:
- dd(alpha=0.1) by (region,service,endpoint,status) window 15s slide 15s: p99_by_service[5m] (merges 20), p99_by_region[5m] (merges 20), p99_by_service,region[5m] (merges 20), p99_by_service,endpoint[5m] (merges 20), p99_by_service,region,status[5m] (merges 20), p99_by_region,endpoint[5m] (merges 20), p99_by_region,service,endpoint,status[5m] (merges 20)

ASAP plan at SLA 3.0:
- dd(alpha=0.1) by (service,region) window 15s slide 15s: p99_by_service[5m] (merges 20), p99_by_region[5m] (merges 20), p99_by_service,region[5m] (merges 20)
- dd(alpha=0.1) by (service,endpoint) window 60s slide 60s: p99_by_service,endpoint[5m] (merges 5)
- dd(alpha=0.1) by (region,service,endpoint,status) window 300s slide 300s: p99_by_service,region,status[5m] (merges 1), p99_by_region,endpoint[5m] (merges 1), p99_by_region,service,endpoint,status[5m] (merges 1)

ASAP plan at SLA 10.0:
- dd(alpha=0.1) by (service,region) window 15s slide 15s: p99_by_service[5m] (merges 20), p99_by_region[5m] (merges 20), p99_by_service,region[5m] (merges 20)
- dd(alpha=0.1) by (region,service,endpoint,status) window 60s slide 60s: p99_by_service,endpoint[5m] (merges 5), p99_by_service,region,status[5m] (merges 5), p99_by_region,endpoint[5m] (merges 5), p99_by_region,service,endpoint,status[5m] (merges 5)

## synthetic/case/anomaly/drilldown_endpoints=25/tp95

| SLA (ms) | asap vCPU (MVs) | perquery vCPU (MVs) | autosketch vCPU (MVs) | ASAP latency ms |
|---|---|---|---|---|
| none | 0.0522 (1) | 0.264 (7) | 2 (7) | 82.7 |
| 3.0 | 0.188 (5) | 0.264 (7) | 2 (7) | 1.19 |
| 10.0 | 0.113 (3) | 0.264 (7) | 2 (7) | 4.02 |
| 30.0 | 0.076 (2) | 0.264 (7) | 2 (7) | 20.5 |
| 100.0 | 0.0522 (1) | 0.264 (7) | 2 (7) | 82.7 |

ASAP plan at SLA none:
- dd(alpha=0.1) by (region,service,endpoint,status) window 15s slide 15s: p99_by_service[5m] (merges 20), p99_by_region[5m] (merges 20), p99_by_service,region[5m] (merges 20), p99_by_service,endpoint[5m] (merges 20), p99_by_service,region,status[5m] (merges 20), p99_by_region,endpoint[5m] (merges 20), p99_by_region,service,endpoint,status[5m] (merges 20)

ASAP plan at SLA 3.0:
- dd(alpha=0.1) by (service,region) window 15s slide 15s: p99_by_service[5m] (merges 20), p99_by_region[5m] (merges 20), p99_by_service,region[5m] (merges 20)
- dd(alpha=0.1) by (service,endpoint) window 60s slide 60s: p99_by_service,endpoint[5m] (merges 5)
- dd(alpha=0.1) by (service,region,status) window 300s slide 300s: p99_by_service,region,status[5m] (merges 1)
- dd(alpha=0.1) by (region,endpoint) window 300s slide 300s: p99_by_region,endpoint[5m] (merges 1)
- dd(alpha=0.1) by (region,service,endpoint,status) window 300s slide 300s: p99_by_region,service,endpoint,status[5m] (merges 1)

ASAP plan at SLA 10.0:
- dd(alpha=0.1) by (service,region) window 15s slide 15s: p99_by_service[5m] (merges 20), p99_by_region[5m] (merges 20), p99_by_service,region[5m] (merges 20)
- dd(alpha=0.1) by (service,endpoint) window 60s slide 60s: p99_by_service,endpoint[5m] (merges 5)
- dd(alpha=0.1) by (region,service,endpoint,status) window 300s slide 300s: p99_by_service,region,status[5m] (merges 1), p99_by_region,endpoint[5m] (merges 1), p99_by_region,service,endpoint,status[5m] (merges 1)

ASAP plan at SLA 30.0:
- dd(alpha=0.1) by (service,region) window 15s slide 15s: p99_by_service[5m] (merges 20), p99_by_region[5m] (merges 20), p99_by_service,region[5m] (merges 20)
- dd(alpha=0.1) by (region,service,endpoint,status) window 60s slide 60s: p99_by_service,endpoint[5m] (merges 5), p99_by_service,region,status[5m] (merges 5), p99_by_region,endpoint[5m] (merges 5), p99_by_region,service,endpoint,status[5m] (merges 5)

## synthetic/case/anomaly/drilldown_endpoints=2/tp95

| SLA (ms) | asap vCPU (MVs) | perquery vCPU (MVs) | autosketch vCPU (MVs) | ASAP latency ms |
|---|---|---|---|---|
| none | 0.0388 (1) | 0.264 (7) | 2 (7) | 6.61 |
| 3.0 | 0.0755 (2) | 0.264 (7) | 2 (7) | 1.64 |
| 10.0 | 0.0388 (1) | 0.264 (7) | 2 (7) | 6.61 |
| 30.0 | 0.0388 (1) | 0.264 (7) | 2 (7) | 6.61 |
| 100.0 | 0.0388 (1) | 0.264 (7) | 2 (7) | 6.61 |

ASAP plan at SLA none:
- dd(alpha=0.1) by (region,service,endpoint,status) window 15s slide 15s: p99_by_service[5m] (merges 20), p99_by_region[5m] (merges 20), p99_by_service,region[5m] (merges 20), p99_by_service,endpoint[5m] (merges 20), p99_by_service,region,status[5m] (merges 20), p99_by_region,endpoint[5m] (merges 20), p99_by_region,service,endpoint,status[5m] (merges 20)

ASAP plan at SLA 3.0:
- dd(alpha=0.1) by (service,region) window 15s slide 15s: p99_by_service[5m] (merges 20), p99_by_region[5m] (merges 20), p99_by_service,region[5m] (merges 20)
- dd(alpha=0.1) by (region,service,endpoint,status) window 60s slide 60s: p99_by_service,endpoint[5m] (merges 5), p99_by_service,region,status[5m] (merges 5), p99_by_region,endpoint[5m] (merges 5), p99_by_region,service,endpoint,status[5m] (merges 5)

## synthetic/case/anomaly/drilldown_endpoints=50/tp95

| SLA (ms) | asap vCPU (MVs) | perquery vCPU (MVs) | autosketch vCPU (MVs) | ASAP latency ms |
|---|---|---|---|---|
| none | 0.0668 (1) | 0.264 (7) | 2 (7) | 165 |
| 3.0 | 0.188 (5) | 0.264 (7) | 2 (7) | 2.38 |
| 10.0 | 0.113 (3) | 0.264 (7) | 2 (7) | 8.15 |
| 30.0 | 0.113 (3) | 0.264 (7) | 2 (7) | 8.15 |
| 100.0 | 0.0765 (2) | 0.264 (7) | 2 (7) | 41.2 |

ASAP plan at SLA none:
- dd(alpha=0.1) by (region,service,endpoint,status) window 15s slide 15s: p99_by_service[5m] (merges 20), p99_by_region[5m] (merges 20), p99_by_service,region[5m] (merges 20), p99_by_service,endpoint[5m] (merges 20), p99_by_service,region,status[5m] (merges 20), p99_by_region,endpoint[5m] (merges 20), p99_by_region,service,endpoint,status[5m] (merges 20)

ASAP plan at SLA 3.0:
- dd(alpha=0.1) by (service,region) window 15s slide 15s: p99_by_service[5m] (merges 20), p99_by_region[5m] (merges 20), p99_by_service,region[5m] (merges 20)
- dd(alpha=0.1) by (service,endpoint) window 60s slide 60s: p99_by_service,endpoint[5m] (merges 5)
- dd(alpha=0.1) by (service,region,status) window 300s slide 300s: p99_by_service,region,status[5m] (merges 1)
- dd(alpha=0.1) by (region,endpoint) window 300s slide 300s: p99_by_region,endpoint[5m] (merges 1)
- dd(alpha=0.1) by (region,service,endpoint,status) window 300s slide 300s: p99_by_region,service,endpoint,status[5m] (merges 1)

ASAP plan at SLA 10.0:
- dd(alpha=0.1) by (service,region) window 15s slide 15s: p99_by_service[5m] (merges 20), p99_by_region[5m] (merges 20), p99_by_service,region[5m] (merges 20)
- dd(alpha=0.1) by (service,endpoint) window 60s slide 60s: p99_by_service,endpoint[5m] (merges 5)
- dd(alpha=0.1) by (region,service,endpoint,status) window 300s slide 300s: p99_by_service,region,status[5m] (merges 1), p99_by_region,endpoint[5m] (merges 1), p99_by_region,service,endpoint,status[5m] (merges 1)

ASAP plan at SLA 100.0:
- dd(alpha=0.1) by (service,region) window 15s slide 15s: p99_by_service[5m] (merges 20), p99_by_region[5m] (merges 20), p99_by_service,region[5m] (merges 20)
- dd(alpha=0.1) by (region,service,endpoint,status) window 60s slide 60s: p99_by_service,endpoint[5m] (merges 5), p99_by_service,region,status[5m] (merges 5), p99_by_region,endpoint[5m] (merges 5), p99_by_region,service,endpoint,status[5m] (merges 5)

## synthetic/case/anomaly/drilldown_endpoints=5/tp95

| SLA (ms) | asap vCPU (MVs) | perquery vCPU (MVs) | autosketch vCPU (MVs) | ASAP latency ms |
|---|---|---|---|---|
| none | 0.0406 (1) | 0.264 (7) | 2 (7) | 16.5 |
| 3.0 | 0.113 (3) | 0.264 (7) | 2 (7) | 0.825 |
| 10.0 | 0.0756 (2) | 0.264 (7) | 2 (7) | 4.1 |
| 30.0 | 0.0406 (1) | 0.264 (7) | 2 (7) | 16.5 |
| 100.0 | 0.0406 (1) | 0.264 (7) | 2 (7) | 16.5 |

ASAP plan at SLA none:
- dd(alpha=0.1) by (region,service,endpoint,status) window 15s slide 15s: p99_by_service[5m] (merges 20), p99_by_region[5m] (merges 20), p99_by_service,region[5m] (merges 20), p99_by_service,endpoint[5m] (merges 20), p99_by_service,region,status[5m] (merges 20), p99_by_region,endpoint[5m] (merges 20), p99_by_region,service,endpoint,status[5m] (merges 20)

ASAP plan at SLA 3.0:
- dd(alpha=0.1) by (service,region) window 15s slide 15s: p99_by_service[5m] (merges 20), p99_by_region[5m] (merges 20), p99_by_service,region[5m] (merges 20)
- dd(alpha=0.1) by (service,endpoint) window 60s slide 60s: p99_by_service,endpoint[5m] (merges 5)
- dd(alpha=0.1) by (region,service,endpoint,status) window 300s slide 300s: p99_by_service,region,status[5m] (merges 1), p99_by_region,endpoint[5m] (merges 1), p99_by_region,service,endpoint,status[5m] (merges 1)

ASAP plan at SLA 10.0:
- dd(alpha=0.1) by (service,region) window 15s slide 15s: p99_by_service[5m] (merges 20), p99_by_region[5m] (merges 20), p99_by_service,region[5m] (merges 20)
- dd(alpha=0.1) by (region,service,endpoint,status) window 60s slide 60s: p99_by_service,endpoint[5m] (merges 5), p99_by_service,region,status[5m] (merges 5), p99_by_region,endpoint[5m] (merges 5), p99_by_region,service,endpoint,status[5m] (merges 5)

## synthetic/case/anomaly/endpoints=100/tp95

| SLA (ms) | asap vCPU (MVs) | perquery vCPU (MVs) | autosketch vCPU (MVs) | ASAP latency ms |
|---|---|---|---|---|
| none | 0.114 (3) | 0.226 (6) | 1.96 (6) | 4.43 |
| 0.3 | infeasible | infeasible | 1.96 (6) |  |
| 0.5 | infeasible | infeasible | 1.96 (6) |  |
| 1.0 | infeasible | infeasible | 1.96 (6) |  |
| 2.0 | 0.301 (4) | 0.377 (6) | 1.96 (6) | 1.19 |
| 3.0 | 0.301 (4) | 0.377 (6) | 1.96 (6) | 1.19 |

ASAP plan at SLA none:
- dd(alpha=0.1) by (service,region,status) window 15s slide 15s: p99_by_service[5m] (merges 20), p99_by_region[5m] (merges 20), p99_by_service,region[5m] (merges 20), p99_by_service,region,status[5m] (merges 20)
- dd(alpha=0.1) by (service,endpoint) window 60s slide 60s: p99_by_service,endpoint[5m] (merges 5)
- dd(alpha=0.1) by (region,endpoint) window 300s slide 300s: p99_by_region,endpoint[5m] (merges 1)

ASAP plan at SLA 2.0:
- dd(alpha=0.1) by (service,region) window 15s slide 15s: p99_by_service[5m] (merges 20), p99_by_region[5m] (merges 20), p99_by_service,region[5m] (merges 20)
- dd(alpha=0.1) by (service,endpoint) window 300s slide 60s: p99_by_service,endpoint[5m] (merges 1)
- dd(alpha=0.1) by (service,region,status) window 300s slide 300s: p99_by_service,region,status[5m] (merges 1)
- dd(alpha=0.1) by (region,endpoint) window 300s slide 300s: p99_by_region,endpoint[5m] (merges 1)

## synthetic/case/anomaly/endpoints=10/tp95

| SLA (ms) | asap vCPU (MVs) | perquery vCPU (MVs) | autosketch vCPU (MVs) | ASAP latency ms |
|---|---|---|---|---|
| none | 0.113 (3) | 0.226 (6) | 1.96 (6) | 3.31 |
| 0.3 | 0.377 (6) | 0.377 (6) | 1.96 (6) | 0.199 |
| 0.5 | 0.188 (4) | 0.226 (6) | 1.96 (6) | 0.443 |
| 1.0 | 0.151 (4) | 0.226 (6) | 1.96 (6) | 0.825 |
| 2.0 | 0.151 (4) | 0.226 (6) | 1.96 (6) | 0.825 |
| 3.0 | 0.151 (4) | 0.226 (6) | 1.96 (6) | 0.825 |

ASAP plan at SLA none:
- dd(alpha=0.1) by (service,endpoint) window 15s slide 15s: p99_by_service[5m] (merges 20), p99_by_service,endpoint[5m] (merges 20)
- dd(alpha=0.1) by (service,region,status) window 15s slide 15s: p99_by_region[5m] (merges 20), p99_by_service,region[5m] (merges 20), p99_by_service,region,status[5m] (merges 20)
- dd(alpha=0.1) by (region,endpoint) window 300s slide 300s: p99_by_region,endpoint[5m] (merges 1)

ASAP plan at SLA 0.3:
- dd(alpha=0.1) by (service) window 15s slide 15s: p99_by_service[5m] (merges 20)
- dd(alpha=0.1) by (region) window 15s slide 15s: p99_by_region[5m] (merges 20)
- dd(alpha=0.1) by (service,region) window 60s slide 60s: p99_by_service,region[5m] (merges 5)
- dd(alpha=0.1) by (service,endpoint) window 300s slide 60s: p99_by_service,endpoint[5m] (merges 1)
- dd(alpha=0.1) by (service,region,status) window 300s slide 300s: p99_by_service,region,status[5m] (merges 1)
- dd(alpha=0.1) by (region,endpoint) window 300s slide 300s: p99_by_region,endpoint[5m] (merges 1)

ASAP plan at SLA 0.5:
- dd(alpha=0.1) by (service,region) window 30s slide 15s: p99_by_service[5m] (merges 10), p99_by_region[5m] (merges 10), p99_by_service,region[5m] (merges 10)
- dd(alpha=0.1) by (service,endpoint) window 60s slide 60s: p99_by_service,endpoint[5m] (merges 5)
- dd(alpha=0.1) by (service,region,status) window 300s slide 300s: p99_by_service,region,status[5m] (merges 1)
- dd(alpha=0.1) by (region,endpoint) window 300s slide 300s: p99_by_region,endpoint[5m] (merges 1)

ASAP plan at SLA 1.0:
- dd(alpha=0.1) by (service,region) window 15s slide 15s: p99_by_service[5m] (merges 20), p99_by_region[5m] (merges 20), p99_by_service,region[5m] (merges 20)
- dd(alpha=0.1) by (service,endpoint) window 60s slide 60s: p99_by_service,endpoint[5m] (merges 5)
- dd(alpha=0.1) by (service,region,status) window 300s slide 300s: p99_by_service,region,status[5m] (merges 1)
- dd(alpha=0.1) by (region,endpoint) window 300s slide 300s: p99_by_region,endpoint[5m] (merges 1)

## synthetic/case/anomaly/endpoints=25/tp95

| SLA (ms) | asap vCPU (MVs) | perquery vCPU (MVs) | autosketch vCPU (MVs) | ASAP latency ms |
|---|---|---|---|---|
| none | 0.114 (3) | 0.226 (6) | 1.96 (6) | 3.31 |
| 0.3 | 0.377 (6) | 0.377 (6) | 1.96 (6) | 0.297 |
| 0.5 | 0.339 (4) | 0.377 (6) | 1.96 (6) | 0.412 |
| 1.0 | 0.301 (4) | 0.377 (6) | 1.96 (6) | 0.825 |
| 2.0 | 0.151 (4) | 0.226 (6) | 1.96 (6) | 1.11 |
| 3.0 | 0.151 (4) | 0.226 (6) | 1.96 (6) | 1.11 |

ASAP plan at SLA none:
- dd(alpha=0.1) by (service,region,status) window 15s slide 15s: p99_by_service[5m] (merges 20), p99_by_region[5m] (merges 20), p99_by_service,region[5m] (merges 20), p99_by_service,region,status[5m] (merges 20)
- dd(alpha=0.1) by (service,endpoint) window 60s slide 60s: p99_by_service,endpoint[5m] (merges 5)
- dd(alpha=0.1) by (region,endpoint) window 300s slide 300s: p99_by_region,endpoint[5m] (merges 1)

ASAP plan at SLA 0.3:
- dd(alpha=0.1) by (service) window 15s slide 15s: p99_by_service[5m] (merges 20)
- dd(alpha=0.1) by (region) window 15s slide 15s: p99_by_region[5m] (merges 20)
- dd(alpha=0.1) by (service,region) window 60s slide 60s: p99_by_service,region[5m] (merges 5)
- dd(alpha=0.1) by (service,endpoint) window 300s slide 60s: p99_by_service,endpoint[5m] (merges 1)
- dd(alpha=0.1) by (service,region,status) window 300s slide 300s: p99_by_service,region,status[5m] (merges 1)
- dd(alpha=0.1) by (region,endpoint) window 300s slide 300s: p99_by_region,endpoint[5m] (merges 1)

ASAP plan at SLA 0.5:
- dd(alpha=0.1) by (service,region) window 30s slide 15s: p99_by_service[5m] (merges 10), p99_by_region[5m] (merges 10), p99_by_service,region[5m] (merges 10)
- dd(alpha=0.1) by (service,endpoint) window 300s slide 60s: p99_by_service,endpoint[5m] (merges 1)
- dd(alpha=0.1) by (service,region,status) window 300s slide 300s: p99_by_service,region,status[5m] (merges 1)
- dd(alpha=0.1) by (region,endpoint) window 300s slide 300s: p99_by_region,endpoint[5m] (merges 1)

ASAP plan at SLA 1.0:
- dd(alpha=0.1) by (service,region) window 15s slide 15s: p99_by_service[5m] (merges 20), p99_by_region[5m] (merges 20), p99_by_service,region[5m] (merges 20)
- dd(alpha=0.1) by (service,endpoint) window 300s slide 60s: p99_by_service,endpoint[5m] (merges 1)
- dd(alpha=0.1) by (service,region,status) window 300s slide 300s: p99_by_service,region,status[5m] (merges 1)
- dd(alpha=0.1) by (region,endpoint) window 300s slide 300s: p99_by_region,endpoint[5m] (merges 1)

ASAP plan at SLA 2.0:
- dd(alpha=0.1) by (service,region) window 15s slide 15s: p99_by_service[5m] (merges 20), p99_by_region[5m] (merges 20), p99_by_service,region[5m] (merges 20)
- dd(alpha=0.1) by (service,endpoint) window 60s slide 60s: p99_by_service,endpoint[5m] (merges 5)
- dd(alpha=0.1) by (service,region,status) window 300s slide 300s: p99_by_service,region,status[5m] (merges 1)
- dd(alpha=0.1) by (region,endpoint) window 300s slide 300s: p99_by_region,endpoint[5m] (merges 1)

## synthetic/case/anomaly/endpoints=2/tp95

| SLA (ms) | asap vCPU (MVs) | perquery vCPU (MVs) | autosketch vCPU (MVs) | ASAP latency ms |
|---|---|---|---|---|
| none | 0.113 (3) | 0.226 (6) | 1.96 (6) | 1.65 |
| 0.3 | 0.226 (6) | 0.226 (6) | 1.96 (6) | 0.199 |
| 0.5 | 0.188 (5) | 0.226 (6) | 1.96 (6) | 0.406 |
| 1.0 | 0.151 (4) | 0.226 (6) | 1.96 (6) | 0.797 |
| 2.0 | 0.113 (3) | 0.226 (6) | 1.96 (6) | 1.65 |
| 3.0 | 0.113 (3) | 0.226 (6) | 1.96 (6) | 1.65 |

ASAP plan at SLA none:
- dd(alpha=0.1) by (service,endpoint) window 15s slide 15s: p99_by_service[5m] (merges 20), p99_by_service,endpoint[5m] (merges 20)
- dd(alpha=0.1) by (region,endpoint) window 15s slide 15s: p99_by_region[5m] (merges 20), p99_by_region,endpoint[5m] (merges 20)
- dd(alpha=0.1) by (service,region,status) window 60s slide 60s: p99_by_service,region[5m] (merges 5), p99_by_service,region,status[5m] (merges 5)

ASAP plan at SLA 0.3:
- dd(alpha=0.1) by (service) window 15s slide 15s: p99_by_service[5m] (merges 20)
- dd(alpha=0.1) by (region) window 15s slide 15s: p99_by_region[5m] (merges 20)
- dd(alpha=0.1) by (service,region) window 60s slide 60s: p99_by_service,region[5m] (merges 5)
- dd(alpha=0.1) by (service,endpoint) window 60s slide 60s: p99_by_service,endpoint[5m] (merges 5)
- dd(alpha=0.1) by (service,region,status) window 300s slide 300s: p99_by_service,region,status[5m] (merges 1)
- dd(alpha=0.1) by (region,endpoint) window 300s slide 300s: p99_by_region,endpoint[5m] (merges 1)

ASAP plan at SLA 0.5:
- dd(alpha=0.1) by (service,endpoint) window 15s slide 15s: p99_by_service[5m] (merges 20), p99_by_service,endpoint[5m] (merges 20)
- dd(alpha=0.1) by (region) window 15s slide 15s: p99_by_region[5m] (merges 20)
- dd(alpha=0.1) by (service,region) window 60s slide 60s: p99_by_service,region[5m] (merges 5)
- dd(alpha=0.1) by (service,region,status) window 300s slide 300s: p99_by_service,region,status[5m] (merges 1)
- dd(alpha=0.1) by (region,endpoint) window 300s slide 300s: p99_by_region,endpoint[5m] (merges 1)

ASAP plan at SLA 1.0:
- dd(alpha=0.1) by (service,endpoint) window 15s slide 15s: p99_by_service[5m] (merges 20), p99_by_service,endpoint[5m] (merges 20)
- dd(alpha=0.1) by (region) window 15s slide 15s: p99_by_region[5m] (merges 20)
- dd(alpha=0.1) by (service,region,status) window 60s slide 60s: p99_by_service,region[5m] (merges 5), p99_by_service,region,status[5m] (merges 5)
- dd(alpha=0.1) by (region,endpoint) window 300s slide 300s: p99_by_region,endpoint[5m] (merges 1)

## synthetic/case/anomaly/endpoints=50/tp95

| SLA (ms) | asap vCPU (MVs) | perquery vCPU (MVs) | autosketch vCPU (MVs) | ASAP latency ms |
|---|---|---|---|---|
| none | 0.114 (3) | 0.226 (6) | 1.96 (6) | 3.31 |
| 0.3 | infeasible | infeasible | 1.96 (6) |  |
| 0.5 | infeasible | infeasible | 1.96 (6) |  |
| 1.0 | 0.301 (4) | 0.377 (6) | 1.96 (6) | 0.825 |
| 2.0 | 0.301 (4) | 0.377 (6) | 1.96 (6) | 0.825 |
| 3.0 | 0.151 (4) | 0.226 (6) | 1.96 (6) | 2.22 |

ASAP plan at SLA none:
- dd(alpha=0.1) by (service,region,status) window 15s slide 15s: p99_by_service[5m] (merges 20), p99_by_region[5m] (merges 20), p99_by_service,region[5m] (merges 20), p99_by_service,region,status[5m] (merges 20)
- dd(alpha=0.1) by (service,endpoint) window 60s slide 60s: p99_by_service,endpoint[5m] (merges 5)
- dd(alpha=0.1) by (region,endpoint) window 300s slide 300s: p99_by_region,endpoint[5m] (merges 1)

ASAP plan at SLA 1.0:
- dd(alpha=0.1) by (service,region) window 15s slide 15s: p99_by_service[5m] (merges 20), p99_by_region[5m] (merges 20), p99_by_service,region[5m] (merges 20)
- dd(alpha=0.1) by (service,endpoint) window 300s slide 60s: p99_by_service,endpoint[5m] (merges 1)
- dd(alpha=0.1) by (service,region,status) window 300s slide 300s: p99_by_service,region,status[5m] (merges 1)
- dd(alpha=0.1) by (region,endpoint) window 300s slide 300s: p99_by_region,endpoint[5m] (merges 1)

ASAP plan at SLA 3.0:
- dd(alpha=0.1) by (service,region) window 15s slide 15s: p99_by_service[5m] (merges 20), p99_by_region[5m] (merges 20), p99_by_service,region[5m] (merges 20)
- dd(alpha=0.1) by (service,endpoint) window 60s slide 60s: p99_by_service,endpoint[5m] (merges 5)
- dd(alpha=0.1) by (service,region,status) window 300s slide 300s: p99_by_service,region,status[5m] (merges 1)
- dd(alpha=0.1) by (region,endpoint) window 300s slide 300s: p99_by_region,endpoint[5m] (merges 1)

## synthetic/case/anomaly/endpoints=5/tp95

| SLA (ms) | asap vCPU (MVs) | perquery vCPU (MVs) | autosketch vCPU (MVs) | ASAP latency ms |
|---|---|---|---|---|
| none | 0.113 (3) | 0.226 (6) | 1.96 (6) | 3.31 |
| 0.3 | 0.226 (6) | 0.226 (6) | 1.96 (6) | 0.222 |
| 0.5 | 0.188 (4) | 0.226 (6) | 1.96 (6) | 0.412 |
| 1.0 | 0.151 (4) | 0.226 (6) | 1.96 (6) | 0.825 |
| 2.0 | 0.151 (4) | 0.226 (6) | 1.96 (6) | 1.03 |
| 3.0 | 0.151 (4) | 0.226 (6) | 1.96 (6) | 1.03 |

ASAP plan at SLA none:
- dd(alpha=0.1) by (service,endpoint) window 15s slide 15s: p99_by_service[5m] (merges 20), p99_by_service,endpoint[5m] (merges 20)
- dd(alpha=0.1) by (service,region,status) window 15s slide 15s: p99_by_region[5m] (merges 20), p99_by_service,region[5m] (merges 20), p99_by_service,region,status[5m] (merges 20)
- dd(alpha=0.1) by (region,endpoint) window 300s slide 300s: p99_by_region,endpoint[5m] (merges 1)

ASAP plan at SLA 0.3:
- dd(alpha=0.1) by (service) window 15s slide 15s: p99_by_service[5m] (merges 20)
- dd(alpha=0.1) by (region) window 15s slide 15s: p99_by_region[5m] (merges 20)
- dd(alpha=0.1) by (service,region) window 60s slide 60s: p99_by_service,region[5m] (merges 5)
- dd(alpha=0.1) by (service,endpoint) window 60s slide 60s: p99_by_service,endpoint[5m] (merges 5)
- dd(alpha=0.1) by (service,region,status) window 300s slide 300s: p99_by_service,region,status[5m] (merges 1)
- dd(alpha=0.1) by (region,endpoint) window 300s slide 300s: p99_by_region,endpoint[5m] (merges 1)

ASAP plan at SLA 0.5:
- dd(alpha=0.1) by (service,region) window 30s slide 15s: p99_by_service[5m] (merges 10), p99_by_region[5m] (merges 10), p99_by_service,region[5m] (merges 10)
- dd(alpha=0.1) by (service,endpoint) window 60s slide 60s: p99_by_service,endpoint[5m] (merges 5)
- dd(alpha=0.1) by (service,region,status) window 300s slide 300s: p99_by_service,region,status[5m] (merges 1)
- dd(alpha=0.1) by (region,endpoint) window 300s slide 300s: p99_by_region,endpoint[5m] (merges 1)

ASAP plan at SLA 1.0:
- dd(alpha=0.1) by (service,region) window 15s slide 15s: p99_by_service[5m] (merges 20), p99_by_region[5m] (merges 20), p99_by_service,region[5m] (merges 20)
- dd(alpha=0.1) by (service,endpoint) window 60s slide 60s: p99_by_service,endpoint[5m] (merges 5)
- dd(alpha=0.1) by (service,region,status) window 300s slide 300s: p99_by_service,region,status[5m] (merges 1)
- dd(alpha=0.1) by (region,endpoint) window 300s slide 300s: p99_by_region,endpoint[5m] (merges 1)

ASAP plan at SLA 2.0:
- dd(alpha=0.1) by (service,endpoint) window 15s slide 15s: p99_by_service[5m] (merges 20), p99_by_service,endpoint[5m] (merges 20)
- dd(alpha=0.1) by (region) window 15s slide 15s: p99_by_region[5m] (merges 20)
- dd(alpha=0.1) by (service,region,status) window 60s slide 60s: p99_by_service,region[5m] (merges 5), p99_by_service,region,status[5m] (merges 5)
- dd(alpha=0.1) by (region,endpoint) window 300s slide 300s: p99_by_region,endpoint[5m] (merges 1)

## synthetic/case/anomaly/interval=15/tp95

| SLA (ms) | asap vCPU (MVs) | perquery vCPU (MVs) | autosketch vCPU (MVs) | ASAP latency ms |
|---|---|---|---|---|
| none | 0.116 (3) | 0.228 (6) | 4.52 (6) | 19.9 |
| 0.3 | 2.03 (4) | 2.11 (6) | 4.52 (6) | 0.297 |
| 0.5 | 1.51 (3) | 1.66 (6) | 4.52 (6) | 0.333 |
| 1.0 | 1.09 (3) | 1.21 (6) | 4.52 (6) | 0.849 |
| 2.0 | 0.603 (3) | 0.716 (6) | 4.52 (6) | 1.65 |
| 3.0 | 0.528 (3) | 0.641 (6) | 4.52 (6) | 2.4 |

ASAP plan at SLA none:
- dd(alpha=0.1) by (service,region,status) window 15s slide 15s: p99_by_service[5m] (merges 20), p99_by_region[5m] (merges 20), p99_by_service,region[5m] (merges 20), p99_by_service,region,status[5m] (merges 20)
- dd(alpha=0.1) by (service,endpoint) window 15s slide 15s: p99_by_service,endpoint[5m] (merges 20)
- dd(alpha=0.1) by (region,endpoint) window 15s slide 15s: p99_by_region,endpoint[5m] (merges 20)

ASAP plan at SLA 0.3:
- dd(alpha=0.1) by (service,region) window 60s slide 15s: p99_by_service[5m] (merges 5), p99_by_region[5m] (merges 5), p99_by_service,region[5m] (merges 5)
- dd(alpha=0.1) by (service,endpoint) window 300s slide 15s: p99_by_service,endpoint[5m] (merges 1)
- dd(alpha=0.1) by (service,region,status) window 150s slide 15s: p99_by_service,region,status[5m] (merges 2)
- dd(alpha=0.1) by (region,endpoint) window 300s slide 15s: p99_by_region,endpoint[5m] (merges 1)

ASAP plan at SLA 0.5:
- dd(alpha=0.1) by (service,region,status) window 150s slide 15s: p99_by_service[5m] (merges 2), p99_by_region[5m] (merges 2), p99_by_service,region[5m] (merges 2), p99_by_service,region,status[5m] (merges 2)
- dd(alpha=0.1) by (service,endpoint) window 150s slide 15s: p99_by_service,endpoint[5m] (merges 2)
- dd(alpha=0.1) by (region,endpoint) window 300s slide 15s: p99_by_region,endpoint[5m] (merges 1)

ASAP plan at SLA 1.0:
- dd(alpha=0.1) by (service,region,status) window 60s slide 15s: p99_by_service[5m] (merges 5), p99_by_region[5m] (merges 5), p99_by_service,region[5m] (merges 5), p99_by_service,region,status[5m] (merges 5)
- dd(alpha=0.1) by (service,endpoint) window 75s slide 15s: p99_by_service,endpoint[5m] (merges 4)
- dd(alpha=0.1) by (region,endpoint) window 300s slide 15s: p99_by_region,endpoint[5m] (merges 1)

ASAP plan at SLA 2.0:
- dd(alpha=0.1) by (service,endpoint) window 60s slide 15s: p99_by_service[5m] (merges 5), p99_by_service,endpoint[5m] (merges 5)
- dd(alpha=0.1) by (service,region,status) window 30s slide 15s: p99_by_region[5m] (merges 10), p99_by_service,region[5m] (merges 10), p99_by_service,region,status[5m] (merges 10)
- dd(alpha=0.1) by (region,endpoint) window 150s slide 15s: p99_by_region,endpoint[5m] (merges 2)

ASAP plan at SLA 3.0:
- dd(alpha=0.1) by (service,region,status) window 30s slide 15s: p99_by_service[5m] (merges 10), p99_by_region[5m] (merges 10), p99_by_service,region[5m] (merges 10), p99_by_service,region,status[5m] (merges 10)
- dd(alpha=0.1) by (service,endpoint) window 30s slide 15s: p99_by_service,endpoint[5m] (merges 10)
- dd(alpha=0.1) by (region,endpoint) window 150s slide 15s: p99_by_region,endpoint[5m] (merges 2)

## synthetic/case/anomaly/interval=300/tp95

| SLA (ms) | asap vCPU (MVs) | perquery vCPU (MVs) | autosketch vCPU (MVs) | ASAP latency ms |
|---|---|---|---|---|
| none | 0.113 (3) | 0.226 (6) | 0.226 (6) | 0.297 |
| 0.3 | 0.113 (3) | 0.226 (6) | 0.226 (6) | 0.297 |
| 0.5 | 0.113 (3) | 0.226 (6) | 0.226 (6) | 0.297 |
| 1.0 | 0.113 (3) | 0.226 (6) | 0.226 (6) | 0.297 |
| 2.0 | 0.113 (3) | 0.226 (6) | 0.226 (6) | 0.297 |
| 3.0 | 0.113 (3) | 0.226 (6) | 0.226 (6) | 0.297 |

ASAP plan at SLA none:
- dd(alpha=0.1) by (service,region,status) window 300s slide 300s: p99_by_service[5m] (merges 1), p99_by_region[5m] (merges 1), p99_by_service,region[5m] (merges 1), p99_by_service,region,status[5m] (merges 1)
- dd(alpha=0.1) by (service,endpoint) window 300s slide 300s: p99_by_service,endpoint[5m] (merges 1)
- dd(alpha=0.1) by (region,endpoint) window 300s slide 300s: p99_by_region,endpoint[5m] (merges 1)

## synthetic/case/anomaly/interval=60/tp95

| SLA (ms) | asap vCPU (MVs) | perquery vCPU (MVs) | autosketch vCPU (MVs) | ASAP latency ms |
|---|---|---|---|---|
| none | 0.113 (3) | 0.226 (6) | 1.13 (6) | 4.43 |
| 0.3 | 0.565 (3) | 0.678 (6) | 1.13 (6) | 0.297 |
| 0.5 | 0.565 (3) | 0.678 (6) | 1.13 (6) | 0.297 |
| 1.0 | 0.414 (3) | 0.527 (6) | 1.13 (6) | 0.825 |
| 2.0 | 0.264 (3) | 0.377 (6) | 1.13 (6) | 1.11 |
| 3.0 | 0.264 (3) | 0.377 (6) | 1.13 (6) | 1.11 |

ASAP plan at SLA none:
- dd(alpha=0.1) by (service,region,status) window 60s slide 60s: p99_by_service[5m] (merges 5), p99_by_region[5m] (merges 5), p99_by_service,region[5m] (merges 5), p99_by_service,region,status[5m] (merges 5)
- dd(alpha=0.1) by (service,endpoint) window 60s slide 60s: p99_by_service,endpoint[5m] (merges 5)
- dd(alpha=0.1) by (region,endpoint) window 60s slide 60s: p99_by_region,endpoint[5m] (merges 5)

ASAP plan at SLA 0.3:
- dd(alpha=0.1) by (service,region,status) window 300s slide 60s: p99_by_service[5m] (merges 1), p99_by_region[5m] (merges 1), p99_by_service,region[5m] (merges 1), p99_by_service,region,status[5m] (merges 1)
- dd(alpha=0.1) by (service,endpoint) window 300s slide 60s: p99_by_service,endpoint[5m] (merges 1)
- dd(alpha=0.1) by (region,endpoint) window 300s slide 60s: p99_by_region,endpoint[5m] (merges 1)

ASAP plan at SLA 1.0:
- dd(alpha=0.1) by (service,endpoint) window 300s slide 60s: p99_by_service[5m] (merges 1), p99_by_service,endpoint[5m] (merges 1)
- dd(alpha=0.1) by (service,region,status) window 60s slide 60s: p99_by_region[5m] (merges 5), p99_by_service,region[5m] (merges 5), p99_by_service,region,status[5m] (merges 5)
- dd(alpha=0.1) by (region,endpoint) window 300s slide 60s: p99_by_region,endpoint[5m] (merges 1)

ASAP plan at SLA 2.0:
- dd(alpha=0.1) by (service,region,status) window 60s slide 60s: p99_by_service[5m] (merges 5), p99_by_region[5m] (merges 5), p99_by_service,region[5m] (merges 5), p99_by_service,region,status[5m] (merges 5)
- dd(alpha=0.1) by (service,endpoint) window 60s slide 60s: p99_by_service,endpoint[5m] (merges 5)
- dd(alpha=0.1) by (region,endpoint) window 300s slide 60s: p99_by_region,endpoint[5m] (merges 1)

## synthetic/case/anomaly/rate=20000000/tp95

| SLA (ms) | asap vCPU (MVs) | perquery vCPU (MVs) | autosketch vCPU (MVs) | ASAP latency ms |
|---|---|---|---|---|
| none | 1.13 (3) | 2.26 (6) | 19.6 (6) | 3.31 |
| 0.3 | infeasible | infeasible | 19.6 (6) |  |
| 0.5 | 3.39 (4) | 3.77 (6) | 19.6 (6) | 0.412 |
| 1.0 | 3.01 (4) | 3.77 (6) | 19.6 (6) | 0.825 |
| 2.0 | 1.51 (4) | 2.26 (6) | 19.6 (6) | 1.11 |
| 3.0 | 1.51 (4) | 2.26 (6) | 19.6 (6) | 1.11 |

ASAP plan at SLA none:
- dd(alpha=0.1) by (service,region,status) window 15s slide 15s: p99_by_service[5m] (merges 20), p99_by_region[5m] (merges 20), p99_by_service,region[5m] (merges 20), p99_by_service,region,status[5m] (merges 20)
- dd(alpha=0.1) by (service,endpoint) window 60s slide 60s: p99_by_service,endpoint[5m] (merges 5)
- dd(alpha=0.1) by (region,endpoint) window 300s slide 300s: p99_by_region,endpoint[5m] (merges 1)

ASAP plan at SLA 0.5:
- dd(alpha=0.1) by (service,region) window 30s slide 15s: p99_by_service[5m] (merges 10), p99_by_region[5m] (merges 10), p99_by_service,region[5m] (merges 10)
- dd(alpha=0.1) by (service,endpoint) window 300s slide 60s: p99_by_service,endpoint[5m] (merges 1)
- dd(alpha=0.1) by (service,region,status) window 300s slide 300s: p99_by_service,region,status[5m] (merges 1)
- dd(alpha=0.1) by (region,endpoint) window 300s slide 300s: p99_by_region,endpoint[5m] (merges 1)

ASAP plan at SLA 1.0:
- dd(alpha=0.1) by (service,region) window 15s slide 15s: p99_by_service[5m] (merges 20), p99_by_region[5m] (merges 20), p99_by_service,region[5m] (merges 20)
- dd(alpha=0.1) by (service,endpoint) window 300s slide 60s: p99_by_service,endpoint[5m] (merges 1)
- dd(alpha=0.1) by (service,region,status) window 300s slide 300s: p99_by_service,region,status[5m] (merges 1)
- dd(alpha=0.1) by (region,endpoint) window 300s slide 300s: p99_by_region,endpoint[5m] (merges 1)

ASAP plan at SLA 2.0:
- dd(alpha=0.1) by (service,region) window 15s slide 15s: p99_by_service[5m] (merges 20), p99_by_region[5m] (merges 20), p99_by_service,region[5m] (merges 20)
- dd(alpha=0.1) by (service,endpoint) window 60s slide 60s: p99_by_service,endpoint[5m] (merges 5)
- dd(alpha=0.1) by (service,region,status) window 300s slide 300s: p99_by_service,region,status[5m] (merges 1)
- dd(alpha=0.1) by (region,endpoint) window 300s slide 300s: p99_by_region,endpoint[5m] (merges 1)

## synthetic/case/anomaly/rate=2000000/tp95

| SLA (ms) | asap vCPU (MVs) | perquery vCPU (MVs) | autosketch vCPU (MVs) | ASAP latency ms |
|---|---|---|---|---|
| none | 0.114 (3) | 0.226 (6) | 1.96 (6) | 3.31 |
| 0.3 | 0.377 (6) | 0.377 (6) | 1.96 (6) | 0.297 |
| 0.5 | 0.339 (4) | 0.377 (6) | 1.96 (6) | 0.412 |
| 1.0 | 0.301 (4) | 0.377 (6) | 1.96 (6) | 0.825 |
| 2.0 | 0.151 (4) | 0.226 (6) | 1.96 (6) | 1.11 |
| 3.0 | 0.151 (4) | 0.226 (6) | 1.96 (6) | 1.11 |

ASAP plan at SLA none:
- dd(alpha=0.1) by (service,region,status) window 15s slide 15s: p99_by_service[5m] (merges 20), p99_by_region[5m] (merges 20), p99_by_service,region[5m] (merges 20), p99_by_service,region,status[5m] (merges 20)
- dd(alpha=0.1) by (service,endpoint) window 60s slide 60s: p99_by_service,endpoint[5m] (merges 5)
- dd(alpha=0.1) by (region,endpoint) window 300s slide 300s: p99_by_region,endpoint[5m] (merges 1)

ASAP plan at SLA 0.3:
- dd(alpha=0.1) by (service) window 15s slide 15s: p99_by_service[5m] (merges 20)
- dd(alpha=0.1) by (region) window 15s slide 15s: p99_by_region[5m] (merges 20)
- dd(alpha=0.1) by (service,region) window 60s slide 60s: p99_by_service,region[5m] (merges 5)
- dd(alpha=0.1) by (service,endpoint) window 300s slide 60s: p99_by_service,endpoint[5m] (merges 1)
- dd(alpha=0.1) by (service,region,status) window 300s slide 300s: p99_by_service,region,status[5m] (merges 1)
- dd(alpha=0.1) by (region,endpoint) window 300s slide 300s: p99_by_region,endpoint[5m] (merges 1)

ASAP plan at SLA 0.5:
- dd(alpha=0.1) by (service,region) window 30s slide 15s: p99_by_service[5m] (merges 10), p99_by_region[5m] (merges 10), p99_by_service,region[5m] (merges 10)
- dd(alpha=0.1) by (service,endpoint) window 300s slide 60s: p99_by_service,endpoint[5m] (merges 1)
- dd(alpha=0.1) by (service,region,status) window 300s slide 300s: p99_by_service,region,status[5m] (merges 1)
- dd(alpha=0.1) by (region,endpoint) window 300s slide 300s: p99_by_region,endpoint[5m] (merges 1)

ASAP plan at SLA 1.0:
- dd(alpha=0.1) by (service,region) window 15s slide 15s: p99_by_service[5m] (merges 20), p99_by_region[5m] (merges 20), p99_by_service,region[5m] (merges 20)
- dd(alpha=0.1) by (service,endpoint) window 300s slide 60s: p99_by_service,endpoint[5m] (merges 1)
- dd(alpha=0.1) by (service,region,status) window 300s slide 300s: p99_by_service,region,status[5m] (merges 1)
- dd(alpha=0.1) by (region,endpoint) window 300s slide 300s: p99_by_region,endpoint[5m] (merges 1)

ASAP plan at SLA 2.0:
- dd(alpha=0.1) by (service,region) window 15s slide 15s: p99_by_service[5m] (merges 20), p99_by_region[5m] (merges 20), p99_by_service,region[5m] (merges 20)
- dd(alpha=0.1) by (service,endpoint) window 60s slide 60s: p99_by_service,endpoint[5m] (merges 5)
- dd(alpha=0.1) by (service,region,status) window 300s slide 300s: p99_by_service,region,status[5m] (merges 1)
- dd(alpha=0.1) by (region,endpoint) window 300s slide 300s: p99_by_region,endpoint[5m] (merges 1)

## synthetic/case/anomaly/rate=250000/tp95

| SLA (ms) | asap vCPU (MVs) | perquery vCPU (MVs) | autosketch vCPU (MVs) | ASAP latency ms |
|---|---|---|---|---|
| none | 0.0146 (3) | 0.0283 (6) | 0.245 (6) | 3.31 |
| 0.3 | 0.0471 (6) | 0.0471 (6) | 0.245 (6) | 0.297 |
| 0.5 | 0.0424 (4) | 0.0471 (6) | 0.245 (6) | 0.412 |
| 1.0 | 0.0378 (4) | 0.0471 (6) | 0.245 (6) | 0.825 |
| 2.0 | 0.019 (4) | 0.0283 (6) | 0.245 (6) | 1.11 |
| 3.0 | 0.019 (4) | 0.0283 (6) | 0.245 (6) | 1.11 |

ASAP plan at SLA none:
- dd(alpha=0.1) by (service,region,status) window 15s slide 15s: p99_by_service[5m] (merges 20), p99_by_region[5m] (merges 20), p99_by_service,region[5m] (merges 20), p99_by_service,region,status[5m] (merges 20)
- dd(alpha=0.1) by (service,endpoint) window 60s slide 60s: p99_by_service,endpoint[5m] (merges 5)
- dd(alpha=0.1) by (region,endpoint) window 300s slide 300s: p99_by_region,endpoint[5m] (merges 1)

ASAP plan at SLA 0.3:
- dd(alpha=0.1) by (service) window 15s slide 15s: p99_by_service[5m] (merges 20)
- dd(alpha=0.1) by (region) window 15s slide 15s: p99_by_region[5m] (merges 20)
- dd(alpha=0.1) by (service,region) window 60s slide 60s: p99_by_service,region[5m] (merges 5)
- dd(alpha=0.1) by (service,endpoint) window 300s slide 60s: p99_by_service,endpoint[5m] (merges 1)
- dd(alpha=0.1) by (service,region,status) window 300s slide 300s: p99_by_service,region,status[5m] (merges 1)
- dd(alpha=0.1) by (region,endpoint) window 300s slide 300s: p99_by_region,endpoint[5m] (merges 1)

ASAP plan at SLA 0.5:
- dd(alpha=0.1) by (service,region) window 30s slide 15s: p99_by_service[5m] (merges 10), p99_by_region[5m] (merges 10), p99_by_service,region[5m] (merges 10)
- dd(alpha=0.1) by (service,endpoint) window 300s slide 60s: p99_by_service,endpoint[5m] (merges 1)
- dd(alpha=0.1) by (service,region,status) window 300s slide 300s: p99_by_service,region,status[5m] (merges 1)
- dd(alpha=0.1) by (region,endpoint) window 300s slide 300s: p99_by_region,endpoint[5m] (merges 1)

ASAP plan at SLA 1.0:
- dd(alpha=0.1) by (service,region) window 15s slide 15s: p99_by_service[5m] (merges 20), p99_by_region[5m] (merges 20), p99_by_service,region[5m] (merges 20)
- dd(alpha=0.1) by (service,endpoint) window 300s slide 60s: p99_by_service,endpoint[5m] (merges 1)
- dd(alpha=0.1) by (service,region,status) window 300s slide 300s: p99_by_service,region,status[5m] (merges 1)
- dd(alpha=0.1) by (region,endpoint) window 300s slide 300s: p99_by_region,endpoint[5m] (merges 1)

ASAP plan at SLA 2.0:
- dd(alpha=0.1) by (service,region) window 15s slide 15s: p99_by_service[5m] (merges 20), p99_by_region[5m] (merges 20), p99_by_service,region[5m] (merges 20)
- dd(alpha=0.1) by (service,endpoint) window 60s slide 60s: p99_by_service,endpoint[5m] (merges 5)
- dd(alpha=0.1) by (service,region,status) window 300s slide 300s: p99_by_service,region,status[5m] (merges 1)
- dd(alpha=0.1) by (region,endpoint) window 300s slide 300s: p99_by_region,endpoint[5m] (merges 1)

## synthetic/case/anomaly/rate=500000/tp95

| SLA (ms) | asap vCPU (MVs) | perquery vCPU (MVs) | autosketch vCPU (MVs) | ASAP latency ms |
|---|---|---|---|---|
| none | 0.0288 (3) | 0.0565 (6) | 0.49 (6) | 3.31 |
| 0.3 | 0.0942 (6) | 0.0942 (6) | 0.49 (6) | 0.297 |
| 0.5 | 0.0848 (4) | 0.0942 (6) | 0.49 (6) | 0.412 |
| 1.0 | 0.0754 (4) | 0.0942 (6) | 0.49 (6) | 0.825 |
| 2.0 | 0.0378 (4) | 0.0565 (6) | 0.49 (6) | 1.11 |
| 3.0 | 0.0378 (4) | 0.0565 (6) | 0.49 (6) | 1.11 |

ASAP plan at SLA none:
- dd(alpha=0.1) by (service,region,status) window 15s slide 15s: p99_by_service[5m] (merges 20), p99_by_region[5m] (merges 20), p99_by_service,region[5m] (merges 20), p99_by_service,region,status[5m] (merges 20)
- dd(alpha=0.1) by (service,endpoint) window 60s slide 60s: p99_by_service,endpoint[5m] (merges 5)
- dd(alpha=0.1) by (region,endpoint) window 300s slide 300s: p99_by_region,endpoint[5m] (merges 1)

ASAP plan at SLA 0.3:
- dd(alpha=0.1) by (service) window 15s slide 15s: p99_by_service[5m] (merges 20)
- dd(alpha=0.1) by (region) window 15s slide 15s: p99_by_region[5m] (merges 20)
- dd(alpha=0.1) by (service,region) window 60s slide 60s: p99_by_service,region[5m] (merges 5)
- dd(alpha=0.1) by (service,endpoint) window 300s slide 60s: p99_by_service,endpoint[5m] (merges 1)
- dd(alpha=0.1) by (service,region,status) window 300s slide 300s: p99_by_service,region,status[5m] (merges 1)
- dd(alpha=0.1) by (region,endpoint) window 300s slide 300s: p99_by_region,endpoint[5m] (merges 1)

ASAP plan at SLA 0.5:
- dd(alpha=0.1) by (service,region) window 30s slide 15s: p99_by_service[5m] (merges 10), p99_by_region[5m] (merges 10), p99_by_service,region[5m] (merges 10)
- dd(alpha=0.1) by (service,endpoint) window 300s slide 60s: p99_by_service,endpoint[5m] (merges 1)
- dd(alpha=0.1) by (service,region,status) window 300s slide 300s: p99_by_service,region,status[5m] (merges 1)
- dd(alpha=0.1) by (region,endpoint) window 300s slide 300s: p99_by_region,endpoint[5m] (merges 1)

ASAP plan at SLA 1.0:
- dd(alpha=0.1) by (service,region) window 15s slide 15s: p99_by_service[5m] (merges 20), p99_by_region[5m] (merges 20), p99_by_service,region[5m] (merges 20)
- dd(alpha=0.1) by (service,endpoint) window 300s slide 60s: p99_by_service,endpoint[5m] (merges 1)
- dd(alpha=0.1) by (service,region,status) window 300s slide 300s: p99_by_service,region,status[5m] (merges 1)
- dd(alpha=0.1) by (region,endpoint) window 300s slide 300s: p99_by_region,endpoint[5m] (merges 1)

ASAP plan at SLA 2.0:
- dd(alpha=0.1) by (service,region) window 15s slide 15s: p99_by_service[5m] (merges 20), p99_by_region[5m] (merges 20), p99_by_service,region[5m] (merges 20)
- dd(alpha=0.1) by (service,endpoint) window 60s slide 60s: p99_by_service,endpoint[5m] (merges 5)
- dd(alpha=0.1) by (service,region,status) window 300s slide 300s: p99_by_service,region,status[5m] (merges 1)
- dd(alpha=0.1) by (region,endpoint) window 300s slide 300s: p99_by_region,endpoint[5m] (merges 1)

## synthetic/case/anomaly/statuses=16/tp95

| SLA (ms) | asap vCPU (MVs) | perquery vCPU (MVs) | autosketch vCPU (MVs) | ASAP latency ms |
|---|---|---|---|---|
| none | 0.115 (3) | 0.226 (6) | 1.96 (6) | 13.2 |
| 0.3 | 0.377 (6) | 0.377 (6) | 1.96 (6) | 0.297 |
| 0.5 | 0.339 (4) | 0.377 (6) | 1.96 (6) | 0.412 |
| 1.0 | 0.301 (4) | 0.377 (6) | 1.96 (6) | 0.825 |
| 2.0 | 0.151 (4) | 0.226 (6) | 1.96 (6) | 1.11 |
| 3.0 | 0.151 (4) | 0.226 (6) | 1.96 (6) | 1.11 |

ASAP plan at SLA none:
- dd(alpha=0.1) by (service,endpoint) window 15s slide 15s: p99_by_service[5m] (merges 20), p99_by_service,endpoint[5m] (merges 20)
- dd(alpha=0.1) by (service,region,status) window 15s slide 15s: p99_by_region[5m] (merges 20), p99_by_service,region[5m] (merges 20), p99_by_service,region,status[5m] (merges 20)
- dd(alpha=0.1) by (region,endpoint) window 300s slide 300s: p99_by_region,endpoint[5m] (merges 1)

ASAP plan at SLA 0.3:
- dd(alpha=0.1) by (service) window 15s slide 15s: p99_by_service[5m] (merges 20)
- dd(alpha=0.1) by (region) window 15s slide 15s: p99_by_region[5m] (merges 20)
- dd(alpha=0.1) by (service,region) window 60s slide 60s: p99_by_service,region[5m] (merges 5)
- dd(alpha=0.1) by (service,endpoint) window 300s slide 60s: p99_by_service,endpoint[5m] (merges 1)
- dd(alpha=0.1) by (service,region,status) window 300s slide 300s: p99_by_service,region,status[5m] (merges 1)
- dd(alpha=0.1) by (region,endpoint) window 300s slide 300s: p99_by_region,endpoint[5m] (merges 1)

ASAP plan at SLA 0.5:
- dd(alpha=0.1) by (service,region) window 30s slide 15s: p99_by_service[5m] (merges 10), p99_by_region[5m] (merges 10), p99_by_service,region[5m] (merges 10)
- dd(alpha=0.1) by (service,endpoint) window 300s slide 60s: p99_by_service,endpoint[5m] (merges 1)
- dd(alpha=0.1) by (service,region,status) window 300s slide 300s: p99_by_service,region,status[5m] (merges 1)
- dd(alpha=0.1) by (region,endpoint) window 300s slide 300s: p99_by_region,endpoint[5m] (merges 1)

ASAP plan at SLA 1.0:
- dd(alpha=0.1) by (service,region) window 15s slide 15s: p99_by_service[5m] (merges 20), p99_by_region[5m] (merges 20), p99_by_service,region[5m] (merges 20)
- dd(alpha=0.1) by (service,endpoint) window 300s slide 60s: p99_by_service,endpoint[5m] (merges 1)
- dd(alpha=0.1) by (service,region,status) window 300s slide 300s: p99_by_service,region,status[5m] (merges 1)
- dd(alpha=0.1) by (region,endpoint) window 300s slide 300s: p99_by_region,endpoint[5m] (merges 1)

ASAP plan at SLA 2.0:
- dd(alpha=0.1) by (service,region) window 15s slide 15s: p99_by_service[5m] (merges 20), p99_by_region[5m] (merges 20), p99_by_service,region[5m] (merges 20)
- dd(alpha=0.1) by (service,endpoint) window 60s slide 60s: p99_by_service,endpoint[5m] (merges 5)
- dd(alpha=0.1) by (service,region,status) window 300s slide 300s: p99_by_service,region,status[5m] (merges 1)
- dd(alpha=0.1) by (region,endpoint) window 300s slide 300s: p99_by_region,endpoint[5m] (merges 1)

## synthetic/case/anomaly/statuses=2/tp95

| SLA (ms) | asap vCPU (MVs) | perquery vCPU (MVs) | autosketch vCPU (MVs) | ASAP latency ms |
|---|---|---|---|---|
| none | 0.113 (3) | 0.226 (6) | 1.96 (6) | 1.65 |
| 0.3 | 0.377 (6) | 0.377 (6) | 1.96 (6) | 0.297 |
| 0.5 | 0.339 (5) | 0.377 (6) | 1.96 (6) | 0.384 |
| 1.0 | 0.301 (4) | 0.377 (6) | 1.96 (6) | 0.825 |
| 2.0 | 0.113 (3) | 0.226 (6) | 1.96 (6) | 1.65 |
| 3.0 | 0.113 (3) | 0.226 (6) | 1.96 (6) | 1.65 |

ASAP plan at SLA none:
- dd(alpha=0.1) by (service,region,status) window 15s slide 15s: p99_by_service[5m] (merges 20), p99_by_region[5m] (merges 20), p99_by_service,region[5m] (merges 20), p99_by_service,region,status[5m] (merges 20)
- dd(alpha=0.1) by (service,endpoint) window 60s slide 60s: p99_by_service,endpoint[5m] (merges 5)
- dd(alpha=0.1) by (region,endpoint) window 300s slide 300s: p99_by_region,endpoint[5m] (merges 1)

ASAP plan at SLA 0.3:
- dd(alpha=0.1) by (service) window 15s slide 15s: p99_by_service[5m] (merges 20)
- dd(alpha=0.1) by (region) window 15s slide 15s: p99_by_region[5m] (merges 20)
- dd(alpha=0.1) by (service,region) window 60s slide 60s: p99_by_service,region[5m] (merges 5)
- dd(alpha=0.1) by (service,endpoint) window 300s slide 60s: p99_by_service,endpoint[5m] (merges 1)
- dd(alpha=0.1) by (service,region,status) window 300s slide 300s: p99_by_service,region,status[5m] (merges 1)
- dd(alpha=0.1) by (region,endpoint) window 300s slide 300s: p99_by_region,endpoint[5m] (merges 1)

ASAP plan at SLA 0.5:
- dd(alpha=0.1) by (service) window 15s slide 15s: p99_by_service[5m] (merges 20)
- dd(alpha=0.1) by (region) window 15s slide 15s: p99_by_region[5m] (merges 20)
- dd(alpha=0.1) by (service,region,status) window 60s slide 60s: p99_by_service,region[5m] (merges 5), p99_by_service,region,status[5m] (merges 5)
- dd(alpha=0.1) by (service,endpoint) window 300s slide 60s: p99_by_service,endpoint[5m] (merges 1)
- dd(alpha=0.1) by (region,endpoint) window 300s slide 300s: p99_by_region,endpoint[5m] (merges 1)

ASAP plan at SLA 1.0:
- dd(alpha=0.1) by (service) window 15s slide 15s: p99_by_service[5m] (merges 20), p99_by_region[5m] (merges 20)
- dd(alpha=0.1) by (service,region,status) window 60s slide 60s: p99_by_service,region[5m] (merges 5), p99_by_service,region,status[5m] (merges 5)
- dd(alpha=0.1) by (service,endpoint) window 300s slide 60s: p99_by_service,endpoint[5m] (merges 1)
- dd(alpha=0.1) by (region,endpoint) window 300s slide 300s: p99_by_region,endpoint[5m] (merges 1)

## synthetic/case/anomaly/statuses=4/tp95

| SLA (ms) | asap vCPU (MVs) | perquery vCPU (MVs) | autosketch vCPU (MVs) | ASAP latency ms |
|---|---|---|---|---|
| none | 0.114 (3) | 0.226 (6) | 1.96 (6) | 3.31 |
| 0.3 | 0.377 (6) | 0.377 (6) | 1.96 (6) | 0.297 |
| 0.5 | 0.339 (4) | 0.377 (6) | 1.96 (6) | 0.412 |
| 1.0 | 0.301 (4) | 0.377 (6) | 1.96 (6) | 0.825 |
| 2.0 | 0.151 (4) | 0.226 (6) | 1.96 (6) | 1.11 |
| 3.0 | 0.151 (4) | 0.226 (6) | 1.96 (6) | 1.11 |

ASAP plan at SLA none:
- dd(alpha=0.1) by (service,region,status) window 15s slide 15s: p99_by_service[5m] (merges 20), p99_by_region[5m] (merges 20), p99_by_service,region[5m] (merges 20), p99_by_service,region,status[5m] (merges 20)
- dd(alpha=0.1) by (service,endpoint) window 60s slide 60s: p99_by_service,endpoint[5m] (merges 5)
- dd(alpha=0.1) by (region,endpoint) window 300s slide 300s: p99_by_region,endpoint[5m] (merges 1)

ASAP plan at SLA 0.3:
- dd(alpha=0.1) by (service) window 15s slide 15s: p99_by_service[5m] (merges 20)
- dd(alpha=0.1) by (region) window 15s slide 15s: p99_by_region[5m] (merges 20)
- dd(alpha=0.1) by (service,region) window 60s slide 60s: p99_by_service,region[5m] (merges 5)
- dd(alpha=0.1) by (service,endpoint) window 300s slide 60s: p99_by_service,endpoint[5m] (merges 1)
- dd(alpha=0.1) by (service,region,status) window 300s slide 300s: p99_by_service,region,status[5m] (merges 1)
- dd(alpha=0.1) by (region,endpoint) window 300s slide 300s: p99_by_region,endpoint[5m] (merges 1)

ASAP plan at SLA 0.5:
- dd(alpha=0.1) by (service,region) window 30s slide 15s: p99_by_service[5m] (merges 10), p99_by_region[5m] (merges 10), p99_by_service,region[5m] (merges 10)
- dd(alpha=0.1) by (service,endpoint) window 300s slide 60s: p99_by_service,endpoint[5m] (merges 1)
- dd(alpha=0.1) by (service,region,status) window 300s slide 300s: p99_by_service,region,status[5m] (merges 1)
- dd(alpha=0.1) by (region,endpoint) window 300s slide 300s: p99_by_region,endpoint[5m] (merges 1)

ASAP plan at SLA 1.0:
- dd(alpha=0.1) by (service,region) window 15s slide 15s: p99_by_service[5m] (merges 20), p99_by_region[5m] (merges 20), p99_by_service,region[5m] (merges 20)
- dd(alpha=0.1) by (service,endpoint) window 300s slide 60s: p99_by_service,endpoint[5m] (merges 1)
- dd(alpha=0.1) by (service,region,status) window 300s slide 300s: p99_by_service,region,status[5m] (merges 1)
- dd(alpha=0.1) by (region,endpoint) window 300s slide 300s: p99_by_region,endpoint[5m] (merges 1)

ASAP plan at SLA 2.0:
- dd(alpha=0.1) by (service,region) window 15s slide 15s: p99_by_service[5m] (merges 20), p99_by_region[5m] (merges 20), p99_by_service,region[5m] (merges 20)
- dd(alpha=0.1) by (service,endpoint) window 60s slide 60s: p99_by_service,endpoint[5m] (merges 5)
- dd(alpha=0.1) by (service,region,status) window 300s slide 300s: p99_by_service,region,status[5m] (merges 1)
- dd(alpha=0.1) by (region,endpoint) window 300s slide 300s: p99_by_region,endpoint[5m] (merges 1)

## synthetic/case/anomaly/statuses=8/tp95

| SLA (ms) | asap vCPU (MVs) | perquery vCPU (MVs) | autosketch vCPU (MVs) | ASAP latency ms |
|---|---|---|---|---|
| none | 0.114 (3) | 0.226 (6) | 1.96 (6) | 6.61 |
| 0.3 | 0.377 (6) | 0.377 (6) | 1.96 (6) | 0.297 |
| 0.5 | 0.339 (4) | 0.377 (6) | 1.96 (6) | 0.412 |
| 1.0 | 0.301 (4) | 0.377 (6) | 1.96 (6) | 0.825 |
| 2.0 | 0.151 (4) | 0.226 (6) | 1.96 (6) | 1.11 |
| 3.0 | 0.151 (4) | 0.226 (6) | 1.96 (6) | 1.11 |

ASAP plan at SLA none:
- dd(alpha=0.1) by (service,endpoint) window 15s slide 15s: p99_by_service[5m] (merges 20), p99_by_service,endpoint[5m] (merges 20)
- dd(alpha=0.1) by (service,region,status) window 15s slide 15s: p99_by_region[5m] (merges 20), p99_by_service,region[5m] (merges 20), p99_by_service,region,status[5m] (merges 20)
- dd(alpha=0.1) by (region,endpoint) window 300s slide 300s: p99_by_region,endpoint[5m] (merges 1)

ASAP plan at SLA 0.3:
- dd(alpha=0.1) by (service) window 15s slide 15s: p99_by_service[5m] (merges 20)
- dd(alpha=0.1) by (region) window 15s slide 15s: p99_by_region[5m] (merges 20)
- dd(alpha=0.1) by (service,region) window 60s slide 60s: p99_by_service,region[5m] (merges 5)
- dd(alpha=0.1) by (service,endpoint) window 300s slide 60s: p99_by_service,endpoint[5m] (merges 1)
- dd(alpha=0.1) by (service,region,status) window 300s slide 300s: p99_by_service,region,status[5m] (merges 1)
- dd(alpha=0.1) by (region,endpoint) window 300s slide 300s: p99_by_region,endpoint[5m] (merges 1)

ASAP plan at SLA 0.5:
- dd(alpha=0.1) by (service,region) window 30s slide 15s: p99_by_service[5m] (merges 10), p99_by_region[5m] (merges 10), p99_by_service,region[5m] (merges 10)
- dd(alpha=0.1) by (service,endpoint) window 300s slide 60s: p99_by_service,endpoint[5m] (merges 1)
- dd(alpha=0.1) by (service,region,status) window 300s slide 300s: p99_by_service,region,status[5m] (merges 1)
- dd(alpha=0.1) by (region,endpoint) window 300s slide 300s: p99_by_region,endpoint[5m] (merges 1)

ASAP plan at SLA 1.0:
- dd(alpha=0.1) by (service,region) window 15s slide 15s: p99_by_service[5m] (merges 20), p99_by_region[5m] (merges 20), p99_by_service,region[5m] (merges 20)
- dd(alpha=0.1) by (service,endpoint) window 300s slide 60s: p99_by_service,endpoint[5m] (merges 1)
- dd(alpha=0.1) by (service,region,status) window 300s slide 300s: p99_by_service,region,status[5m] (merges 1)
- dd(alpha=0.1) by (region,endpoint) window 300s slide 300s: p99_by_region,endpoint[5m] (merges 1)

ASAP plan at SLA 2.0:
- dd(alpha=0.1) by (service,region) window 15s slide 15s: p99_by_service[5m] (merges 20), p99_by_region[5m] (merges 20), p99_by_service,region[5m] (merges 20)
- dd(alpha=0.1) by (service,endpoint) window 60s slide 60s: p99_by_service,endpoint[5m] (merges 5)
- dd(alpha=0.1) by (service,region,status) window 300s slide 300s: p99_by_service,region,status[5m] (merges 1)
- dd(alpha=0.1) by (region,endpoint) window 300s slide 300s: p99_by_region,endpoint[5m] (merges 1)

## synthetic/case/perf/base=0/tp95

| SLA (ms) | asap vCPU (MVs) | perquery vCPU (MVs) | autosketch vCPU (MVs) | ASAP latency ms |
|---|---|---|---|---|
| none | 0.105 (2) | 0.306 (8) | 4.14 (8) | 2.38e+04 |
| 10.0 | 1.24 (5) | 1.55 (8) | 4.14 (8) | 9.45 |
| 30.0 | 0.643 (5) | 0.793 (8) | 4.14 (8) | 21.9 |
| 100.0 | 0.229 (4) | 0.38 (8) | 4.14 (8) | 96.3 |
| 300.0 | 0.155 (4) | 0.306 (8) | 4.14 (8) | 195 |
| 1000.0 | 0.126 (3) | 0.306 (8) | 4.14 (8) | 493 |
| 3000.0 | 0.126 (3) | 0.306 (8) | 4.14 (8) | 493 |
| 10000.0 | 0.122 (3) | 0.306 (8) | 4.14 (8) | 5.95e+03 |

ASAP plan at SLA none:
- dd(alpha=0.1) by (job) window 0.005s slide 0.005s: quantile_by_job_p0.5[1 scrape] (merges 1), quantile_by_job_p0.9[1 scrape] (merges 1)
- dd(alpha=0.1) by (series) window 15s slide 15s: quantile_over_time_p0.5[1m] (merges 4), quantile_over_time_p0.9[1m] (merges 4), quantile_over_time_p0.9[10m] (merges 40), quantile_over_time_p0.9[30m] (merges 120), quantile_over_time_p0.9[6h] (merges 1440), quantile_over_time_p0.9[1d] (merges 5760)

ASAP plan at SLA 10.0:
- dd(alpha=0.1) by (job) window 0.005s slide 0.005s: quantile_by_job_p0.5[1 scrape] (merges 1), quantile_by_job_p0.9[1 scrape] (merges 1)
- dd(alpha=0.1) by (series) window 30s slide 15s: quantile_over_time_p0.5[1m] (merges 2), quantile_over_time_p0.9[1m] (merges 2)
- dd(alpha=0.1) by (series) window 600s slide 60s: quantile_over_time_p0.9[10m] (merges 1), quantile_over_time_p0.9[30m] (merges 3)
- dd(alpha=0.1) by (series) window 7200s slide 1800s: quantile_over_time_p0.9[6h] (merges 3)
- dd(alpha=0.1) by (series) window 28800s slide 1800s: quantile_over_time_p0.9[1d] (merges 3)

ASAP plan at SLA 30.0:
- dd(alpha=0.1) by (job) window 0.005s slide 0.005s: quantile_by_job_p0.5[1 scrape] (merges 1), quantile_by_job_p0.9[1 scrape] (merges 1)
- dd(alpha=0.1) by (series) window 15s slide 15s: quantile_over_time_p0.5[1m] (merges 4), quantile_over_time_p0.9[1m] (merges 4)
- dd(alpha=0.1) by (series) window 300s slide 60s: quantile_over_time_p0.9[10m] (merges 2), quantile_over_time_p0.9[30m] (merges 6)
- dd(alpha=0.1) by (series) window 3600s slide 1800s: quantile_over_time_p0.9[6h] (merges 6)
- dd(alpha=0.1) by (series) window 14400s slide 1800s: quantile_over_time_p0.9[1d] (merges 6)

ASAP plan at SLA 100.0:
- dd(alpha=0.1) by (job) window 0.005s slide 0.005s: quantile_by_job_p0.5[1 scrape] (merges 1), quantile_by_job_p0.9[1 scrape] (merges 1)
- dd(alpha=0.1) by (series) window 15s slide 15s: quantile_over_time_p0.5[1m] (merges 4), quantile_over_time_p0.9[1m] (merges 4)
- dd(alpha=0.1) by (series) window 120s slide 60s: quantile_over_time_p0.9[10m] (merges 5), quantile_over_time_p0.9[30m] (merges 15)
- dd(alpha=0.1) by (series) window 3600s slide 1800s: quantile_over_time_p0.9[6h] (merges 6), quantile_over_time_p0.9[1d] (merges 24)

ASAP plan at SLA 300.0:
- dd(alpha=0.1) by (job) window 0.005s slide 0.005s: quantile_by_job_p0.5[1 scrape] (merges 1), quantile_by_job_p0.9[1 scrape] (merges 1)
- dd(alpha=0.1) by (series) window 15s slide 15s: quantile_over_time_p0.5[1m] (merges 4), quantile_over_time_p0.9[1m] (merges 4)
- dd(alpha=0.1) by (series) window 60s slide 60s: quantile_over_time_p0.9[10m] (merges 10), quantile_over_time_p0.9[30m] (merges 30)
- dd(alpha=0.1) by (series) window 1800s slide 1800s: quantile_over_time_p0.9[6h] (merges 12), quantile_over_time_p0.9[1d] (merges 48)

ASAP plan at SLA 1000.0:
- dd(alpha=0.1) by (job) window 0.005s slide 0.005s: quantile_by_job_p0.5[1 scrape] (merges 1), quantile_by_job_p0.9[1 scrape] (merges 1)
- dd(alpha=0.1) by (series) window 15s slide 15s: quantile_over_time_p0.5[1m] (merges 4), quantile_over_time_p0.9[1m] (merges 4), quantile_over_time_p0.9[10m] (merges 40), quantile_over_time_p0.9[30m] (merges 120)
- dd(alpha=0.1) by (series) window 1800s slide 1800s: quantile_over_time_p0.9[6h] (merges 12), quantile_over_time_p0.9[1d] (merges 48)

ASAP plan at SLA 10000.0:
- dd(alpha=0.1) by (job) window 0.005s slide 0.005s: quantile_by_job_p0.5[1 scrape] (merges 1), quantile_by_job_p0.9[1 scrape] (merges 1)
- dd(alpha=0.1) by (series) window 15s slide 15s: quantile_over_time_p0.5[1m] (merges 4), quantile_over_time_p0.9[1m] (merges 4)
- dd(alpha=0.1) by (series) window 60s slide 60s: quantile_over_time_p0.9[10m] (merges 10), quantile_over_time_p0.9[30m] (merges 30), quantile_over_time_p0.9[6h] (merges 360), quantile_over_time_p0.9[1d] (merges 1440)

## synthetic/case/perf/long_interval=15/tp95

| SLA (ms) | asap vCPU (MVs) | perquery vCPU (MVs) | autosketch vCPU (MVs) | ASAP latency ms |
|---|---|---|---|---|
| none | 0.627 (3) | 0.966 (8) | 273 (8) | 2.97e+03 |
| 10.0 | infeasible | infeasible | 273 (8) |  |
| 30.0 | infeasible | infeasible | 273 (8) |  |
| 100.0 | infeasible | infeasible | 273 (8) |  |
| 300.0 | 3.53 (4) | 4.41 (8) | 273 (8) | 274 |
| 1000.0 | 1.07 (3) | 1.49 (8) | 273 (8) | 989 |
| 3000.0 | 0.627 (3) | 0.975 (8) | 273 (8) | 2.97e+03 |
| 10000.0 | 0.627 (3) | 0.966 (8) | 273 (8) | 2.97e+03 |

ASAP plan at SLA none:
- dd(alpha=0.1) by (job) window 0.005s slide 0.005s: quantile_by_job_p0.5[1 scrape] (merges 1), quantile_by_job_p0.9[1 scrape] (merges 1)
- dd(alpha=0.1) by (series) window 15s slide 15s: quantile_over_time_p0.5[1m] (merges 4), quantile_over_time_p0.9[1m] (merges 4)
- dd(alpha=0.1) by (series) window 120s slide 15s: quantile_over_time_p0.9[10m] (merges 5), quantile_over_time_p0.9[30m] (merges 15), quantile_over_time_p0.9[6h] (merges 180), quantile_over_time_p0.9[1d] (merges 720)

ASAP plan at SLA 300.0:
- dd(alpha=0.1) by (job) window 0.005s slide 0.005s: quantile_by_job_p0.5[1 scrape] (merges 1), quantile_by_job_p0.9[1 scrape] (merges 1)
- dd(alpha=0.1) by (series) window 15s slide 15s: quantile_over_time_p0.5[1m] (merges 4), quantile_over_time_p0.9[1m] (merges 4)
- dd(alpha=0.1) by (series) window 60s slide 60s: quantile_over_time_p0.9[10m] (merges 10), quantile_over_time_p0.9[30m] (merges 30)
- dd(alpha=0.1) by (series) window 1350s slide 15s: quantile_over_time_p0.9[6h] (merges 16), quantile_over_time_p0.9[1d] (merges 64)

ASAP plan at SLA 1000.0:
- dd(alpha=0.1) by (job) window 0.005s slide 0.005s: quantile_by_job_p0.5[1 scrape] (merges 1), quantile_by_job_p0.9[1 scrape] (merges 1)
- dd(alpha=0.1) by (series) window 15s slide 15s: quantile_over_time_p0.5[1m] (merges 4), quantile_over_time_p0.9[1m] (merges 4), quantile_over_time_p0.9[10m] (merges 40)
- dd(alpha=0.1) by (series) window 360s slide 15s: quantile_over_time_p0.9[30m] (merges 5), quantile_over_time_p0.9[6h] (merges 60), quantile_over_time_p0.9[1d] (merges 240)

## synthetic/case/perf/long_interval=1800/tp95

| SLA (ms) | asap vCPU (MVs) | perquery vCPU (MVs) | autosketch vCPU (MVs) | ASAP latency ms |
|---|---|---|---|---|
| none | 0.105 (2) | 0.306 (8) | 4.14 (8) | 2.38e+04 |
| 10.0 | 1.24 (5) | 1.55 (8) | 4.14 (8) | 9.45 |
| 30.0 | 0.643 (5) | 0.793 (8) | 4.14 (8) | 21.9 |
| 100.0 | 0.229 (4) | 0.38 (8) | 4.14 (8) | 96.3 |
| 300.0 | 0.155 (4) | 0.306 (8) | 4.14 (8) | 195 |
| 1000.0 | 0.126 (3) | 0.306 (8) | 4.14 (8) | 493 |
| 3000.0 | 0.126 (3) | 0.306 (8) | 4.14 (8) | 493 |
| 10000.0 | 0.122 (3) | 0.306 (8) | 4.14 (8) | 5.95e+03 |

ASAP plan at SLA none:
- dd(alpha=0.1) by (job) window 0.005s slide 0.005s: quantile_by_job_p0.5[1 scrape] (merges 1), quantile_by_job_p0.9[1 scrape] (merges 1)
- dd(alpha=0.1) by (series) window 15s slide 15s: quantile_over_time_p0.5[1m] (merges 4), quantile_over_time_p0.9[1m] (merges 4), quantile_over_time_p0.9[10m] (merges 40), quantile_over_time_p0.9[30m] (merges 120), quantile_over_time_p0.9[6h] (merges 1440), quantile_over_time_p0.9[1d] (merges 5760)

ASAP plan at SLA 10.0:
- dd(alpha=0.1) by (job) window 0.005s slide 0.005s: quantile_by_job_p0.5[1 scrape] (merges 1), quantile_by_job_p0.9[1 scrape] (merges 1)
- dd(alpha=0.1) by (series) window 30s slide 15s: quantile_over_time_p0.5[1m] (merges 2), quantile_over_time_p0.9[1m] (merges 2)
- dd(alpha=0.1) by (series) window 600s slide 60s: quantile_over_time_p0.9[10m] (merges 1), quantile_over_time_p0.9[30m] (merges 3)
- dd(alpha=0.1) by (series) window 7200s slide 1800s: quantile_over_time_p0.9[6h] (merges 3)
- dd(alpha=0.1) by (series) window 28800s slide 1800s: quantile_over_time_p0.9[1d] (merges 3)

ASAP plan at SLA 30.0:
- dd(alpha=0.1) by (job) window 0.005s slide 0.005s: quantile_by_job_p0.5[1 scrape] (merges 1), quantile_by_job_p0.9[1 scrape] (merges 1)
- dd(alpha=0.1) by (series) window 15s slide 15s: quantile_over_time_p0.5[1m] (merges 4), quantile_over_time_p0.9[1m] (merges 4)
- dd(alpha=0.1) by (series) window 300s slide 60s: quantile_over_time_p0.9[10m] (merges 2), quantile_over_time_p0.9[30m] (merges 6)
- dd(alpha=0.1) by (series) window 3600s slide 1800s: quantile_over_time_p0.9[6h] (merges 6)
- dd(alpha=0.1) by (series) window 14400s slide 1800s: quantile_over_time_p0.9[1d] (merges 6)

ASAP plan at SLA 100.0:
- dd(alpha=0.1) by (job) window 0.005s slide 0.005s: quantile_by_job_p0.5[1 scrape] (merges 1), quantile_by_job_p0.9[1 scrape] (merges 1)
- dd(alpha=0.1) by (series) window 15s slide 15s: quantile_over_time_p0.5[1m] (merges 4), quantile_over_time_p0.9[1m] (merges 4)
- dd(alpha=0.1) by (series) window 120s slide 60s: quantile_over_time_p0.9[10m] (merges 5), quantile_over_time_p0.9[30m] (merges 15)
- dd(alpha=0.1) by (series) window 3600s slide 1800s: quantile_over_time_p0.9[6h] (merges 6), quantile_over_time_p0.9[1d] (merges 24)

ASAP plan at SLA 300.0:
- dd(alpha=0.1) by (job) window 0.005s slide 0.005s: quantile_by_job_p0.5[1 scrape] (merges 1), quantile_by_job_p0.9[1 scrape] (merges 1)
- dd(alpha=0.1) by (series) window 15s slide 15s: quantile_over_time_p0.5[1m] (merges 4), quantile_over_time_p0.9[1m] (merges 4)
- dd(alpha=0.1) by (series) window 60s slide 60s: quantile_over_time_p0.9[10m] (merges 10), quantile_over_time_p0.9[30m] (merges 30)
- dd(alpha=0.1) by (series) window 1800s slide 1800s: quantile_over_time_p0.9[6h] (merges 12), quantile_over_time_p0.9[1d] (merges 48)

ASAP plan at SLA 1000.0:
- dd(alpha=0.1) by (job) window 0.005s slide 0.005s: quantile_by_job_p0.5[1 scrape] (merges 1), quantile_by_job_p0.9[1 scrape] (merges 1)
- dd(alpha=0.1) by (series) window 15s slide 15s: quantile_over_time_p0.5[1m] (merges 4), quantile_over_time_p0.9[1m] (merges 4), quantile_over_time_p0.9[10m] (merges 40), quantile_over_time_p0.9[30m] (merges 120)
- dd(alpha=0.1) by (series) window 1800s slide 1800s: quantile_over_time_p0.9[6h] (merges 12), quantile_over_time_p0.9[1d] (merges 48)

ASAP plan at SLA 10000.0:
- dd(alpha=0.1) by (job) window 0.005s slide 0.005s: quantile_by_job_p0.5[1 scrape] (merges 1), quantile_by_job_p0.9[1 scrape] (merges 1)
- dd(alpha=0.1) by (series) window 15s slide 15s: quantile_over_time_p0.5[1m] (merges 4), quantile_over_time_p0.9[1m] (merges 4)
- dd(alpha=0.1) by (series) window 60s slide 60s: quantile_over_time_p0.9[10m] (merges 10), quantile_over_time_p0.9[30m] (merges 30), quantile_over_time_p0.9[6h] (merges 360), quantile_over_time_p0.9[1d] (merges 1440)

## synthetic/case/perf/long_interval=300/tp95

| SLA (ms) | asap vCPU (MVs) | perquery vCPU (MVs) | autosketch vCPU (MVs) | ASAP latency ms |
|---|---|---|---|---|
| none | 0.131 (3) | 0.311 (8) | 15.4 (8) | 1.19e+03 |
| 10.0 | infeasible | infeasible | 15.4 (8) |  |
| 30.0 | 2.53 (5) | 2.68 (8) | 15.4 (8) | 26 |
| 100.0 | 0.606 (4) | 0.833 (8) | 15.4 (8) | 96.3 |
| 300.0 | 0.269 (4) | 0.421 (8) | 15.4 (8) | 295 |
| 1000.0 | 0.166 (3) | 0.346 (8) | 15.4 (8) | 592 |
| 3000.0 | 0.131 (3) | 0.311 (8) | 15.4 (8) | 1.19e+03 |
| 10000.0 | 0.131 (3) | 0.311 (8) | 15.4 (8) | 1.19e+03 |

ASAP plan at SLA none:
- dd(alpha=0.1) by (job) window 0.005s slide 0.005s: quantile_by_job_p0.5[1 scrape] (merges 1), quantile_by_job_p0.9[1 scrape] (merges 1)
- dd(alpha=0.1) by (series) window 15s slide 15s: quantile_over_time_p0.5[1m] (merges 4), quantile_over_time_p0.9[1m] (merges 4), quantile_over_time_p0.9[10m] (merges 40), quantile_over_time_p0.9[30m] (merges 120)
- dd(alpha=0.1) by (series) window 300s slide 300s: quantile_over_time_p0.9[6h] (merges 72), quantile_over_time_p0.9[1d] (merges 288)

ASAP plan at SLA 30.0:
- dd(alpha=0.1) by (job) window 0.005s slide 0.005s: quantile_by_job_p0.5[1 scrape] (merges 1), quantile_by_job_p0.9[1 scrape] (merges 1)
- dd(alpha=0.1) by (series) window 15s slide 15s: quantile_over_time_p0.5[1m] (merges 4), quantile_over_time_p0.9[1m] (merges 4)
- dd(alpha=0.1) by (series) window 300s slide 60s: quantile_over_time_p0.9[10m] (merges 2), quantile_over_time_p0.9[30m] (merges 6)
- dd(alpha=0.1) by (series) window 3600s slide 300s: quantile_over_time_p0.9[6h] (merges 6)
- dd(alpha=0.1) by (series) window 14400s slide 300s: quantile_over_time_p0.9[1d] (merges 6)

ASAP plan at SLA 100.0:
- dd(alpha=0.1) by (job) window 0.005s slide 0.005s: quantile_by_job_p0.5[1 scrape] (merges 1), quantile_by_job_p0.9[1 scrape] (merges 1)
- dd(alpha=0.1) by (series) window 15s slide 15s: quantile_over_time_p0.5[1m] (merges 4), quantile_over_time_p0.9[1m] (merges 4)
- dd(alpha=0.1) by (series) window 120s slide 60s: quantile_over_time_p0.9[10m] (merges 5), quantile_over_time_p0.9[30m] (merges 15)
- dd(alpha=0.1) by (series) window 3600s slide 300s: quantile_over_time_p0.9[6h] (merges 6), quantile_over_time_p0.9[1d] (merges 24)

ASAP plan at SLA 300.0:
- dd(alpha=0.1) by (job) window 0.005s slide 0.005s: quantile_by_job_p0.5[1 scrape] (merges 1), quantile_by_job_p0.9[1 scrape] (merges 1)
- dd(alpha=0.1) by (series) window 15s slide 15s: quantile_over_time_p0.5[1m] (merges 4), quantile_over_time_p0.9[1m] (merges 4)
- dd(alpha=0.1) by (series) window 60s slide 60s: quantile_over_time_p0.9[10m] (merges 10), quantile_over_time_p0.9[30m] (merges 30)
- dd(alpha=0.1) by (series) window 1200s slide 300s: quantile_over_time_p0.9[6h] (merges 18), quantile_over_time_p0.9[1d] (merges 72)

ASAP plan at SLA 1000.0:
- dd(alpha=0.1) by (job) window 0.005s slide 0.005s: quantile_by_job_p0.5[1 scrape] (merges 1), quantile_by_job_p0.9[1 scrape] (merges 1)
- dd(alpha=0.1) by (series) window 15s slide 15s: quantile_over_time_p0.5[1m] (merges 4), quantile_over_time_p0.9[1m] (merges 4), quantile_over_time_p0.9[10m] (merges 40), quantile_over_time_p0.9[30m] (merges 120)
- dd(alpha=0.1) by (series) window 600s slide 300s: quantile_over_time_p0.9[6h] (merges 36), quantile_over_time_p0.9[1d] (merges 144)

## synthetic/case/perf/long_interval=60/tp95

| SLA (ms) | asap vCPU (MVs) | perquery vCPU (MVs) | autosketch vCPU (MVs) | ASAP latency ms |
|---|---|---|---|---|
| none | 0.216 (3) | 0.418 (8) | 69.7 (8) | 2.97e+03 |
| 10.0 | infeasible | infeasible | 69.7 (8) |  |
| 30.0 | infeasible | infeasible | 69.7 (8) |  |
| 100.0 | 2.87 (4) | 3.55 (8) | 69.7 (8) | 88 |
| 300.0 | 0.877 (4) | 1.18 (8) | 69.7 (8) | 295 |
| 1000.0 | 0.327 (3) | 0.561 (8) | 69.7 (8) | 989 |
| 3000.0 | 0.216 (3) | 0.418 (8) | 69.7 (8) | 2.97e+03 |
| 10000.0 | 0.216 (3) | 0.418 (8) | 69.7 (8) | 2.97e+03 |

ASAP plan at SLA none:
- dd(alpha=0.1) by (job) window 0.005s slide 0.005s: quantile_by_job_p0.5[1 scrape] (merges 1), quantile_by_job_p0.9[1 scrape] (merges 1)
- dd(alpha=0.1) by (series) window 15s slide 15s: quantile_over_time_p0.5[1m] (merges 4), quantile_over_time_p0.9[1m] (merges 4)
- dd(alpha=0.1) by (series) window 120s slide 60s: quantile_over_time_p0.9[10m] (merges 5), quantile_over_time_p0.9[30m] (merges 15), quantile_over_time_p0.9[6h] (merges 180), quantile_over_time_p0.9[1d] (merges 720)

ASAP plan at SLA 100.0:
- dd(alpha=0.1) by (job) window 0.005s slide 0.005s: quantile_by_job_p0.5[1 scrape] (merges 1), quantile_by_job_p0.9[1 scrape] (merges 1)
- dd(alpha=0.1) by (series) window 15s slide 15s: quantile_over_time_p0.5[1m] (merges 4), quantile_over_time_p0.9[1m] (merges 4)
- dd(alpha=0.1) by (series) window 120s slide 60s: quantile_over_time_p0.9[10m] (merges 5), quantile_over_time_p0.9[30m] (merges 15)
- dd(alpha=0.1) by (series) window 4320s slide 60s: quantile_over_time_p0.9[6h] (merges 5), quantile_over_time_p0.9[1d] (merges 20)

ASAP plan at SLA 300.0:
- dd(alpha=0.1) by (job) window 0.005s slide 0.005s: quantile_by_job_p0.5[1 scrape] (merges 1), quantile_by_job_p0.9[1 scrape] (merges 1)
- dd(alpha=0.1) by (series) window 15s slide 15s: quantile_over_time_p0.5[1m] (merges 4), quantile_over_time_p0.9[1m] (merges 4)
- dd(alpha=0.1) by (series) window 60s slide 60s: quantile_over_time_p0.9[10m] (merges 10), quantile_over_time_p0.9[30m] (merges 30)
- dd(alpha=0.1) by (series) window 1200s slide 60s: quantile_over_time_p0.9[6h] (merges 18), quantile_over_time_p0.9[1d] (merges 72)

ASAP plan at SLA 1000.0:
- dd(alpha=0.1) by (job) window 0.005s slide 0.005s: quantile_by_job_p0.5[1 scrape] (merges 1), quantile_by_job_p0.9[1 scrape] (merges 1)
- dd(alpha=0.1) by (series) window 15s slide 15s: quantile_over_time_p0.5[1m] (merges 4), quantile_over_time_p0.9[1m] (merges 4), quantile_over_time_p0.9[10m] (merges 40)
- dd(alpha=0.1) by (series) window 360s slide 60s: quantile_over_time_p0.9[30m] (merges 5), quantile_over_time_p0.9[6h] (merges 60), quantile_over_time_p0.9[1d] (merges 240)

## synthetic/case/perf/per_series=1000/tp95

| SLA (ms) | asap vCPU (MVs) | perquery vCPU (MVs) | autosketch vCPU (MVs) | ASAP latency ms |
|---|---|---|---|---|
| none | 0.406 (2) | 1.51 (8) | 20.7 (8) | 2.38e+04 |
| 10.0 | infeasible | infeasible | 20.7 (8) |  |
| 30.0 | 3.2 (5) | 3.96 (8) | 20.7 (8) | 26 |
| 100.0 | 1.13 (4) | 1.89 (8) | 20.7 (8) | 96.3 |
| 300.0 | 0.758 (4) | 1.51 (8) | 20.7 (8) | 195 |
| 1000.0 | 0.578 (3) | 1.51 (8) | 20.7 (8) | 493 |
| 3000.0 | 0.578 (3) | 1.51 (8) | 20.7 (8) | 493 |
| 10000.0 | 0.574 (3) | 1.51 (8) | 20.7 (8) | 5.95e+03 |

ASAP plan at SLA none:
- dd(alpha=0.1) by (job) window 0.001s slide 0.001s: quantile_by_job_p0.5[1 scrape] (merges 1), quantile_by_job_p0.9[1 scrape] (merges 1)
- dd(alpha=0.1) by (series) window 15s slide 15s: quantile_over_time_p0.5[1m] (merges 4), quantile_over_time_p0.9[1m] (merges 4), quantile_over_time_p0.9[10m] (merges 40), quantile_over_time_p0.9[30m] (merges 120), quantile_over_time_p0.9[6h] (merges 1440), quantile_over_time_p0.9[1d] (merges 5760)

ASAP plan at SLA 30.0:
- dd(alpha=0.1) by (job) window 0.001s slide 0.001s: quantile_by_job_p0.5[1 scrape] (merges 1), quantile_by_job_p0.9[1 scrape] (merges 1)
- dd(alpha=0.1) by (series) window 15s slide 15s: quantile_over_time_p0.5[1m] (merges 4), quantile_over_time_p0.9[1m] (merges 4)
- dd(alpha=0.1) by (series) window 300s slide 60s: quantile_over_time_p0.9[10m] (merges 2), quantile_over_time_p0.9[30m] (merges 6)
- dd(alpha=0.1) by (series) window 3600s slide 1800s: quantile_over_time_p0.9[6h] (merges 6)
- dd(alpha=0.1) by (series) window 14400s slide 1800s: quantile_over_time_p0.9[1d] (merges 6)

ASAP plan at SLA 100.0:
- dd(alpha=0.1) by (job) window 0.001s slide 0.001s: quantile_by_job_p0.5[1 scrape] (merges 1), quantile_by_job_p0.9[1 scrape] (merges 1)
- dd(alpha=0.1) by (series) window 15s slide 15s: quantile_over_time_p0.5[1m] (merges 4), quantile_over_time_p0.9[1m] (merges 4)
- dd(alpha=0.1) by (series) window 120s slide 60s: quantile_over_time_p0.9[10m] (merges 5), quantile_over_time_p0.9[30m] (merges 15)
- dd(alpha=0.1) by (series) window 3600s slide 1800s: quantile_over_time_p0.9[6h] (merges 6), quantile_over_time_p0.9[1d] (merges 24)

ASAP plan at SLA 300.0:
- dd(alpha=0.1) by (job) window 0.001s slide 0.001s: quantile_by_job_p0.5[1 scrape] (merges 1), quantile_by_job_p0.9[1 scrape] (merges 1)
- dd(alpha=0.1) by (series) window 15s slide 15s: quantile_over_time_p0.5[1m] (merges 4), quantile_over_time_p0.9[1m] (merges 4)
- dd(alpha=0.1) by (series) window 60s slide 60s: quantile_over_time_p0.9[10m] (merges 10), quantile_over_time_p0.9[30m] (merges 30)
- dd(alpha=0.1) by (series) window 1800s slide 1800s: quantile_over_time_p0.9[6h] (merges 12), quantile_over_time_p0.9[1d] (merges 48)

ASAP plan at SLA 1000.0:
- dd(alpha=0.1) by (job) window 0.001s slide 0.001s: quantile_by_job_p0.5[1 scrape] (merges 1), quantile_by_job_p0.9[1 scrape] (merges 1)
- dd(alpha=0.1) by (series) window 15s slide 15s: quantile_over_time_p0.5[1m] (merges 4), quantile_over_time_p0.9[1m] (merges 4), quantile_over_time_p0.9[10m] (merges 40), quantile_over_time_p0.9[30m] (merges 120)
- dd(alpha=0.1) by (series) window 1800s slide 1800s: quantile_over_time_p0.9[6h] (merges 12), quantile_over_time_p0.9[1d] (merges 48)

ASAP plan at SLA 10000.0:
- dd(alpha=0.1) by (job) window 0.001s slide 0.001s: quantile_by_job_p0.5[1 scrape] (merges 1), quantile_by_job_p0.9[1 scrape] (merges 1)
- dd(alpha=0.1) by (series) window 15s slide 15s: quantile_over_time_p0.5[1m] (merges 4), quantile_over_time_p0.9[1m] (merges 4)
- dd(alpha=0.1) by (series) window 60s slide 60s: quantile_over_time_p0.9[10m] (merges 10), quantile_over_time_p0.9[30m] (merges 30), quantile_over_time_p0.9[6h] (merges 360), quantile_over_time_p0.9[1d] (merges 1440)

## synthetic/case/perf/per_series=100/tp95

| SLA (ms) | asap vCPU (MVs) | perquery vCPU (MVs) | autosketch vCPU (MVs) | ASAP latency ms |
|---|---|---|---|---|
| none | 0.0651 (3) | 0.155 (8) | 2.07 (8) | 5.95e+03 |
| 10.0 | 0.622 (5) | 0.773 (8) | 2.07 (8) | 9.45 |
| 30.0 | 0.322 (5) | 0.398 (8) | 2.07 (8) | 21.9 |
| 100.0 | 0.116 (4) | 0.192 (8) | 2.07 (8) | 96.3 |
| 300.0 | 0.0799 (4) | 0.155 (8) | 2.07 (8) | 195 |
| 1000.0 | 0.0694 (3) | 0.155 (8) | 2.07 (8) | 493 |
| 3000.0 | 0.0694 (3) | 0.155 (8) | 2.07 (8) | 493 |
| 10000.0 | 0.0651 (3) | 0.155 (8) | 2.07 (8) | 5.95e+03 |

ASAP plan at SLA none:
- dd(alpha=0.1) by (job) window 0.01s slide 0.01s: quantile_by_job_p0.5[1 scrape] (merges 1), quantile_by_job_p0.9[1 scrape] (merges 1)
- dd(alpha=0.1) by (series) window 15s slide 15s: quantile_over_time_p0.5[1m] (merges 4), quantile_over_time_p0.9[1m] (merges 4)
- dd(alpha=0.1) by (series) window 60s slide 60s: quantile_over_time_p0.9[10m] (merges 10), quantile_over_time_p0.9[30m] (merges 30), quantile_over_time_p0.9[6h] (merges 360), quantile_over_time_p0.9[1d] (merges 1440)

ASAP plan at SLA 10.0:
- dd(alpha=0.1) by (job) window 0.01s slide 0.01s: quantile_by_job_p0.5[1 scrape] (merges 1), quantile_by_job_p0.9[1 scrape] (merges 1)
- dd(alpha=0.1) by (series) window 30s slide 15s: quantile_over_time_p0.5[1m] (merges 2), quantile_over_time_p0.9[1m] (merges 2)
- dd(alpha=0.1) by (series) window 600s slide 60s: quantile_over_time_p0.9[10m] (merges 1), quantile_over_time_p0.9[30m] (merges 3)
- dd(alpha=0.1) by (series) window 7200s slide 1800s: quantile_over_time_p0.9[6h] (merges 3)
- dd(alpha=0.1) by (series) window 28800s slide 1800s: quantile_over_time_p0.9[1d] (merges 3)

ASAP plan at SLA 30.0:
- dd(alpha=0.1) by (job) window 0.01s slide 0.01s: quantile_by_job_p0.5[1 scrape] (merges 1), quantile_by_job_p0.9[1 scrape] (merges 1)
- dd(alpha=0.1) by (series) window 15s slide 15s: quantile_over_time_p0.5[1m] (merges 4), quantile_over_time_p0.9[1m] (merges 4)
- dd(alpha=0.1) by (series) window 300s slide 60s: quantile_over_time_p0.9[10m] (merges 2), quantile_over_time_p0.9[30m] (merges 6)
- dd(alpha=0.1) by (series) window 3600s slide 1800s: quantile_over_time_p0.9[6h] (merges 6)
- dd(alpha=0.1) by (series) window 14400s slide 1800s: quantile_over_time_p0.9[1d] (merges 6)

ASAP plan at SLA 100.0:
- dd(alpha=0.1) by (job) window 0.01s slide 0.01s: quantile_by_job_p0.5[1 scrape] (merges 1), quantile_by_job_p0.9[1 scrape] (merges 1)
- dd(alpha=0.1) by (series) window 15s slide 15s: quantile_over_time_p0.5[1m] (merges 4), quantile_over_time_p0.9[1m] (merges 4)
- dd(alpha=0.1) by (series) window 120s slide 60s: quantile_over_time_p0.9[10m] (merges 5), quantile_over_time_p0.9[30m] (merges 15)
- dd(alpha=0.1) by (series) window 3600s slide 1800s: quantile_over_time_p0.9[6h] (merges 6), quantile_over_time_p0.9[1d] (merges 24)

ASAP plan at SLA 300.0:
- dd(alpha=0.1) by (job) window 0.01s slide 0.01s: quantile_by_job_p0.5[1 scrape] (merges 1), quantile_by_job_p0.9[1 scrape] (merges 1)
- dd(alpha=0.1) by (series) window 15s slide 15s: quantile_over_time_p0.5[1m] (merges 4), quantile_over_time_p0.9[1m] (merges 4)
- dd(alpha=0.1) by (series) window 60s slide 60s: quantile_over_time_p0.9[10m] (merges 10), quantile_over_time_p0.9[30m] (merges 30)
- dd(alpha=0.1) by (series) window 1800s slide 1800s: quantile_over_time_p0.9[6h] (merges 12), quantile_over_time_p0.9[1d] (merges 48)

ASAP plan at SLA 1000.0:
- dd(alpha=0.1) by (job) window 0.01s slide 0.01s: quantile_by_job_p0.5[1 scrape] (merges 1), quantile_by_job_p0.9[1 scrape] (merges 1)
- dd(alpha=0.1) by (series) window 15s slide 15s: quantile_over_time_p0.5[1m] (merges 4), quantile_over_time_p0.9[1m] (merges 4), quantile_over_time_p0.9[10m] (merges 40), quantile_over_time_p0.9[30m] (merges 120)
- dd(alpha=0.1) by (series) window 1800s slide 1800s: quantile_over_time_p0.9[6h] (merges 12), quantile_over_time_p0.9[1d] (merges 48)

## synthetic/case/perf/per_series=200/tp95

| SLA (ms) | asap vCPU (MVs) | perquery vCPU (MVs) | autosketch vCPU (MVs) | ASAP latency ms |
|---|---|---|---|---|
| none | 0.105 (2) | 0.306 (8) | 4.14 (8) | 2.38e+04 |
| 10.0 | 1.24 (5) | 1.55 (8) | 4.14 (8) | 9.45 |
| 30.0 | 0.643 (5) | 0.793 (8) | 4.14 (8) | 21.9 |
| 100.0 | 0.229 (4) | 0.38 (8) | 4.14 (8) | 96.3 |
| 300.0 | 0.155 (4) | 0.306 (8) | 4.14 (8) | 195 |
| 1000.0 | 0.126 (3) | 0.306 (8) | 4.14 (8) | 493 |
| 3000.0 | 0.126 (3) | 0.306 (8) | 4.14 (8) | 493 |
| 10000.0 | 0.122 (3) | 0.306 (8) | 4.14 (8) | 5.95e+03 |

ASAP plan at SLA none:
- dd(alpha=0.1) by (job) window 0.005s slide 0.005s: quantile_by_job_p0.5[1 scrape] (merges 1), quantile_by_job_p0.9[1 scrape] (merges 1)
- dd(alpha=0.1) by (series) window 15s slide 15s: quantile_over_time_p0.5[1m] (merges 4), quantile_over_time_p0.9[1m] (merges 4), quantile_over_time_p0.9[10m] (merges 40), quantile_over_time_p0.9[30m] (merges 120), quantile_over_time_p0.9[6h] (merges 1440), quantile_over_time_p0.9[1d] (merges 5760)

ASAP plan at SLA 10.0:
- dd(alpha=0.1) by (job) window 0.005s slide 0.005s: quantile_by_job_p0.5[1 scrape] (merges 1), quantile_by_job_p0.9[1 scrape] (merges 1)
- dd(alpha=0.1) by (series) window 30s slide 15s: quantile_over_time_p0.5[1m] (merges 2), quantile_over_time_p0.9[1m] (merges 2)
- dd(alpha=0.1) by (series) window 600s slide 60s: quantile_over_time_p0.9[10m] (merges 1), quantile_over_time_p0.9[30m] (merges 3)
- dd(alpha=0.1) by (series) window 7200s slide 1800s: quantile_over_time_p0.9[6h] (merges 3)
- dd(alpha=0.1) by (series) window 28800s slide 1800s: quantile_over_time_p0.9[1d] (merges 3)

ASAP plan at SLA 30.0:
- dd(alpha=0.1) by (job) window 0.005s slide 0.005s: quantile_by_job_p0.5[1 scrape] (merges 1), quantile_by_job_p0.9[1 scrape] (merges 1)
- dd(alpha=0.1) by (series) window 15s slide 15s: quantile_over_time_p0.5[1m] (merges 4), quantile_over_time_p0.9[1m] (merges 4)
- dd(alpha=0.1) by (series) window 300s slide 60s: quantile_over_time_p0.9[10m] (merges 2), quantile_over_time_p0.9[30m] (merges 6)
- dd(alpha=0.1) by (series) window 3600s slide 1800s: quantile_over_time_p0.9[6h] (merges 6)
- dd(alpha=0.1) by (series) window 14400s slide 1800s: quantile_over_time_p0.9[1d] (merges 6)

ASAP plan at SLA 100.0:
- dd(alpha=0.1) by (job) window 0.005s slide 0.005s: quantile_by_job_p0.5[1 scrape] (merges 1), quantile_by_job_p0.9[1 scrape] (merges 1)
- dd(alpha=0.1) by (series) window 15s slide 15s: quantile_over_time_p0.5[1m] (merges 4), quantile_over_time_p0.9[1m] (merges 4)
- dd(alpha=0.1) by (series) window 120s slide 60s: quantile_over_time_p0.9[10m] (merges 5), quantile_over_time_p0.9[30m] (merges 15)
- dd(alpha=0.1) by (series) window 3600s slide 1800s: quantile_over_time_p0.9[6h] (merges 6), quantile_over_time_p0.9[1d] (merges 24)

ASAP plan at SLA 300.0:
- dd(alpha=0.1) by (job) window 0.005s slide 0.005s: quantile_by_job_p0.5[1 scrape] (merges 1), quantile_by_job_p0.9[1 scrape] (merges 1)
- dd(alpha=0.1) by (series) window 15s slide 15s: quantile_over_time_p0.5[1m] (merges 4), quantile_over_time_p0.9[1m] (merges 4)
- dd(alpha=0.1) by (series) window 60s slide 60s: quantile_over_time_p0.9[10m] (merges 10), quantile_over_time_p0.9[30m] (merges 30)
- dd(alpha=0.1) by (series) window 1800s slide 1800s: quantile_over_time_p0.9[6h] (merges 12), quantile_over_time_p0.9[1d] (merges 48)

ASAP plan at SLA 1000.0:
- dd(alpha=0.1) by (job) window 0.005s slide 0.005s: quantile_by_job_p0.5[1 scrape] (merges 1), quantile_by_job_p0.9[1 scrape] (merges 1)
- dd(alpha=0.1) by (series) window 15s slide 15s: quantile_over_time_p0.5[1m] (merges 4), quantile_over_time_p0.9[1m] (merges 4), quantile_over_time_p0.9[10m] (merges 40), quantile_over_time_p0.9[30m] (merges 120)
- dd(alpha=0.1) by (series) window 1800s slide 1800s: quantile_over_time_p0.9[6h] (merges 12), quantile_over_time_p0.9[1d] (merges 48)

ASAP plan at SLA 10000.0:
- dd(alpha=0.1) by (job) window 0.005s slide 0.005s: quantile_by_job_p0.5[1 scrape] (merges 1), quantile_by_job_p0.9[1 scrape] (merges 1)
- dd(alpha=0.1) by (series) window 15s slide 15s: quantile_over_time_p0.5[1m] (merges 4), quantile_over_time_p0.9[1m] (merges 4)
- dd(alpha=0.1) by (series) window 60s slide 60s: quantile_over_time_p0.9[10m] (merges 10), quantile_over_time_p0.9[30m] (merges 30), quantile_over_time_p0.9[6h] (merges 360), quantile_over_time_p0.9[1d] (merges 1440)

## synthetic/case/perf/per_series=20/tp95

| SLA (ms) | asap vCPU (MVs) | perquery vCPU (MVs) | autosketch vCPU (MVs) | ASAP latency ms |
|---|---|---|---|---|
| none | 0.0197 (4) | 0.0347 (8) | 0.414 (8) | 195 |
| 10.0 | 0.125 (5) | 0.155 (8) | 0.414 (8) | 9.45 |
| 30.0 | 0.0663 (5) | 0.0816 (8) | 0.414 (8) | 21.9 |
| 100.0 | 0.0258 (4) | 0.0412 (8) | 0.414 (8) | 96.3 |
| 300.0 | 0.0197 (4) | 0.0347 (8) | 0.414 (8) | 195 |
| 1000.0 | 0.0197 (4) | 0.0347 (8) | 0.414 (8) | 195 |
| 3000.0 | 0.0197 (4) | 0.0347 (8) | 0.414 (8) | 195 |
| 10000.0 | 0.0197 (4) | 0.0347 (8) | 0.414 (8) | 195 |

ASAP plan at SLA none:
- dd(alpha=0.1) by (job) window 0.05s slide 0.05s: quantile_by_job_p0.5[1 scrape] (merges 1), quantile_by_job_p0.9[1 scrape] (merges 1)
- dd(alpha=0.1) by (series) window 15s slide 15s: quantile_over_time_p0.5[1m] (merges 4), quantile_over_time_p0.9[1m] (merges 4)
- dd(alpha=0.1) by (series) window 60s slide 60s: quantile_over_time_p0.9[10m] (merges 10), quantile_over_time_p0.9[30m] (merges 30)
- dd(alpha=0.1) by (series) window 1800s slide 1800s: quantile_over_time_p0.9[6h] (merges 12), quantile_over_time_p0.9[1d] (merges 48)

ASAP plan at SLA 10.0:
- dd(alpha=0.1) by (job) window 0.05s slide 0.05s: quantile_by_job_p0.5[1 scrape] (merges 1), quantile_by_job_p0.9[1 scrape] (merges 1)
- dd(alpha=0.1) by (series) window 30s slide 15s: quantile_over_time_p0.5[1m] (merges 2), quantile_over_time_p0.9[1m] (merges 2)
- dd(alpha=0.1) by (series) window 600s slide 60s: quantile_over_time_p0.9[10m] (merges 1), quantile_over_time_p0.9[30m] (merges 3)
- dd(alpha=0.1) by (series) window 7200s slide 1800s: quantile_over_time_p0.9[6h] (merges 3)
- dd(alpha=0.1) by (series) window 28800s slide 1800s: quantile_over_time_p0.9[1d] (merges 3)

ASAP plan at SLA 30.0:
- dd(alpha=0.1) by (job) window 0.05s slide 0.05s: quantile_by_job_p0.5[1 scrape] (merges 1), quantile_by_job_p0.9[1 scrape] (merges 1)
- dd(alpha=0.1) by (series) window 15s slide 15s: quantile_over_time_p0.5[1m] (merges 4), quantile_over_time_p0.9[1m] (merges 4)
- dd(alpha=0.1) by (series) window 300s slide 60s: quantile_over_time_p0.9[10m] (merges 2), quantile_over_time_p0.9[30m] (merges 6)
- dd(alpha=0.1) by (series) window 3600s slide 1800s: quantile_over_time_p0.9[6h] (merges 6)
- dd(alpha=0.1) by (series) window 14400s slide 1800s: quantile_over_time_p0.9[1d] (merges 6)

ASAP plan at SLA 100.0:
- dd(alpha=0.1) by (job) window 0.05s slide 0.05s: quantile_by_job_p0.5[1 scrape] (merges 1), quantile_by_job_p0.9[1 scrape] (merges 1)
- dd(alpha=0.1) by (series) window 15s slide 15s: quantile_over_time_p0.5[1m] (merges 4), quantile_over_time_p0.9[1m] (merges 4)
- dd(alpha=0.1) by (series) window 120s slide 60s: quantile_over_time_p0.9[10m] (merges 5), quantile_over_time_p0.9[30m] (merges 15)
- dd(alpha=0.1) by (series) window 3600s slide 1800s: quantile_over_time_p0.9[6h] (merges 6), quantile_over_time_p0.9[1d] (merges 24)

## synthetic/case/perf/per_series=500/tp95

| SLA (ms) | asap vCPU (MVs) | perquery vCPU (MVs) | autosketch vCPU (MVs) | ASAP latency ms |
|---|---|---|---|---|
| none | 0.218 (2) | 0.758 (8) | 10.4 (8) | 2.38e+04 |
| 10.0 | infeasible | infeasible | 10.4 (8) |  |
| 30.0 | 1.6 (5) | 1.98 (8) | 10.4 (8) | 21.9 |
| 100.0 | 0.568 (4) | 0.945 (8) | 10.4 (8) | 96.3 |
| 300.0 | 0.381 (4) | 0.758 (8) | 10.4 (8) | 195 |
| 1000.0 | 0.295 (3) | 0.758 (8) | 10.4 (8) | 493 |
| 3000.0 | 0.295 (3) | 0.758 (8) | 10.4 (8) | 493 |
| 10000.0 | 0.291 (3) | 0.758 (8) | 10.4 (8) | 5.95e+03 |

ASAP plan at SLA none:
- dd(alpha=0.1) by (job) window 0.002s slide 0.002s: quantile_by_job_p0.5[1 scrape] (merges 1), quantile_by_job_p0.9[1 scrape] (merges 1)
- dd(alpha=0.1) by (series) window 15s slide 15s: quantile_over_time_p0.5[1m] (merges 4), quantile_over_time_p0.9[1m] (merges 4), quantile_over_time_p0.9[10m] (merges 40), quantile_over_time_p0.9[30m] (merges 120), quantile_over_time_p0.9[6h] (merges 1440), quantile_over_time_p0.9[1d] (merges 5760)

ASAP plan at SLA 30.0:
- dd(alpha=0.1) by (job) window 0.002s slide 0.002s: quantile_by_job_p0.5[1 scrape] (merges 1), quantile_by_job_p0.9[1 scrape] (merges 1)
- dd(alpha=0.1) by (series) window 15s slide 15s: quantile_over_time_p0.5[1m] (merges 4), quantile_over_time_p0.9[1m] (merges 4)
- dd(alpha=0.1) by (series) window 300s slide 60s: quantile_over_time_p0.9[10m] (merges 2), quantile_over_time_p0.9[30m] (merges 6)
- dd(alpha=0.1) by (series) window 3600s slide 1800s: quantile_over_time_p0.9[6h] (merges 6)
- dd(alpha=0.1) by (series) window 14400s slide 1800s: quantile_over_time_p0.9[1d] (merges 6)

ASAP plan at SLA 100.0:
- dd(alpha=0.1) by (job) window 0.002s slide 0.002s: quantile_by_job_p0.5[1 scrape] (merges 1), quantile_by_job_p0.9[1 scrape] (merges 1)
- dd(alpha=0.1) by (series) window 15s slide 15s: quantile_over_time_p0.5[1m] (merges 4), quantile_over_time_p0.9[1m] (merges 4)
- dd(alpha=0.1) by (series) window 120s slide 60s: quantile_over_time_p0.9[10m] (merges 5), quantile_over_time_p0.9[30m] (merges 15)
- dd(alpha=0.1) by (series) window 3600s slide 1800s: quantile_over_time_p0.9[6h] (merges 6), quantile_over_time_p0.9[1d] (merges 24)

ASAP plan at SLA 300.0:
- dd(alpha=0.1) by (job) window 0.002s slide 0.002s: quantile_by_job_p0.5[1 scrape] (merges 1), quantile_by_job_p0.9[1 scrape] (merges 1)
- dd(alpha=0.1) by (series) window 15s slide 15s: quantile_over_time_p0.5[1m] (merges 4), quantile_over_time_p0.9[1m] (merges 4)
- dd(alpha=0.1) by (series) window 60s slide 60s: quantile_over_time_p0.9[10m] (merges 10), quantile_over_time_p0.9[30m] (merges 30)
- dd(alpha=0.1) by (series) window 1800s slide 1800s: quantile_over_time_p0.9[6h] (merges 12), quantile_over_time_p0.9[1d] (merges 48)

ASAP plan at SLA 1000.0:
- dd(alpha=0.1) by (job) window 0.002s slide 0.002s: quantile_by_job_p0.5[1 scrape] (merges 1), quantile_by_job_p0.9[1 scrape] (merges 1)
- dd(alpha=0.1) by (series) window 15s slide 15s: quantile_over_time_p0.5[1m] (merges 4), quantile_over_time_p0.9[1m] (merges 4), quantile_over_time_p0.9[10m] (merges 40), quantile_over_time_p0.9[30m] (merges 120)
- dd(alpha=0.1) by (series) window 1800s slide 1800s: quantile_over_time_p0.9[6h] (merges 12), quantile_over_time_p0.9[1d] (merges 48)

ASAP plan at SLA 10000.0:
- dd(alpha=0.1) by (job) window 0.002s slide 0.002s: quantile_by_job_p0.5[1 scrape] (merges 1), quantile_by_job_p0.9[1 scrape] (merges 1)
- dd(alpha=0.1) by (series) window 15s slide 15s: quantile_over_time_p0.5[1m] (merges 4), quantile_over_time_p0.9[1m] (merges 4)
- dd(alpha=0.1) by (series) window 60s slide 60s: quantile_over_time_p0.9[10m] (merges 10), quantile_over_time_p0.9[30m] (merges 30), quantile_over_time_p0.9[6h] (merges 360), quantile_over_time_p0.9[1d] (merges 1440)

## synthetic/case/perf/per_series=50/tp95

| SLA (ms) | asap vCPU (MVs) | perquery vCPU (MVs) | autosketch vCPU (MVs) | ASAP latency ms |
|---|---|---|---|---|
| none | 0.0368 (3) | 0.0799 (8) | 1.04 (8) | 5.95e+03 |
| 10.0 | 0.312 (5) | 0.387 (8) | 1.04 (8) | 9.45 |
| 30.0 | 0.162 (5) | 0.2 (8) | 1.04 (8) | 21.9 |
| 100.0 | 0.0597 (4) | 0.0977 (8) | 1.04 (8) | 96.3 |
| 300.0 | 0.0423 (4) | 0.0799 (8) | 1.04 (8) | 195 |
| 1000.0 | 0.0411 (3) | 0.0799 (8) | 1.04 (8) | 493 |
| 3000.0 | 0.0411 (3) | 0.0799 (8) | 1.04 (8) | 493 |
| 10000.0 | 0.0368 (3) | 0.0799 (8) | 1.04 (8) | 5.95e+03 |

ASAP plan at SLA none:
- dd(alpha=0.1) by (job) window 0.02s slide 0.02s: quantile_by_job_p0.5[1 scrape] (merges 1), quantile_by_job_p0.9[1 scrape] (merges 1)
- dd(alpha=0.1) by (series) window 15s slide 15s: quantile_over_time_p0.5[1m] (merges 4), quantile_over_time_p0.9[1m] (merges 4)
- dd(alpha=0.1) by (series) window 60s slide 60s: quantile_over_time_p0.9[10m] (merges 10), quantile_over_time_p0.9[30m] (merges 30), quantile_over_time_p0.9[6h] (merges 360), quantile_over_time_p0.9[1d] (merges 1440)

ASAP plan at SLA 10.0:
- dd(alpha=0.1) by (job) window 0.02s slide 0.02s: quantile_by_job_p0.5[1 scrape] (merges 1), quantile_by_job_p0.9[1 scrape] (merges 1)
- dd(alpha=0.1) by (series) window 30s slide 15s: quantile_over_time_p0.5[1m] (merges 2), quantile_over_time_p0.9[1m] (merges 2)
- dd(alpha=0.1) by (series) window 600s slide 60s: quantile_over_time_p0.9[10m] (merges 1), quantile_over_time_p0.9[30m] (merges 3)
- dd(alpha=0.1) by (series) window 7200s slide 1800s: quantile_over_time_p0.9[6h] (merges 3)
- dd(alpha=0.1) by (series) window 28800s slide 1800s: quantile_over_time_p0.9[1d] (merges 3)

ASAP plan at SLA 30.0:
- dd(alpha=0.1) by (job) window 0.02s slide 0.02s: quantile_by_job_p0.5[1 scrape] (merges 1), quantile_by_job_p0.9[1 scrape] (merges 1)
- dd(alpha=0.1) by (series) window 15s slide 15s: quantile_over_time_p0.5[1m] (merges 4), quantile_over_time_p0.9[1m] (merges 4)
- dd(alpha=0.1) by (series) window 300s slide 60s: quantile_over_time_p0.9[10m] (merges 2), quantile_over_time_p0.9[30m] (merges 6)
- dd(alpha=0.1) by (series) window 3600s slide 1800s: quantile_over_time_p0.9[6h] (merges 6)
- dd(alpha=0.1) by (series) window 14400s slide 1800s: quantile_over_time_p0.9[1d] (merges 6)

ASAP plan at SLA 100.0:
- dd(alpha=0.1) by (job) window 0.02s slide 0.02s: quantile_by_job_p0.5[1 scrape] (merges 1), quantile_by_job_p0.9[1 scrape] (merges 1)
- dd(alpha=0.1) by (series) window 15s slide 15s: quantile_over_time_p0.5[1m] (merges 4), quantile_over_time_p0.9[1m] (merges 4)
- dd(alpha=0.1) by (series) window 120s slide 60s: quantile_over_time_p0.9[10m] (merges 5), quantile_over_time_p0.9[30m] (merges 15)
- dd(alpha=0.1) by (series) window 3600s slide 1800s: quantile_over_time_p0.9[6h] (merges 6), quantile_over_time_p0.9[1d] (merges 24)

ASAP plan at SLA 300.0:
- dd(alpha=0.1) by (job) window 0.02s slide 0.02s: quantile_by_job_p0.5[1 scrape] (merges 1), quantile_by_job_p0.9[1 scrape] (merges 1)
- dd(alpha=0.1) by (series) window 15s slide 15s: quantile_over_time_p0.5[1m] (merges 4), quantile_over_time_p0.9[1m] (merges 4)
- dd(alpha=0.1) by (series) window 60s slide 60s: quantile_over_time_p0.9[10m] (merges 10), quantile_over_time_p0.9[30m] (merges 30)
- dd(alpha=0.1) by (series) window 1800s slide 1800s: quantile_over_time_p0.9[6h] (merges 12), quantile_over_time_p0.9[1d] (merges 48)

ASAP plan at SLA 1000.0:
- dd(alpha=0.1) by (job) window 0.02s slide 0.02s: quantile_by_job_p0.5[1 scrape] (merges 1), quantile_by_job_p0.9[1 scrape] (merges 1)
- dd(alpha=0.1) by (series) window 15s slide 15s: quantile_over_time_p0.5[1m] (merges 4), quantile_over_time_p0.9[1m] (merges 4), quantile_over_time_p0.9[10m] (merges 40), quantile_over_time_p0.9[30m] (merges 120)
- dd(alpha=0.1) by (series) window 1800s slide 1800s: quantile_over_time_p0.9[6h] (merges 12), quantile_over_time_p0.9[1d] (merges 48)

## synthetic/case/perf/series=100000/tp95

| SLA (ms) | asap vCPU (MVs) | perquery vCPU (MVs) | autosketch vCPU (MVs) | ASAP latency ms |
|---|---|---|---|---|
| none | 1.05 (2) | 3.06 (8) | 41.4 (8) | 2.38e+05 |
| 10.0 | infeasible | infeasible | 41.4 (8) |  |
| 30.0 | infeasible | infeasible | 41.4 (8) |  |
| 100.0 | infeasible | infeasible | 41.4 (8) |  |
| 300.0 | infeasible | infeasible | 41.4 (8) |  |
| 1000.0 | 2.29 (4) | 3.8 (8) | 41.4 (8) | 963 |
| 3000.0 | 1.55 (4) | 3.06 (8) | 41.4 (8) | 1.95e+03 |
| 10000.0 | 1.26 (3) | 3.06 (8) | 41.4 (8) | 4.93e+03 |

ASAP plan at SLA none:
- dd(alpha=0.1) by (job) window 0.005s slide 0.005s: quantile_by_job_p0.5[1 scrape] (merges 1), quantile_by_job_p0.9[1 scrape] (merges 1)
- dd(alpha=0.1) by (series) window 15s slide 15s: quantile_over_time_p0.5[1m] (merges 4), quantile_over_time_p0.9[1m] (merges 4), quantile_over_time_p0.9[10m] (merges 40), quantile_over_time_p0.9[30m] (merges 120), quantile_over_time_p0.9[6h] (merges 1440), quantile_over_time_p0.9[1d] (merges 5760)

ASAP plan at SLA 1000.0:
- dd(alpha=0.1) by (job) window 0.005s slide 0.005s: quantile_by_job_p0.5[1 scrape] (merges 1), quantile_by_job_p0.9[1 scrape] (merges 1)
- dd(alpha=0.1) by (series) window 15s slide 15s: quantile_over_time_p0.5[1m] (merges 4), quantile_over_time_p0.9[1m] (merges 4)
- dd(alpha=0.1) by (series) window 120s slide 60s: quantile_over_time_p0.9[10m] (merges 5), quantile_over_time_p0.9[30m] (merges 15)
- dd(alpha=0.1) by (series) window 3600s slide 1800s: quantile_over_time_p0.9[6h] (merges 6), quantile_over_time_p0.9[1d] (merges 24)

ASAP plan at SLA 3000.0:
- dd(alpha=0.1) by (job) window 0.005s slide 0.005s: quantile_by_job_p0.5[1 scrape] (merges 1), quantile_by_job_p0.9[1 scrape] (merges 1)
- dd(alpha=0.1) by (series) window 15s slide 15s: quantile_over_time_p0.5[1m] (merges 4), quantile_over_time_p0.9[1m] (merges 4)
- dd(alpha=0.1) by (series) window 60s slide 60s: quantile_over_time_p0.9[10m] (merges 10), quantile_over_time_p0.9[30m] (merges 30)
- dd(alpha=0.1) by (series) window 1800s slide 1800s: quantile_over_time_p0.9[6h] (merges 12), quantile_over_time_p0.9[1d] (merges 48)

ASAP plan at SLA 10000.0:
- dd(alpha=0.1) by (job) window 0.005s slide 0.005s: quantile_by_job_p0.5[1 scrape] (merges 1), quantile_by_job_p0.9[1 scrape] (merges 1)
- dd(alpha=0.1) by (series) window 15s slide 15s: quantile_over_time_p0.5[1m] (merges 4), quantile_over_time_p0.9[1m] (merges 4), quantile_over_time_p0.9[10m] (merges 40), quantile_over_time_p0.9[30m] (merges 120)
- dd(alpha=0.1) by (series) window 1800s slide 1800s: quantile_over_time_p0.9[6h] (merges 12), quantile_over_time_p0.9[1d] (merges 48)

## synthetic/case/perf/series=10000/tp95

| SLA (ms) | asap vCPU (MVs) | perquery vCPU (MVs) | autosketch vCPU (MVs) | ASAP latency ms |
|---|---|---|---|---|
| none | 0.105 (2) | 0.306 (8) | 4.14 (8) | 2.38e+04 |
| 10.0 | 1.24 (5) | 1.55 (8) | 4.14 (8) | 9.45 |
| 30.0 | 0.643 (5) | 0.793 (8) | 4.14 (8) | 21.9 |
| 100.0 | 0.229 (4) | 0.38 (8) | 4.14 (8) | 96.3 |
| 300.0 | 0.155 (4) | 0.306 (8) | 4.14 (8) | 195 |
| 1000.0 | 0.126 (3) | 0.306 (8) | 4.14 (8) | 493 |
| 3000.0 | 0.126 (3) | 0.306 (8) | 4.14 (8) | 493 |
| 10000.0 | 0.122 (3) | 0.306 (8) | 4.14 (8) | 5.95e+03 |

ASAP plan at SLA none:
- dd(alpha=0.1) by (job) window 0.005s slide 0.005s: quantile_by_job_p0.5[1 scrape] (merges 1), quantile_by_job_p0.9[1 scrape] (merges 1)
- dd(alpha=0.1) by (series) window 15s slide 15s: quantile_over_time_p0.5[1m] (merges 4), quantile_over_time_p0.9[1m] (merges 4), quantile_over_time_p0.9[10m] (merges 40), quantile_over_time_p0.9[30m] (merges 120), quantile_over_time_p0.9[6h] (merges 1440), quantile_over_time_p0.9[1d] (merges 5760)

ASAP plan at SLA 10.0:
- dd(alpha=0.1) by (job) window 0.005s slide 0.005s: quantile_by_job_p0.5[1 scrape] (merges 1), quantile_by_job_p0.9[1 scrape] (merges 1)
- dd(alpha=0.1) by (series) window 30s slide 15s: quantile_over_time_p0.5[1m] (merges 2), quantile_over_time_p0.9[1m] (merges 2)
- dd(alpha=0.1) by (series) window 600s slide 60s: quantile_over_time_p0.9[10m] (merges 1), quantile_over_time_p0.9[30m] (merges 3)
- dd(alpha=0.1) by (series) window 7200s slide 1800s: quantile_over_time_p0.9[6h] (merges 3)
- dd(alpha=0.1) by (series) window 28800s slide 1800s: quantile_over_time_p0.9[1d] (merges 3)

ASAP plan at SLA 30.0:
- dd(alpha=0.1) by (job) window 0.005s slide 0.005s: quantile_by_job_p0.5[1 scrape] (merges 1), quantile_by_job_p0.9[1 scrape] (merges 1)
- dd(alpha=0.1) by (series) window 15s slide 15s: quantile_over_time_p0.5[1m] (merges 4), quantile_over_time_p0.9[1m] (merges 4)
- dd(alpha=0.1) by (series) window 300s slide 60s: quantile_over_time_p0.9[10m] (merges 2), quantile_over_time_p0.9[30m] (merges 6)
- dd(alpha=0.1) by (series) window 3600s slide 1800s: quantile_over_time_p0.9[6h] (merges 6)
- dd(alpha=0.1) by (series) window 14400s slide 1800s: quantile_over_time_p0.9[1d] (merges 6)

ASAP plan at SLA 100.0:
- dd(alpha=0.1) by (job) window 0.005s slide 0.005s: quantile_by_job_p0.5[1 scrape] (merges 1), quantile_by_job_p0.9[1 scrape] (merges 1)
- dd(alpha=0.1) by (series) window 15s slide 15s: quantile_over_time_p0.5[1m] (merges 4), quantile_over_time_p0.9[1m] (merges 4)
- dd(alpha=0.1) by (series) window 120s slide 60s: quantile_over_time_p0.9[10m] (merges 5), quantile_over_time_p0.9[30m] (merges 15)
- dd(alpha=0.1) by (series) window 3600s slide 1800s: quantile_over_time_p0.9[6h] (merges 6), quantile_over_time_p0.9[1d] (merges 24)

ASAP plan at SLA 300.0:
- dd(alpha=0.1) by (job) window 0.005s slide 0.005s: quantile_by_job_p0.5[1 scrape] (merges 1), quantile_by_job_p0.9[1 scrape] (merges 1)
- dd(alpha=0.1) by (series) window 15s slide 15s: quantile_over_time_p0.5[1m] (merges 4), quantile_over_time_p0.9[1m] (merges 4)
- dd(alpha=0.1) by (series) window 60s slide 60s: quantile_over_time_p0.9[10m] (merges 10), quantile_over_time_p0.9[30m] (merges 30)
- dd(alpha=0.1) by (series) window 1800s slide 1800s: quantile_over_time_p0.9[6h] (merges 12), quantile_over_time_p0.9[1d] (merges 48)

ASAP plan at SLA 1000.0:
- dd(alpha=0.1) by (job) window 0.005s slide 0.005s: quantile_by_job_p0.5[1 scrape] (merges 1), quantile_by_job_p0.9[1 scrape] (merges 1)
- dd(alpha=0.1) by (series) window 15s slide 15s: quantile_over_time_p0.5[1m] (merges 4), quantile_over_time_p0.9[1m] (merges 4), quantile_over_time_p0.9[10m] (merges 40), quantile_over_time_p0.9[30m] (merges 120)
- dd(alpha=0.1) by (series) window 1800s slide 1800s: quantile_over_time_p0.9[6h] (merges 12), quantile_over_time_p0.9[1d] (merges 48)

ASAP plan at SLA 10000.0:
- dd(alpha=0.1) by (job) window 0.005s slide 0.005s: quantile_by_job_p0.5[1 scrape] (merges 1), quantile_by_job_p0.9[1 scrape] (merges 1)
- dd(alpha=0.1) by (series) window 15s slide 15s: quantile_over_time_p0.5[1m] (merges 4), quantile_over_time_p0.9[1m] (merges 4)
- dd(alpha=0.1) by (series) window 60s slide 60s: quantile_over_time_p0.9[10m] (merges 10), quantile_over_time_p0.9[30m] (merges 30), quantile_over_time_p0.9[6h] (merges 360), quantile_over_time_p0.9[1d] (merges 1440)

## synthetic/case/perf/series=30000/tp95

| SLA (ms) | asap vCPU (MVs) | perquery vCPU (MVs) | autosketch vCPU (MVs) | ASAP latency ms |
|---|---|---|---|---|
| none | 0.314 (2) | 0.918 (8) | 12.4 (8) | 7.14e+04 |
| 10.0 | infeasible | infeasible | 12.4 (8) |  |
| 30.0 | infeasible | infeasible | 12.4 (8) |  |
| 100.0 | 1.48 (4) | 2.15 (8) | 12.4 (8) | 90.4 |
| 300.0 | 0.687 (4) | 1.14 (8) | 12.4 (8) | 289 |
| 1000.0 | 0.466 (4) | 0.918 (8) | 12.4 (8) | 586 |
| 3000.0 | 0.378 (3) | 0.918 (8) | 12.4 (8) | 1.48e+03 |
| 10000.0 | 0.378 (3) | 0.918 (8) | 12.4 (8) | 1.48e+03 |

ASAP plan at SLA none:
- dd(alpha=0.1) by (job) window 0.005s slide 0.005s: quantile_by_job_p0.5[1 scrape] (merges 1), quantile_by_job_p0.9[1 scrape] (merges 1)
- dd(alpha=0.1) by (series) window 15s slide 15s: quantile_over_time_p0.5[1m] (merges 4), quantile_over_time_p0.9[1m] (merges 4), quantile_over_time_p0.9[10m] (merges 40), quantile_over_time_p0.9[30m] (merges 120), quantile_over_time_p0.9[6h] (merges 1440), quantile_over_time_p0.9[1d] (merges 5760)

ASAP plan at SLA 100.0:
- dd(alpha=0.1) by (job) window 0.005s slide 0.005s: quantile_by_job_p0.5[1 scrape] (merges 1), quantile_by_job_p0.9[1 scrape] (merges 1)
- dd(alpha=0.1) by (series) window 15s slide 15s: quantile_over_time_p0.5[1m] (merges 4), quantile_over_time_p0.9[1m] (merges 4)
- dd(alpha=0.1) by (series) window 300s slide 60s: quantile_over_time_p0.9[10m] (merges 2), quantile_over_time_p0.9[30m] (merges 6)
- dd(alpha=0.1) by (series) window 10800s slide 1800s: quantile_over_time_p0.9[6h] (merges 2), quantile_over_time_p0.9[1d] (merges 8)

ASAP plan at SLA 300.0:
- dd(alpha=0.1) by (job) window 0.005s slide 0.005s: quantile_by_job_p0.5[1 scrape] (merges 1), quantile_by_job_p0.9[1 scrape] (merges 1)
- dd(alpha=0.1) by (series) window 15s slide 15s: quantile_over_time_p0.5[1m] (merges 4), quantile_over_time_p0.9[1m] (merges 4)
- dd(alpha=0.1) by (series) window 120s slide 60s: quantile_over_time_p0.9[10m] (merges 5), quantile_over_time_p0.9[30m] (merges 15)
- dd(alpha=0.1) by (series) window 3600s slide 1800s: quantile_over_time_p0.9[6h] (merges 6), quantile_over_time_p0.9[1d] (merges 24)

ASAP plan at SLA 1000.0:
- dd(alpha=0.1) by (job) window 0.005s slide 0.005s: quantile_by_job_p0.5[1 scrape] (merges 1), quantile_by_job_p0.9[1 scrape] (merges 1)
- dd(alpha=0.1) by (series) window 15s slide 15s: quantile_over_time_p0.5[1m] (merges 4), quantile_over_time_p0.9[1m] (merges 4)
- dd(alpha=0.1) by (series) window 60s slide 60s: quantile_over_time_p0.9[10m] (merges 10), quantile_over_time_p0.9[30m] (merges 30)
- dd(alpha=0.1) by (series) window 1800s slide 1800s: quantile_over_time_p0.9[6h] (merges 12), quantile_over_time_p0.9[1d] (merges 48)

ASAP plan at SLA 3000.0:
- dd(alpha=0.1) by (job) window 0.005s slide 0.005s: quantile_by_job_p0.5[1 scrape] (merges 1), quantile_by_job_p0.9[1 scrape] (merges 1)
- dd(alpha=0.1) by (series) window 15s slide 15s: quantile_over_time_p0.5[1m] (merges 4), quantile_over_time_p0.9[1m] (merges 4), quantile_over_time_p0.9[10m] (merges 40), quantile_over_time_p0.9[30m] (merges 120)
- dd(alpha=0.1) by (series) window 1800s slide 1800s: quantile_over_time_p0.9[6h] (merges 12), quantile_over_time_p0.9[1d] (merges 48)
