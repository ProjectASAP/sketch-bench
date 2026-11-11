#!/usr/bin/env python3

import argparse
import json
import os
import re
import shutil
import signal
import socket
import statistics
import subprocess
import sys
import time
import urllib.error
import urllib.request

import bounds
import data_root
import rate_limit_rules
import rules
import run_use_case as in_process
import scenario
import topk_set
import video_rules
from approx_rows import row, write_run

REPO = in_process.REPO
VICTORIAMETRICS = "VictoriaMetrics"
BACKEND = "ASAPQuery-backend"
SYSTEM_FLAG = {VICTORIAMETRICS: "victoriametrics", BACKEND: "asapquery-backend"}
ARM_OF = {VICTORIAMETRICS: "exact", BACKEND: "approx"}
BACKEND_ENV = "ASAPQUERY_BACKEND_DIR"
READY_SECONDS = 60


def backend_arms(case):
    return [arm for arm in case["arms"] if "noraw" in arm.split("-")]


def requirements(query, max_error, failure_probability):
    if max_error is None:
        return {}
    accuracy = {"max_error": max_error, "failure_probability": failure_probability}
    if query.get("accuracy_metric"):
        accuracy["accuracy_metric"] = query["accuracy_metric"]
    return {"accuracy": accuracy}


def rows_per_second(metric):
    if "label_from_value" in metric:
        streams = 1
    elif "import" in metric:
        streams = metric["planned_series_count"]
    else:
        return sum(1000.0 / g.get("sample_interval_ms", metric["sample_interval_ms"]) * g.get("count", 1)
                   for g in metric["series"])
    return streams * 1000.0 / metric["sample_interval_ms"]


def series_count(metric):
    return metric.get("planned_series_count") or sum(g.get("count", 1) for g in metric["series"])


def planned_sample_interval(metric):
    if "label_from_value" in metric:
        return metric["sample_interval_ms"] * series_count(metric)
    return metric["sample_interval_ms"]


def workload(spec, queries, max_error, failure_probability):
    sources = {}
    for metric in spec["metrics"]:
        source = {"series_count": series_count(metric), "sample_interval": planned_sample_interval(metric)}
        if "max_distinct_per_evaluation" in metric:
            source["max_distinct_per_evaluation"] = metric["max_distinct_per_evaluation"]
        if metric.get("non_negative"):
            source["values_non_negative"] = True
        sources[metric["name"]] = source
    return {
        "query_workload": {
            "language": "promql",
            "repeating_queries": [{
                "query": q["promql"],
                "demand": {"fixed_interval": q["interval_ms"]},
                "requirements": requirements(q, max_error, failure_probability),
                "predictability": {"predictable": {"known_at": None}},
            } for q in queries],
        },
        "data_workload": {
            "arrival": {"ingesting": {"rows_per_second": sum(rows_per_second(m) for m in spec["metrics"])}},
            "sources": sources,
        },
    }


def free_port():
    with socket.socket() as s:
        s.bind(("127.0.0.1", 0))
        return s.getsockname()[1]


def cpu_seconds(text):
    parts = [float(p) for p in re.split("[:-]", text.strip())]
    seconds = 0.0
    for p in parts:
        seconds = seconds * 60 + p
    return seconds


def snapshot(pid):
    out = subprocess.run(["ps", "-o", "rss=,time=", "-p", str(pid)], capture_output=True, text=True).stdout.split()
    return {"resident_bytes": int(out[0]) * 1024, "cpu_seconds": cpu_seconds(out[1])}


def directory_bytes(path):
    total = 0
    for root, _, files in os.walk(path):
        for name in files:
            try:
                total += os.path.getsize(os.path.join(root, name))
            except OSError:
                pass
    return total


def get(url, timeout=5):
    with urllib.request.urlopen(url, timeout=timeout) as response:
        return response.status, response.read().decode()


