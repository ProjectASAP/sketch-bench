# Cloud Resource Scaling

A cloud platform scales resources up and down with the load, such as the number of pods or their CPU and memory.
A rule reads query results at every check and decides to scale up, to scale down, or to change a resource setting.
This page lists what public tools query and decide, and the end-to-end cases we benchmark.
Sources: PromSketch (PVLDB 2025), Google Autopilot, Kubernetes, AWS, Microsoft Azure, Uber, Grafana Labs.

## 1. Industry examples

| Source | Metrics | Query | Compared against | Decision |
| --- | --- | --- | --- | --- |
| [Google Autopilot](https://research.google/pubs/autopilot-workload-autoscaling-at-google-scale/) (EuroSys 2020) | Per-task CPU and memory usage; job CPU usage | Vertical: a percentile of a decayed usage histogram (half life 12 h for CPU, 48 h for memory); serving jobs use p95 or p90 of CPU, memory uses p98 or the maximum. Horizontal: maximum or p95 of job CPU over a horizon of 72 h (default), from 5-minute averages. | Horizontal: a target average utilization | Vertical: limit = statistic plus a 10–15% safety margin, then the maximum over the last hour. Horizontal: replicas = statistic / target; deferred downscaling keeps the maximum recommendation over the last T_d, a window the job owner sets (about 40% of jobs use 2 days, 35% use 3 days). |
| [Kubernetes Vertical Pod Autoscaler](https://github.com/kubernetes/autoscaler/blob/master/vertical-pod-autoscaler/pkg/recommender/config/config.go) ([histograms](https://github.com/kubernetes/autoscaler/blob/master/vertical-pod-autoscaler/pkg/recommender/model/aggregations_config.go)) | CPU and memory usage per container | Percentiles of decaying histograms whose buckets grow by 5%: target p90, lower bound p50, upper bound p95; memory keeps one peak per 24 h over 8 days; metrics fetched every minute | Usage history | Set the container's CPU and memory requests to the target plus a 15% safety margin |
| [Kubernetes Horizontal Pod Autoscaler](https://kubernetes.io/docs/concepts/workloads/autoscaling/horizontal-pod-autoscale/) | Pod CPU or memory utilization relative to the request, or custom metrics | Mean across pods, every 15 s | Target value | desiredReplicas = ceil(currentReplicas × current / target); no change within a 10% tolerance. Scale down takes the highest recommendation over 300 s; scale up adds at most 100% or 4 pods per 15 s, whichever is more. |
| [AWS Compute Optimizer](https://docs.aws.amazon.com/compute-optimizer/latest/ug/rightsizing-preferences.html) | EC2 CPU and memory utilization | CPU threshold p90, p95 or p99.5 (default p99.5) over a lookback of 14 days (default), 32 or 93 days | The default preset targets CPU below 80% for more than 99.5% of the time, and memory below 80% | Recommend an instance type, with 20% CPU and memory headroom by default |
| [AWS EC2 Auto Scaling](https://docs.aws.amazon.com/autoscaling/ec2/userguide/as-scaling-simple-step.html) with [CloudWatch](https://docs.aws.amazon.com/AmazonCloudWatch/latest/monitoring/cloudwatch_concepts.html) | A CloudWatch metric such as EC2 CPU utilization; an alarm can use a percentile such as the 95th as its statistic | Alarm statistic per period, over a number of evaluation periods | Alarm threshold with step bounds; the example adds 10% of the group from 60% and 30% from 70%, and removes 10% at 40% and 30% at 30% | Step scaling changes capacity by the step that matches the breach size; new instances count only after their warmup |
| [Microsoft Azure Advisor](https://learn.microsoft.com/en-us/azure/advisor/advisor-cost-recommendations) | VM CPU, memory and outbound network utilization | Sampled every 30 s, aggregated to 1 minute, then to 30 minutes (maximum of averages); lookback 7 days by default | User-facing: p95 of CPU and network at most 40% and p99 of memory at most 60% on the new size; other workloads 80% and 80%. Shutdown: p95 of maximum CPU below 3%. | Recommend a smaller VM size, fewer scale set instances, or shutdown |
| [Uber](https://www.uber.com/en-NL/blog/vertical-cpu-scaling/) | CPU utilization of storage pods: Docstore, Schemaless, MySQL, Cassandra and others | p99 per 8-hour window over 14 days; the cluster value is the busiest pod's (p95 for Cassandra-like clusters); the peak is the third-highest of the 42 points | Target peak usage ratio of 40% | Set CPU cores per pod, rounded up; all pods in a cluster get the same |
| [Grafana Labs](https://grafana.com/blog/how-were-using-prometheus-subqueries-at-grafana-labs/) | Container CPU usage | `quantile_over_time(0.95, container_name_pod_name_namespace:container_cpu_usage_seconds_total:sum_rate_5m[7d])` | CPU requests from kube-state-metrics | Size CPU requests at p95; pods whose p95 exceeds the request are under-provisioned |

## 2. Query Candidates

| ID | Query | From | ASAPPlanner (b822553) |
| --- | --- | --- | --- |
| Q1 | p95 per container over 5 minutes: `quantile_over_time(0.95, container_cpu{dimension="used"}[5m])`, and the same for `container_memory` | PromSketch, AWS CloudWatch (percentile alarm) | Runs; without raw data at error target 0.01 the plan uses KLL |
| Q2 | Percentile over a long window: `quantile_over_time(0.95, container_cpu{dimension="used"}[7d])`; p90 for VPA, p99.5 over 14 days for Compute Optimizer | Kubernetes VPA, Grafana Labs, Azure Advisor, AWS Compute Optimizer, Uber | Runs; without raw data at error target 0.01 the plan uses KLL |
| Q3 | Mean across pods: `avg(avg_over_time(container_cpu{dimension="used"}[1m]))` | Kubernetes HPA | Runs; no sketch in the plan, even without raw data |
| Q4 | Peak memory: `max_over_time(container_memory{dimension="used"}[15m])` | Google Autopilot (memory maximum), Kubernetes VPA (memory peaks) | Runs; no sketch in the plan, even without raw data |
| Q5 | p95 of the job's total CPU over a horizon: `quantile_over_time(0.95, (sum(avg_over_time(container_cpu[5m])))[72h:5m])` | Google Autopilot (horizontal) | Does not run: the planner does not support a subquery above the range |
| Q6 | Average pod count: `avg_over_time(pod_number[5m])` | PromSketch | Not checked; `pod_number` is not generated |

Checked by running each query through `planeval` on a small synthetic spec, exact and at error target 0.01 without raw data. Q5 was checked over `[15m:5m]`.

The workload (`aqpbm-planeval/workloads/uc2_autoscaling.json`) runs Q1 for CPU and memory (`p95_cpu`, `p95_memory`) every minute. Q2 to Q6 are not in the workload.

## 3. Data shape

Two metrics, CPU and memory, one series per container, one sample every 500 ms.

| Column | Values | Distribution | Source |
| --- | --- | --- | --- |
| time | Timestamp | One sample per container every 500 ms | Current workload |
| container | 200 containers in four groups: 120 `steady`, 40 `surge`, 20 `leak`, 20 `idle` | steady: no change. surge: CPU rises. leak: memory grows. idle: low CPU that later rises. | Current workload |
| dimension | `used` | — | PromSketch query |
| container_cpu | Cores | steady, surge and leak: Normal, mean 0.45, standard deviation 0.08. idle: mean 0.15, standard deviation 0.03. <br>surge: × 1.7 in minutes 30 to 80, ramping over 15 minutes, each container starting 30 s after the previous one. <br>idle: × 2.2 in minutes 50 to 100, ramping over 10 minutes, 30 s apart. | Current workload, around the rule's 0.8 and 0.4 core thresholds. Real data: Google Cluster Data 2019, `average_usage.cpus` with `start_time` as the time. |
| container_memory | Bytes | steady, surge and leak: Normal, mean 0.9 GB, standard deviation 80 MB. idle: mean 350 MB, standard deviation 30 MB. <br>leak: × 1.8 from minute 40 to the end, ramping over 60 minutes, each container starting 60 s after the previous one. | Current workload, around the rule's 1.5 GB and 0.75 GB thresholds. Real data: Google Cluster Data 2019, `average_usage.memory` with `start_time` as the time, used by PromSketch. |
| pod_number | Count | Not generated | PromSketch query `avg_over_time(pod_number[5m])` |

The workload runs 120 minutes and has time, container, dimension, container_cpu and container_memory. Google Cluster Data 2019, which PromSketch uses, reports usage per 5-minute period, with a CPU usage histogram for each period ([ClusterData2019](https://github.com/google/cluster-data/blob/master/ClusterData2019.md)).

## 4. Decision rules

A rule has two parts: what it concludes from each query result, and how it aggregates many query results into a scaling action or a resource setting. Numbers are the values in each source's own example.

On each query result:

- Kubernetes Horizontal Pod Autoscaler: ratio = current / target. No change when the ratio is within 10% of 1; otherwise desiredReplicas = ceil(currentReplicas × ratio).
- Kubernetes Vertical Pod Autoscaler: the request is p90 × 1.15.
- Google Autopilot: vertical, the limit is the statistic plus a 10–15% safety margin; horizontal, replicas = statistic / target average utilization.
- AWS Compute Optimizer: recommend the instance type that fits p99.5 CPU plus 20% headroom. The default preset targets CPU below 80% for more than 99.5% of the time.
- AWS EC2 Auto Scaling: the step follows the breach size. From 60%, add 10% of the group; from 70%, add 30%; at 40%, remove 10%; at 30%, remove 30%.
- Azure Advisor: a smaller size fits when, on it, p95 of CPU and network ≤ 40% and p99 of memory ≤ 60% for user-facing workloads, 80% and 80% otherwise. Shut down when p95 of maximum CPU < 3%.
- Uber: cores per pod are set for a 40% peak usage ratio, rounded up.
- Grafana Labs: the CPU request is p95; a pod whose p95 exceeds its request is under-provisioned.
- Ours (current): scale up when p95 CPU > 0.8 cores or p95 memory > 1.5 GB; scale down when both are below 0.4 cores and 0.75 GB.

Aggregation over multiple query results:

- Kubernetes Horizontal Pod Autoscaler: scale down to the highest recommendation of the last 300 s; scale up by at most 100% or 4 pods per 15 s, whichever is more.
- Google Autopilot: vertical, the maximum recommendation of the last hour. Horizontal, deferred downscaling keeps the maximum recommendation of the last T_d, a window the job owner sets: 2 days for about 40% of jobs and 3 days for 35%.
- AWS EC2 Auto Scaling: the alarm must breach for the set number of evaluation periods; a new instance counts only after its warmup.
- AWS Compute Optimizer, Azure Advisor, Uber, Grafana Labs: one recommendation from the lookback window, 14 days, 7 days, 14 days and 7 days.
- Ours (current): act once per crossing.

Ours, not from a public source, to study how a rule can use the error bound:

- Each approximate result carries an interval from its error bound. A threshold is crossed when the whole interval is past it, not crossed when the whole interval is short of it, and unsure otherwise.
- Same, but an unsure result reruns that one query with the exact plan.

## 5. End-to-end candidates

| ID | Query | On each result | Aggregation | From | Rule in `scripts/` |
| --- | --- | --- | --- | --- | --- |
| E1 | Q1 for CPU and memory, every minute | Up when p95 CPU > 0.8 cores or p95 memory > 1.5 GB; down when both are below 0.4 cores and 0.75 GB | Once per crossing | Current workload (PromSketch) | `decision.py` |
| E2 | Q3, every 15 s | No change within 10% of the target; otherwise replicas scale with the ratio | Scale down to the highest recommendation of the last 300 s; scale up by at most 100% or 4 pods per 15 s | Kubernetes HPA | Not implemented |
| E3 | Q2 with p90 for CPU, and Q4 for memory | Request = p90 × 1.15 | None | Kubernetes VPA | Not implemented |
| E4 | Q5 | Replicas = p95 / target average utilization | Deferred downscaling: the highest recommendation of the last 2 days | Google Autopilot | Not implemented; Q5 does not run |
| E5 | Q1 as the alarm statistic | Step by breach size: from 60% add 10% of the group, from 70% add 30%; at 40% remove 10%, at 30% remove 30% | The alarm breaches for the set number of evaluation periods; new instances count after warmup | AWS EC2 Auto Scaling | Not implemented |
| E6 | Q2 over 7 days: p95 for CPU, p99 for memory | A smaller size when, on it, p95 CPU ≤ 40% and p99 memory ≤ 60% | One recommendation per lookback | Azure Advisor | Not implemented |
| E7 | Q2 over 7 days | Under-provisioned when p95 > request | One recommendation per lookback | Grafana Labs | Not implemented |

## 6. Benchmark metrics

| Group | Measure | Status |
| --- | --- | --- |
| Query accuracy | Rank error of each p95 CPU and p95 memory | Not yet |
| Decision | For E1: up, down or none per container per evaluation, exact against approx; decision agreement; onset delay; extra and missed actions, matched within 2 evaluations | Not yet |
| Decision | The same for E2 to E7 | Not yet |
| Decision | Agreement with the injected load changes | Not yet |
| Cost | Query latency per evaluation, without plan compile time | Not yet |
| Cost | View refresh time and insert throughput; the sketch insertion is replayed in one batch per window, not streamed | Not yet |
| Cost | Bytes per evaluation: sketch state bytes plus raw sample bytes read at query (samples × 16 bytes) | Not yet |
| Cost | Plan compile time; planning time | Not yet |
| Cost | Process peak memory and CPU time | Not yet |

Every exact and approximate plan comes from ASAPPlanner and runs in `planeval`.

## 7. Reproduce

TODO
