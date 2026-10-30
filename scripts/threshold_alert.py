#!/usr/bin/env python3

import argparse
import sys

from approx_rows import read_index, read_run, row, write_run

APP = "threshold_alert"
FIRING = "firing"
OK = "ok"


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


def episodes_of(exact_on, approx_on):
    events, delays, breaks, missed = [], [], 0, 0
    episodes = spans(exact_on, True)
    for start, end in episodes:
        hits = [i for i in range(start, end) if approx_on[i]]
        if not hits:
            missed += 1
            events.append((start, "missed_episode", end - start))
            continue
        onset = hits[0]
        delays.append(onset - start)
        if onset > start:
            events.append((start, "late_onset", onset - start))
        for gap_start, gap_end in spans(approx_on[onset:end], False):
            breaks += 1
            events.append((onset + gap_start, "break", gap_end - gap_start))
    for start, end in spans([a and not e for e, a in zip(exact_on, approx_on)], True):
        events.append((start, "false_positive", end - start))
    return {"events": events, "delays": delays, "breaks": breaks, "missed": missed, "episodes": len(episodes)}


def alert_fidelity(run_id, app, times, exact_on, approx_on):
    out = []
    for t, e_on, a_on in zip(times, exact_on, approx_on):
        out.append(row(t, "app_state", run_id, {**app, "arm": "exact"}, "alert", FIRING if e_on else OK, "state"))
        out.append(row(t, "app_state", run_id, {**app, "arm": "approx"}, "alert", FIRING if a_on else OK, "state"))

    found = episodes_of(exact_on, approx_on)
    for i, metric, length in found["events"]:
        out.append(row(times[i], "fidelity_event", run_id, app, metric, length, "evaluations"))
    delays, breaks, missed, episodes = found["delays"], found["breaks"], found["missed"], found["episodes"]

    pairs = list(zip(exact_on, approx_on))
    tp = pairs.count((True, True))
    fn = pairs.count((True, False))
    summary = [
        ("true_positive", tp, "evaluations"),
        ("false_positive", pairs.count((False, True)), "evaluations"),
        ("false_negative", fn, "evaluations"),
        ("true_negative", pairs.count((False, False)), "evaluations"),
        ("episodes", episodes, "episodes"),
        ("missed_episodes", missed, "episodes"),
        ("breaks", breaks, "breaks"),
    ]
    if delays:
        summary.append(("mean_onset_delay", sum(delays) / len(delays), "evaluations"))
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