class Server:
    def __init__(self, name, cmd, ready_url, log):
        self.name, self.ready_url = name, ready_url
        print("$ " + " ".join(cmd), file=sys.stderr)
        self.log = open(log, "w")
        self.started = time.monotonic()
        self.proc = subprocess.Popen(cmd, stdout=self.log, stderr=subprocess.STDOUT)
        deadline = time.monotonic() + READY_SECONDS
        while True:
            if self.proc.poll() is not None:
                self.log.close()
                with open(log) as f:
                    tail = f.read()[-2000:]
                raise RuntimeError(f"{name} exited with {self.proc.returncode} before it was ready: {tail}")
            try:
                if get(ready_url, timeout=1)[0] == 200:
                    break
            except (urllib.error.URLError, ConnectionError, OSError):
                pass
            if time.monotonic() > deadline:
                self.proc.kill()
                raise RuntimeError(f"{name} was not ready within {READY_SECONDS} s")
            time.sleep(0.1)

    def stop(self):
        self.proc.send_signal(signal.SIGTERM)
        _, status, usage = os.wait4(self.proc.pid, 0)
        self.log.close()
        return {"peak_resident_bytes": usage.ru_maxrss * (1 if sys.platform == "darwin" else 1024),
                "cpu_seconds": usage.ru_utime + usage.ru_stime,
                "wall_seconds": time.monotonic() - self.started}


def systemeval(binary, spec, system, url, out, extra):
    cmd = [binary, "--spec", spec, "--system", SYSTEM_FLAG[system], "--url", url, "--out", out] + extra
    print("$ " + " ".join(cmd), file=sys.stderr)
    subprocess.run(cmd, check=True)
    with open(out) as f:
        return json.load(f)


