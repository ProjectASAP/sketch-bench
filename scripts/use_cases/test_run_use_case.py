#!/usr/bin/env python3

import unittest

from approx_rows import row
from cases.uc4_canary_release import stage2 as uc4_stage2
from rollout import rollout_errors
from run_use_case import rank_errors, relative_errors, stage1


def query(id_, answers, costs=(), planning_ms=None):
    return {"id": id_, "answers": answers, "costs": list(costs), "plan": {"planning_ms": planning_ms}}


def answer(time, value, exact_plan):
    return {"time": time, "series": {"__name__": "m", "version": "canary"}, "value": value,
            "exact_plan": exact_plan}


class ExactPlanErrorsTest(unittest.TestCase):
    def test_one_row_per_answer_scored_against_the_exact_plan(self):
        approx = query("p99", [answer(1000, 1.1, {"relative_error": 0.1, "rank_error": 0.005}),
                               answer(2000, 1.0, {"relative_error": 0.0, "rank_error": 0.0})])
        rows = rank_errors("r1", "uc", "sig", approx)
        self.assertEqual([(r["time"], r["value"]) for r in rows], [(1000, 0.005), (2000, 0.0)])
        self.assertEqual(rows[0]["labels"], {"use_case": "uc", "signal": "sig", "query": "p99", "version": "canary"})
        self.assertEqual([r["value"] for r in relative_errors("r1", "uc", "sig", approx)], [0.1, 0.0])

    def test_answers_without_a_score_give_no_rows(self):
        plain = query("avg", [{"time": 1000, "series": {}, "value": 1.0},
                              answer(2000, 1.0, {"relative_error": 0.0})])
        self.assertEqual(rank_errors("r1", "uc", "sig", plain), [])
        self.assertEqual(len(relative_errors("r1", "uc", "sig", plain)), 1)


class Stage1Test(unittest.TestCase):
    def test_error_against_target_and_cost(self):
        exact = query("p99", [{"time": 1000, "series": {"__name__": "m", "version": "canary"}, "value": 1.0}],
                      [{"refresh_seconds": 0.0, "samples_ingested": 0, "query_seconds": 0.01,
                        "compile_seconds": 0.001, "samples_read_at_query": 500, "state_bytes": 0}], 20.0)
        approx = query("p99", [answer(1000, 1.1, {"relative_error": 0.1, "rank_error": 0.005})],
                       [{"refresh_seconds": 0.5, "samples_ingested": 1000, "query_seconds": 0.002,
                         "compile_seconds": 0.0005, "samples_read_at_query": 0, "state_bytes": 4096}], 300.0)
        derived = [row(t, "low_level", "r1", {"query": "p99"}, "rank_error", e, "rank")
                   for t, e in ((1000, 0.002), (2000, 0.02))]
        out = stage1({"max_error": 0.01, "failure_probability": 0.01}, exact, approx, derived)
        self.assertEqual((out["error metric"], out["max error"], out["evaluations"], out["evaluations over target"]),
                         ("rank_error", 0.02, 2, 1))
        self.assertEqual((out["insert samples per second"], out["peak state bytes"]), (2000.0, 4096))
        self.assertAlmostEqual(out["mean query ms"], 2.0)
        self.assertEqual((out["exact insert samples per second"], out["exact mean samples read at query"]),
                         (None, 500))
        self.assertEqual((out["peak raw sample bytes read at query"],
                          out["exact peak raw sample bytes read at query"]), (0, 8000))
        self.assertEqual((out["planning ms"], out["exact planning ms"]), (300.0, 20.0))
        self.assertAlmostEqual(out["exact mean compile ms"], 1.0)


class RolloutErrorsTest(unittest.TestCase):
    def test_max_error_of_each_query_over_the_rollout_checks(self):
        derived = [row(t, "app_state", "r1", {"app": "a", "signal": rule}, "check", "passed", "state")
                   for rule, times in (("one", range(4)), ("other", (0, 3))) for t in times]
        derived += [row(t, "fidelity_event", "r1",
                        {"signal": rule, "rollout": n, "exact": "promote", "approx": approx}, "rollout", outcome,
                        "outcome")
                    for rule, t, n, approx, outcome in (("one", 0, "1", "rollback", "false_positive"),
                                                        ("one", 2, "2", "promote", "true_negative"),
                                                        ("other", 0, "1", "promote", "true_negative"))]
        errors = [row(t, "low_level", "r1", {"query": q}, "rank_error", e, "rank")
                  for q, values in (("canary", (0.1, 0.3, 0.2, 0.0)), ("stable", (0.0, 0.0, 0.0, 0.4)))
                  for t, e in enumerate(values)]
        rules = {"one": {"checks": 2, "queries": ("canary", "stable")}, "other": {"checks": 2, "queries": ("canary",)}}
        found = rollout_errors(derived, errors, rules)
        self.assertEqual([(r["rule"], r["rollout"], r["outcome"], r["max error"]) for r in found],
                         [("one", 1, "false_positive", {"canary": 0.3, "stable": 0.0}),
                          ("one", 2, "true_negative", {"canary": 0.2, "stable": 0.4}),
                          ("other", 1, "true_negative", {"canary": 0.1})])


    def test_success_rate_uses_its_relative_error(self):
        derived = [row(0, "app_state", "r1", {"signal": "argo_rollouts"}, "check", "passed", "state"),
                   row(0, "fidelity_event", "r1", {"signal": "argo_rollouts", "rollout": "1", "exact": "promote",
                                                   "approx": "promote"}, "rollout", "true_negative", "outcome")]
        derived += [row(0, "low_level", "r1", {"query": q}, "relative_error", e, "ratio")
                    for q, e in (("success_rate_canary", 0.02), ("p99_canary", 0.5))]
        errors = [row(0, "low_level", "r1", {"query": "p99_canary"}, "rank_error", 0.001, "rank")]
        found = [r for r in uc4_stage2(derived, errors) if r["rule"] == "argo_rollouts"]
        self.assertEqual([r["max error"] for r in found], [{"success_rate_canary": 0.02}])


if __name__ == "__main__":
    unittest.main()
