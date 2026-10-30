#!/usr/bin/env python3

import unittest

from approx_rows import row
from decision import decide, derive

RULE = {"cpu_up": 0.8, "memory_up": 1.5e9, "cpu_down": 0.3, "memory_down": 0.5e9}


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
        self.assertEqual(decide(0.2, 0.4e9, RULE), "down")
        self.assertEqual(decide(0.2, 0.6e9, RULE), "none")


class DeriveTest(unittest.TestCase):
    def test_late_scale_up_and_a_spurious_scale_down(self):
        exact_cpu = [0.5, 0.9, 0.9, 0.5, 0.5]
        approx_cpu = [0.5, 0.7, 0.9, 0.5, 0.2]
        memory = [0.4e9] * 5
        rows = derive(query_rows((exact_cpu, approx_cpu), (memory, memory)), "s", "cpu", "memory", "container", RULE)
        s = summary(rows)
        self.assertEqual(s["exact_up_approx_none"], 1)
        self.assertEqual(s["exact_up_approx_up"], 1)
        self.assertEqual(s["exact_none_approx_down"], 1)
        self.assertEqual(s["exact_none_approx_none"], 2)
        self.assertEqual(s["agreement"], 3 / 5)
        self.assertEqual(s["up_episodes"], 1)
        self.assertEqual(s["up_mean_onset_delay"], 1)
        self.assertEqual(s["down_false_actions"], 1)
        events = sorted((r["time"] // 1000, r["labels"]["action"], r["metric"]) for r in rows
                        if r["kind"] == "fidelity_event")
        self.assertEqual(events, [(1, "up", "late_onset"), (4, "down", "false_positive")])

    def test_queries_must_cover_the_same_containers(self):
        rows = query_rows(([0.5], [0.5]), ([1e9], [1e9])) + [
            row(0, "query_result", "r1", {"use_case": "uc", "query": "cpu", "container": "c1", "arm": "exact"},
                "value", 0.5, "unit")]
        with self.assertRaises(ValueError):
            derive(rows, "s", "cpu", "memory", "container", RULE)


if __name__ == "__main__":
    unittest.main()
