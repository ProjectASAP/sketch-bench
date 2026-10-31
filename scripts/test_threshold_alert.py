#!/usr/bin/env python3

import tempfile
import unittest

from approx_rows import read_run, row, write_run
from threshold_alert import derive, main, spans

THRESHOLD = 5.0


def query_rows(exact, approx, query="q", run_id="r1"):
    rows = []
    for i, (e, a) in enumerate(zip(exact, approx)):
        labels = {"use_case": "uc", "query": query}
        rows.append(row(i * 1000, "query_result", run_id, {**labels, "arm": "exact"}, "value", e, "addresses"))
        rows.append(row(i * 1000, "query_result", run_id, {**labels, "arm": "approx"}, "value", a, "addresses"))
    return rows


def on_off(bits):
    return [10.0 if b == "1" else 0.0 for b in bits]


def by_metric(rows, kind):
    return {r["metric"]: r["value"] for r in rows if r["kind"] == kind}


def events(rows):
    return sorted((r["time"] // 1000, r["metric"], r["value"]) for r in rows if r["kind"] == "fidelity_event")


class SpansTest(unittest.TestCase):
    def test_half_open_runs(self):
        self.assertEqual(spans([0, 1, 1, 0, 1], 1), [(1, 3), (4, 5)])
        self.assertEqual(spans([], 1), [])


class DeriveTest(unittest.TestCase):
    def test_use_case_document_example(self):
        out = derive(query_rows(on_off("001111110000"), on_off("000110111010")), {"q": THRESHOLD})
        summary = by_metric(out, "fidelity_summary")
        self.assertEqual((summary["true_positive"], summary["false_positive"],
                          summary["false_negative"], summary["true_negative"]), (4, 2, 2, 4))
        self.assertEqual((summary["mean_onset_delay"], summary["mean_absolute_end_delay"], summary["gaps"]), (1, 1, 1))
        self.assertEqual((summary["early_ends"], summary["late_ends"]), (0, 1))
        self.assertEqual(events(out), [(3, "onset_delay", 1), (5, "gap", 1), (8, "end_delay", 1),
                                       (10, "false_positive", 1)])
        marks = {r["time"] // 1000 + 1: r["value"] for r in out
                 if r["metric"] == "annotation" and r["value"] != "none"}
        self.assertEqual(marks, {4: "onset delay", 6: "gap", 9: "end delay", 11: "false positive"})

    def test_end_delay_stops_at_the_next_episode(self):
        out = derive(query_rows(on_off("0110110"), on_off("0111110")), {"q": THRESHOLD})
        self.assertEqual(events(out), [(3, "end_delay", 1)])

    def test_late_onset_gap_and_false_positives(self):
        out = derive(query_rows(on_off("0011111000"), on_off("0100101010")), {"q": THRESHOLD})
        summary = by_metric(out, "fidelity_summary")
        self.assertEqual((summary["true_positive"], summary["false_positive"],
                          summary["false_negative"], summary["true_negative"]), (2, 2, 3, 3))
        self.assertEqual(summary["episodes"], 1)
        self.assertEqual(summary["gaps"], 1)
        self.assertEqual(summary["mean_onset_delay"], 2)
        self.assertAlmostEqual(summary["episode_coverage"], 2 / 5)
        self.assertEqual((summary["mean_absolute_end_delay"], summary["early_ends"], summary["late_ends"]), (0, 0, 0))
        self.assertEqual(events(out), [(1, "false_positive", 1), (4, "onset_delay", 2),
                                       (5, "gap", 1), (8, "false_positive", 1)])

    def test_missed_episode_has_no_onset_delay(self):
        out = derive(query_rows(on_off("0110"), on_off("0000")), {"q": THRESHOLD})
        summary = by_metric(out, "fidelity_summary")
        self.assertEqual(summary["missed_episodes"], 1)
        self.assertNotIn("mean_onset_delay", summary)
        self.assertNotIn("mean_absolute_end_delay", summary)
        self.assertEqual(events(out), [(1, "missed_episode", 2)])

    def test_approx_stopping_early_is_a_negative_end_delay(self):
        out = derive(query_rows(on_off("1111"), on_off("1100")), {"q": THRESHOLD})
        self.assertEqual(events(out), [(2, "end_delay", -2)])
        summary = by_metric(out, "fidelity_summary")
        self.assertEqual((summary["mean_absolute_end_delay"], summary["early_ends"], summary["late_ends"]), (2, 1, 0))

    def test_value_at_threshold_is_not_firing(self):
        out = derive(query_rows([THRESHOLD], [THRESHOLD + 0.1]), {"q": THRESHOLD})
        states = {r["labels"]["arm"]: r["value"] for r in out if r["metric"] == "alert"}
        self.assertEqual(states, {"exact": "ok", "approx": "firing"})

    def test_zero_exact_value_has_no_relative_error(self):
        out = derive(query_rows([0.0, 4.0], [1.0, 5.0]), {"q": THRESHOLD})
        errors = [(r["time"], r["value"]) for r in out if r["kind"] == "low_level"]
        self.assertEqual(errors, [(1000, 0.25)])

    def test_misaligned_arms_rejected(self):
        rows = query_rows([1.0, 2.0], [1.0, 2.0])
        with self.assertRaises(ValueError):
            derive(rows[:-1], {"q": THRESHOLD})

    def test_every_query_needs_a_threshold(self):
        with self.assertRaises(ValueError):
            derive(query_rows([1.0], [1.0]), {"other": THRESHOLD})

    def test_cli_rewrites_run_with_derived_kinds(self):
        with tempfile.TemporaryDirectory() as out:
            write_run(out, "r1", "uc", query_rows(on_off("0110"), on_off("0110")))
            main(["--data", out, "--run", "r1", "--threshold", f"q={THRESHOLD}"])
            self.assertEqual(len(read_run(out, "r1", "query_result")), 8)
            self.assertEqual(by_metric(read_run(out, "r1", "fidelity_summary"), "fidelity_summary")["true_positive"], 2)


if __name__ == "__main__":
    unittest.main()
