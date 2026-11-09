# Rate Limiting Top Clients

Rate limiting caps the requests of clients that send too many.
A rule reads query results at every check and decides which clients to limit and for how long.
This page lists what public tools query and decide, and the end-to-end cases we benchmark.
Sources: Cloudflare, AWS WAF, Google Cloud Armor, Microsoft Azure Front Door, Fastly.

## 1. Industry examples

| Source | Metrics | Query | Compared against | Decision |
| --- | --- | --- | --- | --- |
| [Cloudflare (2017)](https://blog.cloudflare.com/counting-things-a-lot-of-different-things/) | Requests per client, counted per point of presence | Sliding-window estimate: the previous window's count weighted by the share of it still in the window, plus the current count | Fixed limit | Over the limit, every server in the point of presence mitigates the client until a known end time. On 400 million requests from 270,000 sources: 0.003% of requests wrongly allowed or limited, no false positives. |
| [Cloudflare rate limiting rules](https://developers.cloudflare.com/waf/rate-limiting-rules/parameters/) ([threshold chart](https://developers.cloudflare.com/waf/rate-limiting-rules/find-rate-limit/), [preset](https://blog.cloudflare.com/rate-limiting-delivering-more-rules-and-greater-control/)) | Requests per characteristic: IP, path, header, cookie, country, ASN (autonomous system number), JA3/JA4 (fingerprints of how the client sets up TLS) and others | Count per `period` | Fixed `requests_per_period`; to pick it, a chart shows the top 50 clients over 1 min, 5 min or 1 h | Block (429 by default), challenge or log. `mitigation_timeout` > 0: act on all of the client's requests for that time; 0: act only on requests over the limit. Preset: more than 5 logins in 5 minutes blocks for 15 minutes. |
| [AWS WAF rate-based rule](https://docs.aws.amazon.com/waf/latest/developerguide/waf-rule-statement-type-rate-based-high-level-settings.html) ([caveats](https://docs.aws.amazon.com/waf/latest/developerguide/waf-rule-statement-type-rate-based-caveats.html), [managed keys](https://docs.aws.amazon.com/waf/latest/developerguide/listing-managed-ips.html), [blog](https://aws.amazon.com/blogs/security/three-most-important-aws-waf-rate-based-rules/)) | Requests per source IP by default; also forwarded IP or custom keys | Count over a trailing window of 60, 120, 300 (default) or 600 s, estimated with more weight on recent requests | Fixed limit, at least 10; the AWS blog uses 2,000 per 5 minutes, and 10 for a login page | Rule action (block by default) while the key is over the limit; detection and release usually lag less than 30 s. At most 10,000 IPs per rule; when more exceed the limit, those with the highest rates. |
| [AWS Security Automations for AWS WAF](https://docs.aws.amazon.com/solutions/latest/security-automations-for-aws-waf/step-1.-launch-the-stack.html) ([queries](https://github.com/aws-solutions/aws-waf-security-automations/blob/main/source/log_parser/build_athena_queries.py)) | Requests per IP, optionally also per country or URI | With the Athena log parser: SQL every 5 minutes, `COUNT(*)` per IP per minute (`date_trunc('minute', …)`), `ORDER BY … DESC LIMIT 10000` | Default 100 requests per 5 minutes per IP; 50 bad requests per minute per IP | Add the IP to a blocked set for 240 minutes (`WAF Block Period`) |
| [Google Cloud Armor](https://docs.cloud.google.com/armor/docs/rate-limiting-overview) | Requests per key: IP, header, cookie, path, region, ASN, JA3/JA4 and others, up to 3 combined | Count per `interval_sec`, 10 to 3,600 s | Fixed `rate_limit_threshold_count`; 500 per 60 s in a default security policy; sizing guidance: the 99th percentile of the maximum requests per IP per minute | Throttle: deny requests over the limit for the rest of the interval. Ban: block all of the client's requests for the rest of the interval plus `ban_duration_sec` (60 to 3,600 s), optionally only above a second, higher `ban_threshold_count`. |
| [Cloudflare Pingora `pingora-limits`](https://blog.cloudflare.com/how-pingora-keeps-count/) ([source](https://docs.rs/crate/pingora-limits/latest/source/src/estimator.rs)) | Connections per customer ID, server IP and hostname | Lock-free Count-Min sketch estimate per interval | Fixed connection limit | Reject with 503 over the limit. The estimate never undercounts. |
| [Microsoft Azure Front Door WAF](https://learn.microsoft.com/en-us/azure/web-application-firewall/afds/waf-front-door-rate-limit) | Requests per socket IP by default; in preview, per country or region, ASN or JA4 fingerprint | Count over a fixed window of 1 or 5 minutes, per Front Door server | Fixed `rateLimitThreshold`, e.g. 500 per 5 minutes per JA4 fingerprint; sizing guidance: peak and 99th percentile per key in 5-minute bins over 7 days | Block, JavaScript challenge or log for the rest of the fixed window. Below about 200 requests per minute, some requests over the threshold can get through. |
| [Fastly](https://www.fastly.com/documentation/guides/security/rate-limiting/about-rate-limiting/) | Requests per client IP, shared within a point of presence | Rate counter over a 1, 10 or 60 s window, converted to requests per second; may undercount by up to 10% | Fixed requests per second | Put the client in a penalty box, which logs and blocks its later requests for at least 2 minutes. |

## 2. Query Candidates

| ID | Query | From | ASAPPlanner (b822553) |
| --- | --- | --- | --- |
| Q1 | Top clients: `topk(10, sum by (client_id) (count_over_time(requests[5m])))` | Current workload; Cloudflare threshold chart (top 50); AWS WAF (10,000 highest) | Runs; no sketch without raw data at error target 0.01; with `--ignore-guarantees` the plan uses Count-Min with a heap |
| Q2 | Clients over a limit: `sum by (client_id) (count_over_time(requests[5m])) > 2000` | AWS WAF, Cloudflare, Azure Front Door | Runs; no sketch in the plan, even without raw data |
| Q3 | Count per client over 1 minute: `sum by (client_id) (count_over_time(requests[1m]))`; Fastly also uses `[10s]` and `[1s]` | Google Cloud Armor, AWS Security Automations, Azure Front Door, Fastly | Runs; no sketch in the plan, even without raw data |
| Q4 | Largest per-minute count over the block period: `max_over_time((sum by (client_id) (count_over_time(requests[1m])))[240m:1m])` | AWS Security Automations | Does not run: the planner does not support a subquery above the range |

Checked by running each query through `planeval` on a small synthetic spec, exact and at error target 0.01 without raw data.

The workload (`aqpbm-planeval/workloads/uc5_top_clients.json`) runs Q1 (`top10_clients`) every minute. Q2, Q3 and Q4 are not in the workload.

## 3. Data shape

One stream of requests, one row per request; the value is the client ID, which becomes the `client_id` label.

| Column | Values | Distribution | Source |
| --- | --- | --- | --- |
| time | Timestamp | One request every 2 ms, so 500 per second | Current workload |
| client_id | Integer ID | Normal traffic: Zipf, skew 1.0, over 100,000 clients. <br>Minutes 30 to 70: 3% of requests come from 3 heavy clients (IDs 200000 to 200002), ramping in and out over 20 minutes. <br>Minutes 80 to 110: 2% from 2 heavy clients (IDs 300000 and 300001), ramping over 5 minutes. | Current workload. Many clients: 400 million requests from 270,000 sources ([Cloudflare, 2017](https://blog.cloudflare.com/counting-things-a-lot-of-different-things/)). Real data: [NASA-HTTP](https://ita.ee.lbl.gov/html/contrib/NASA-HTTP.html) logs, 3.46 million requests in July and August 1995, with the client host and a 1-second timestamp; [Online Shopping Store - Web Server Logs](https://dataverse.harvard.edu/dataset.xhtml?persistentId=doi:10.7910/DVN/3QBYB5) (Zaker, 2019), the Nginx access log of the e-commerce site zanbil.ir, CC0, access log restricted on request to the author, with a 3.3 GB [mirror on Kaggle](https://www.kaggle.com/datasets/eliasdabbas/web-server-access-logs). |

At full share each heavy client sends 1% of requests, between the 8th and 9th largest Zipf clients.

The workload runs 120 minutes and has time and client_id. Path, status code and other keys are not generated.

## 4. Decision rules

A rule has two parts: what it concludes from each query result, and how it aggregates many query results into how long a client is limited. Numbers are the values in each source's own example.

On each query result:

- Cloudflare (2017): a client is over when its sliding-window estimate is above the limit.
- Cloudflare rules: a client is over when its count in `period` is above `requests_per_period`. The preset: more than 5 logins in 5 minutes.
- AWS WAF: a key is over when its count in the window is above the limit, at least 10; the AWS blog uses 2,000 per 5 minutes. At most 10,000 IPs are limited; when more are over, those with the highest rates.
- AWS Security Automations: an IP is over at more than 100 requests per 5 minutes (default), or 50 bad requests per minute.
- Google Cloud Armor: a key is over at more than `rate_limit_threshold_count`, 500 per 60 s in a default security policy. A second, higher `ban_threshold_count` can decide the ban.
- Azure Front Door: a key is over when its count in the fixed 1- or 5-minute window is above `rateLimitThreshold`.
- Fastly: a client is over when its rate is above the configured requests per second. The counter may undercount by up to 10%.
- Cloudflare Pingora: a key is over when its estimated connection count is above the limit. The estimate never undercounts.
- Ours (current): a client is limited when it is in the top 10.

Aggregation over multiple query results:

- Cloudflare (2017): mitigate until a known end time.
- Cloudflare rules: with `mitigation_timeout` > 0, act on all of the client's requests for that time; with 0, act only on requests over the limit. The preset blocks for 15 minutes.
- AWS WAF: limit while the key is over; release usually less than 30 s after it drops below.
- AWS Security Automations: block for 240 minutes.
- Google Cloud Armor: throttle denies requests over the limit for the rest of the interval. Ban blocks all requests for the rest of the interval plus `ban_duration_sec`, 60 to 3,600 s.
- Azure Front Door: limit for the rest of the fixed window.
- Fastly: put the client in the penalty box for at least 2 minutes.
- Cloudflare Pingora: reject with 503 while over the limit.
- Ours (current): cap until the next evaluation, 1 minute later.

Ours, not from a public source, to study how a rule can use the error bound:

- Each approximate result carries an interval from its error bound. A client is over when the whole interval is above the limit, not over when it is wholly below, and unsure otherwise.
- Same, but an unsure result reruns that one query with the exact plan.

## 5. End-to-end candidates

| ID | Query | On each result | Aggregation | From | Rule in `scripts/` |
| --- | --- | --- | --- | --- | --- |
| E1 | Q1, every minute | In the top 10 | Capped until the next evaluation | Current workload | `topk_set.py` |
| E2 | Q2 | > 2,000 | Limited while over; released within about 30 s; at most 10,000 keys, the highest first | AWS WAF | Not implemented |
| E3 | Q2 on login requests | > 5 | Blocked for 15 minutes | Cloudflare (preset) | Not implemented |
| E4 | Q3 | > 500 | Banned for the rest of the interval plus `ban_duration_sec` | Google Cloud Armor | Not implemented |
| E5 | Q2 on a fixed 5-minute window | > `rateLimitThreshold` | Limited for the rest of the window | Azure Front Door | Not implemented |
| E6 | Q3 over 10 s | > the configured requests per second | Penalty box for at least 2 minutes | Fastly | Not implemented |
| E7 | Q4, every 5 minutes | > the default 100 requests per 5 minutes | Blocked for 240 minutes | AWS Security Automations | Not implemented; Q4 does not run |

## 6. Benchmark metrics

| Group | Measure | Status |
| --- | --- | --- |
| Query accuracy | Overlap with the exact top 10; relative error of each answer | Not yet |
| Decision | For E1: TP / FP / FN / TN per client per evaluation; precision and recall; evaluations with identical sets; clients wrongly in the top 10 | Not yet |
| Decision | The same for E2 to E7 | Not yet |
| Decision | Agreement with the injected heavy clients | Not yet |
| Cost | Query latency per evaluation, without plan compile time | Not yet |
| Cost | View refresh time and insert throughput; the sketch insertion is replayed in one batch per window, not streamed | Not yet |
| Cost | Bytes per evaluation: sketch state bytes plus raw sample bytes read at query (samples × 16 bytes) | Not yet |
| Cost | Plan compile time; planning time | Not yet |
| Cost | Process peak memory and CPU time | Not yet |

Every exact and approximate plan comes from ASAPPlanner and runs in `planeval`.

## 7. Reproduce

TODO
