#!/usr/bin/env python3

import argparse
import json
import os
import re
import subprocess
import sys
import time

sys.path.insert(0, os.path.join(os.path.dirname(os.path.dirname(os.path.abspath(__file__))), "dataset_tools"))

import bounds
import cases
import data_root
import decision
import rollout
import rules
import scenario
import threshold_alert
import topk_set
from approx_rows import row, write_run

REPO = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
SAMPLE_BYTES = 16


def arm_flags(arm):
    flags = []
    for part in arm.split("-"):
        if part == "noraw":
            flags.append("--no-raw")
        elif part == "ignore":
            flags.append("--ignore-guarantees")
        elif part.startswith("eps"):
            flags += ["--max-error", part[3:]]
        elif part != "raw":
            raise ValueError(f"arm {arm}: unknown part {part}")
    return flags


def max_error_of(arm):
    found = [float(part[3:]) for part in arm.split("-") if part.startswith("eps")]
    return found[0] if found else None


USE_CASES = {case_id: module.CASE for case_id, module in cases.load().items()}


POLICY_TEXT = {
    bounds.BOUND: "An answer whose error-bound interval leaves the check open takes no action",
    bounds.RERUN: "An answer whose error-bound interval leaves the check open reruns the check's queries with "
                  "the exact plan",
}


def run_binary(binary, spec, out, flags):
    """Run planeval and return its output, with the process's peak resident memory and CPU time."""
    cmd = [binary, "--spec", spec, "--out", out] + flags
    print("$ " + " ".join(cmd), file=sys.stderr)
    started = time.monotonic()
    proc = subprocess.Popen(cmd)
    _, status, usage = os.wait4(proc.pid, 0)
    proc.returncode = os.waitstatus_to_exitcode(status)
    if proc.returncode:
        raise subprocess.CalledProcessError(proc.returncode, cmd)
    with open(out) as f:
        document = json.load(f)
    document["process"] = {
        "peak_memory_bytes": usage.ru_maxrss * (1 if sys.platform == "darwin" else 1024),
        "cpu_seconds": usage.ru_utime + usage.ru_stime,
        "wall_seconds": time.monotonic() - started,
    }
    return document


def describe_plan(plan):
    sketches = [
        f"{s.get('algorithm', '?')} {json.dumps(s.get('params'), separators=(',', ':'))}"
        for s in plan["sketches"]
    ]
    refresh = [v["refresh"] for v in plan["materialized_views"]]
    where = "query time" if not refresh else (
        f"{refresh.count('at_ingestion')} views at ingestion" if "at_ingestion" in refresh
        else f"{len(refresh)} views kept across evaluations")
    return f"{'; '.join(sketches) or 'exact'} ({where})"


def sketch_bounds(query):
    plan = query["plan"]
    rows = []
    for sketch, analytics in zip(plan["sketches"], plan.get("sketch_analytics", [])):
        for readout in analytics["readouts"] or [{}]:
            error = readout.get("error") or {}
            rows.append({
                "query": query["id"],
                "sketch": f"{sketch.get('algorithm', '?')} {json.dumps(sketch.get('params'), separators=(',', ':'))}",
                "statistic": readout.get("statistic"),
                "bound metric": error.get("metric"),
                "epsilon": error.get("epsilon"),
                "delta": error.get("delta"),
                "merged error": readout.get("merged_error"),
                "sketchlib bytes per state": analytics["memory_bytes"],
                "planner peak bytes for the plan": plan["predicted_peak_memory_bytes"],
                "update ops": analytics["update_ops"],
            })
    return rows


def short_reason(reason):
    m = re.search(r'choice \d+: ([^"]+?)(?: AccuracyTarget|")', reason)
    return m.group(1) if m else reason[:160]


