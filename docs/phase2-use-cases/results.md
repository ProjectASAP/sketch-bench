# Phase 2 Results: Approximate versus Exact on Five Use Cases

## Setup

- Each use case has one workload: data, a set of PromQL queries, and a set of decision rules. ASAPPlanner gives each query an exact plan and an approximate plan, and both run in planeval. The rules read the query results of each side and decide on their own.
- Approximate configurations (arms):
  - `raw`: raw data is retained, and the planner may read raw samples.
  - `noraw`: raw data is not retained.
  - `ignore`: the planner may use sketches without an accuracy guarantee.
  - `epsX`: error target X.
- Measures:
  - **Query error**: the largest error over all evaluations. Value queries use relative error; quantile queries use rank error.
  - **Decision agreement**: the share of decisions where approximate and exact agree. Threshold rules and per-entity rules count entity × evaluation; scaling counts actions, matched within 2 evaluations; canary counts the final decision of each rollout; the top-10 set counts the mean overlap.
  - **Query latency**: the sum over all queries of the mean latency per evaluation, without plan compile time, in ms.
  - **Memory**: the sum over all queries of the peak per evaluation, as sketch state bytes plus raw sample bytes read at query time (16 bytes per sample), in MB.
- Each configuration runs once; rules read the approximate answers directly, without error intervals. Real data is replayed at its native speed.

## Summary: approximate compared with exact

| Use case | Data | Query latency | Memory |
|---|---|---|---|
| DDoS detection | Synthetic | 5 of 7 configurations 2.8–120× faster; the two eps0.01 configurations 6.6–12× slower | 7 of 7 save 56–99% |
| DDoS detection | CAIDA 2019 | 5 of 7 1.9–343× faster; the two eps0.01 configurations 8.3–11× slower | 7 of 7 save 52–99% |
| DDoS detection | MAWI 2019 | 5 of 7 2.1–231× faster; the two eps0.01 configurations 2.4–2.7× slower | 7 of 7 save 40–98% |
| Resource scaling | Synthetic | noraw 4 of 4 5.9–24× faster; raw 1.1× slower | noraw saves 7–8%; raw the same |
| Resource scaling | Google 2011 | noraw 4 of 4 2.1–2.2× slower; raw the same | noraw uses 41× more; raw the same |
| Video experience | Synthetic | 3 of 3 5.5–6.8× faster | 3 of 3 use 62× more |
| Video experience | Puffer 2024 | 3 of 3 3.9–5.5× faster | 3 of 3 use 89× more |
| Canary release | Synthetic | noraw 4 of 4 45–70× faster; raw 1.1× faster | noraw saves 83–99%; raw the same |
| Canary release | Alibaba 2021 | 5 of 5 162–224× faster | 5 of 5 save 94–99.6% |
| Rate limiting | Synthetic | 4 of 4 88–475× faster | 4 of 4 save 46–99% |
| Rate limiting | CAIDA 2019 | 4 of 4 2,849–9,289× faster | 4 of 4 save 98–99.9% |
| Rate limiting | MAWI 2019 | 4 of 4 1,564–6,152× faster | 4 of 4 save 97–99.9% |
| Rate limiting | NASA 1995 | 4 of 4 1.3–6.2× faster | 4 of 4 use 1.2–8.5× more |

---

## 1. DDoS detection

**Data**
- Synthetic, 30 minutes: a stream of SYN packets to one server, one packet every 1 ms at baseline; the value is the source IP.
  - Normal traffic: Zipf, skew 1.1, over 2,000 addresses.
  - Attack 1, minutes 5 to 11: 60% of packets come from addresses spread uniformly over [10^6, 2^32), and the packet rate rises 25×, ramping in and out over 30 s.
  - Attack 2, minutes 20 to 24: 40% of packets come from such addresses, and the packet rate rises 10×, ramping over 1 minute.
- Real, SYN packets only, no injected attack:
  - CAIDA 2019 equinix-nyc dirA, 585 s
  - MAWI samplepoint-F 2019-01-16, 895 s

**Queries** (evaluated every 5 s)

