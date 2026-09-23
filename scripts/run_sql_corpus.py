#!/usr/bin/env python3

import argparse
import json
import re
import subprocess
import sys
from collections import Counter
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
CLASSIFIED_REFUSAL = re.compile(
    r"^REFUSED (?P<variant>.+?) refused \[(?P<reason>[^\]]+)\]: (?P<detail>.*)$"
)
UNCLASSIFIED_REFUSAL = re.compile(r"^REFUSED (?P<detail>.*)$")
REASON_CLASSES = ("promql_only", "time_axis", "no_constructor", "deferred", "unclassified")


def statements_split_on_semicolon(text):
    body = "\n".join(
        line.strip()
        for line in text.splitlines()
        if line.strip() and not line.strip().startswith("--") and not line.strip().startswith("#")
    )
    return [
        (str(position + 1), statement.strip())
        for position, statement in enumerate(body.split(";"))
        if statement.strip()
    ]


def statements_one_per_line_with_marker_id(text):
    out = []
    marker = None
    for line in text.splitlines():
        line = line.strip()
        if line.startswith("-- U-"):
            marker = line[len("-- U-") :]
        elif line and not line.startswith("--"):
            out.append((marker or str(len(out) + 1), line))
            marker = None
    return out


def statements_whole_file(text):
    return [("1", text.strip())]


CORPUS = [
    {
        "corpus": "netflow",
        "sql": "crates/frontend-sql/tests/netflow/data/netflow.sql",
        "spec": "configs/datagen/netflow_table.yaml",
        "split": statements_split_on_semicolon,
    },
    {
        "corpus": "packets",
        "sql": "crates/frontend-sql/tests/data_quality_check/data/synthetic_packet_trace_queries.sql",
        "spec": "configs/datagen/packets.yaml",
        "split": statements_split_on_semicolon,
    },
    {
        "corpus": "tpch_deequ",
        "sql": "crates/frontend-sql/tests/data_quality_check/data/tpch_deequ_queries.sql",
        "spec": "configs/datagen/lineitem.yaml",
        "split": statements_one_per_line_with_marker_id,
    },
    {
        "corpus": "o11y",
        "sql": "crates/frontend-sql/tests/data/o11y_q07.sql",
        "spec": "configs/datagen/raw_samples.yaml",
        "split": statements_whole_file,
    },
    {
        "corpus": "o11y",
        "sql": "crates/frontend-sql/tests/data/o11y_q09.sql",
        "spec": "configs/datagen/raw_samples.yaml",
        "split": statements_whole_file,
    },
    {
        "corpus": "o11y",
        "sql": "crates/frontend-sql/tests/data/o11y_q10.sql",
        "spec": "configs/datagen/raw_samples.yaml",
        "split": statements_whole_file,
    },
    {
        "corpus": "o11y",
        "sql": "crates/frontend-sql/tests/data/o11y_q12.sql",
        "spec": "configs/datagen/raw_samples.yaml",
        "split": statements_whole_file,
    },
    {
        "corpus": "o11y",
        "sql": "crates/frontend-sql/tests/data/o11y_q27.sql",
        "spec": "configs/datagen/raw_samples.yaml",
        "split": statements_whole_file,
    },
]


def invoke(binary, arguments, timeout):
    try:
        finished = subprocess.run(
            [binary] + arguments,
            cwd=REPO,
            capture_output=True,
            text=True,
            timeout=timeout,
        )
        return finished.returncode, finished.stdout, finished.stderr
    except subprocess.TimeoutExpired:
        return None, "", f"timed out after {timeout}s"


def refusals_in(stdout):
    found = []
    for line in stdout.splitlines():
        matched = CLASSIFIED_REFUSAL.match(line)
        if matched:
            reason = matched.group("reason")
            found.append(
                {
                    "variant": matched.group("variant"),
                    "reason": reason,
                    "reason_class": reason.split("(")[0],
                    "detail": matched.group("detail"),
                }
            )
            continue
        matched = UNCLASSIFIED_REFUSAL.match(line)
        if matched:
            found.append(
                {
                    "variant": matched.group("detail").split(":")[0],
                    "reason": "unclassified",
                    "reason_class": "unclassified",
                    "detail": matched.group("detail"),
                }
            )
    return found