def query_rows(run_id, use_case, signal, exact_query, approx_query):
    labels = {"use_case": use_case, "signal": signal, "query": exact_query["id"]}
    rows = []
    for arm, query in (("exact", exact_query), ("approx", approx_query)):
        for a in query["answers"]:
            series = {k: v for k, v in a["series"].items() if k != "__name__"}
            rows.append(row(a["time"], "query_result", run_id, {**labels, **series, "arm": arm}, "value",
                            a["value"], exact_query["unit"]))
    return sorted(rows, key=lambda r: r["time"])


def unnamed(series):
    return {k: v for k, v in series.items() if k != "__name__"}


def exact_plan_errors(run_id, use_case, signal, approx_query, error, metric, unit):
    labels = {"use_case": use_case, "signal": signal, "query": approx_query["id"]}
    return [row(a["time"], "low_level", run_id, {**labels, **unnamed(a["series"])}, metric,
                a["exact_plan"][error], unit)
            for a in approx_query["answers"] if error in a.get("exact_plan", {})]


def rank_errors(run_id, use_case, signal, approx_query):
    return exact_plan_errors(run_id, use_case, signal, approx_query, "rank_error", "rank_error", "rank")


def relative_errors(run_id, use_case, signal, approx_query):
    return exact_plan_errors(run_id, use_case, signal, approx_query, "relative_error", "relative_error", "ratio")


def query_costs(run_id, use_case, signal, exact_query, approx_query):
    labels = {"use_case": use_case, "signal": signal, "query": exact_query["id"]}
    rows = []
    for arm, query in (("exact", exact_query), ("approx", approx_query)):
        for c in query.get("costs", []):
            rows.append(row(c["time"], "low_level", run_id, {**labels, "arm": arm}, "query_latency",
                            c["query_seconds"], "s"))
            rows.append(row(c["time"], "low_level", run_id, {**labels, "arm": arm}, "query_memory",
                            c["state_bytes"] + c["samples_read_at_query"] * SAMPLE_BYTES, "bytes"))
    return rows


def mean(values):
    return sum(values) / len(values) if values else None


def costs_of(query):
    costs = query.get("costs", [])
    refresh = sum(c["refresh_seconds"] for c in costs)
    return {
        "mean query ms": mean([c["query_seconds"] * 1e3 for c in costs]),
        "mean compile ms": mean([c["compile_seconds"] * 1e3 for c in costs]),
        "planning ms": query.get("plan", {}).get("planning_ms"),
        "insert samples per second": sum(c["samples_ingested"] for c in costs) / refresh if refresh else None,
        "peak state bytes": max((c["state_bytes"] for c in costs), default=None),
        "mean samples read at query": mean([c["samples_read_at_query"] for c in costs]),
        "peak raw sample bytes read at query": max((c["samples_read_at_query"] * SAMPLE_BYTES for c in costs),
                                                   default=None),
    }


def stage1(arm, exact_query, approx_query, derived):
    query = exact_query["id"]
    ranked = any("rank_error" in a.get("exact_plan", {}) for a in approx_query["answers"])
    metric = "rank_error" if ranked else "relative_error"
    errors = [r["value"] for r in derived
              if r["kind"] == "low_level" and r["metric"] == metric and r["labels"]["query"] == query]
    target = arm["max_error"]
    out = {"query": query, "error metric": metric, "target": target,
           "failure probability": arm["failure_probability"],
           "max error": max(errors, default=None), "mean error": mean(errors), "evaluations": len(errors),
           "evaluations over target": None if target is None else sum(e > target for e in errors)}
    out.update(costs_of(approx_query))
    out.update({f"exact {k}": v for k, v in costs_of(exact_query).items()})
    return out


def delay_text(s, metric, name):
    return f"; {name} {s[metric]:.2f}" if metric in s else ""


def signed_delay_text(s, metric, name, early, late):
    if metric not in s:
        return ""
    return f"; {name} {s[metric]:.2f} ({s[early]} early, {s[late]} late)"


def counted(n, noun, plural=None):
    return f"{n} {noun}" if n == 1 else f"{n} {plural or noun + 's'}"