| ID | PromQL |
|---|---|
| entropy_10s | `entropy_over_time(src_ip{vm="instance1"}[10s])` |
| distinct_10s | `distinct_over_time(src_ip{vm="instance1"}[10s])` |
| distinct_5s | `distinct_over_time(src_ip{vm="instance1"}[5s])` |
| packets_5s | `count_over_time(src_ip{vm="instance1"}[5s])` |
| distinct_1m | `distinct_over_time(src_ip{vm="instance1"}[1m])` |

**Decision rules**
- PromSketch thresholds, one per query: entropy_10s > 8.5, distinct_10s > 3,000, distinct_5s > 1,700, packets_5s > 100,000, distinct_1m > 18,000.
- PromSketch for 15s: any of the first three thresholds holds for 4 evaluations in a row.
- Datadog: distinct_10s above the mean plus 3 standard deviations of the previous 5 minutes.
- Kentik: distinct_10s above 200% of the mean of the previous 5 minutes, at least 3 times in the last 6 evaluations.
- NetFlow Logic: one-sided cumulative sum on entropy_10s, with the mean and standard deviation from the first 50 evaluations, slack 0.5, and an alarm at 5 standard deviations.
- FastNetMon: more than 20,000 packets per second averaged over 5 s; ban for 1,900 s, and until the attack ends.
- Red Hat: more than 300 flows per second over 1 minute, counting each source address as one flow, for 15 s.

**Results**

| Data | Configuration | Query error | Decision agreement | Latency ms exact | approx | Change | Memory MB exact | approx | Change |
|---|---|---|---|---|---|---|---|---|---|
| Synthetic | raw-eps0.05 | distinct 3.2–3.9%; others 0 | All 100% | 482 | 175 | 2.8× faster | 36.0 | 15.9 | 56% less |
| Synthetic | noraw-eps0.1 | distinct 6.7–10.2%; entropy has no plan | Kentik 99.7%; others 100% | 449 | 4.4 | 102× faster | 32.0 | 0.25 | 99% less |
| Synthetic | noraw-eps0.05 | distinct 3.2–3.9%; entropy has no plan | All 100% | 449 | 50 | 9.0× faster | 32.0 | 4.9 | 85% less |
| Synthetic | noraw-eps0.01 | distinct ≤ 0.59%; entropy has no plan | All 100% | 449 | 5,571 | 12× slower | 32.0 | 11.5 | 64% less |
| Synthetic | noraw-ignore-eps0.1 | entropy 1.8%, distinct 6.7–10.2% | entropy threshold 99.7%, Kentik 99.7%, NetFlow Logic 98.6%; others 100% | 482 | 4.0 | 120× faster | 36.0 | 3.4 | 91% less |
| Synthetic | noraw-ignore-eps0.05 | entropy 1.8%, distinct 2.1–3.9% | entropy threshold 99.7%; others 100% | 482 | 17 | 29× faster | 36.0 | 4.5 | 87% less |
| Synthetic | noraw-ignore-eps0.01 | entropy 1.8%, distinct ≤ 0.59% | entropy threshold 99.7%, NetFlow Logic 99.2%; others 100% | 482 | 3,179 | 6.6× slower | 36.0 | 14.7 | 59% less |
| CAIDA 2019 | raw-eps0.05 | distinct 2.1–3.6% | All 100% | 653 | 344 | 1.9× faster | 28.7 | 13.6 | 52% less |
| CAIDA 2019 | noraw-eps0.1 | distinct 5.3–7.0%; entropy has no plan | All 100% | 572 | 2.8 | 202× faster | 25.5 | 0.25 | 99% less |
| CAIDA 2019 | noraw-eps0.05 | distinct 2.1–3.6%; entropy has no plan | All 100% | 572 | 80 | 7.2× faster | 25.5 | 4.9 | 81% less |
| CAIDA 2019 | noraw-eps0.01 | All 0; entropy has no plan | All 100% | 572 | 4,761 | 8.3× slower | 25.5 | 4.9 | 81% less |
| CAIDA 2019 | noraw-ignore-eps0.1 | entropy 2.8%, distinct 2.4–4.0% | NetFlow Logic 95.7%; others 100% | 653 | 1.9 | 343× faster | 28.7 | 3.3 | 89% less |
| CAIDA 2019 | noraw-ignore-eps0.05 | entropy 2.7%, distinct 2.1–3.6% | NetFlow Logic 95.7%; others 100% | 653 | 43 | 15× faster | 28.7 | 4.5 | 84% less |
| CAIDA 2019 | noraw-ignore-eps0.01 | entropy 2.8%; others 0 | NetFlow Logic 95.7%; others 100% | 653 | 7,181 | 11× slower | 28.7 | 8.0 | 72% less |
| MAWI 2019 | raw-eps0.05 | distinct 2.3–3.7% | All 100% | 515 | 245 | 2.1× faster | 14.5 | 8.7 | 40% less |
| MAWI 2019 | noraw-eps0.1 | distinct 6.2–8.1%; entropy has no plan | All 100% | 461 | 3.8 | 121× faster | 12.5 | 0.25 | 98% less |
| MAWI 2019 | noraw-eps0.05 | distinct 2.3–3.7%; entropy has no plan | All 100% | 461 | 87 | 5.3× faster | 12.5 | 2.3 | 82% less |
| MAWI 2019 | noraw-eps0.01 | All 0; entropy has no plan | All 100% | 461 | 1,260 | 2.7× slower | 12.5 | 2.3 | 82% less |
| MAWI 2019 | noraw-ignore-eps0.1 | entropy 3.4%, distinct 2.4–2.8% | NetFlow Logic 99.4%; others 100% | 515 | 2.2 | 231× faster | 14.5 | 3.3 | 78% less |
| MAWI 2019 | noraw-ignore-eps0.05 | entropy 3.3%, distinct 1.2–3.7% | NetFlow Logic 99.4%; others 100% | 515 | 39 | 13× faster | 14.5 | 4.0 | 72% less |
| MAWI 2019 | noraw-ignore-eps0.01 | entropy 3.4%; others 0 | NetFlow Logic 99.4%; others 100% | 515 | 1,212 | 2.4× slower | 14.5 | 5.4 | 63% less |

