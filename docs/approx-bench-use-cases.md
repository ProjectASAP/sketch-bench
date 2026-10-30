# Approx-bench: Five Use Cases

## 1. DDoS detection

Attackers can exhaust a server's bandwidth or resources with a TCP SYN flood sent from a botnet.
Victims detect an ongoing attack by monitoring the volume and entropy of source IPs sending to a single destination.
For example, operators track these indicators for a virtual machine with alerting rules evaluated every 5 seconds, as below.
Because the best window size is unknown, the rules query several windows (e.g., 10 s and 5 s).
A target server may receive millions of packets per second, so a window holds 100K to over 1M samples.
Source: PromSketch (Zhu et al., PVLDB 18(8), 2025), §2.2. <https://www.vldb.org/pvldb/vol18/p2348-zhu.pdf>

```yaml
rules:
- evaluation_interval: 5s
- type: alert
- expr: entropy_over_time(src_ip{vm="instance1"}[10s]) > entropy_threshold
- expr: distinct_over_time(src_ip{vm="instance1"}[10s]) > volume_threshold
- expr: distinct_over_time(src_ip{vm="instance1"}[5s]) > volume_threshold
```

**Data.**
PromSketch evaluates entropy and distinct counting on CAIDA traces of source IP addresses from a NYC Equinix backbone link, captured on 2018-12-20 and 2019-01-17.
It also uses synthetic data: 10M Zipf-distributed samples, 10M uniform samples in [0, 10^5], and a dynamic trace that cycles through Zipf, uniform and normal distributions, 1M samples each.

**Application.**
An alert fires while any rule's condition holds.
The paper uses the alert to find the start and the end of an attack.

**Question.**
Does the approx alert fire at exactly the evaluations where the exact alert fires?

**Fidelity metrics.**
- TP / FP / FN / TN per evaluation.
- Onset delay: evaluations from the exact alert's start to the approx alert's start.
- End delay: evaluations from the exact alert's end to the approx alert's end.
- Gaps in the approx alert while the exact alert stays on.

**Example graph.**

```text
evaluation    1 2 3 4 5 6 7 8 9 10 11 12
exact alert   . . █ █ █ █ █ █ .  .  .  .
approx alert  . . . █ █ . █ █ █  .  █  .
                    a   b     c     d

a: onset delay   b: gap   c: end delay   d: FP outside the attack
```

## 2. Cloud resource scaling

Cloud-native platforms scale resources, such as pods, up and down to reduce cost.
They decide from aggregate statistics over time windows, such as averages and quantiles of CPU, memory and pod counts.
For example, recording rules compute each container's 0.95-quantile (p95) of memory and CPU usage, and the average pod count, over the past 5 minutes, as below.
The cloud resource scheduler reads the stored results.
A standard Google Cloud cluster can have up to 256 pods per node and 100 nodes, so cluster-level monitoring can query 100K or more series.
Source: PromSketch, §2.2.

```yaml
rules:
- evaluation_interval: 1m
- type: record
- expr: quantile_over_time(0.95, container_memory{dimension="used"}[5m])
- expr: quantile_over_time(0.95, container_cpu{dimension="used"}[5m])
- expr: avg_over_time(pod_number[5m])
```

**Data.**
PromSketch evaluates `quantile_over_time` and `avg_over_time` on Google Cluster Data 2019, using `start_time` as the time and `average_usage.memory` as the memory usage. <https://github.com/google/cluster-data>
It also uses the synthetic data of use case 1.

**Application.**
The scheduler reads the results every minute.
It scales a container up when its p95 CPU rises above 0.8 cores or its p95 memory rises above 1.5 GB (assumed rule).
It scales the container down when its p95 CPU falls below 0.4 cores and its p95 memory below 0.75 GB (assumed rule).
It acts once per crossing.

**Question.**
Does the scheduler take the same scaling actions on each container, at the same evaluations, on approx results as on exact results?

