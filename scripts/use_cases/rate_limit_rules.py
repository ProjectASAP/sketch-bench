#!/usr/bin/env python3

from rules import above, latch

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
