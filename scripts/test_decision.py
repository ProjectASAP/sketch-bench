#!/usr/bin/env python3

import unittest

from approx_rows import row
from decision import actions_of, decide, derive, match_actions

RULE = {"cpu_up": 0.8, "memory_up": 1.5e9, "cpu_down": 0.4, "memory_down": 0.75e9}


def query_rows(cpu, memory, container="c0", run_id="r1"):
    rows = []
    for query, arms in (("cpu", cpu), ("memory", memory)):
        for arm, values in zip(("exact", "approx"), arms):
            for i, v in enumerate(values):
                labels = {"use_case": "uc", "query": query, "container": container, "arm": arm}
                rows.append(row(i * 1000, "query_result", run_id, labels, "value", v, "unit"))
    return rows


def summary(rows):
    return {r["metric"]: r["value"] for r in rows if r["kind"] == "fidelity_summary" and "container" not in r["labels"]}


class DecideTest(unittest.TestCase):
    def test_up_wins_over_down_and_down_needs_both(self):
        self.assertEqual(decide(0.9, 0.1e9, RULE), "up")
        self.assertEqual(decide(0.2, 1.6e9, RULE), "up")
        self.assertEqual(decide(0.2, 0.7e9, RULE), "down")
        self.assertEqual(decide(0.2, 0.8e9, RULE), "none")
        self.assertEqual(decide(0.5, 0.7e9, RULE), "none")


class ActionsTest(unittest.TestCase):
    def test_an_action_is_a_change_into_up_or_down(self):
        self.assertEqual(actions_of(["up", "up", "none", "up", "down", "down", "none"]),
                         ["none", "none", "none", "up", "down", "none", "none"])

    def test_use_case_document_example(self):
        exact = actions_of(["none", "none", "up", "up", "up", "up", "down", "down"])
        approx = actions_of(["none", "none", "none", "up", "none", "up", "down", "down"])
        self.assertEqual(exact, ["none", "none", "up", "none", "none", "none", "down", "none"])
        self.assertEqual(approx, ["none", "none", "none", "up", "none", "up", "down", "none"])
        self.assertEqual(match_actions(exact, approx), ({2: 3, 6: 6}, [], [5]))

    def test_early_by_one_is_matched_with_a_negative_delay(self):
        self.assertEqual(match_actions(["none", "none", "up", "none"], ["none", "up", "none", "none"]),
                         ({2: 1}, [], []))

    def test_late_by_three_is_missed_and_false(self):
        self.assertEqual(match_actions(["none", "up", "none", "none", "none"], ["none", "none", "none", "none", "up"]),
                         ({}, [1], [4]))

    def test_tie_in_distance_prefers_the_earlier_action(self):
        self.assertEqual(match_actions(["none", "none", "up", "none", "none"], ["none", "up", "none", "up", "none"]),
                         ({2: 1}, [], [3]))

    def test_one_approx_action_serves_one_exact_action(self):
        self.assertEqual(match_actions(["none", "up", "none", "up", "none"], ["none", "none", "up", "none", "none"]),
                         ({1: 2}, [3], []))

    def test_late_by_two_is_matched_with_a_positive_delay(self):
        self.assertEqual(match_actions(["none", "up", "none", "none", "none"], ["none", "none", "none", "up", "none"]),
                         ({1: 3}, [], []))

    def test_closest_pair_is_taken_first_whatever_the_exact_order(self):
        exact = ["none", "none", "none", "up", "none", "up", "none"]
        approx = ["none", "none", "none", "none", "none", "up", "none"]
        self.assertEqual(match_actions(exact, approx), ({5: 5}, [3], []))

    def test_tie_on_one_approx_action_goes_to_the_earlier_exact_action(self):
        exact = ["none", "none", "none", "up", "none", "up", "none"]
        approx = ["none", "up", "none", "none", "up", "none", "none"]
        self.assertEqual(match_actions(exact, approx), ({3: 4}, [5], [1]))

    def test_kinds_must_agree(self):
        self.assertEqual(match_actions(["none", "up", "none"], ["none", "down", "none"]), ({}, [1], [1]))


class DeriveTest(unittest.TestCase):
    def test_use_case_document_example(self):
        exact_cpu = [0.5, 0.5, 0.9, 0.9, 0.9, 0.9, 0.2, 0.2]
        approx_cpu = [0.5, 0.5, 0.5, 0.9, 0.5, 0.9, 0.2, 0.2]
        memory = [0.5e9] * 8
        rows = derive(query_rows((exact_cpu, approx_cpu), (memory, memory)), "s", "cpu", "memory", "container", RULE)
        s = summary(rows)
        self.assertEqual((s["true_positive"], s["false_positive"], s["false_negative"], s["true_negative"]),
                         (2, 1, 0, 4))
        self.assertEqual((s["up_true_positive"], s["up_false_positive"], s["down_true_positive"]), (1, 1, 1))
        self.assertEqual((s["mean_absolute_onset_delay"], s["early_actions"], s["late_actions"]), (0.5, 0, 1))
        self.assertEqual((s["up_mean_absolute_onset_delay"], s["down_mean_absolute_onset_delay"]), (1, 0))
        self.assertEqual(s["action_agreement"], 2 / 3)
        self.assertEqual(s["exact_up_approx_none"], 2)
        events = sorted((r["time"] // 1000, r["labels"]["action"], r["metric"], r["value"]) for r in rows
                        if r["kind"] == "fidelity_event")
        self.assertEqual(events, [(3, "up", "onset_delay", 1), (5, "up", "false_positive", 1)])
        marks = {r["time"] // 1000 + 1: r["value"] for r in rows
                 if r["metric"] == "annotation" and r["value"] != "none"}
        self.assertEqual(marks, {4: "onset delay", 6: "false positive"})
        acted = {(r["time"] // 1000, r["labels"]["arm"]): r["value"] for r in rows
                 if r["metric"] == "action" and r["value"] != "none"}
        self.assertEqual(acted, {(2, "exact"): "up", (6, "exact"): "down", (3, "approx"): "up",
                                 (5, "approx"): "up", (6, "approx"): "down"})
        disagreements = [r["value"] for r in rows if r["metric"] == "disagreements"]
        self.assertEqual(disagreements, [3])

    def test_early_scale_down_is_marked_on_the_approx_action(self):
        exact_cpu = [0.5, 0.5, 0.5, 0.2, 0.2]
        approx_cpu = [0.5, 0.2, 0.2, 0.2, 0.2]
        memory = [0.5e9] * 5
        rows = derive(query_rows((exact_cpu, approx_cpu), (memory, memory)), "s", "cpu", "memory", "container", RULE)
        s = summary(rows)
        self.assertEqual((s["true_positive"], s["mean_absolute_onset_delay"], s["early_actions"], s["late_actions"]),
                         (1, 2, 1, 0))
        marks = {r["time"] // 1000: r["value"] for r in rows if r["metric"] == "annotation" and r["value"] != "none"}
        self.assertEqual(marks, {1: "onset delay"})

    def test_queries_must_cover_the_same_containers(self):
        rows = query_rows(([0.5], [0.5]), ([1e9], [1e9])) + [
            row(0, "query_result", "r1", {"use_case": "uc", "query": "cpu", "container": "c1", "arm": "exact"},
                "value", 0.5, "unit")]
        with self.assertRaises(ValueError):
            derive(rows, "s", "cpu", "memory", "container", RULE)


if __name__ == "__main__":
    unittest.main()
