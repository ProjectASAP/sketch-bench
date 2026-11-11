#!/usr/bin/env python3

import bounds
import decision
import rules
from bounds import any_of, gt, lt
from rules import above, at_least, below, both, for_consecutive

ID = "2"

CPU_REQUEST, MEMORY_REQUEST = 0.6, 1.6e9
EC2_OUT, EC2_IN, EC2_PERIODS = 0.6, 0.4, 3
VPA_MARGIN = 1.15
SMALLER_CPU, SMALLER_MEMORY = 0.5, 1e9
ADVISOR_CPU, ADVISOR_MEMORY = 0.4, 0.6


def container(labels):
    return labels["container"]


def ec2_scale_out(v):
    return at_least(v["p95_cpu"], EC2_OUT)


def ec2_scale_in(v):
    return below(v["p95_cpu"], EC2_IN)


def vpa(v):
    return [any_of(gt(p50, CPU_REQUEST / VPA_MARGIN), lt(p95, CPU_REQUEST / VPA_MARGIN),
                   gt(peak, MEMORY_REQUEST / VPA_MARGIN))
            for p50, p95, peak in zip(v["p50_cpu_1h"], v["p95_cpu_1h"], v["max_memory_1h"])]


def grafana_labs(v):
    return above(v["p95_cpu_1h"], CPU_REQUEST)


def azure_advisor(v):
    return both(below(v["p95_cpu_1h"], ADVISOR_CPU * SMALLER_CPU),
                below(v["p99_memory_1h"], ADVISOR_MEMORY * SMALLER_MEMORY))


RULES = {
    "ec2_scale_out": {
        "queries": ("p95_cpu",), "condition": ec2_scale_out, "entity": container,
        "hold": for_consecutive(EC2_PERIODS),
        "description": "p95 CPU over 5 minutes at or above 60% of one core for 3 periods in a row: the step "
                       "alarm that adds capacity (AWS EC2 Auto Scaling; 3 periods ours)"},
    "ec2_scale_in": {
        "queries": ("p95_cpu",), "condition": ec2_scale_in, "entity": container,
        "hold": for_consecutive(EC2_PERIODS),
        "description": "p95 CPU over 5 minutes below 40% of one core for 3 periods in a row: the step alarm "
                       "that removes capacity (AWS EC2 Auto Scaling; 3 periods ours)"},
    "vpa": {
        "queries": ("p50_cpu_1h", "p95_cpu_1h", "max_memory_1h"), "condition": vpa, "entity": container,
        "description": "The CPU request of 0.6 cores falls outside [p50, p95] × 1.15 of the last hour, or the "
                       "memory peak × 1.15 exceeds the 1.6 GB request: the updater resizes the pod "
                       "(Kubernetes Vertical Pod Autoscaler; 1 hour instead of 8 days; requests ours)"},
    "grafana_labs": {
        "queries": ("p95_cpu_1h",), "condition": grafana_labs, "entity": container,
        "description": "p95 CPU of the last hour above the 0.6-core request: under-provisioned "
                       "(Grafana Labs; 1 hour instead of 7 days; request ours)"},
    "azure_advisor": {
        "queries": ("p95_cpu_1h", "p99_memory_1h"), "condition": azure_advisor, "entity": container,
        "description": "On half the size (0.5 cores, 1 GB), p95 CPU of the last hour stays at most 40% and p99 "
                       "memory at most 60%: recommend the smaller size (Azure Advisor, user-facing; 1 hour "
                       "instead of 7 days; sizes ours)"},
}


DECISION = {"signal": "scale_decision", "cpu": "p95_cpu", "memory": "p95_memory", "entity": "container",
            "rule": {"cpu_up": 0.8, "memory_up": 1.5e9, "cpu_down": 0.4, "memory_down": 0.75e9}}


def derive(rows, queries, policy=bounds.PLAIN, widen=None):
    out = decision.derive(rows, DECISION["signal"], DECISION["cpu"], DECISION["memory"], DECISION["entity"],
                          DECISION["rule"], policy, widen)
    return out + rules.derive(rows, RULES, policy, widen)


DASHBOARD = {"uid": "autoscaling", "title": "2 · Autoscaling",
             "parts": ("overview", "cost", "stage1", "decision", "rules", "rule_error"),
             "rule_error": {"metrics": ("relative_error", "rank_error"), "file_var": "entity_in_run"}}

CASE = {
    "spec": "uc2_autoscaling.json",
    "arms": ["raw-eps0.01", "noraw-eps0.1", "noraw-eps0.05", "noraw-eps0.01", "noraw-eps0.005"],
    "planeval_flags": ["--executor-max-bytes", str(2 << 30)],
    "signal": lambda query: "scale_decision",
    "signals": lambda query: (["scale_decision"] if query in ("p95_cpu", "p95_memory") else [])
    + rules.rule_signals(RULES)(query),
    "rules": RULES,
    "scenario_entity": container,
    "derive": derive,
    "dashboard": "decision",
    "needs_every_query": True,
    "split": {"query_result": "container", "low_level": "container", "app_state": ("container", "entity")},
}