def headline(app, s):
    if app == threshold_alert.APP:
        total = s["true_positive"] + s["false_positive"] + s["false_negative"] + s["true_negative"]
        return ((s["true_positive"] + s["true_negative"]) / total,
                f"{s['false_positive']} false positive and {s['false_negative']} false negative evaluations; "
                f"{s['missed_episodes']} of {counted(s['episodes'], 'episode')} missed; {counted(s['gaps'], 'gap')}"
                + delay_text(s, "mean_onset_delay", "mean onset delay")
                + signed_delay_text(s, "mean_absolute_end_delay", "mean absolute end delay", "early_ends",
                                    "late_ends"))
    if app == decision.APP:
        return (s["action_agreement"],
                f"{s['true_positive']} matched, {s['false_positive']} false and {s['false_negative']} missed "
                f"scaling actions ({s['up_true_positive']} up, {s['down_true_positive']} down matched)"
                + signed_delay_text(s, "mean_absolute_onset_delay", "mean absolute onset delay", "early_actions",
                                    "late_actions"))
    if app == rollout.APP:
        return (s["agreement"],
                f"{s['true_positive'] + s['true_negative']} of {counted(s['rollouts'], 'rollout')} decided the same; "
                f"{counted(s['false_positive'], 'false rollback')}, "
                f"{counted(s['false_negative'], 'missed rollback')}; {counted(s['gaps'], 'gap')} in failed checks"
                + signed_delay_text(s, "mean_absolute_rollback_delay", "mean absolute rollback delay",
                                    "early_rollbacks", "late_rollbacks"))
    if app == rules.APP:
        return (s["agreement"],
                f"{s['true_positive']} true positive, {s['false_positive']} false positive and "
                f"{s['false_negative']} false negative entity-evaluations over "
                f"{counted(s['entities'], 'entity', 'entities')}; "
                f"{s['missed_entries']} of {counted(s['entries'], 'episode')} missed; {counted(s['gaps'], 'gap')}"
                + delay_text(s, "mean_onset_delay", "mean onset delay"))
    if app == topk_set.APP:
        size = s["mean_list_size"]
        return (s["mean_overlap"] / size,
                f"mean overlap {s['mean_overlap']:.2f} of "
                f"{counted(int(size) if float(size).is_integer() else round(size, 2), 'client')}; "
                f"identical lists {s['identical_sets']} of {s['evaluations']}; "
                f"{s['true_positive']} true positive, {s['false_positive']} false positive and "
                f"{s['false_negative']} false negative client-evaluations; precision {s['precision']:.3f}; "
                f"{s['missed_entries']} of {counted(s['entries'], 'entry', 'entries')} missed; "
                f"{counted(s['gaps'], 'gap')}"
                + delay_text(s, "mean_onset_delay", "mean onset delay"))
    raise ValueError(f"no headline for application {app}")


def headlines(derived):
    by_signal = {}
    for r in derived:
        if r["kind"] == "fidelity_summary" and r["metric"] != "disagreements" and "arm" not in r["labels"]:
            key = (r["labels"]["app"], r["labels"]["signal"])
            by_signal.setdefault(key, {})[r["metric"]] = r["value"]
    out = {}
    for (app, signal), summary in by_signal.items():
        agreement, text = headline(app, summary)
        if "unsure_checks" in summary:
            text += f"; {counted(summary['unsure_checks'], 'unsure check')}"
        out[signal] = {"application": app, "agreement": agreement, "fidelity": text}
    return out


def max_relative_errors(derived):
    out = {}
    for r in derived:
        if r["kind"] == "low_level" and r["metric"] == "relative_error":
            query = r["labels"]["query"]
            out[query] = max(out.get(query, 0.0), r["value"])
    return out


def run_name(prefix, arm, trial, policy):
    name = f"{prefix}-{arm}" if policy == bounds.PLAIN else f"{prefix}-{arm}-{policy}"
    return name if trial == 1 else f"{name}-trial{trial}"


