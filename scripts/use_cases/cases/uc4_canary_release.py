#!/usr/bin/env python3

import bounds
import rollout

ID = "4"
RULES = rollout.RULES
QUERY_COLORS = {"p99_canary": "orange", "p99_stable": "blue", "success_rate_canary": "green", "p95_canary": "purple"}


def derive(rows, queries, policy=bounds.PLAIN, widen=None):
    return rollout.derive(rows, RULES, policy, widen)


def stage2(derived, errors):
    ranked = {r["labels"]["query"] for r in errors}
    relative = [r for r in derived if r["kind"] == "low_level" and r["metric"] == "relative_error"
                and r["labels"]["query"] not in ranked]
    return rollout.rollout_errors(derived, errors + relative, RULES)


DASHBOARD = {"uid": "canary-release", "title": "4 · Canary release", "parts": ("overview", "rollout", "two_stages")}

CASE = {
    "spec": "uc4_canary.json",
    "arms": ["raw-eps0.01", "noraw-eps0.05", "noraw-eps0.01", "noraw-eps0.005", "noraw-eps0.001"],
    "signal": lambda query: "rollout",
    "signals": lambda query: [name for name, rule in RULES.items() if query in rule["queries"]],
    "derive": derive,
    "dashboard": "rollout",
    "needs_every_query": True,
    "split": {},
    "stage2": stage2,
}
