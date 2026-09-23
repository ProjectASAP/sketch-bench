#!/usr/bin/env python3

import argparse
import json
import re
import subprocess
import sys
from collections import Counter
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
REFUSAL_CARRYING_A_REASON = re.compile(
    r"^(?P<variant>.+?) refused \[(?P<reason>[^\]]+)\]: (?P<detail>.*)$"
)
REASON_CLASSES = ("promql_only", "time_axis", "no_constructor", "deferred", "unclassified")
LOWERING_FAILED = "planning failed: lower "
DIAGNOSTIC_PREFIX = "planeval:"

PLAN_EVAL_RECORD_KEYS = frozenset(
    ("schema_version", "runtime", "refusals", "plan", "nodes", "approximate", "readouts")
)
REFUSED_PLAN_RECORD_KEYS = frozenset(
    ("schema_version", "runtime", "plan_id", "query", "seed", "refusals", "refused")
)
READOUT_GUARANTEE_KEYS = frozenset(("schema_version", "node", "group", "query", "check"))
ARM_PHASES = ("build", "update", "readout", "evaluate", "maintenance", "read")


class OutputNotUnderstood(Exception):
    pass


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


def documents_on_stdout(stdout, where):
    documents = []
    for number, line in enumerate(stdout.splitlines(), start=1):
        line = line.strip()
        if not line:
            continue
        try:
            parsed = json.loads(line)
        except json.JSONDecodeError as error:
            raise OutputNotUnderstood(
                f"{where}: stdout line {number} is not JSON ({error}): {line[:200]}"
            ) from error
        if not isinstance(parsed, dict):
            raise OutputNotUnderstood(
                f"{where}: stdout line {number} is JSON but not an object: {line[:200]}"
            )
        documents.append(parsed)
    return documents


def kind_of(document, where):
    keys = frozenset(document)
    if PLAN_EVAL_RECORD_KEYS <= keys:
        return "plan_eval_record"
    if REFUSED_PLAN_RECORD_KEYS <= keys:
        return "refused_plan_record"
    if READOUT_GUARANTEE_KEYS <= keys:
        return "readout_guarantee"
    raise OutputNotUnderstood(
        f"{where}: stdout carries a JSON object matching no known record shape; "
        f"its keys are {sorted(keys)}"
    )


def refusals_listed_by(record):
    listed = []
    for line in record["refused"]:
        matched = REFUSAL_CARRYING_A_REASON.match(line)
        if matched:
            reason = matched.group("reason")
            listed.append(
                {
                    "variant": matched.group("variant"),
                    "reason": reason,
                    "reason_class": reason.split("(")[0],
                    "detail": matched.group("detail"),
                }
            )
        else:
            listed.append(
                {
                    "variant": line.split(":")[0],
                    "reason": "unclassified",
                    "reason_class": "unclassified",
                    "detail": line,
                }
            )
    return listed


def counts_declared_by(record, where):
    declared = record["refusals"]
    if not isinstance(declared, dict):
        raise OutputNotUnderstood(f"{where}: the record's refusal counts are not an object")
    unknown = sorted(set(declared) - set(REASON_CLASSES))
    if unknown:
        raise OutputNotUnderstood(
            f"{where}: the record's refusal counts carry classes this sweep does not know: "
            f"{unknown}"
        )
    return {name: int(declared.get(name, 0)) for name in REASON_CLASSES}


def nonzero(counts):
    return {name: count for name, count in counts.items() if count}


def arm_ran(record, arm, where):
    body = record.get(arm)
    if not isinstance(body, dict):
        raise OutputNotUnderstood(f"{where}: the record carries no {arm} arm")
    return any(body.get(phase) is not None for phase in ARM_PHASES)


def diagnostic_in(text):
    lines = text.splitlines()
    complaints = [
        position
        for position, line in enumerate(lines)
        if line.strip().startswith(DIAGNOSTIC_PREFIX)
    ]
    if complaints:
        return " ".join(line.strip() for line in lines[complaints[-1] :] if line.strip())
    remaining = [line.strip() for line in lines if line.strip()]
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


def runtime_result(code, stdout, stderr, where):
    if code is None:
        return {
            "status": "timed_out",
            "exit_code": None,
            "refusals": [],
            "refusal_counts": {name: 0 for name in REASON_CLASSES},
            "error": stderr.strip(),
        }, None

    documents = documents_on_stdout(stdout, where)
    runs = [d for d in documents if kind_of(d, where) == "plan_eval_record"]
    refused = [d for d in documents if kind_of(d, where) == "refused_plan_record"]

    listed = []
    counts = {name: 0 for name in REASON_CLASSES}
    for record in refused:
        listed.extend(refusals_listed_by(record))
        for name, count in counts_declared_by(record, where).items():
            counts[name] += count
    tallied = Counter(refusal["reason_class"] for refusal in listed)
    if nonzero(counts) != nonzero(tallied):
        raise OutputNotUnderstood(
            f"{where}: the refusal strings tally {dict(nonzero(tallied))} but the record's "
            f"counts say {nonzero(counts)}"
        )

    if code != 0:
        status = "failed"
    elif runs and refused:
        status = "some_seeds_refused"
    elif refused:
        status = "refused"
    elif runs:
        status = "ran"
    else:
        raise OutputNotUnderstood(
            f"{where}: the process exited 0 and stdout carries neither a run record nor a "
            f"refusal record"
        )

    body = {
        "status": status,
        "exit_code": code,
        "refusals": listed,
        "refusal_counts": counts,
        "error": diagnostic_in(stderr) if code != 0 else None,
    }
    return body, (runs[-1] if runs else None)