**Fidelity metrics.**
- TP / FP / FN / TN of scaling actions per container per evaluation.
- Onset delay: evaluations from an exact scaling action to the matching approx action.

**Example graph.**
One container:

```text
evaluation     1 2 3 4 5 6 7 8
exact action   . . ↑ . . . ↓ .
approx action  . . . ↑ . ↑ ↓ .
                     a   b

↑: scale up   ↓: scale down
a: onset delay   b: FP
```

## 3. Video experience monitoring

Video providers need to detect issues that degrade viewer experience, to protect ad and subscription revenue.
Analysts collect a summary of each viewing session and, every minute, compute statistics of video quality over them.
They monitor these statistics across subpopulations of viewers, such as cities, devices, ISPs and CDNs, and their combinations.
For example, to track the entropy of bitrate and the L1 norm of the buffering ratio per city, analysts run the query below.
The L1 norm of the buffering ratio is a common indicator of streaming anomalies.
Source: Hydra (Manousis et al., PVLDB 15(11), 2022), §2.1. <https://vldb.org/pvldb/vol15/p3249-manousis.pdf>

```sql
SELECT City, Entropy(Bitrate), L1Norm(Buffering)
FROM SessionSummaries
GROUP BY City
```

**Data.**
Hydra uses a real trace of video session summaries from one major US-based streaming-video provider.
The trace is about 5 GB and clusters into up to 700K subpopulations.
It also uses synthetic multidimensional data drawn from Zipf distributions with skewness from 0.7 to 0.99.

**Application.**
An anomaly detector reads the result every minute.
It flags a city when its L1 norm of buffering is more than 3 standard deviations above its mean over the previous 30 minutes (assumed rule).

**Question.**
Does the detector flag the same cities, at the same evaluations, on approx results as on exact results?

**Fidelity metrics.**
- TP / FP / FN / TN per city per evaluation.
- Onset delay: evaluations from a city's exact flag starting to its approx flag starting.
- Gaps in a city's approx flag while its exact flag stays on.

**Example graph.**

```text
evaluation            1 2 3 4 5 6 7 8
city A flag  exact    . . █ █ █ █ █ .
             approx   . . . █ █ . █ .
                            a   b
city B flag  exact    . . . . . . . .
             approx   . . . . █ . . .
                              c

a: onset delay   b: gap   c: FP
```

## 4. Canary release analysis

Before a full rollout, operators send a small share of traffic to a canary of the new version and compare it with the stable version.
If the canary's tail latency is clearly worse, the new version is rolled back.
Otherwise, the canary is promoted.
For example, an analysis step compares the p99 latency of both versions over the past 5 minutes, every minute, as below.
In Flagger's example, the canary starts with 2% of the traffic and gains 2% per minute, up to 50%.
So early in a rollout, the canary's p99 rests on far fewer samples than the stable version's.
Flagger usually reads pre-bucketed histograms (`histogram_quantile`).
We assume raw latency samples, so the quantile must be computed exactly or sketched.
Source: Flagger documentation, Deployment Strategies. <https://docs.flagger.app/usage/deployment-strategies>

```promql
quantile_over_time(0.99, request_latency{version="canary"}[5m])
  > 1.1 * quantile_over_time(0.99, request_latency{version="stable"}[5m])
```

**Data.**
Real: no public data labels latency as canary or stable.
The Alibaba microservices trace 2021 has real response times for each call. <https://github.com/alibaba/clusterdata/tree/master/cluster-trace-microservices-v2021>
We split one service's calls into canary and stable by Flagger's traffic shares, and raise the canary's latency during chosen periods.
Synthetic (in sketch-bench): both versions draw latency from the same Pareto distribution, and the canary gets a fixed 5% of the samples.
The canary's latency rises by 8%, which stays under the 10% tolerance, and later by 25%.

**Application.**
A rollout controller reads the result every minute.
It rolls back after the canary is judged slower in 3 consecutive checks (assumed rule).
It promotes the canary after 10 checks with no 3 consecutive slower ones (assumed rule).