def caught_text(by_arm):
    return "; incidents caught " + ", ".join(
        f"{arm} {by_arm[arm]['caught']} of {by_arm[arm]['incidents']}" for arm in ("exact", "approx") if arm in by_arm)


def write_arm(data, case, prefix, arm, exact, approx, trial=1, policy=bounds.PLAIN, widen=None, spec=None):
    run_id = run_name(prefix, arm, trial, policy)
    use_case = exact["use_case"]
    exact_by_id = {q["id"]: q for q in exact["queries"]}
    rows, errors, relative, costs, planned, plans, unsupported = [], [], [], [], [], {}, {}
    for q in approx["queries"]:
        if bounds.is_bound_query(q["id"]):
            continue
        if "unsupported" in q:
            unsupported[q["id"]] = short_reason(q["unsupported"])
            continue
        rows += query_rows(run_id, use_case, case["signal"](q["id"]), exact_by_id[q["id"]], q)
        errors += rank_errors(run_id, use_case, case["signal"](q["id"]), q)
        relative += relative_errors(run_id, use_case, case["signal"](q["id"]), q)
        costs += query_costs(run_id, use_case, case["signal"](q["id"]), exact_by_id[q["id"]], q)
        planned.append(q)
        plans[q["id"]] = describe_plan(q["plan"])
    if not rows or unsupported and case["needs_every_query"]:
        print(f"{run_id}: queries {sorted(unsupported)} have no plan; nothing written", file=sys.stderr)
        return
    details = {
        "arm": approx["arm"],
        "trial": trial,
        "plans": plans,
        "unsupported": unsupported,
        "exact_plans": {q["id"]: describe_plan(q["plan"]) for q in exact["queries"] if "plan" in q},
        "promql": {q["id"]: q["promql"] for q in exact["queries"]},
        "planner_rev": approx["planner_rev"],
        "process": {"exact": exact.get("process"), "approx": approx.get("process")},
        "dashboard": case["dashboard"],
        "signals": {q["id"]: case.get("signals", case["signal"])(q["id"]) for q in exact["queries"]},
    }
    if "rules" in case:
        details["rules"] = {name: rule["description"] for name, rule in case["rules"].items()}
        details["signal_dashboards"] = {name: "rules" for name in case["rules"]}
    if policy != bounds.PLAIN:
        details["policy"] = POLICY_TEXT[policy]
        details["bounded_queries"] = widen.bounded_queries()
    derived = case["derive"](rows, planned, policy, widen) + relative + errors
    if spec is not None:
        scored_rows, scored = scenario.derive(run_id, spec, case.get("scenario_entity"), derived,
                                              case.get("signals", case["signal"]))
        derived += scored_rows
        details["scenario"] = scored
    details["max_relative_error"] = max_relative_errors(derived)
    details["sketch_bounds"] = [r for q in planned for r in sketch_bounds(q)]
    details["fidelity"] = headlines(derived)
    for signal, by_arm in details.get("scenario", {}).items():
        if signal in details["fidelity"]:
            details["fidelity"][signal]["fidelity"] += caught_text(by_arm)
    details["stage1"] = [stage1(approx["arm"], exact_by_id[q["id"]], q, derived) for q in planned]
    if "stage2" in case:
        details["stage2"] = case["stage2"](derived, errors)
    write_run(data, run_id, use_case, rows + derived + costs, details, case["split"])
    print(f"{run_id}: {len(plans)} queries, {len(unsupported)} unsupported", file=sys.stderr)


