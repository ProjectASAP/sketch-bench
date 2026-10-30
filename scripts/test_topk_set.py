#!/usr/bin/env python3

import unittest

from approx_rows import row
from topk_set import CAPPED, NOT_CAPPED, derive


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
        summary = {r["metric"]: r["value"] for r in rows
                   if r["kind"] == "fidelity_summary" and "client_id" not in r["labels"]}
        self.assertEqual(summary["identical_sets"], 2)
        self.assertEqual(summary["intruder_slots"], 1)
        self.assertEqual(summary["precision"], 5 / 6)
        self.assertEqual(summary["entries"], 3)
        self.assertEqual(summary["mean_onset_delay"], 1 / 3)
        events = sorted((r["time"] // 1000, r["labels"]["client_id"], r["metric"]) for r in rows
                        if r["kind"] == "fidelity_event")
        self.assertEqual(events, [(1, "b", "false_positive"), (2, "c", "onset_delay")])
        capped = {(r["time"] // 1000, r["labels"]["client_id"], r["labels"]["arm"]): r["value"] for r in rows
                  if r["metric"] == "capped"}
        self.assertEqual((capped[(1, "b", "exact")], capped[(1, "b", "approx")]), (NOT_CAPPED, CAPPED))
        self.assertEqual((capped[(1, "c", "exact")], capped[(1, "c", "approx")]), (CAPPED, NOT_CAPPED))
        self.assertEqual((capped[(2, "c", "exact")], capped[(2, "c", "approx")]), (CAPPED, CAPPED))
        errors = [r["value"] for r in rows if r["metric"] == "relative_error"]
        self.assertEqual(len(errors), 5)

    def test_use_case_document_example(self):
        exact = [{"z": 1}] * 2 + [{"x": 5, "z": 1}] * 6
        approx = [{"z": 1}] * 3 + [{"x": 5, "z": 1}, {"x": 5, "y": 4, "z": 1}, {"z": 1},
                                   {"x": 5, "z": 1}, {"x": 5, "z": 1}]
        rows = derive(query_rows(exact, approx), "s", "top", "client_id")
        summary = {r["metric"]: r["value"] for r in rows
                   if r["kind"] == "fidelity_summary" and "client_id" not in r["labels"]}
        self.assertEqual((summary["true_positive"], summary["false_positive"],
                          summary["false_negative"], summary["true_negative"]), (12, 1, 2, 9))
        marks = {(r["time"] // 1000 + 1, r["labels"]["client_id"]): r["value"] for r in rows
                 if r["metric"] == "annotation" and r["value"] != "none"}
        self.assertEqual(marks, {(4, "x"): "onset delay", (6, "x"): "gap", (5, "y"): "false positive"})
        disagreements = [(r["labels"]["client_id"], r["value"]) for r in rows if r["metric"] == "disagreements"]
        self.assertEqual(disagreements, [("x", 2), ("y", 1), ("z", 0)])

    def test_arms_must_share_evaluation_times(self):
        with self.assertRaises(ValueError):
            derive(query_rows([{"a": 1}, {"a": 1}], [{"a": 1}]), "s", "top", "client_id")


if __name__ == "__main__":
    unittest.main()