def counts_in(stdout):
    for line in reversed(stdout.splitlines()):
        line = line.strip()
        if not line.startswith("{") or '"promql_only"' not in line:
            continue
        try:
            parsed = json.loads(line)
        except json.JSONDecodeError:
            continue
        if set(parsed) <= set(REASON_CLASSES):
            return parsed
    return None


def record_in(stdout):
    for line in stdout.splitlines():
        line = line.strip()
        if line.startswith("{") and '"schema_version"' in line:
            try:
                return json.loads(line)
            except json.JSONDecodeError:
                continue
    return None


def diagnostic_in(text):
    complaints = [line.strip() for line in text.splitlines() if line.strip().startswith("planeval:")]
    if complaints:
        return complaints[-1]
    remaining = [line.strip() for line in text.splitlines() if line.strip()]
    return remaining[-1] if remaining else ""


def plan_shape(document):
    payloads = []
    families = []
    for node in document.get("dag", {}).get("nodes", []):
        payload = node.get("payload", {})
        kind = payload.get("kind", "unknown")
        payloads.append(kind)
        family = payload.get("family")
        if family is not None:
            families.append(family_name(family))
        if kind == "summary_estimate":
            families.append("estimate:" + family_name(payload.get("query")))
    return payloads, families


def family_name(family):
    if isinstance(family, str):
        return family
    if isinstance(family, dict):
        sketch = family.get("Sketch")
        if isinstance(sketch, list) and sketch and isinstance(sketch[0], dict):
            algorithm = sketch[0].get("algorithm", "unknown")
            grouping = sketch[1] if len(sketch) > 1 else None
            return f"Sketch({algorithm},{family_name(grouping)})"
        exact = family.get("ExactAggregate")
        if isinstance(exact, list) and exact:
            return f"ExactAggregate({family_name(exact[0])})"
        if len(family) == 1:
            key, value = next(iter(family.items()))
            if isinstance(value, (str, int, float)) or not value:
                return key
            return f"{key}({compact(value)})"
        return compact(family)
    if family is None:
        return "none"
    return str(family)


def compact(value):
    return json.dumps(value, separators=(",", ":"), sort_keys=True)


def arm_ran(record, names):
    if record is None:
        return False
    arm = record.get("approximate", {})
    return any(arm.get(name) for name in names)


def readout_key(readout):
    return (readout.get("node"), readout.get("group"), readout.get("query"))


def compare_readouts(interp, datafusion):
    if interp is None or datafusion is None:
        return None
    left = {readout_key(r): r for r in interp.get("readouts", [])}
    right = {readout_key(r): r for r in datafusion.get("readouts", [])}
    shared = sorted(set(left) & set(right), key=lambda key: tuple(str(part) for part in key))
    if not shared:
        return {"shared_readouts": 0, "agree": None, "rows": []}
    rows = []
    agree = True
    for key in shared:
        approximate_left = left[key]["approximate"]
        approximate_right = right[key]["approximate"]
        exact_left = left[key].get("exact")
        exact_right = right[key].get("exact")
        same = approximate_left == approximate_right
        agree = agree and same
        rows.append(
            {
                "node": key[0],
                "group": key[1],
                "query": key[2],
                "interp_approximate": approximate_left,
                "datafusion_approximate": approximate_right,
                "interp_exact": exact_left,
                "datafusion_exact": exact_right,
                "identical": same,
            }
        )
    return {"shared_readouts": len(shared), "agree": agree, "rows": rows}


