#!/usr/bin/env python3

import argparse
import sys

from approx_rows import read_index, read_run, row, write_run

APP = "threshold_alert"
FIRING = "firing"
OK = "ok"
NOTHING = "none"
EVENT_LABELS = {"onset_delay": "onset delay", "gap": "gap", "end_delay": "end delay",
                "false_positive": "false positive", "false_negative": "false negative",
                "missed_episode": "missed episode", "decision_delay": "decision delay"}
MARKED_AT_ONE_CELL = ("onset_delay", "decision_delay")


def spans(flags, value):
    out, start = [], None
    for i, flag in enumerate(flags):
        if flag == value and start is None:
            start = i
        elif flag != value and start is not None:
            out.append((start, i))
            start = None
    if start is not None:
        out.append((start, len(flags)))
    return out


def series(query_rows, query):
    arms = {"exact": {}, "approx": {}}
    labels, unit = None, None
    for r in query_rows:
        if r["labels"].get("query") != query:
            continue
        arm = r["labels"].get("arm")
        if arm not in arms:
            raise ValueError(f"query {query}: arm {arm!r} is neither exact nor approx")
        if r["time"] in arms[arm]:
            raise ValueError(f"query {query}: {arm} evaluated twice at {r['time']}")
        arms[arm][r["time"]] = r["value"]
        labels = {k: v for k, v in r["labels"].items() if k != "arm"}
        unit = r["unit"]
    if not arms["exact"]:
        raise ValueError(f"query {query} has no evaluations")
    if set(arms["exact"]) != set(arms["approx"]):
        raise ValueError(f"query {query}: exact and approx are not evaluated at the same times")
    times = sorted(arms["exact"])
    return labels, unit, times, [arms["exact"][t] for t in times], [arms["approx"][t] for t in times]


def derive(query_rows, thresholds):
    if any(r["kind"] != "query_result" for r in query_rows):
        raise ValueError("derive reads query_result rows only")
    run_ids = {r["run_id"] for r in query_rows}
    if len(run_ids) != 1:
        raise ValueError(f"rows span runs {sorted(run_ids)}, not one")
    queries = {r["labels"].get("query") for r in query_rows}
    if queries != set(thresholds):
        raise ValueError(f"queries {sorted(queries)} and thresholds {sorted(thresholds)} differ")
    (run_id,) = run_ids
    out = []
    for query in sorted(thresholds):
        out += derive_query(run_id, *series(query_rows, query), thresholds[query])
    return out


def relative_errors(run_id, labels, times, exact, approx):
    return [row(t, "low_level", run_id, labels, "relative_error", abs(a - e) / abs(e), "ratio")
            for t, e, a in zip(times, exact, approx) if e != 0]


def derive_query(run_id, labels, unit, times, exact, approx, threshold):
    app = {**labels, "app": APP}
    out = relative_errors(run_id, labels, times, exact, approx)
    out += [row(t, "app_state", run_id, app, "threshold", threshold, unit) for t in times]
    return out + alert_fidelity(run_id, app, times, [e > threshold for e in exact], [a > threshold for a in approx])


def episodes_of(exact_on, approx_on, end_delay=False):
    events, delays, end_delays, gaps, missed = [], [], [], 0, 0
    episodes = spans(exact_on, True)
    continued = [False] * len(exact_on)
    for n, (start, end) in enumerate(episodes):
        hits = [i for i in range(start, end) if approx_on[i]]
        if not hits:
            missed += 1
            events.append((start, "missed_episode", end - start))
            continue
        onset = hits[0]
        delays.append(onset - start)
        if onset > start:
            events.append((onset, "onset_delay", onset - start))
        last = hits[-1] + 1 if end_delay else end
        for gap_start, gap_end in spans(approx_on[onset:last], False):
            gaps += 1
            events.append((onset + gap_start, "gap", gap_end - gap_start))
        if end_delay:
            limit = episodes[n + 1][0] if n + 1 < len(episodes) else len(exact_on)
            stop = last
            while stop < limit and approx_on[stop]:
                stop += 1
            end_delays.append(stop - end)
            if stop != end:
                events.append((min(stop, end), "end_delay", stop - end))
            for i in range(end, stop):
                continued[i] = True
    for start, end in spans([a and not e and not c for e, a, c in zip(exact_on, approx_on, continued)], True):
        events.append((start, "false_positive", end - start))
    return {"events": events, "delays": delays, "end_delays": end_delays, "gaps": gaps, "missed": missed,
            "episodes": len(episodes)}


