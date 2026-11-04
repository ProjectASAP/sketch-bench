#!/usr/bin/env python3

import unittest

from approx_rows import row
from run_use_case import rank_errors, rank_gap, rollout_errors, stage1


def query(id_, answers, costs=()):
    return {"id": id_, "answers": answers, "costs": list(costs)}


def answer(time, value, rank):
    return {"time": time, "series": {"__name__": "m", "version": "canary"}, "value": value, "rank": rank}


class RankGapTest(unittest.TestCase):
    def test_overlapping_intervals_have_no_gap(self):
        self.assertEqual(rank_gap((0.98, 0.99), (0.985, 0.995)), 0.0)

    def test_gap_is_the_distance_between_intervals_on_either_side(self):
        self.assertAlmostEqual(rank_gap((0.989, 0.99), (0.993, 0.994)), 0.003)
        self.assertAlmostEqual(rank_gap((0.989, 0.99), (0.985, 0.986)), 0.003)


class RankErrorsTest(unittest.TestCase):
    def test_one_row_per_answer_matched_by_time_and_labels(self):
        exact = query("p99", [answer(1000, 1.0, (0.99, 0.99)), answer(2000, 1.0, (0.99, 0.99))])
        approx = query("p99", [answer(1000, 1.1, (0.995, 0.995)), answer(2000, 1.0, (0.99, 0.99))])
        rows = rank_errors("r1", "uc", "sig", exact, approx)
        self.assertEqual([(r["time"], round(r["value"], 6)) for r in rows], [(1000, 0.005), (2000, 0.0)])
        self.assertEqual(rows[0]["labels"], {"use_case": "uc", "signal": "sig", "query": "p99", "version": "canary"})

    def test_answers_without_rank_give_no_rows(self):
        plain = query("avg", [{"time": 1000, "series": {}, "value": 1.0}])
        self.assertEqual(rank_errors("r1", "uc", "sig", plain, plain), [])


class Stage1Test(unittest.TestCase):
    def test_error_against_target_and_cost(self):
        exact = query("p99", [answer(1000, 1.0, (0.99, 0.99))],
                      [{"refresh_seconds": 0.0, "samples_ingested": 0, "query_seconds": 0.01,
                        "samples_read_at_query": 500, "state_bytes": 0}])
        approx = query("p99", [answer(1000, 1.1, (0.995, 0.995))],
                       [{"refresh_seconds": 0.5, "samples_ingested": 1000, "query_seconds": 0.002,
                         "samples_read_at_query": 0, "state_bytes": 4096}])
        derived = [row(t, "low_level", "r1", {"query": "p99"}, "rank_error", e, "rank")
                   for t, e in ((1000, 0.002), (2000, 0.02))]
        out = stage1({"max_error": 0.01, "failure_probability": 0.01}, exact, approx, derived)
        self.assertEqual((out["error metric"], out["max error"], out["evaluations"], out["evaluations over target"]),
                         ("rank_error", 0.02, 2, 1))
        self.assertEqual((out["insert samples per second"], out["peak state bytes"]), (2000.0, 4096))
        self.assertAlmostEqual(out["mean query ms"], 2.0)
        self.assertEqual((out["exact insert samples per second"], out["exact mean samples read at query"]),
                         (None, 500))


class RolloutErrorsTest(unittest.TestCase):
    def test_max_error_of_each_query_over_the_rollout_checks(self):
        derived = [row(t, "app_state", "r1", {"app": "a"}, "ratio", 1.0, "ratio") for t in range(4)]
        derived += [row(0, "fidelity_event", "r1", {"rollout": "1", "exact": "promote", "approx": "rollback"},
                        "rollout", "false_positive", "outcome"),
                    row(2, "fidelity_event", "r1", {"rollout": "2", "exact": "promote", "approx": "promote"},
                        "rollout", "true_negative", "outcome")]
        errors = [row(t, "low_level", "r1", {"query": q}, "rank_error", e, "rank")
                  for q, values in (("canary", (0.1, 0.3, 0.2, 0.0)), ("stable", (0.0, 0.0, 0.0, 0.4)))
                  for t, e in enumerate(values)]
        found = rollout_errors(derived, errors, 2)
        self.assertEqual([(r["rollout"], r["outcome"], r["max error"]) for r in found],
                         [(1, "false_positive", {"canary": 0.3, "stable": 0.0}),
                          (2, "true_negative", {"canary": 0.2, "stable": 0.4})])


if __name__ == "__main__":
    unittest.main()
