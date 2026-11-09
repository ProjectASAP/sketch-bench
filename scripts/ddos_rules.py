#!/usr/bin/env python3

from rules import above, above_band, above_mean_times, cusum_above, either, for_consecutive, x_of_last_y

THRESHOLDS = {"entropy_10s": 8.5, "distinct_10s": 3000, "distinct_5s": 1700}
RED_HAT_EVALUATIONS = 3
DATADOG_HISTORY, DATADOG_SIGMAS = 60, 3
KENTIK_HISTORY, KENTIK_FACTOR, KENTIK_TIMES, KENTIK_WINDOW = 60, 2.0, 3, 6
CUSUM_TRAINING, CUSUM_K, CUSUM_H = 50, 0.5, 5


def server(labels):
    return labels["vm"]


def any_threshold(v):
    out = [False] * len(v["entropy_10s"])
    for query, threshold in THRESHOLDS.items():
        out = either(out, above(v[query], threshold))
    return out


def datadog(v):
    return above_band(v["distinct_10s"], DATADOG_HISTORY, DATADOG_SIGMAS)


def kentik(v):
    return above_mean_times(v["distinct_10s"], KENTIK_HISTORY, KENTIK_FACTOR)


def netflow_logic(v):
    return cusum_above(v["entropy_10s"], CUSUM_TRAINING, CUSUM_K, CUSUM_H)


RULES = {
    "red_hat_for_15s": {
        "queries": tuple(THRESHOLDS), "condition": any_threshold, "entity": server,
        "hold": for_consecutive(RED_HAT_EVALUATIONS),
        "description": "Any of the PromSketch thresholds (entropy_10s > 8.5, distinct_10s > 3,000, "
                       "distinct_5s > 1,700) holds for 15 s, 3 evaluations in a row (Red Hat for: 15s)"},
    "datadog": {
        "queries": ("distinct_10s",), "condition": datadog, "entity": server,
        "description": "Distinct source IPs over 10 s above the mean plus 3 standard deviations of the previous "
                       "5 minutes (Datadog baseline deviation; window and width ours)"},
    "kentik": {
        "queries": ("distinct_10s",), "condition": kentik, "entity": server,
        "hold": x_of_last_y(KENTIK_TIMES, KENTIK_WINDOW),
        "description": "Distinct source IPs over 10 s above 200% of the mean of the previous 5 minutes, at least "
                       "3 times in the last 6 evaluations (Kentik; X, Y and baseline window ours)"},
    "netflow_logic": {
        "queries": ("entropy_10s",), "condition": netflow_logic, "entity": server,
        "description": "One-sided CUSUM on source IP entropy: mean and standard deviation from the first "
                       "50 evaluations, slack 0.5 and alarm at 5 standard deviations (NetFlow Logic patent; "
                       "parameters ours)"},
}