def latency(document):
    seconds = [c["query_seconds"] for q in document["queries"] if "unsupported" not in q for c in q["costs"]
               if c.get("series_returned", 0) is not None]
    if not seconds:
        return {}
    ordered = sorted(seconds)
    return {"mean query ms": statistics.fmean(seconds) * 1e3, "median query ms": ordered[len(ordered) // 2] * 1e3,
            "p99 query ms": ordered[min(len(ordered) - 1, int(0.99 * len(ordered)))] * 1e3,
            "evaluations": len(seconds)}


def system_summary(asked, at_end, process, disk_bytes=None):
    stats = asked["write"]
    out = {"insert samples per second": stats["samples_per_second"], "samples": stats["samples"],
           "write seconds": stats["send_seconds"] + stats["barrier_seconds"],
           "visibility seconds": stats["barrier_seconds"], "write retries": stats["retries"],
           "resident bytes at end": at_end["resident_bytes"],
           "peak resident bytes": process["peak_resident_bytes"], "cpu seconds": process["cpu_seconds"],
           "disk bytes": disk_bytes, "max replay lag seconds": stats.get("max_lag_seconds")}
    out.update(latency(asked))
    out["unsupported"] = {q["id"]: q["unsupported"] for q in asked["queries"] if "unsupported" in q}
    out["failed evaluations"] = {q["id"]: q["failed_evaluations"] for q in asked["queries"]
                                 if q["failed_evaluations"] and "unsupported" not in q}
    finish = (stats.get("status") or {}).get("precompute")
    if finish:
        out["samples dropped"] = {k: v for k, v in finish.items() if k.startswith("samples_dropped") and v}
    return out


def merged_exact(victoriametrics, truth):
    truth_by_id = {q["id"]: q for q in truth["queries"]}
    queries, sources = [], {}
    for q in victoriametrics["queries"]:
        if "unsupported" in q:
            queries.append({**truth_by_id[q["id"]], "costs": []})
            sources[q["id"]] = "the in-process exact plan: VictoriaMetrics does not support the query"
        else:
            queries.append(q)
            sources[q["id"]] = VICTORIAMETRICS
    return {**victoriametrics, "queries": queries}, sources


def truth_errors(victoriametrics):
    out = {}
    for q in victoriametrics["queries"]:
        errors = [a["exact_plan"]["relative_error"] for a in q.get("answers", []) if "exact_plan" in a]
        if errors:
            out[q["id"]] = {"max relative error": max(errors),
                            "answers differing": sum(e > 1e-9 for e in errors), "answers": len(errors)}
    return out


def sketches(node, out):
    if isinstance(node, dict):
        if "Sketch" in node:
            sketch = node["Sketch"]
            sketch = sketch[0] if isinstance(sketch, list) and sketch else sketch
            if sketch not in out:
                out.append(sketch)
        for value in node.values():
            sketches(value, out)
    elif isinstance(node, list):
        for item in node:
            sketches(item, out)


def reachable(dag, root):
    producers = {}
    for edge in dag.get("edges", []):
        producers.setdefault(edge["consumer"], []).append(edge["producer"])
    seen, stack = set(), [root]
    while stack:
        node = stack.pop()
        if node not in seen:
            seen.add(node)
            stack += producers.get(node, [])
    return seen


def describe_backend_plan(plan, promql):
    dag = plan["dag"]["dag"]
    root = next(q["root_node_id"] for q in plan["queries"] if q["query"] == promql)
    nodes = reachable(dag, root)
    found = []
    sketches([n["payload"] for n in dag["nodes"] if n["id"] in nodes], found)
    names = [f"{s.get('algorithm', '?')} {json.dumps(s.get('params'), separators=(',', ':'))}"
             for s in found if isinstance(s, dict)]
    stored = sum(o["node_id"] in nodes for o in plan.get("stored_outputs", []))
    return f"{'; '.join(names) or 'exact'} ({stored} stored outputs)"


def control_plane(binary, spec, queries, arm, workdir, name):
    max_error = in_process.max_error_of(arm)
    path = os.path.join(workdir, f"{name}-workload.json")
    with open(path, "w") as f:
        json.dump(workload(spec, queries, max_error, 0.01), f)
    capabilities = os.path.join(workdir, "capabilities.json")
    with open(capabilities, "w") as f:
        json.dump({"raw_data_retained": False}, f)
    out = os.path.join(workdir, f"{name}-plan.json")
    cmd = [binary, "--workload", path, "--deployment-capabilities", capabilities, "--out", out]
    if "ignore" in arm.split("-"):
        cmd.append("--ignore-accuracy-guarantees")
    proc = subprocess.run(cmd, capture_output=True, text=True)
    if proc.returncode:
        return None, (proc.stderr.strip().splitlines() or ["control plane failed"])[-1]
    with open(out) as f:
        return out, json.load(f)


def plan_arm(binary, spec, arm, workdir):
    planned, plan = control_plane(binary, spec, spec["queries"], arm, workdir, arm)
    if planned:
        return planned, plan, {}
    unsupported = {}
    for q in spec["queries"]:
        alone, reason = control_plane(binary, spec, [q], arm, workdir, f"{arm}-{q['id']}")
        if not alone:
            unsupported[q["id"]] = reason
    supported = [q for q in spec["queries"] if q["id"] not in unsupported]
    if not supported:
        return None, None, unsupported
    planned, plan = control_plane(binary, spec, supported, arm, workdir, arm)
    if not planned:
        raise RuntimeError(f"{arm}: every query plans alone but not together: {plan}")
    return planned, plan, unsupported


def uc3_supported_rules(rows, queries):
    usable = in_process.planned_rules(video_rules.RULES, queries)
    return rules.derive(rows, usable) if usable else []


def uc5_supported_rules(rows, queries):
    out = topk_set.derive(rows, "top_clients", "top10_clients", "client_id") \
        if "top10_clients" in {q["id"] for q in queries} else []
    usable = in_process.planned_rules(rate_limit_rules.RULES, queries)
    return out + (rules.derive(rows, usable) if usable else [])


SUPPORTED_RULES = {"ddos_syn_flood": lambda rows, queries: in_process.uc1_derive(rows, queries),
                   "video_experience": uc3_supported_rules, "top_clients": uc5_supported_rules}


def query_costs(run_id, use_case, signal, query, system):
    labels = {"use_case": use_case, "signal": signal, "query": query["id"], "arm": ARM_OF[system]}
    return [row(c["time"], "low_level", run_id, labels, "query_latency", c["query_seconds"], "s")
            for c in query["costs"] if c.get("series_returned", 0) is not None]


def write_arm(data, case, run_id, exact, approx, unsupported, details, spec):
    use_case = exact["use_case"]
    exact_by_id = {q["id"]: q for q in exact["queries"]}
    rows, errors, relative, costs, planned = [], [], [], [], []
    for q in approx["queries"]:
        signal = case["signal"](q["id"])
        if q["id"] in unsupported:
            costs += query_costs(run_id, use_case, signal, exact_by_id[q["id"]], VICTORIAMETRICS)
            continue
        rows += in_process.query_rows(run_id, use_case, signal, exact_by_id[q["id"]], q)
        errors += in_process.rank_errors(run_id, use_case, signal, q)
        relative += in_process.relative_errors(run_id, use_case, signal, q)
        costs += query_costs(run_id, use_case, signal, q, BACKEND)
        costs += query_costs(run_id, use_case, signal, exact_by_id[q["id"]], VICTORIAMETRICS)
        planned.append(q)
    if not rows:
        if not costs:
            print(f"{run_id}: no query was asked; nothing written", file=sys.stderr)
            return
        write_run(data, run_id, use_case, costs, details, case["split"])
        print(f"{run_id}: no query answered by {BACKEND}; only costs written", file=sys.stderr)
        return
    if unsupported:
        partial = SUPPORTED_RULES.get(use_case)
        if partial is None:
            raise RuntimeError(f"{run_id}: {use_case} has no rules without {sorted(unsupported)}")
        derived = partial(rows, planned)
    else:
        derived = case["derive"](rows, planned, bounds.PLAIN, None)
    derived += relative + errors
    scored_rows, scored = scenario.derive(run_id, spec, case.get("scenario_entity"), derived,
                                          case.get("signals", case["signal"]))
    derived += scored_rows
    details["scenario"] = scored
    details["max_relative_error"] = in_process.max_relative_errors(derived)
    details["fidelity"] = in_process.headlines(derived)
    for signal, by_arm in details.get("scenario", {}).items():
        if signal in details["fidelity"]:
            details["fidelity"][signal]["fidelity"] += in_process.caught_text(by_arm)
    if "stage2" in case and not unsupported:
        details["stage2"] = case["stage2"](derived, errors)
    write_run(data, run_id, use_case, rows + derived + costs, details, case["split"])
    print(f"{run_id}: {len(planned)} queries, {len(unsupported)} not supported by {BACKEND}", file=sys.stderr)


def backend_rev(backend_dir):
    repo = os.path.dirname(os.path.dirname(os.path.abspath(backend_dir)))
    proc = subprocess.run(["git", "-C", repo, "rev-parse", "--short", "HEAD"], capture_output=True, text=True)
    pin = None
    manifest = os.path.join(repo, "Cargo.toml")
    if os.path.exists(manifest):
        with open(manifest) as f:
            found = re.search(r'asap-planner = \{[^}]*rev = "([0-9a-f]+)"', f.read())
            pin = found.group(1) if found else None
    return (proc.stdout.strip() or None), pin


def default_backend_dir():
    return os.environ.get(BACKEND_ENV) or os.path.join(os.path.dirname(REPO), "ASAPQuery-backend", "target",
                                                       "release")


def run_victoriametrics(args, spec_path, workdir, write_flags, truth_path):
    data = os.path.join(workdir, "victoriametrics-data")
    shutil.rmtree(data, ignore_errors=True)
    port = free_port()
    url = f"http://127.0.0.1:{port}"
    server = Server(VICTORIAMETRICS, [args.victoriametrics, f"-storageDataPath={data}", "-retentionPeriod=100y",
                                      f"-httpListenAddr=127.0.0.1:{port}", "-loggerLevel=ERROR"],
                    url + "/health", os.path.join(workdir, "victoriametrics.log"))
    try:
        asked = systemeval(args.systemeval, spec_path, VICTORIAMETRICS, url,
                           os.path.join(workdir, "victoriametrics.json"), write_flags + ["--exact", truth_path])
        at_end = snapshot(server.proc.pid)
        disk = directory_bytes(data)
    finally:
        process = server.stop()
    summary = system_summary(asked, at_end, process, max(disk, directory_bytes(data)))
    if not args.keep_data:
        shutil.rmtree(data, ignore_errors=True)
    return asked, summary


def run_backend(args, spec_path, spec, arm, workdir, write_flags, exact_path):
    planned, plan, unsupported = plan_arm(os.path.join(args.backend_dir, "control_plane"), spec, arm, workdir)
    if not planned:
        return None, None, None, unsupported
    port = free_port()
    url = f"http://127.0.0.1:{port}"
    server = Server(BACKEND, [os.path.join(args.backend_dir, "data_plane"), "--plan", planned, "--no-forward",
                              "--stream-start-ms", str(spec["start_ms"]), "--listen", f"127.0.0.1:{port}"],
                    url + "/-/ready", os.path.join(workdir, f"{arm}-data-plane.log"))
    flags = write_flags + ["--exact", exact_path]
    for q in spec["queries"]:
        if q["id"] not in unsupported:
            flags += ["--query", q["id"]]
    try:
        asked = systemeval(args.systemeval, spec_path, BACKEND, url, os.path.join(workdir, f"{arm}.json"), flags)
        at_end = snapshot(server.proc.pid)
    finally:
        process = server.stop()
    summary = system_summary(asked, at_end, process, 0)
    for q in asked["queries"]:
        if "unsupported" in q:
            unsupported[q["id"]] = q["unsupported"]
    summary["unsupported"] = dict(unsupported)
    return asked, summary, plan, unsupported


def main(argv=None):
    parser = argparse.ArgumentParser(
        description=f"Run a use case on {VICTORIAMETRICS}, which keeps every raw sample, and on {BACKEND} "
                    "planned to keep none, then score both"
    )
    parser.add_argument("use_case", choices=sorted(in_process.USE_CASES))
    parser.add_argument("--planeval", default=os.path.join(REPO, "target", "release", "planeval"))
    parser.add_argument("--systemeval", default=os.path.join(REPO, "target", "release", "systemeval"))
    parser.add_argument("--backend-dir", default=default_backend_dir(),
                        help=f"directory with {BACKEND}'s control_plane and data_plane binaries; default "
                             f"${BACKEND_ENV}, else ../ASAPQuery-backend/target/release beside this repository")
    parser.add_argument("--victoriametrics", default=shutil.which("victoria-metrics") or "victoria-metrics")
    parser.add_argument("--data", default=os.path.join(REPO, "target", "grafana-data"))
    parser.add_argument("--raw-out", default=os.path.join(REPO, "target", "systems"))
    parser.add_argument("--arm", action="append", help="repeatable; default every arm of the use case that keeps "
                                                       "no raw samples")
    parser.add_argument("--speed", type=float,
                        help="replay the data at this many times its own rate; default writes as fast as each "
                             "system accepts")
    parser.add_argument("--batch-samples", type=int, default=10_000)
    parser.add_argument("--real", nargs="?", const="", metavar="DATASET",
                        help="read the use case's recorded data from --data-root, as run_use_case.py does")
    parser.add_argument("--data-root", default=data_root.default(),
                        help=f"default ${data_root.ENV}, else datasets/ in the repository")
    parser.add_argument("--keep-data", action="store_true", help=f"keep {VICTORIAMETRICS}'s data directory")
    args = parser.parse_args(argv)
    case = in_process.USE_CASES[args.use_case]
    arms = args.arm or backend_arms(case)
    for arm in arms:
        if arm not in backend_arms(case):
            parser.error(f"arm {arm} is not one of {backend_arms(case)}")
    real = args.real is not None
    suffix = f"real-{args.real}" if args.real else "real"
    prefix = f"uc{args.use_case}-{suffix}" if real else f"uc{args.use_case}"
    spec_name = case["spec"].replace(".json", f"_{suffix.replace('-', '_')}.json") if real else case["spec"]
    spec_path = os.path.join(REPO, "aqpbm-planeval", "workloads", spec_name)
    if not os.path.exists(spec_path):
        parser.error(f"use case {args.use_case} has no {spec_name}")
    with open(spec_path) as f:
        spec = json.load(f)
    args.data_flags = ["--data-root", args.data_root]
    if real:
        for metric in spec["metrics"]:
            table = metric.get("import", {}).get("table")
            if table:
                data_root.existing(os.path.join(args.data_root, table["dataset"]), args.data_root)
    mode = "bulk" if args.speed is None else f"speed{args.speed:g}"
    run_prefix = f"{prefix}-systems-{mode}"
    workdir = os.path.join(args.raw_out, run_prefix)
    os.makedirs(workdir, exist_ok=True)
    write_flags = args.data_flags + ["--batch-samples", str(args.batch_samples)]
    if args.speed is not None:
        write_flags += ["--speed", str(args.speed)]

    truth_path = os.path.join(workdir, "truth.json")
    truth = in_process.run_binary(args.planeval, spec_path, truth_path,
                                  case.get("planeval_flags", []) + args.data_flags)
    victoriametrics, victoriametrics_summary = run_victoriametrics(args, spec_path, workdir, write_flags, truth_path)
    exact, sources = merged_exact(victoriametrics, truth)
    exact_path = os.path.join(workdir, "exact.json")
    with open(exact_path, "w") as f:
        json.dump(exact, f)
    rev, planner_pin = backend_rev(args.backend_dir)
    for arm in arms:
        approx, backend_summary, plan, unsupported = run_backend(args, spec_path, spec, arm, workdir, write_flags,
                                                                 exact_path)
        run_id = f"{run_prefix}-{arm}"
        if approx is None:
            print(f"{run_id}: no query is supported by {BACKEND}: {unsupported}", file=sys.stderr)
            continue
        details = {
            "arm": {"max_error": in_process.max_error_of(arm), "failure_probability": 0.01,
                    "raw_data_retained": False, "ignore_guarantees": "ignore" in arm.split("-")},
            "trial": 1,
            "plans": {q["id"]: describe_backend_plan(plan, q["promql"]) for q in approx["queries"]
                      if q["id"] not in unsupported},
            "unsupported": {q: f"not supported by {BACKEND}: {reason}" for q, reason in unsupported.items()},
            "exact_plans": {q: source for q, source in sources.items()},
            "promql": {q["id"]: q["promql"] for q in spec["queries"]},
            "planner_rev": planner_pin,
            "backend_rev": rev,
            "dashboard": case["dashboard"],
            "signals": {q["id"]: case.get("signals", case["signal"])(q["id"]) for q in spec["queries"]},
            "write mode": "as fast as each system accepts" if args.speed is None
            else f"replay at {args.speed:g}x the data rate",
            "systems": {VICTORIAMETRICS: victoriametrics_summary, BACKEND: backend_summary},
            "victoriametrics_vs_truth": truth_errors(victoriametrics),
        }
        if "rules" in case:
            details["rules"] = {name: rule["description"] for name, rule in case["rules"].items()}
            details["signal_dashboards"] = {name: "rules" for name in case["rules"]}
        write_arm(args.data, case, run_id, exact, approx, unsupported, details, spec)


if __name__ == "__main__":
    main()