---

## 2. Resource scaling

**Data**
- Synthetic, 120 minutes: 200 containers, one CPU series and one memory series each, one sample every 500 ms.
  - 120 steady: CPU normal with mean 0.45 cores; memory mean 0.9 GB.
  - 40 surge: CPU × 1.7 in minutes 30 to 80.
  - 20 leak: memory × 1.8 from minute 40 to the end.
  - 20 idle: CPU mean 0.15 cores, memory mean 350 MB; CPU × 2.2 in minutes 50 to 100.
- Real, 85 minutes: Google cluster 2011, 200 containers, one CPU sample and one memory sample every 5 minutes.

**Queries** (evaluated every 60 s)

| ID | PromQL |
|---|---|
| p95_cpu | `quantile_over_time(0.95, container_cpu{dimension="used"}[5m])` |
| p95_memory | `quantile_over_time(0.95, container_memory{dimension="used"}[5m])` |
| p50_cpu_1h | `quantile_over_time(0.5, container_cpu{dimension="used"}[1h])` |
| p95_cpu_1h | `quantile_over_time(0.95, container_cpu{dimension="used"}[1h])` |
| p99_memory_1h | `quantile_over_time(0.99, container_memory{dimension="used"}[1h])` |
| max_memory_1h | `max_over_time(container_memory{dimension="used"}[1h])` |

**Decision rules**
- Ours: scale up when p95 CPU > 0.8 cores or p95 memory > 1.5 GB; scale down when both are below 0.4 cores and 0.75 GB.
- EC2 scale out: p95 CPU over 5 minutes ≥ 0.6 cores for 3 periods in a row adds capacity.
- EC2 scale in: p95 CPU over 5 minutes < 0.4 cores for 3 periods in a row removes capacity.
- Vertical Pod Autoscaler: the pod is resized when the CPU request of 0.6 cores falls outside [p50, p95] × 1.15 of the last hour, or the memory peak × 1.15 exceeds the 1.6 GB request.
- Grafana Labs: under-provisioned when p95 CPU of the last hour is above the 0.6-core request.
- Azure Advisor: recommend the smaller size when, on half the size (0.5 cores, 1 GB), p95 CPU of the last hour stays at most 40% and p99 memory at most 60%.