def evaluate(binary, spec, statement, epsilon, timeout):
    common = ["--sql", statement, "--spec", spec, "--epsilon", str(epsilon)]
    code, stdout, stderr = invoke(binary, common + ["--emit-json"], timeout)
    entry = {}
    if code == 0:
        document = json.loads(stdout)
        payloads, families = plan_shape(document)
        entry["lowers"] = True
        entry["plans"] = True
        entry["payloads"] = payloads
        entry["families"] = families
        entry["plan_nodes"] = len(document.get("dag", {}).get("nodes", []))
    else:
        entry["lowers"] = "planning failed: lower " not in stderr
        entry["plans"] = False
        entry["plan_error"] = diagnostic_in(stderr)
        entry["payloads"] = []
        entry["families"] = []
        return entry, None, None

    runtimes = {}
    for runtime in ("interp", "datafusion"):
        code, stdout, stderr = invoke(
            binary,
            common + ["--runtime", runtime, "--jsonl", "--seeds", "1"],
            timeout,
        )
        record = record_in(stdout)
        runtimes[runtime] = {
            "exit_code": code,
            "ran": code == 0 and record is not None,
            "refusals": refusals_in(stdout),
            "refusal_counts": counts_in(stdout),
            "error": diagnostic_in(stderr) if code != 0 else None,
            "record": record,
        }
    entry["runtimes"] = {
        name: {key: value for key, value in body.items() if key != "record"}
        for name, body in runtimes.items()
    }
    interp = runtimes["interp"]["record"]
    datafusion = runtimes["datafusion"]["record"]
    entry["arm_a_runs"] = {
        "interp": arm_ran(interp, ("evaluate",)) or bool(interp and interp.get("pre_asap", {}).get("evaluate")),
        "datafusion": bool(datafusion and datafusion.get("pre_asap", {}).get("evaluate")),
    }
    entry["arm_b_runs"] = {
        "interp": bool(interp and (interp.get("approximate", {}).get("update") or interp.get("approximate", {}).get("readout"))),
        "datafusion": bool(
            datafusion
            and (
                datafusion.get("approximate", {}).get("maintenance")
                or datafusion.get("approximate", {}).get("read")
            )
        ),
    }
    entry["cross_runtime"] = compare_readouts(interp, datafusion)
    return entry, interp, datafusion


def reason_classes_of(entry):
    classes = Counter()
    for runtime, body in entry.get("runtimes", {}).items():
        for refusal in body["refusals"]:
            classes[(runtime, refusal["reason_class"])] += 1
    return classes


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--binary", default=str(REPO / "target" / "release" / "planeval"))
    parser.add_argument("--upstream", required=True)
    parser.add_argument("--epsilon", type=float, default=0.01)
    parser.add_argument("--timeout", type=float, default=300.0)
    parser.add_argument("--out", default=str(REPO / "target" / "sql-corpus.jsonl"))
    parser.add_argument("--only", default=None)
    arguments = parser.parse_args()

    upstream = Path(arguments.upstream)
    out = Path(arguments.out)
    out.parent.mkdir(parents=True, exist_ok=True)

    by_class = Counter()
    by_outcome = Counter()
    written = 0
    with out.open("w") as sink:
        for source in CORPUS:
            if arguments.only and arguments.only not in source["corpus"]:
                continue
            text = (upstream / source["sql"]).read_text()
            statements = source["split"](text)
            stem = Path(source["sql"]).stem
            for identifier, statement in statements:
                name = f"{source['corpus']}/{stem}/{identifier}"
                entry, _, _ = evaluate(
                    arguments.binary,
                    source["spec"],
                    statement,
                    arguments.epsilon,
                    arguments.timeout,
                )
                entry["query_id"] = name
                entry["corpus"] = source["corpus"]
                entry["spec"] = source["spec"]
                entry["sql"] = statement
                sink.write(json.dumps(entry) + "\n")
                sink.flush()
                written += 1
                by_class.update(reason_classes_of(entry))
                if not entry["plans"]:
                    by_outcome["plan_refused"] += 1
                elif entry["runtimes"]["datafusion"]["ran"] and entry["runtimes"]["interp"]["ran"]:
                    by_outcome["both_runtimes_ran"] += 1
                elif entry["runtimes"]["datafusion"]["ran"]:
                    by_outcome["datafusion_only"] += 1
                elif entry["runtimes"]["interp"]["ran"]:
                    by_outcome["interp_only"] += 1
                else:
                    by_outcome["neither_runtime_ran"] += 1
                print(f"{name}\t{'plans' if entry['plans'] else 'no-plan'}", file=sys.stderr)

    summary = {
        "queries": written,
        "outcomes": dict(by_outcome),
        "reason_classes": {f"{runtime}.{name}": count for (runtime, name), count in sorted(by_class.items())},
    }
    print(json.dumps(summary, indent=2))


if __name__ == "__main__":
    main()
