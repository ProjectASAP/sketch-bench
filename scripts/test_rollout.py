#!/usr/bin/env python3

import unittest

from approx_rows import row
from rollout import PROMOTE, ROLLBACK, decisions_of, derive, rollouts


def flags(bits):
    return [b == "1" for b in bits]


def query_rows(canary, stable, run_id="r1"):
    rows = []
    for query, arms in (("canary", canary), ("stable", stable)):
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
        rows = derive(query_rows(canary, stable), "s", "canary", "stable", 1.1)
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
        rows = derive(query_rows(canary, stable), "s", "canary", "stable", 1.1)
        summary = {r["metric"]: r["value"] for r in rows if r["kind"] == "fidelity_summary"}
        self.assertEqual((summary["true_positive"], summary["true_negative"]), (1, 1))
        self.assertEqual((summary["mean_absolute_rollback_delay"], summary["early_rollbacks"],
                          summary["late_rollbacks"]), (1, 1, 0))
        events = [(r["time"] // 1000, r["metric"], r["value"]) for r in rows
                  if r["kind"] == "fidelity_event" and r["metric"] == "decision_delay"]
        self.assertEqual(events, [(3, "decision_delay", -1)])


if __name__ == "__main__":
    unittest.main()