**Results**

| Data | Configuration | Query error | Decision agreement | Latency ms exact | approx | Change | Memory MB exact | approx | Change |
|---|---|---|---|---|---|---|---|---|---|
| Synthetic | raw-eps0.01 | 1-hour quantiles 0.67–0.83%; others 0 | All 100% | 1,547 | 1,722 | 1.1× slower | 75.0 | 75.0 | Same |
| Synthetic | noraw-eps0.1 | quantiles 5.0–7.2% | Ours 71.8%, EC2 out 94.6%, EC2 in 99.9%, Vertical Pod Autoscaler 99.3%, Grafana Labs 92.5%, Azure Advisor 99.9% | 1,547 | 64 | 24× faster | 75.0 | 69.9 | 7% less |
| Synthetic | noraw-eps0.05 | quantiles 2.3–3.8% | Ours 96.4%, EC2 out 99.9%, Vertical Pod Autoscaler 99.8%, Grafana Labs 95.4%; others 100% | 1,547 | 78 | 20× faster | 75.0 | 69.2 | 8% less |
| Synthetic | noraw-eps0.01 | quantiles 0.17–0.58% | All 100% | 1,547 | 178 | 8.7× faster | 75.0 | 69.2 | 8% less |
| Synthetic | noraw-eps0.005 | 1-hour quantiles 0.29–0.31%; others 0 | All 100% | 1,547 | 260 | 5.9× faster | 75.0 | 69.2 | 8% less |
| Google 2011 | raw-eps0.01 | All 0 | Vertical Pod Autoscaler 98.5%, Grafana Labs 98.0%, Azure Advisor 99.6%; others 100% | 5.6 | 5.6 | Same | 0.18 | 0.18 | Same |
| Google 2011 | noraw-eps0.1 / 0.05 / 0.01 / 0.005 | All 0 | Ours 95.4%, EC2 in 99.9%, Vertical Pod Autoscaler 98.5%, Grafana Labs 98.0%, Azure Advisor 99.6%; others 100% | 5.6 | 11.9–12.3 | 2.1–2.2× slower | 0.18 | 7.5 | 41× more |

---

## 3. Video experience

**Data**
- Synthetic, 120 minutes: 72 subpopulations (city × ISP × CDN × device), each with five series: bitrate, buffering, play_time, startup_time, device_id.
  - Minutes 40 to 55: 40% of buffering samples fall in 5 to 20 s.
  - Minutes 70 to 85: 50% of bitrate samples fall in a new range.
  - Minutes 95 to 110: startup_time × 2.
- Real, 120 minutes: Puffer 2024, 56 subpopulations.

**Queries** (evaluated every 60 s, per subpopulation)

| ID | PromQL |
|---|---|
| entropy_bitrate | `entropy_over_time(bitrate[1m])` |
| buffering_l1 | `sum_over_time(buffering[1m])` |
| sessions | `count_over_time(bitrate[1m])` |
| distinct_devices | `distinct_over_time(device_id[1m])` |
| startup_p95 | `quantile_over_time(0.95, startup_time[5m])` |
| rebuffer_ratio | `sum_over_time(buffering[5m]) / (sum_over_time(play_time[5m]) + sum_over_time(buffering[5m]))` |

**Decision rules**
- Hydra L1: buffering_l1 above the mean plus 3 standard deviations of the previous 30 minutes.
- Hydra entropy: bitrate entropy more than 3 standard deviations from the mean of the previous 30 minutes.
- Mux: rebuffering above 5% or p95 start time above 4 s over 5 minutes, with at least 100 viewers.
- Conviva AI Alerts: p95 start time above the mean plus 3 standard deviations of up to 6 hours, with at least 25 plays; the alert then holds for 1 hour.
- Conviva patent: rebuffering above a baseline decaying by 0.998 per minute plus 3 standard deviations for 5 minutes, with at least 100 devices.

