# DDoS Detection

A distributed denial-of-service (DDoS) attack floods a server with traffic from many sources.
A rule reads query results at every check and decides to alert or to mitigate.
This page lists what public tools query and decide, and the end-to-end cases we benchmark.
Sources: PromSketch (PVLDB 2025), Cloudflare, AWS Shield, Google Cloud Armor, Datadog, Red Hat, Kentik, FastNetMon, NetFlow Logic.

## 1. Industry examples

| Source | Metrics | Query | Compared against | Decision |
| --- | --- | --- | --- | --- |
| [Cloudflare Adaptive DDoS Protection](https://developers.cloudflare.com/ddos-protection/managed-rulesets/adaptive-protection/) | Layer 3/4: rate per IP protocol for each prefix; UDP rate per client country and region | Profile from each day's maximum rate over the past 7 days, taking the 95th percentile; recalculated daily | The profile | Sensitivity low, medium or high. The default action is Log; the user overrides it to mitigate. |
| [AWS Shield](https://docs.aws.amazon.com/waf/latest/developerguide/ddos-event-detection-infrastructure.html) | Layer 3/4 traffic to each resource, across dimensions such as port and protocol | Traffic to each resource evaluated every minute; when it is elevated, the resource's capacity is measured | The resource's capacity, which depends on instance type and size; Shield Advanced uses half of it | Mitigation when the capacity is exceeded, or a fraction of it on a known attack vector. The mitigation has a time to live that is extended while the attack continues; Shield Advanced grows it exponentially for frequent attacks. |
| [Google Cloud Armor Adaptive Protection](https://docs.cloud.google.com/armor/docs/adaptive-protection-overview) ([auto deploy](https://docs.cloud.google.com/armor/docs/adaptive-protection-auto-deploy)) | Layer 7 requests per backend service, with request attributes such as client IP, region and headers | Model of each backend service's traffic, trained for at least one hour | The learned baseline | Alert with a confidence from 0 to 1, an attack signature and a suggested rule; alerting is suggested from confidence > 0.5. Auto deploy defaults: load 0.8, confidence 0.5, impacted baseline 1%; the rule expires after 7,200 s. |
| [Datadog Cloud SIEM](https://docs.datadoghq.com/security/default_rules/def-000-kny/) | Unique IP addresses connecting to a server | Rule "Suricata baseline deviation from expected IP requests", over a short period | Baseline (anomaly rule) | Security signal; the goal names DDoS. Response: rate limit, block or filter the IPs. |
| [Red Hat Network Observability (OKD)](https://docs.okd.io/4.17/observability/network_observability/metrics-alerts-dashboards.html) | Rate of SYN-only flows per destination | `sum(rate(netobserv_flows_with_flags_per_destination_total{Flags="2"}[1m])) by (job, DstK8S_HostName, DstK8S_Namespace, DstK8S_Name) > 300` | Fixed: 300, an example to tune | Alert when the condition holds for 15 s (`for: 15s`); it clears when the condition stops holding. |
| [Kentik](https://kb.kentik.com/v4/Ga12.htm) ([alert details](https://kb.kentik.com/docs/alert-details)) | A primary metric such as packets/s, plus secondary metrics such as unique source IPs | Alert policy keyed by dimensions such as destination IP; one data point every 60 s, 2 min or 5 min | Static threshold or baseline, e.g. flows/s above 200% of baseline | Alarm when the conditions hold at least X times within a rolling window of Y; the user sets X and Y. The policy can trigger a mitigation. |
| [FastNetMon](https://github.com/pavel-odintsov/fastnetmon/blob/master/src/fastnetmon.conf) | Packets/s, Mbit/s and flows/s per host | Per-host rate averaged over 5 s (`average_calculation_time`), recomputed every second | Fixed: 20,000 packets/s, 1,000 Mbit/s, 3,500 flows/s; banning on flows is off by default | Ban (BGP blackhole or Flowspec) when a threshold is exceeded. The ban lasts 1,900 s (`ban_time`); with `unban_only_if_attack_finished`, it stays until the attack ends. |
| [NetFlow Logic patent US9843488](https://patents.google.com/patent/US9843488B2/en) | Traffic entropy; number of observed IP addresses | One collection interval of 10 to 60 s | Change in the distribution over time, e.g. CUSUM change-point detection | Anomaly. The onset of a DDoS attack is often an increase in observed IP addresses. |

## 2. Query Candidates

| ID | Query | From | ASAPPlanner (b822553) |
| --- | --- | --- | --- |
| Q1 | Distinct source IPs: `distinct_over_time(src_ip{vm="instance1"}[10s])`, also over `[5s]` | PromSketch, Datadog, Kentik | Runs; without raw data at error target 0.01 the plan uses a K-minimum-values sketch |
| Q2 | Entropy of source IPs: `entropy_over_time(src_ip{vm="instance1"}[10s])` | PromSketch, NetFlow Logic | Runs exactly; without raw data at error target 0.01 no plan has an accuracy guarantee; with `--ignore-guarantees` the plan uses UnivMon |
| Q3 | Packet rate per destination: `count_over_time(src_ip{vm="instance1"}[5s])`; the SYN-flow rate of Network Observability is the same over `[1m]` | FastNetMon, Red Hat Network Observability, AWS Shield | Runs; no sketch in the plan, even without raw data |
| Q4 | Historical profile: `quantile_over_time(0.95, max_over_time(count_over_time(src_ip{vm="instance1"}[1m])[1d:1m])[7d:1d])` | Cloudflare | Does not run: the planner does not support a subquery above the range |

Checked by running each query through `planeval` on a small synthetic spec, exact and at error target 0.01 without raw data.

The workload (`aqpbm-planeval/workloads/uc1_syn_flood.json`) runs Q1 over 10 s and 5 s (`distinct_10s`, `distinct_5s`) and Q2 over 10 s (`entropy_10s`), every 5 s. Q3 and Q4 are not in the workload.

## 3. Data shape

One stream of SYN packets to one server, one row per packet; the value is the packet's source IP.

| Column | Values | Distribution | Source |
| --- | --- | --- | --- |
| time | Timestamp | One packet every 1 ms | Current workload |
| vm | `instance1` | One target server | PromSketch query |
| src_ip | IPv4 address as an integer | Normal traffic: Zipf, skew 1.1, over 2,000 addresses. <br>Attack, minutes 5 to 11: 60% of packets come from addresses spread uniformly over [10^6, 2^32), ramping in and out over 30 s. <br>Attack, minutes 20 to 24: 40%, ramping over 1 minute. | Zipf: PromSketch synthetic data. Spread sources: "SYN-only flows with different source IPs" ([Red Hat Network Observability](https://docs.okd.io/4.17/observability/network_observability/metrics-alerts-dashboards.html)). Attacks under 10 minutes: 89% of layer 3/4 attacks end within 10 minutes ([Cloudflare, 2025 Q1](https://blog.cloudflare.com/ddos-threat-report-for-2025-q1)). Real data: [CAIDA traces](https://www.caida.org/catalog/datasets/passive_dataset/) of source IPs on a NYC Equinix backbone link, 2018-12-20 and 2019-01-17, used by PromSketch. |

The workload runs 30 minutes and has time, vm and src_ip. Destination, protocol and TCP flags are not generated: every row is a SYN to `instance1`.

## 4. Decision rules

A rule has two parts: what it concludes from each query result, and how it aggregates many query results into an alert or a mitigation. Numbers are the values in each source's own example.

On each query result:

- Cloudflare: traffic is an attack when its rate is above the profile, the 95th percentile of each day's maximum over the past 7 days. Sensitivity is low, medium or high; the thresholds are not public.
- AWS Shield: traffic is an attack when it exceeds the resource's capacity, or a fraction of it on a known attack vector. Shield Advanced uses half of the capacity.
- Google Cloud Armor: the model gives a confidence from 0 to 1; alerting is suggested from confidence > 0.5. Auto deploy needs load > 0.8, confidence > 0.5 and impacted baseline < 1%.
- Datadog: the signal fires when the number of unique IPs connecting to a server deviates from its baseline.
- Red Hat: the condition holds when the rate of SYN-only flows to a destination is > 300 per second.
- Kentik: the condition holds when the primary metric is above a static threshold or the baseline, e.g. flows/s above 200% of baseline.
- FastNetMon: a host is attacked when its 5-second average is > 20,000 packets/s or > 1,000 Mbit/s.
- NetFlow Logic: an anomaly is a change point, e.g. by CUSUM, in traffic entropy or in the number of observed IPs.
- Ours (current, from PromSketch): the condition holds when `entropy_10s` > 8.5, `distinct_10s` > 3,000 or `distinct_5s` > 1,700. The thresholds are ours.

Aggregation over multiple query results:

- Red Hat: alert when the condition holds for 15 s (`for: 15s`); clear when it stops holding.
- Kentik: alarm when the condition holds at least X times within a rolling window of Y; the user sets X and Y.
- FastNetMon: ban for 1,900 s; with `unban_only_if_attack_finished`, keep the ban until the attack ends.
- AWS Shield: the mitigation has a time to live, extended while the attack continues. Shield Advanced grows it exponentially for frequent attacks.
- Google Cloud Armor: an auto-deployed rule expires after 7,200 s.
- Cloudflare, Datadog, NetFlow Logic: the sources give no aggregation.
- Ours (current, from PromSketch): the alert fires while any condition holds; no aggregation.

Ours, not from a public source, to study how a rule can use the error bound:

- Each approximate result carries an interval from its error bound. The condition holds when the whole interval is above the threshold, does not hold when it is wholly below, and is unsure otherwise.
- Same, but an unsure result reruns that one query with the exact plan.

## 5. End-to-end candidates

| ID | Query | On each result | Aggregation | From | Rule in `scripts/` |
| --- | --- | --- | --- | --- | --- |
| E1 | Q1 over 10 s and 5 s, and Q2 over 10 s, every 5 s | Holds when `distinct_10s` > 3,000, `distinct_5s` > 1,700 or `entropy_10s` > 8.5 | Fires while any condition holds | Current workload (PromSketch) | `threshold_alert.py` |
| E2 | Q1 | Unique source IPs deviate from the baseline | None | Datadog | Not implemented |
| E3 | Q3 and Q1 per destination, every 60 s | Above a static threshold or the baseline | At least X times within a rolling window of Y, both set by the user; no defaults | Kentik | Not implemented |
| E4 | Q2 | A change point in entropy, e.g. by CUSUM | None | NetFlow Logic | Not implemented |
| E5 | Q3 over 5 s, every second | > 20,000 packets/s | Ban for 1,900 s, until the attack ends | FastNetMon | Not implemented |
| E6 | Q3 over 1 minute | > 300 SYN-only flows per second | Alert after 15 s | Red Hat | Not implemented |
| E7 | Q4 | Rate above the profile | None | Cloudflare | Not implemented; Q4 does not run |

AWS Shield and Google Cloud Armor are not here: one compares with the resource's capacity, the other with a learned model, and neither is a query.

## 6. Benchmark metrics

| Group | Measure | Status |
| --- | --- | --- |
| Query accuracy | Relative error of each distinct count and entropy | Not yet |
| Decision | For E1: TP / FP / FN / TN per evaluation; alert episodes and missed episodes; onset delay, end delay and gaps | Not yet |
| Decision | The same for E2 to E7 | Not yet |
| Decision | Agreement with the injected attacks | Not yet |
| Cost | Query latency per evaluation, without plan compile time | Not yet |
| Cost | View refresh time and insert throughput; the sketch insertion is replayed in one batch per window, not streamed | Not yet |
| Cost | Bytes per evaluation: sketch state bytes plus raw sample bytes read at query (samples × 16 bytes) | Not yet |
| Cost | Plan compile time; planning time | Not yet |
| Cost | Process peak memory and CPU time | Not yet |

Every exact and approximate plan comes from ASAPPlanner and runs in `planeval`.

## 7. Reproduce

TODO
