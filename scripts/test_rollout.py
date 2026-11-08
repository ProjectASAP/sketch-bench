#!/usr/bin/env python3

import unittest

from approx_rows import row
from rollout import (PROMOTE, ROLLBACK, RULES, argo_rollouts_failed, cloudwatch_breaching, decisions_of, derive,
                     failures_or_passes, failures_over_limit, flagger_failed, m_of_last_n, rollouts)


def flags(bits):
    return [b == "1" for b in bits]


def query_rows(canary, stable, run_id="r1", success_rate=None):
    rows = []
    queries = (("p99_canary", canary), ("p99_stable", stable))
    if success_rate:
        queries += (("success_rate_canary", success_rate),)
    for query, arms in queries:
        for arm, values in zip(("exact", "approx"), arms):
            for i, v in enumerate(values):
                labels = {"use_case": "uc", "query": query, "arm": arm}
                rows.append(row(i * 1000, "query_result", run_id, labels, "value", v, "s"))
    return rows


class DecisionsTest(unittest.TestCase):
    def test_rollback_on_the_third_slower_check_else_promote_on_the_tenth(self):
        self.assertEqual(decisions_of(flags("0110111000" "1101101101" "11")),
                         [(6, ROLLBACK), (19, PROMOTE)])

    def test_a_count_does_not_carry_into_the_next_rollout(self):
        self.assertEqual(decisions_of(flags("0000000011" "1000000000")), [(9, PROMOTE), (19, PROMOTE)])


class ChecksTest(unittest.TestCase):
    def test_flagger_fails_on_either_p99_or_success_rate(self):
        self.assertEqual(flagger_failed([0.4, 0.6, 0.4, 0.5], [0.995, 0.995, 0.98, 0.99]),
                         [False, True, True, False])

    def test_argo_rollouts_and_cloudwatch_thresholds(self):
        self.assertEqual(argo_rollouts_failed([0.96, 0.95, 0.94]), [False, False, True])
        self.assertEqual(cloudwatch_breaching([0.5, 0.51]), [False, True])


class AggregationTest(unittest.TestCase):
    def test_flagger_counts_failures_in_total_and_promotes_after_25_passes(self):
        self.assertEqual(failures_or_passes(flags("10" * 9 + "0" * 16)), (33, PROMOTE))
        self.assertEqual(failures_or_passes(flags("10" * 10)), (18, ROLLBACK))

    def test_argo_rollouts_rolls_back_when_failures_exceed_the_limit(self):
        self.assertEqual(failures_over_limit(flags("1010100000")), (9, PROMOTE))
        self.assertEqual(failures_over_limit(flags("1010101000")), (6, ROLLBACK))

    def test_cloudwatch_needs_4_of_the_last_5_not_in_a_row(self):
        self.assertEqual(m_of_last_n(flags("1101100000")), (4, ROLLBACK))
        self.assertEqual(m_of_last_n(flags("1100110011")), (9, PROMOTE))

    def test_flagger_rollouts_take_34_checks(self):
        self.assertEqual(decisions_of(flags("1" * 10 + "0" * 24 + "0" * 25), 34, failures_or_passes),
                         [(9, ROLLBACK)])


class RolloutsTest(unittest.TestCase):
    def test_use_case_document_example(self):
        found = rollouts(flags("0011111100"), flags("0011011100"))
        (rollout,) = found["rollouts"]
        self.assertEqual((rollout["exact"], rollout["approx"]), ((4, ROLLBACK), (7, ROLLBACK)))
        self.assertEqual((rollout["outcome"], rollout["delay"]), ("true_positive", 3))
        self.assertEqual(found["gaps"], [(4, 1)])
        marks = {i + 1: m for i, m in enumerate(found["annotations"]) if m != "none"}
        self.assertEqual(marks, {5: "gap", 8: "decision delay"})

    def test_false_rollback_and_missed_rollback(self):
        found = rollouts(flags("0000000000" "0111000000"), flags("1110000000" "0101000000"))
        self.assertEqual([r["outcome"] for r in found["rollouts"]], ["false_positive", "false_negative"])
        self.assertEqual([r["delay"] for r in found["rollouts"]], [None, None])


class DeriveTest(unittest.TestCase):
    def test_slower_needs_more_than_the_factor(self):
        canary = ([1.2] * 3 + [1.0] * 7, [1.2, 1.1, 1.2] + [1.0] * 7)
        stable = ([1.0] * 10, [1.0] * 10)
        rows = derive(query_rows(canary, stable), {"ours": RULES["ours"]})
        summary = {r["metric"]: r["value"] for r in rows if r["kind"] == "fidelity_summary"}
        self.assertEqual((summary["true_positive"], summary["false_negative"], summary["rollouts"]), (0, 1, 1))
        self.assertEqual(summary["agreement"], 0)
        acted = {(r["time"] // 1000, r["labels"]["arm"]): r["value"] for r in rows
                 if r["metric"] == "action" and r["value"] != "none"}
        self.assertEqual(acted, {(2, "exact"): ROLLBACK, (9, "approx"): PROMOTE})
        self.assertNotIn("mean_absolute_rollback_delay", summary)

    def test_rollback_delay_averages_rollbacks_only(self):
        canary = ([1.2 if b == "1" else 1.0 for b in "0011100000" "0000000000"],
                  [1.2 if b == "1" else 1.0 for b in "0111000000" "0000000000"])
        stable = ([1.0] * 20, [1.0] * 20)
        rows = derive(query_rows(canary, stable), {"ours": RULES["ours"]})
        summary = {r["metric"]: r["value"] for r in rows if r["kind"] == "fidelity_summary"}
        self.assertEqual((summary["true_positive"], summary["true_negative"]), (1, 1))
        self.assertEqual((summary["mean_absolute_rollback_delay"], summary["early_rollbacks"],
                          summary["late_rollbacks"]), (1, 1, 0))
        events = [(r["time"] // 1000, r["metric"], r["value"]) for r in rows
                  if r["kind"] == "fidelity_event" and r["metric"] == "decision_delay"]
        self.assertEqual(events, [(3, "decision_delay", -1)])

    def test_every_rule_carries_its_label_and_argo_rollouts_checks_every_fifth_evaluation(self):
        canary = ([0.3] * 50, [0.3] * 50)
        stable = ([0.3] * 50, [0.3] * 50)
        success_rate = ([0.9] * 20 + [0.999] * 30, [0.999] * 10 + [0.9] * 20 + [0.999] * 20)
        rows = derive(query_rows(canary, stable, success_rate=success_rate))
        checks = {}
        for r in rows:
            if r["metric"] == "check" and r["labels"]["arm"] == "exact":
                checks.setdefault(r["labels"]["signal"], []).append(r["time"] // 1000)
        self.assertEqual(sorted(checks), sorted(RULES))
        self.assertEqual(checks["argo_rollouts"], list(range(0, 50, 5)))
        outcomes = {r["labels"]["signal"]: r["value"] for r in rows if r["metric"] == "rollout"}
        self.assertEqual(outcomes, {"ours": "true_negative", "flagger": "true_positive",
                                    "argo_rollouts": "true_positive", "cloudwatch": "true_negative"})
        delays = {r["labels"]["signal"]: r["value"] for r in rows if r["metric"] == "decision_delay"}
        self.assertEqual(delays, {"flagger": 10, "argo_rollouts": 10})
        self.assertEqual(sum(r["metric"] == "ratio" for r in rows), 100)


if __name__ == "__main__":
    unittest.main()
