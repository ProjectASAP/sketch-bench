#!/usr/bin/env python3

import bounds
import rules
import topk_set
from rules import above, latch

ID = "5"

AWS_WAF_LIMIT = 2000
CLOUD_ARMOR_LIMIT, CLOUD_ARMOR_BAN_MINUTES = 500, 10
FASTLY_RATE, FASTLY_PENALTY_MINUTES = 20, 2


def client(labels):
    return labels["client_id"]


def aws_waf(v):
    return above(v["top10_clients"], AWS_WAF_LIMIT)


def cloud_armor(v):
    return above(v["top10_clients_1m"], CLOUD_ARMOR_LIMIT)


def fastly(v):
    return above(v["top10_clients_1m"], FASTLY_RATE * 60)


RULES = {
    "aws_waf": {
        "queries": ("top10_clients",), "condition": aws_waf, "entity": client,
        "description": "A client among the top 10 over 5 minutes with more than 2,000 requests is limited "
                       "while it stays over (AWS WAF rate-based rule, the AWS blog's limit)"},
    "cloud_armor": {
        "queries": ("top10_clients_1m",), "condition": cloud_armor, "entity": client,
        "hold": latch(CLOUD_ARMOR_BAN_MINUTES),
        "description": "A client among the top 10 over 1 minute with more than 500 requests is banned for the "
                       "rest of the interval plus 600 s (Google Cloud Armor default policy's 500 per 60 s; "
                       "ban_duration_sec ours)"},
    "fastly": {
        "queries": ("top10_clients_1m",), "condition": fastly, "entity": client,
        "hold": latch(FASTLY_PENALTY_MINUTES),
        "description": "A client among the top 10 over 1 minute above 20 requests per second goes into the "
                       "penalty box for at least 2 minutes (Fastly; the rate ours, over 1 minute instead of "
                       "10 s)"},
}


def derive(rows, queries, policy=bounds.PLAIN, widen=None):
    if policy != bounds.PLAIN:
        raise ValueError("the top clients rule has no use for an error bound")
    out = topk_set.derive(rows, "top_clients", "top10_clients", "client_id")
    return out + rules.derive(rows, RULES)


def supported_rules(rows, queries):
    out = topk_set.derive(rows, "top_clients", "top10_clients", "client_id") \
        if "top10_clients" in {q["id"] for q in queries} else []
    usable = rules.planned_rules(RULES, queries)
    return out + (rules.derive(rows, usable) if usable else [])


DASHBOARD = {"uid": "rate-limiting", "title": "5 · Top clients and rate limiting",
             "parts": ("overview", "cost", "stage1", "topk", "rules")}

CASE = {
    "spec": "uc5_top_clients.json",
    "arms": ["noraw-ignore-eps0.05", "noraw-ignore-eps0.01", "noraw-ignore-eps0.005", "noraw-ignore-eps0.001"],
    "planeval_flags": ["--executor-max-bytes", str(4 << 30)],
    "signal": lambda query: "top_clients",
    "signals": lambda query: (["top_clients"] if query == "top10_clients" else [])
    + rules.rule_signals(RULES)(query),
    "rules": RULES,
    "derive": derive,
    "supported_rules": supported_rules,
    "dashboard": "topk",
    "bounds": False,
    "needs_every_query": True,
    "split": {"app_state": ("client_id", "entity")},
}