def main(argv=None):
    parser = argparse.ArgumentParser(
        description="Run a use case through ASAPPlanner in process: the exact plan, then each approximate arm"
    )
    parser.add_argument("use_case", choices=sorted(USE_CASES))
    parser.add_argument("--binary", default=os.path.join(REPO, "target", "release", "planeval"))
    parser.add_argument("--data", default=os.path.join(REPO, "target", "grafana-data"))
    parser.add_argument("--raw-out", default=os.path.join(REPO, "target", "planeval"))
    parser.add_argument("--arm", action="append", help="repeatable; default every arm of the use case")
    parser.add_argument("--trials", type=int, default=1,
                        help="runs of each approximate arm, each with fresh sketch seeds")
    parser.add_argument("--real", nargs="?", const="", metavar="DATASET",
                        help="read the use case's recorded data from --data-root instead of generating it: its "
                             "_real spec, or with DATASET (e.g. caida-2019) its _real_<DATASET> spec")
    parser.add_argument("--data-root", default=data_root.default(),
                        help=f"default ${data_root.ENV}, else datasets/ in the repository")
    parser.add_argument("--policy", action="append", choices=bounds.POLICIES,
                        help="repeatable; how a rule treats an answer whose error bound leaves its check open: "
                             "plain ignores the bound, bound takes no action, rerun asks the exact plan; "
                             "default every policy the use case supports")
    args = parser.parse_args(argv)
    case = USE_CASES[args.use_case]
    real = args.real is not None
    suffix = f"real-{args.real}" if args.real else "real"
    prefix = f"uc{args.use_case}-{suffix}" if real else f"uc{args.use_case}"
    spec_name = case["spec"].replace(".json", f"_{suffix.replace('-', '_')}.json") if real else case["spec"]
    spec = os.path.join(cases.WORKLOADS, spec_name)
    if not os.path.exists(spec):
        parser.error(f"use case {args.use_case} has no {spec_name}")
    for arm in args.arm or []:
        if arm not in case["arms"]:
            parser.error(f"arm {arm} is not one of {case['arms']}")
    os.makedirs(args.raw_out, exist_ok=True)
    exact_out = os.path.join(args.raw_out, f"{prefix}-exact.json")
    with open(spec) as f:
        spec_doc = json.load(f)
    if real:
        for metric in spec_doc["metrics"]:
            table = metric.get("import", {}).get("table")
            if table:
                data_root.existing(os.path.join(args.data_root, table["dataset"]), args.data_root)
    extra = case.get("planeval_flags", []) + (["--data-root", args.data_root] if real else [])
    exact = run_binary(args.binary, spec, exact_out, extra)
    for q in exact["queries"]:
        if "unsupported" in q:
            sys.exit(f"the exact plan of {q['id']} is unsupported: {q['unsupported']}")
    policies = args.policy or (bounds.POLICIES if case.get("bounds", True) else (bounds.PLAIN,))
    if not case.get("bounds", True) and set(policies) != {bounds.PLAIN}:
        parser.error(f"use case {args.use_case} supports only the plain policy")
    for arm in args.arm or case["arms"]:
        max_error = max_error_of(arm)
        arm_spec = spec
        if max_error is not None and set(policies) != {bounds.PLAIN}:
            widened = bounds.with_bound_queries(spec_doc, max_error)
            arm_spec = os.path.join(args.raw_out, f"{prefix}-{arm}-spec.json")
            with open(arm_spec, "w") as f:
                json.dump(widened, f)
        for trial in range(1, args.trials + 1):
            name = arm if trial == 1 else f"{arm}-trial{trial}"
            approx = run_binary(args.binary, arm_spec, os.path.join(args.raw_out, f"{prefix}-{name}.json"),
                                arm_flags(arm) + extra + ["--exact", exact_out])
            approx["process"]["includes_bound_queries"] = arm_spec != spec
            widen = bounds.Widen(approx, max_error) if max_error is not None else None
            for policy in policies:
                if policy != bounds.PLAIN and not (widen and widen.bounded_queries()):
                    print(f"{run_name(prefix, arm, trial, policy)}: no plan has an error bound; not written",
                          file=sys.stderr)
                    continue
                write_arm(args.data, case, prefix, arm, exact, approx, trial, policy,
                          widen if policy != bounds.PLAIN else None, spec_doc)

if __name__ == "__main__":
    main()