**Results**

| Data | Configuration | Query error | Decision agreement | Latency ms exact | approx | Change | Memory MB exact | approx | Change |
|---|---|---|---|---|---|---|---|---|---|
| Synthetic | noraw-ignore-eps0.1 | distinct_devices 5.3%, startup_p95 7.2%; others 0 | Mux 99.2%, Conviva AI 74.8%; others 100% | 19 | 3.5 | 5.5× faster | 1.8 | 114 | 62× more |
| Synthetic | noraw-ignore-eps0.05 | startup_p95 3.1%; others 0 | Conviva AI 83.9%; others 100% | 19 | 2.8 | 6.8× faster | 1.8 | 114 | 62× more |
| Synthetic | noraw-ignore-eps0.01 | startup_p95 0.52%; others 0 | Conviva AI 96.4%; others 100% | 19 | 3.4 | 5.7× faster | 1.8 | 114 | 62× more |
| Puffer 2024 | noraw-ignore-eps0.1 | distinct_devices 4.8%; others 0 | Mux 99.4%, Conviva AI 95.6%; others 100% | 10 | 2.6 | 3.9× faster | 0.95 | 85 | 89× more |
| Puffer 2024 | noraw-ignore-eps0.05 | All 0 | Mux 99.4%, Conviva AI 95.6%; others 100% | 10 | 2.0 | 5.1× faster | 0.95 | 85 | 89× more |
| Puffer 2024 | noraw-ignore-eps0.01 | All 0 | Mux 99.4%, Conviva AI 95.6%; others 100% | 10 | 1.8 | 5.5× faster | 0.95 | 85 | 89× more |

---

## 4. Canary release

**Data**
- Synthetic, 250 minutes: one row per request. Stable sends one request every 5 ms, canary one every 95 ms (about 5%). Latency is Pareto, alpha 3, scale 0.05 s.
  - Slow canary periods:
    - minutes 30 to 60: × 1.08
    - minutes 70 to 110: × 1.25
    - minutes 160 to 190: 5% of requests take at least 2 s
    - minutes 200 to 230: 8.7% of requests take 1 s
  - Errors: 0.1% of requests return 500 on both versions; in minutes 120 to 150, 20% of canary requests return 500.
- Real, 120 minutes: latency from the Alibaba 2021 microservices trace; status codes generated as in the synthetic workload, with the canary error period in minutes 57 to 72.

**Queries** (evaluated every 60 s)

| ID | PromQL |
|---|---|
| p99_canary | `quantile_over_time(0.99, request_latency{version="canary"}[5m])` |
| p95_canary | `quantile_over_time(0.95, request_latency{version="canary"}[5m])` |
| p99_stable | `quantile_over_time(0.99, request_latency{version="stable"}[5m])` |
| success_rate_canary | `sum(count_over_time(requests{version="canary", code="200"}[5m])) / sum(count_over_time(requests{version="canary"}[5m]))` |

**Decision rules** (at the end of each rollout: roll back or promote)
- Ours: p99_canary / p99_stable > 1.1; 10 checks per rollout, roll back after 3 failures in a row.
- Flagger: a check fails when p99_canary > 500 ms or the success rate < 99%; roll back after 10 failures, promote after 25 passes.
- Argo Rollouts: a measurement fails when the success rate < 95%; one measurement every 5 evaluations, 10 in all; roll back when more than 3 fail.
- CloudWatch: p99_canary > 500 ms; roll back when 4 of the last 5 points breach.
- Keptn: an evaluation fails when p95_canary is more than 10% above the mean of the last 3 passing evaluations, or ≥ 600 ms; one decision every 10 evaluations.

**Results**

