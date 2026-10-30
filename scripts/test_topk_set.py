#!/usr/bin/env python3

import unittest

from approx_rows import row
from topk_set import APPROX_ONLY, BOTH, EXACT_ONLY, derive


def query_rows(exact, approx, run_id="r1"):
    rows = []
    for arm, sets in (("exact", exact), ("approx", approx)):
        for i, members in enumerate(sets):
            for member, count in members.items():
                labels = {"use_case": "uc", "query": "top", "client_id": member, "arm": arm}
                rows.append(row(i * 1000, "query_result", run_id, labels, "value", count, "requests"))
    return rows


class DeriveTest(unittest.TestCase):
    def test_late_entry_and_an_intruder(self):
        exact = [{"a": 10, "b": 5}, {"a": 10, "c": 8}, {"a": 10, "c": 9}]
        approx = [{"a": 11, "b": 5}, {"a": 10, "b": 7}, {"a": 9, "c": 9}]
        rows = derive(query_rows(exact, approx), "s", "top", "client_id")
        summary = {r["metric"]: r["value"] for r in rows if r["kind"] == "fidelity_summary"}
        self.assertEqual(summary["identical_sets"], 2)
        self.assertEqual(summary["intruder_slots"], 1)
        self.assertEqual(summary["precision"], 5 / 6)
        self.assertEqual(summary["entries"], 3)
        self.assertEqual(summary["mean_onset_delay"], 1 / 3)
        events = sorted((r["time"] // 1000, r["labels"]["client_id"], r["metric"]) for r in rows
                        if r["kind"] == "fidelity_event")
        self.assertEqual(events, [(1, "b", "false_positive"), (1, "c", "late_onset")])
        membership = {(r["time"] // 1000, r["labels"]["client_id"]): r["value"] for r in rows
                      if r["metric"] == "membership"}
        self.assertEqual(membership[(1, "b")], APPROX_ONLY)
        self.assertEqual(membership[(1, "c")], EXACT_ONLY)
        self.assertEqual(membership[(2, "c")], BOTH)
        errors = [r["value"] for r in rows if r["metric"] == "relative_error"]
        self.assertEqual(len(errors), 5)

    def test_arms_must_share_evaluation_times(self):
        with self.assertRaises(ValueError):
            derive(query_rows([{"a": 1}, {"a": 1}], [{"a": 1}]), "s", "top", "client_id")


if __name__ == "__main__":
    unittest.main()
