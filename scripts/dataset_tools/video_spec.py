#!/usr/bin/env python3

import argparse
import itertools
import json
import os

REPO = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
OUT = os.path.join(REPO, "aqpbm-usecase", "workloads", "uc3_video.json")

START_MS = 1767225600000
MINUTE = 60000
DIMENSIONS = {
    "city": ([f"city-{i}" for i in range(6)], 0.9),
    "isp": ([f"isp-{i}" for i in range(3)], 0.8),
    "cdn": (["cdn-0", "cdn-1"], 0.7),
    "device": (["mobile", "tv"], 0.7),
}
SESSIONS_PER_MINUTE = 6000
RUNGS = 10

INCIDENTS = (
    {"name": "buffering", "match": {"city": "city-0", "cdn": "cdn-1"}, "metric": "buffering",
     "start_minute": 40, "end_minute": 55, "ramp_minutes": 2,
     "change": {"mix": {"share": 0.4, "distribution": {"kind": "uniform", "lower_bound": 5, "upper_bound": 20,
                                                       "seed": 301}}}},
    {"name": "bitrate", "match": {"city": "city-1", "isp": "isp-2"}, "metric": "bitrate",
     "start_minute": 70, "end_minute": 85, "ramp_minutes": 2,
     "change": {"mix": {"share": 0.5, "distribution": {"kind": "uniform", "lower_bound": 8, "upper_bound": 11,
                                                       "seed": 302}}}},
    {"name": "startup", "match": {"cdn": "cdn-0", "device": "tv"}, "metric": "startup_time",
     "start_minute": 95, "end_minute": 110, "ramp_minutes": 2, "change": {"scale": {"factor": 2.0}}},
)


def zipf_weights(n, skew):
    raw = [1 / (k + 1) ** skew for k in range(n)]
    return [w / sum(raw) for w in raw]


def subpopulations():
    names = list(DIMENSIONS)
    weights = {d: dict(zip(values, zipf_weights(len(values), skew))) for d, (values, skew) in DIMENSIONS.items()}
    for combo in itertools.product(*(DIMENSIONS[d][0] for d in names)):
        labels = dict(zip(names, combo))
        share = 1.0
        for d in names:
            share *= weights[d][labels[d]]
        yield labels, max(1, round(SESSIONS_PER_MINUTE * share))


def baselines(sessions, seed):
    return {
        "bitrate": {"kind": "zipf", "skewness": 1.0, "population_size": RUNGS, "seed": seed},
        "buffering": {"kind": "normal", "mean": 0.3, "standard_deviation": 0.6, "seed": seed + 1},
        "play_time": {"kind": "uniform", "lower_bound": 55, "upper_bound": 60, "seed": seed + 2},
        "startup_time": {"kind": "pareto", "alpha": 3.0, "scale": 1.0, "seed": seed + 3},
        "device_id": {"kind": "uniform", "lower_bound": 0, "upper_bound": 3 * sessions, "seed": seed + 4},
    }


METRICS = {"bitrate": {"floor": True}, "buffering": {"non_negative": True}, "play_time": {},
           "startup_time": {"non_negative": True}, "device_id": {"floor": True}}


def phases_of(labels, metric):
    out = []
    for incident in INCIDENTS:
        if incident["metric"] != metric or any(labels[k] != v for k, v in incident["match"].items()):
            continue
        out.append({"start_offset_ms": incident["start_minute"] * MINUTE,
                    "end_offset_ms": incident["end_minute"] * MINUTE,
                    "ramp_ms": incident["ramp_minutes"] * MINUTE, "change": incident["change"]})
    return out


def queries():
    every = {"window_ms": MINUTE, "interval_ms": MINUTE}
    five = {"window_ms": 5 * MINUTE, "interval_ms": MINUTE}
    return [
        {"id": "entropy_bitrate", "promql": "entropy_over_time(bitrate[1m])", **every, "unit": "bits",
         "truth": {"statistic": "entropy", "metric": "bitrate"}},
        {"id": "buffering_l1", "promql": "sum_over_time(buffering[1m])", **every, "unit": "s"},
        {"id": "sessions", "promql": "count_over_time(bitrate[1m])", **every, "unit": "sessions"},
        {"id": "distinct_devices", "promql": "distinct_over_time(device_id[1m])", **every, "unit": "devices",
         "truth": {"statistic": "distinct", "metric": "device_id"}},
        {"id": "startup_p95", "promql": "quantile_over_time(0.95, startup_time[5m])", **five, "unit": "s",
         "accuracy_metric": "rank", "truth": {"statistic": {"quantile": 0.95}, "metric": "startup_time"}},
        {"id": "rebuffer_ratio",
         "promql": "sum_over_time(buffering[5m]) / (sum_over_time(play_time[5m]) + sum_over_time(buffering[5m]))",
         **five, "unit": "ratio"},
    ]


def spec(duration_minutes):
    groups = list(subpopulations())
    sessions = sum(MINUTE // max(1, MINUTE // n) for _, n in groups)
    mean_interval = round(len(groups) * MINUTE / sessions)
    metrics = []
    for n, (name, flags) in enumerate(METRICS.items()):
        series = []
        for i, (labels, sessions) in enumerate(groups):
            group = {"sample_interval_ms": max(1, MINUTE // sessions), "labels": labels,
                     "baseline": baselines(sessions, 1000 + 10 * i)[name]}
            phases = phases_of(labels, name)
            if phases:
                group["phases"] = phases
            series.append(group)
        metrics.append({"name": name, "sample_interval_ms": mean_interval, **flags, "series": series})
    return {"use_case": "video_experience", "start_ms": START_MS, "duration_ms": duration_minutes * MINUTE,
            "seed": 31, "metrics": metrics, "queries": queries()}


def main(argv=None):
    parser = argparse.ArgumentParser(description="Write the video experience workload: one series per "
                                                 "subpopulation of city, ISP, CDN and device")
    parser.add_argument("--out", default=OUT)
    parser.add_argument("--minutes", type=int, default=120)
    args = parser.parse_args(argv)
    with open(args.out, "w") as f:
        json.dump(spec(args.minutes), f, indent=1)
        f.write("\n")


if __name__ == "__main__":
    main()