| Data | Configuration | Query error | Decision agreement | Latency ms exact | approx | Change | Memory MB exact | approx | Change |
|---|---|---|---|---|---|---|---|---|---|
| Synthetic | raw-eps0.01 | quantiles 0.76–0.92% | Ours 75.0%, Keptn 92.0%; others 100% | 56 | 51 | 1.1× faster | 3.0 | 3.0 | Same |
| Synthetic | noraw-eps0.05 | quantiles 3.0–4.9% | Ours 87.5%, Keptn 56.0%; others 100% | 56 | 0.81 | 70× faster | 3.0 | 0.04 | 99% less |
| Synthetic | noraw-eps0.01 | quantiles 0.66–0.79% | Ours 83.3%, CloudWatch 95.8%, Keptn 96.0%; others 100% | 56 | 0.88 | 64× faster | 3.0 | 0.11 | 96% less |
| Synthetic | noraw-eps0.005 | quantiles 0.32–0.35% | Ours 95.8%; others 100% | 56 | 0.88 | 64× faster | 3.0 | 0.19 | 94% less |
| Synthetic | noraw-eps0.001 | quantiles 0.03–0.07% | Ours 91.7%, CloudWatch 95.8%; others 100% | 56 | 1.3 | 45× faster | 3.0 | 0.51 | 83% less |
| Alibaba 2021 | raw-eps0.01 | quantiles 0.53–0.73% | Ours 81.8%, Keptn 91.7%; others 100% | 220 | 0.99 | 221× faster | 13.2 | 0.16 | 99% less |
| Alibaba 2021 | noraw-eps0.05 | quantiles 2.0–2.7% | Ours 63.6%, Keptn 66.7%; others 100% | 220 | 0.98 | 224× faster | 13.2 | 0.05 | 99.6% less |
| Alibaba 2021 | noraw-eps0.01 | quantiles 0.61–0.64% | Ours 81.8%, Keptn 83.3%; others 100% | 220 | 1.0 | 213× faster | 13.2 | 0.16 | 99% less |
| Alibaba 2021 | noraw-eps0.005 | quantiles 0.22–0.31% | Ours 81.8%; others 100% | 220 | 1.0 | 212× faster | 13.2 | 0.24 | 98% less |
| Alibaba 2021 | noraw-eps0.001 | quantiles 0.04–0.08% | All 100% | 220 | 1.4 | 162× faster | 13.2 | 0.75 | 94% less |

---

## 5. Rate limiting

**Data**
- Synthetic, 120 minutes: one request every 2 ms; the value is the client_id.
  - Normal traffic: Zipf, skew 1.0, over 100,000 clients.
  - Minutes 30 to 70: 3 heavy clients send 3% of requests.
  - Minutes 80 to 110: 2 heavy clients send 2% of requests.
- Real:
  - CAIDA 2019, 1 in 48 packets, source IP as the client, 585 s
  - MAWI 2019, 1 in 16 packets, 895 s
  - NASA-HTTP, the busiest two hours of July 1995, 1,577 clients

**Queries** (evaluated every 60 s)

| ID | PromQL |
|---|---|
| top10_clients | `topk(10, sum by (client_id) (count_over_time(requests[5m])))` |
| top10_clients_1m | `topk(10, sum by (client_id) (count_over_time(requests[1m])))` |

**Decision rules**
- Top-10 set: the overlap between the approximate and the exact top 10.
- AWS WAF: a client among the top 10 over 5 minutes with more than 2,000 requests is limited while it stays over.
- Cloud Armor: a client among the top 10 over 1 minute with more than 500 requests is banned for the rest of the interval plus 600 s.
- Fastly: a client among the top 10 over 1 minute above 20 requests per second goes into the penalty box for at least 2 minutes.

**Results**