def annotations(length, cells):
    out = [[] for _ in range(length)]
    for start, stop, label in cells:
        for i in range(start, stop):
            if label not in out[i]:
                out[i].append(label)
    return [", ".join(labels) or NOTHING for labels in out]


def cells_of(events, kept=None):
    return [(i, i + (1 if metric in MARKED_AT_ONE_CELL else abs(length)), EVENT_LABELS[metric])
            for i, metric, length in events if kept is None or metric in kept]


def mean(values):
    return sum(values) / len(values)


def delay_summary(delays, name, unit, early, late):
    if not delays:
        return []
    return [(f"mean_absolute_{name}", mean([abs(d) for d in delays]), "evaluations"),
            (early, sum(1 for d in delays if d < 0), unit),
            (late, sum(1 for d in delays if d > 0), unit)]


def alert_fidelity(run_id, app, times, exact_on, approx_on):
    out = []
    found = episodes_of(exact_on, approx_on, end_delay=True)
    marks = annotations(len(times), cells_of(found["events"]))
    for t, e_on, a_on, mark in zip(times, exact_on, approx_on, marks):
        out.append(row(t, "app_state", run_id, {**app, "arm": "exact"}, "alert", FIRING if e_on else OK, "state"))
        out.append(row(t, "app_state", run_id, {**app, "arm": "approx"}, "alert", FIRING if a_on else OK, "state"))
        out.append(row(t, "app_state", run_id, app, "annotation", mark, "state"))

    for i, metric, length in found["events"]:
        out.append(row(times[i], "fidelity_event", run_id, app, metric, length, "evaluations"))
    delays, end_delays = found["delays"], found["end_delays"]

    pairs = list(zip(exact_on, approx_on))
    tp = pairs.count((True, True))
    fn = pairs.count((True, False))
    summary = [
        ("true_positive", tp, "evaluations"),
        ("false_positive", pairs.count((False, True)), "evaluations"),
        ("false_negative", fn, "evaluations"),
        ("true_negative", pairs.count((False, False)), "evaluations"),
        ("episodes", found["episodes"], "episodes"),
        ("missed_episodes", found["missed"], "episodes"),
        ("gaps", found["gaps"], "gaps"),
    ]
    if delays:
        summary.append(("mean_onset_delay", mean(delays), "evaluations"))
    summary += delay_summary(end_delays, "end_delay", "episodes", "early_ends", "late_ends")
    if tp + fn:
        summary.append(("episode_coverage", tp / (tp + fn), "ratio"))
    for metric, value, metric_unit in summary:
        out.append(row(times[-1], "fidelity_summary", run_id, app, metric, value, metric_unit))
    return out


def parse_threshold(text):
    query, sep, value = text.partition("=")
    if not sep or not query:
        raise argparse.ArgumentTypeError(f"{text!r} is not QUERY=VALUE")
    return query, float(value)


def main(argv=None):
    parser = argparse.ArgumentParser(
        description="Derive threshold-alert state and fidelity rows from a run's query results"
    )
    parser.add_argument("--data", required=True)
    parser.add_argument("--run", required=True)
    parser.add_argument("--threshold", type=parse_threshold, action="append", required=True,
                        metavar="QUERY=VALUE")
    args = parser.parse_args(argv)
    entry = next((e for e in read_index(args.data) if e["run_id"] == args.run), None)
    if entry is None:
        sys.exit(f"run {args.run} is not in {args.data}")
    query_rows = read_run(args.data, args.run, "query_result")
    write_run(args.data, args.run, entry["use_case"], query_rows + derive(query_rows, dict(args.threshold)))


if __name__ == "__main__":
    main()