**Question.**
Does the controller make the same promote-or-rollback decision, at the same evaluation, on approx results as on exact results?

**Fidelity metrics.**
- TP / FP / FN / TN over rollouts: FP rolls back a good canary, FN promotes a bad one.
- Decision delay: evaluations from the exact rollback (or promotion) to the approx one.
- Gaps in the approx "slower" judgement while the exact one stays on, since each gap resets the 3-check count.

**Example graph.**

```text
evaluation       1 2 3 4 5 6 7 8
exact  slower    . . █ █ █ █ █ █
approx slower    . . █ █ . █ █ █
exact  action    . . . . R . . .
approx action    . . . . . . . R
                         a     b

R: rollback
a: gap, which resets the count   b: decision delay of 3 evaluations
```

## 5. Rate limiting top clients

An API service limits the clients that send the most requests, to protect itself from overload.
Every minute, it finds the top 10 clients by request count over the past 5 minutes, as below.
Cloudflare's rate limiter counts requests per client IP and handles several billion requests a day.
In one sample Cloudflare reports, 400 million requests came from 270,000 distinct client IPs.
Source: Cloudflare blog, "How we built rate limiting capable of scaling to millions of domains", 2017. <https://blog.cloudflare.com/counting-things-a-lot-of-different-things/>

```sql
SELECT client_id, COUNT(*) AS requests
FROM requests
WHERE ts >= now() - INTERVAL '5 minutes'
GROUP BY client_id
ORDER BY requests DESC
LIMIT 10
```

**Data.**
Real: the NASA-HTTP logs from 1995 have 3.46M requests, each with the client host and a 1-second timestamp, and may be freely redistributed. <https://ita.ee.lbl.gov/html/contrib/NASA-HTTP.html>
They average under 1 request per second, so a 5-minute window holds about 200 requests.
Synthetic (in sketch-bench): 100K clients send requests following a Zipf distribution.
Two groups of heavy clients appear in turn, sending 3% and then 2% of all requests.

**Application.**
A rate limiter caps the requests of every client in the top 10 until the next evaluation (assumed rule).

**Question.**
Does the rate limiter cap the same clients, at the same evaluations, on approx results as on exact results?

**Fidelity metrics.**
- TP / FP / FN / TN per client per evaluation: FP caps a client outside the exact top 10, FN misses one inside it.
- Onset delay: evaluations from a client entering the exact top 10 to it being capped on approx results.
- Gaps in a client's approx cap while its exact cap stays on.

**Example graph.**

```text
evaluation              1 2 3 4 5 6 7 8
client X capped  exact  . . █ █ █ █ █ █
                 approx . . . █ █ . █ █
                              a   b
client Y capped  exact  . . . . . . . .
                 approx . . . . █ . . .
                                c

a: onset delay   b: gap   c: FP
```

---

## Miscellaneous

**Low-level metrics.**
Per evaluation, we can also compare the query results directly: relative error of each value, rank error of quantiles, overlap of top-k sets.
These help explain where a fidelity loss comes from.

**Other applications on the same queries.**
Use case 3 can also feed a dashboard: when exact results show a city's buffering rising over several minutes, do approx results show the rise too?

**Open items.**

- Use case 3 has no synthetic workload in sketch-bench yet.

<!--
Original notes:
- The query can return the Entropy and L1Norm
- The output: The error can be considered as "approx Entropy is 5% different from exact Entropy".
- Assuming there is a dashboard, if user see a trend of exact result going up, will user see a trend of approx result going up
- Or, if a user see anomaly detected at exact query, will user also see that anomaly is detected (high level)
  - to do that, that means the benchmark will define some metric like what percent of anomaly is also detected (low level)

Query result is the low level thing.
Query result show something is the high level thing.
One benchmark can serve different high level goal differently, if we interpreting the result differently.
-->
