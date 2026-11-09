#!/usr/bin/env python3

import unittest

import rules
import scenario
from approx_rows import row

SPEC = {
    "use_case": "uc", "start_ms": 0, "duration_ms": 600000,
    "metrics": [
        {"name": "cpu", "series": [
            {"count": 2, "labels": {"container": "c-{i}"},
             "phases": [{"start_offset_ms": 60000, "end_offset_ms": 120000, "ramp_ms": 0, "stagger_ms": 30000,
                         "change": {"scale": {"factor": 2}}}]},
            {"labels": {"container": "bg"},
             "phases": [{"start_offset_ms": 0, "end_offset_ms": 600000, "ramp_ms": 0,
                         "change": {"scale": {"factor": 2}}}]}]},
        {"name": "requests", "label_from_value": "client_id", "series": [
            {"labels": {}, "phases": [{"start_offset_ms": 300000, "end_offset_ms": 360000, "ramp_ms": 0,
                                       "change": {"mix": {"share": 0.1, "distribution": {
                                           "kind": "uniform", "lower_bound": 7, "upper_bound": 9}}}}]}]},
    ],
    "queries": [{"window_ms": 60000}],
}


class IncidentsTest(unittest.TestCase):
    def test_staggered_series_and_mixed_values_skip_the_background(self):
        found = scenario.incidents(SPEC, lambda labels: labels["container"])
        self.assertEqual([(i["entity"], i["start"], i["end"]) for i in found],
                         [("c-0", 60000, 120000), ("c-1", 90000, 150000), ("7", 300000, 360000),
                          ("8", 300000, 360000)])
        self.assertEqual({i["name"] for i in found}, {"cpu x2 1-2 min", "requests 10% mix 5-6 min"})


class ScoreTest(unittest.TestCase):
    def test_caught_delay_and_actions_outside(self):
        found = scenario.incidents(SPEC, lambda labels: labels["container"])
        acted = [(120000, "c-1"), (150000, "c-0"), (500000, "c-0"), (420000, "8"), (480000, "9")]
        out = scenario.score(found, 60000, acted)
        self.assertEqual(out, {"incidents": 2, "caught": 2, "actions_outside": 2, "mean_detection_seconds": 75.0})

    def test_derive_reads_rule_states_per_arm(self):
        derived = [row(t, "app_state", "r1", {"use_case": "uc", "app": rules.APP, "signal": "s", "arm": arm,
                                               "entity": "c-0"}, "state", value, "state")
                   for t, arm, value in ((90000, "exact", rules.ACTIVE), (90000, "approx", rules.IDLE))]
        rows, summary = scenario.derive("r1", SPEC, lambda labels: labels["container"], derived)
        self.assertEqual((summary["s"]["exact"]["caught"], summary["s"]["approx"]["caught"]), (1, 0))
        self.assertEqual({r["metric"] for r in rows if r["labels"]["arm"] == "approx"},
                         {"scenario_incidents", "scenario_caught", "scenario_actions_outside"})


if __name__ == "__main__":
    unittest.main()