| Data | Configuration | Query error (count, 5m / 1m) | Decision agreement | Latency ms exact | approx | Change | Memory MB exact | approx | Change |
|---|---|---|---|---|---|---|---|---|---|
| Synthetic | noraw-ignore-eps0.05 | 185% / 180% | Top-10 67.5%, AWS WAF 90.0%, Cloud Armor 70.1%, Fastly 99.4% | 163 | 0.34 | 475× faster | 2.9 | 0.04 | 99% less |
| Synthetic | noraw-ignore-eps0.01 | 21.7% / 26.0% | Top-10 97.9%, AWS WAF 96.7%, Cloud Armor 97.9%, Fastly 99.6% | 163 | 0.48 | 344× faster | 2.9 | 0.17 | 94% less |
| Synthetic | noraw-ignore-eps0.005 | 11.1% / 12.3% | Top-10 99.7%, AWS WAF 99.0%, Cloud Armor 99.9%, Fastly 99.6% | 163 | 0.69 | 235× faster | 2.9 | 0.34 | 88% less |
| Synthetic | noraw-ignore-eps0.001 | 1.6% / 1.8% | Top-10 99.8%, AWS WAF 99.6%, Fastly 99.9%; others 100% | 163 | 1.9 | 88× faster | 2.9 | 1.5 | 46% less |
| CAIDA 2019 | noraw-ignore-eps0.05 | 143% / 584% | Top-10 30.0%, AWS WAF 51.7%, Cloud Armor 42.7%, Fastly 73.8% | 8,618 | 0.99 | 8,734× faster | 75.7 | 0.04 | 99.9% less |
| CAIDA 2019 | noraw-ignore-eps0.01 | 105% / 112% | Top-10 88.0%, AWS WAF 85.9%, Cloud Armor 72.2%, Fastly 83.3% | 8,618 | 0.93 | 9,289× faster | 75.7 | 0.17 | 99.8% less |
| CAIDA 2019 | noraw-ignore-eps0.005 | 57.7% / 59.2% | Top-10 90.0%, AWS WAF 85.7%, Cloud Armor 92.2%, Fastly 92.2% | 8,618 | 1.5 | 5,930× faster | 75.7 | 0.34 | 99.6% less |
| CAIDA 2019 | noraw-ignore-eps0.001 | 8.2% / 9.9% | Top-10 98.0%, AWS WAF 97.1%, Cloud Armor 97.1%, Fastly 97.7% | 8,618 | 3.0 | 2,849× faster | 75.7 | 1.8 | 98% less |
| MAWI 2019 | noraw-ignore-eps0.05 | 126% / 116% | Top-10 58.0%, AWS WAF 77.9%, Cloud Armor 56.9%, Fastly 85.5% | 4,454 | 0.72 | 6,152× faster | 50.7 | 0.04 | 99.9% less |
| MAWI 2019 | noraw-ignore-eps0.01 | 22.4% / 25.1% | Top-10 97.0%, AWS WAF 96.2%, Cloud Armor 90.8%, Fastly 97.4% | 4,454 | 0.90 | 4,949× faster | 50.7 | 0.17 | 99.7% less |
| MAWI 2019 | noraw-ignore-eps0.005 | 6.6% / 6.5% | Top-10 97.0%, AWS WAF 96.5%, Cloud Armor 99.3%, Fastly 99.5% | 4,454 | 1.2 | 3,804× faster | 50.7 | 0.34 | 99% less |
| MAWI 2019 | noraw-ignore-eps0.001 | 0.65% / 0.56% | All 100% | 4,454 | 2.9 | 1,564× faster | 50.7 | 1.6 | 97% less |
| NASA 1995 | noraw-ignore-eps0.05 | 213% / 167% | Top-10 47.0%; others 100% | 1.8 | 0.28 | 6.2× faster | 0.03 | 0.04 | 1.2× more |
| NASA 1995 | noraw-ignore-eps0.01 | 53.3% / 33.3% | Top-10 90.6%; others 100% | 1.8 | 0.48 | 3.7× faster | 0.03 | 0.16 | 5.1× more |
| NASA 1995 | noraw-ignore-eps0.005 | 22.2% / 0 | Top-10 96.4%; others 100% | 1.8 | 0.56 | 3.1× faster | 0.03 | 0.27 | 8.5× more |
| NASA 1995 | noraw-ignore-eps0.001 | 0 / 0 | All 100% | 1.8 | 1.4 | 1.3× faster | 0.03 | 0.17 | 5.2× more |
