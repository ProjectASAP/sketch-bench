# Approx-bench: Five Use Cases

Use cases 1–3 come from prior papers; 4–5 from industry practice.
Each query runs exactly and approximately over the same data, evaluated at the same times.
Low-level metrics compare the two results per evaluation.
User intents say what a user sees from exact output and whether approx output shows the same; the benchmark reports how often it does.

## 1. DDoS / TCP SYN flood alerting

**Case.**
Alert when the distinct count or entropy of source IPs hitting a server crosses a threshold.
Several window sizes run, since the best one is unknown.
Per the paper, a window holds 100K–1M samples.

**Source.**
PromSketch (Zhu et al., PVLDB 18(8), 2025), §2.2.
<https://www.vldb.org/pvldb/vol18/p2348-zhu.pdf>

**Queries.**
Every 5 s; `*_over_time` functions are PromSketch extensions.

```promql
entropy_over_time(src_ip{vm="instance1"}[10s]) > entropy_threshold
distinct_over_time(src_ip{vm="instance1"}[10s]) > volume_threshold
distinct_over_time(src_ip{vm="instance1"}[5s])  > volume_threshold
```

**Low-level metrics.**
Relative error of entropy and distinct count.

**User intents.**

- Alert: From exact output, an attack alert fires. From approx output, the same alert fires, and no extra ones.
- Onset: From exact output, the alert starts at some evaluation. From approx output, it starts at the same one.
- Continuity: From exact output, the alert stays on. From approx output, it stays on too.

## 2. Kubernetes autoscaling

**Case.**
An autoscaler adds or removes pods based on recording rules over recent usage.
Per the paper, a GKE cluster reaches 100K+ series.

**Source.**
PromSketch, §2.2.

**Queries.**
Every 1 min:

```promql
quantile_over_time(0.95, container_memory{dimension="used"}[5m])
quantile_over_time(0.95, container_cpu{dimension="used"}[5m])
```

**Low-level metrics.**
Rank or relative error of each container's p95.

**User intents.**

- Decision: From exact output, the autoscaler scales up or down (assumed rule: p95 CPU > 0.8 cores or p95 memory > 1.5 GB). From approx output, it does the same.
- Onset: From exact output, a scaling action happens at some evaluation. From approx output, it happens at the same one.
- Continuity: From exact output, no scaling happens for a while. From approx output, none happens either.

## 3. Video quality of experience by subpopulation

**Case.**
Find viewer groups (any combination of city, device, ISP, CDN) with degraded experience.
The paper's trace has about 700K groups.

**Source.**
Hydra (Manousis et al., PVLDB 15(11), 2022), §2.1.
<https://vldb.org/pvldb/vol15/p3249-manousis.pdf>

**Queries.**
Every minute, over several groupings:

```sql
SELECT City, L1Norm(Buffering)
FROM SessionSummaries
GROUP BY City
```

**Low-level metrics.**
Relative error of each group's L1 norm.

**User intents.**

- Ranking: From exact output, k groups buffer the most. From approx output, the same k groups are seen.
- Trend: From exact output, a top group's buffering rises over W evaluations. From approx output, the rise is seen too.
- Anomaly: From exact output, a group spikes above its recent normal. From approx output, the spike is seen too.

<!--
- The query can return the Entropy and L1Norm
- The output: The error can be considered as "approx Entropy is 5% different from exact Entropy".
- Assuming there is a dashboard, if user see a trend of exact result going up, will user see a trend of approx result going up
- Or, if a user see anomaly detected at exact query, will user also see that anomaly is detected (high level)
  - to do that, that means the benchmark will define some metric like what percent of anomaly is also detected (low level)

Query result is the low level thing.
Query result show something is the high level thing.
One benchmark can serve different high level goal differently, if we interpreting the result differently.
-->

## 4. Canary release analysis

**Case.**
Compare a canary's tail latency with the stable version's, then promote or roll back.
The canary gets little traffic, so its side rests on few samples.

**Source.**
Industry practice (e.g., Flagger, Argo Rollouts), which usually uses `histogram_quantile`; we assume raw latency samples.

**Queries.**
Every minute during rollout; 1.1 is an illustrative tolerance.

```promql
quantile_over_time(0.99, request_latency{version="canary"}[5m])
  > 1.1 * quantile_over_time(0.99, request_latency{version="stable"}[5m])
```

**Low-level metrics.**
Rank or relative error of both p99s.

**User intents.**

- Decision: From exact output, the canary is promoted or rolled back. From approx output, the same decision is made.
- Onset: From exact output, a slower canary is caught at some evaluation. From approx output, it is caught at the same one.
- Continuity: From exact output, the canary stays judged slower. From approx output, it stays slower too.

## 5. Top clients for rate limiting

**Case.**
Find the clients sending the most requests among millions of client IDs, to rate-limit them.

**Source.**
Industry practice.

**Queries.**
Every minute:

```sql
SELECT client_id, COUNT(*) AS requests
FROM requests
WHERE ts >= now() - INTERVAL '5 minutes'
GROUP BY client_id
ORDER BY requests DESC
LIMIT 10
```

**Low-level metrics.**
Relative error of the counts of the exact top 10 clients.

**User intents.**

- Ranking: From exact output, 10 top clients are seen. From approx output, the same 10 are seen, with no normal client among them.
- Onset: From exact output, a new heavy client enters the top 10. From approx output, it enters at the same evaluation.