def evaluate(binary, spec, statement, epsilon, timeout, where):
    common = ["--sql", statement, "--spec", spec, "--epsilon", str(epsilon)]
    code, stdout, stderr = invoke(binary, common + ["--emit-json"], timeout)
    entry = {}
    if code == 0:
        try:
            document = json.loads(stdout)
        except json.JSONDecodeError as error:
            raise OutputNotUnderstood(
                f"{where}: --emit-json exited 0 but stdout is not one JSON document ({error})"
            ) from error
        payloads, families = plan_shape(document)
        entry["lowers"] = True
        entry["plans"] = True
        entry["payloads"] = payloads
        entry["families"] = families
        entry["plan_nodes"] = len(document.get("dag", {}).get("nodes", []))
    else:
        diagnostic = diagnostic_in(stderr)
        if code is None:
            raise OutputNotUnderstood(f"{where}: --emit-json {stderr.strip()}")
        if not diagnostic.startswith(DIAGNOSTIC_PREFIX):
            raise OutputNotUnderstood(
                f"{where}: --emit-json exited {code} without a planeval diagnostic: "
                f"{diagnostic[:200]}"
            )
        entry["lowers"] = LOWERING_FAILED not in diagnostic
        entry["plans"] = False
        entry["plan_error"] = diagnostic
        entry["payloads"] = []
        entry["families"] = []
        return entry, None, None

    runtimes = {}
    records = {}
    for runtime in ("interp", "datafusion"):
        code, stdout, stderr = invoke(
            binary,
            common + ["--runtime", runtime, "--jsonl", "--seeds", "1"],
            timeout,
        )
        runtimes[runtime], records[runtime] = runtime_result(
            code, stdout, stderr, f"{where} [{runtime}]"
        )
    entry["runtimes"] = runtimes

    interp = records["interp"]
    datafusion = records["datafusion"]
    entry["arm_a_runs"] = {
        name: None if record is None else arm_ran(record, "pre_asap", f"{where} [{name}]")
        for name, record in records.items()
    }
    entry["arm_b_runs"] = {
        name: None if record is None else arm_ran(record, "approximate", f"{where} [{name}]")
        for name, record in records.items()
    }
    entry["cross_runtime"] = compare_readouts(interp, datafusion)
    return entry, interp, datafusion


def reason_classes_of(entry):
    classes = Counter()
    for runtime, body in entry.get("runtimes", {}).items():
        for name, count in body["refusal_counts"].items():
            if count:
                classes[(runtime, name)] += count
    return classes


def outcome_of(entry):
    if entry.get("status") == "output_not_understood":
        return "output_not_understood"
    if not entry["plans"]:
        return "plan_refused"
    ran = {name: body["status"] == "ran" for name, body in entry["runtimes"].items()}
    if ran["interp"] and ran["datafusion"]:
        return "both_runtimes_ran"
    if ran["datafusion"]:
        return "datafusion_only"
    if ran["interp"]:
        return "interp_only"
    return "neither_runtime_ran"


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
    unreadable = []
    with out.open("w") as sink:
        for source in CORPUS:
            if arguments.only and arguments.only not in source["corpus"]:
                continue
            text = (upstream / source["sql"]).read_text()
            statements = source["split"](text)
            stem = Path(source["sql"]).stem
            for identifier, statement in statements:
                name = f"{source['corpus']}/{stem}/{identifier}"
                try:
                    entry, _, _ = evaluate(
                        arguments.binary,
                        source["spec"],
                        statement,
                        arguments.epsilon,
                        arguments.timeout,
                        name,
                    )
                except OutputNotUnderstood as error:
                    entry = {
                        "status": "output_not_understood",
                        "complaint": str(error),
                        "lowers": None,
                        "plans": None,
                    }
                    unreadable.append(str(error))
                    print(f"UNREADABLE {error}", file=sys.stderr)
                entry["query_id"] = name
                entry["corpus"] = source["corpus"]
                entry["spec"] = source["spec"]
                entry["sql"] = statement
                sink.write(json.dumps(entry) + "\n")
                sink.flush()
                written += 1
                by_class.update(reason_classes_of(entry))
                by_outcome[outcome_of(entry)] += 1
                print(f"{name}\t{outcome_of(entry)}", file=sys.stderr)

    summary = {
        "queries": written,
        "outcomes": dict(by_outcome),
        "reason_classes": {
            f"{runtime}.{name}": count for (runtime, name), count in sorted(by_class.items())
        },
        "unreadable": unreadable,
    }
    print(json.dumps(summary, indent=2))
    return 1 if unreadable else 0


if __name__ == "__main__":
    sys.exit(main())
