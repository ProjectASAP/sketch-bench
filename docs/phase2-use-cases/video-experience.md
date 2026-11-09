# Video Experience Monitoring

A video provider watches its viewers' experience, such as bitrate and rebuffering, for each group of viewers: city, internet service provider (ISP), content delivery network (CDN), device, and their combinations.
A rule reads query results at every check and decides which groups have a problem.
This page lists what public tools query and decide, and the end-to-end cases we benchmark.
Sources: Hydra (PVLDB 2022), Conviva, Mux, Bitmovin, Netflix, Twitch, ClickHouse.

## 1. Industry examples

| Source | Metrics | Query | Compared against | Decision |
| --- | --- | --- | --- | --- |
| [Conviva AI Alerts](https://docs.conviva.ai/vsi-video/ai-alerts/) ([diagnostics](https://docs.conviva.ai/vsi-video/dashboards/ai-alert-diagnostics-dashboard/)) | Video startup time, connection-induced rebuffering ratio, exits before video start, concurrent plays, technical start and playback failures | Per cohort every minute. Cohorts by city, ISP, CDN, device, asset and others, and combinations such as ISP with city, or CDN with ISP and city. At least 25 attempts, concurrent plays or ended plays per minute. | Baseline and range from the mean and standard deviation of history; the diagnostics view shows a typical value from 6 hours of recent history | Anomaly when the upper boundary is exceeded; diagnosis then decides whether to alert. The same alert is suppressed for up to 1 hour. Concurrent plays alerts need at least 2,000 concurrent plays. |
| [Conviva patent US11044533B1](https://patents.google.com/patent/US11044533B1/en) | A quality metric per group of sessions; impacted unique devices counted with HyperLogLog | Groups by CDN, device, live or VoD, asset, ISP, city and their combinations, every minute | Baseline: mean over about 40 hours with decay 0.998 per minute; threshold: baseline plus a multiple of the standard deviation | Issue when above the threshold for e.g. 5 minutes with e.g. 100 sessions. A group is a root cause candidate when it is anomalous and more than e.g. 80% of its subgroups are. |
| [Conviva distributions](https://docs.conviva.ai/vsi-video/quick-guide/diagnostics-and-distribution/) ([unique devices](https://docs.conviva.ai/vsi-video/metric-dictionary/unique-devices-with-attempts/)) | Ended plays distributed over average peak bitrate buckets; distributions also of rebuffering ratio, startup time, bitrate switches and others; unique devices per dimension value | Per dimension value, as a time series | — | Drill-down. Unique devices use an approximation algorithm for de-duplication. |
| [Mux](https://www.mux.com/docs/guides/setup-alerts) ([defaults](https://www.mux.com/blog/monitor-viewer-experience-problems-with-new-customizable-alerts)) | Failures, rebuffering percentage, video start time, concurrent viewers | Threshold alerts filtered by ASN (autonomous system number), CDN, country and other dimensions, over an interval of 1 to 60 minutes (default 5). Anomaly alerts over buckets of 1,000 views (organization) or 100 views (per title). | Threshold, e.g. rebuffering above 5%; anomaly alerts: the historical failure rate | Incident, which opens or closes only with an average of at least 100 concurrent viewers (default). Anomaly incidents close when the rate returns to normal. |
| [Bitmovin Analytics](https://developer.bitmovin.com/playback/reference/postanalyticsqueriescount) ([example queries](https://developer.bitmovin.com/playback/docs/how-to-recreate-dashboard-queries-via-the-api-1)) | Counts, unique users (`count('USER_ID')`), average bitrate, median startup time | `groupBy` dimensions such as country, at interval minute, hour, day or month | — | Dashboards and queries |
| [Netflix](https://imply.io/case-studies/netflix-delivers-real-time-observability-for-playback-quality/) (Imply case study) | Startup times, buffering rates, error codes; over 2 million events per second | Sliced by app version, device type and country | A new app version against baseline metrics | Automatic rollback when a regression is detected; continuous anomaly detection |
| [Twitch](https://blog.twitch.tv/en/2025/06/26/the-qoux-journey/) | Client experience events: availability, latency, user actions, failures | By device type, app version and region; 5-minute intervals and minutely distributions | Adaptive thresholds from 2 to 4 weeks of history | Page the on-call engineer when 5 data points are under the threshold, or no data arrives, within 25 minutes |
| [ClickHouse `entropy`](https://clickhouse.com/docs/sql-reference/aggregate-functions/reference/entropy) | Shannon entropy of a column of values | Aggregate function, usable with any `GROUP BY` | — | — |

## 2. Query Candidates

Each column of the session table is a metric labelled with `city`, `isp`, `cdn` and `device`.

| ID | Query | From | ASAPPlanner (b822553) |
| --- | --- | --- | --- |
| Q1 | Entropy of bitrate per city: `entropy_over_time(bitrate{city="X"}[1m])` | Hydra | Runs exactly; without raw data at error target 0.01 no plan has an accuracy guarantee; with `--ignore-guarantees` the plan uses UnivMon |
| Q2 | L1 norm per city, the number of sessions: `count_over_time(bitrate{city="X"}[1m])` | Hydra, Conviva (concurrent plays) | Runs; no sketch in the plan, even without raw data |
| Q3 | Rebuffering ratio per group: `sum by (city) (sum_over_time(buffering[1m])) / (sum by (city) (sum_over_time(play_time[1m])) + sum by (city) (sum_over_time(buffering[1m])))` | Conviva, Mux | Runs; no sketch in the plan, even without raw data |
| Q4 | Unique devices per group: `distinct_over_time(device_id{city="X"}[1m])` | Conviva (unique devices), Bitmovin (unique users) | Runs; without raw data at error target 0.01 the plan uses a K-minimum-values sketch |
| Q5 | Startup time per group: `quantile_over_time(0.5, startup_time{cdn="X"}[5m])`, also p95 | Mux, Bitmovin (median startup time) | Runs; without raw data at error target 0.01 the plan uses KLL |
| Q6 | Band from history: `stddev_over_time((sum_over_time(buffering{city="X"}[1m]))[30m:1m])`, with the mean the same way | Hydra rule, Conviva (mean and standard deviation of history) | Does not run: planning ends before it finds a plan, with or without raw data |

Checked by running each query through `planeval` on a small synthetic spec, exact and at error target 0.01 without raw data. Q3 was checked as a ratio of two sums, and Q6 over `[15m:1m]`.

No workload runs these queries yet.

## 3. Data shape

One wide table of session summaries, one row per session per minute.

| Column | Values | Distribution | Source |
| --- | --- | --- | --- |
| time | Minute | — | Hydra (every minute), Conviva AI Alerts (1-minute intervals) |
| city, isp, cdn, device | Dimension values | Group sizes Zipf, skew 0.7 to 0.99 | Hydra synthetic data; dimensions from Conviva AI Alerts. Real data: Hydra's trace of session summaries from a US streaming provider, about 5 GB and up to 700,000 subpopulations; not public. |
| bitrate | kbps | Not chosen | Hydra `Entropy(Bitrate)`; Conviva bitrate distributions |
| buffering | Rebuffering time, seconds | Not chosen | Hydra `L1Norm(Buffering)`; Conviva and Mux rebuffering metrics |
| play_time | Seconds | Not chosen | Conviva and Mux rebuffering ratio: rebuffering time over play time plus rebuffering time |
| startup_time | Seconds | Not chosen | Mux video start time, Bitmovin median startup time |
| device_id | ID | Not chosen | Conviva unique devices, Bitmovin unique users |

No workload generates this table yet.

## 4. Decision rules

A rule has two parts: what it concludes from each query result, and how it aggregates many query results into an alert or an action. Numbers are the values in each source's own example.

On each query result:

- Conviva AI Alerts: a cohort is anomalous above the upper bound from the mean and standard deviation of its history; the multiplier is not public. A cohort needs at least 25 attempts, concurrent plays or ended plays per minute.
- Conviva patent: a group is anomalous above its baseline, a mean over about 40 hours with decay 0.998 per minute, plus a multiple of the standard deviation.
- Mux: a threshold alert holds when e.g. rebuffering is above 5%. An anomaly alert compares the failure rate of each bucket of views with the historical rate.
- Netflix: a new app version regresses when it is worse than the baseline metrics.
- Twitch: a data point is bad when it is under the adaptive threshold from 2 to 4 weeks of history.
- Ours (current, from Hydra): a city is flagged when its L1 norm of buffering is more than 3 standard deviations above its mean over the previous 30 minutes.

Aggregation over multiple query results:

- Conviva AI Alerts: after an anomaly, diagnosis decides whether to alert; the same alert is suppressed for up to 1 hour.
- Conviva patent: an issue needs e.g. 5 minutes above the threshold and e.g. 100 sessions. A group is a root cause candidate when it is anomalous and more than e.g. 80% of its subgroups are.
- Mux: the interval is 1 to 60 minutes, 5 by default; an incident opens or closes only with an average of at least 100 concurrent viewers.
- Netflix: roll back automatically.
- Twitch: page when 5 data points are under the threshold, or no data arrives, within 25 minutes.
- Ours (current): none; each evaluation flags on its own.

Ours, not from a public source, to study how a rule can use the error bound:

- Each approximate result carries an interval from its error bound. A group is flagged when the whole interval is above the threshold, not flagged when it is wholly below, and unsure otherwise.
- Same, but an unsure result reruns that one query with the exact plan.

## 5. End-to-end candidates

| ID | Query | On each result | Aggregation | From | Rule in `scripts/` |
| --- | --- | --- | --- | --- | --- |
| E1 | Q1 and Q2 per city, every minute; Q6 for the band | L1 > mean + 3 standard deviations of the previous 30 minutes | None | Current rule (Hydra) | Not implemented |
| E2 | Q3 per city, ISP, CDN and their combinations, every minute; Q6 for the band | Above mean + k standard deviations of history, k not public; at least 25 plays per minute | The same alert at most once per hour | Conviva AI Alerts | Not implemented |
| E3 | Q3 per group and Q4 for impacted devices, every minute | Above a baseline over about 40 hours with decay, plus k standard deviations, k not given | e.g. 5 minutes above with e.g. 100 sessions; a root cause when more than e.g. 80% of subgroups are anomalous | Conviva patent | Not implemented |
| E4 | Q3 per CDN, or Q5 | Rebuffering above 5%, or start time above a set threshold | 5-minute interval; at least 100 concurrent viewers | Mux | Not implemented |
| E5 | Q3 and Q5 by app version | A new version worse than the baseline | Roll back automatically | Netflix | Not implemented |

## 6. Benchmark metrics

| Group | Measure | Status |
| --- | --- | --- |
| Query accuracy | Relative error of each city's entropy and L1 norm | Not yet |
| Decision | For E1: TP / FP / FN / TN per city per evaluation; onset delay; gaps | Not yet |
| Decision | The same for E2 to E5 | Not yet |
| Cost | Query latency per evaluation, without plan compile time | Not yet |
| Cost | View refresh time and insert throughput; the sketch insertion is replayed in one batch per window, not streamed | Not yet |
| Cost | Bytes per evaluation: sketch state bytes plus raw sample bytes read at query (samples × 16 bytes) | Not yet |
| Cost | Plan compile time; planning time | Not yet |
| Cost | Process peak memory and CPU time | Not yet |

Every exact and approximate plan comes from ASAPPlanner and runs in `planeval`.

## 7. Reproduce

TODO
