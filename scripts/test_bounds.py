#!/usr/bin/env python3

import unittest

import bounds
from bounds import Bounded


class TriStateTest(unittest.TestCase):
    def test_comparisons(self):
        self.assertIs(bounds.gt(Bounded(5, 4, 6), 3), True)
        self.assertIs(bounds.gt(Bounded(5, 4, 6), 6), False)
        self.assertIsNone(bounds.gt(Bounded(5, 4, 6), 5))
        self.assertIs(bounds.lt(Bounded(5, 4, 6), 7), True)
        self.assertIsNone(bounds.ge(Bounded(5, 4, 6), 5))
        self.assertIs(bounds.gt(2.0, 1.0), True)
        self.assertIs(bounds.gt(None, 1.0), False)

    def test_ratio_takes_the_widest_corners(self):
        r = bounds.ratio(Bounded(2, 1, 3), Bounded(2, 1, 4))
        self.assertEqual((r.value, r.lo, r.hi), (1, 0.25, 3))
        self.assertEqual(bounds.ratio(4.0, 2.0), 2.0)

    def test_three_valued_logic(self):
        self.assertIs(bounds.any_of(None, True), True)
        self.assertIsNone(bounds.any_of(None, False))
        self.assertIs(bounds.all_of(None, False), False)
        self.assertIsNone(bounds.all_of(None, True))

    def test_settle(self):
        self.assertEqual(bounds.settle([True, None, None], [False, True, False], bounds.BOUND),
                         ([True, False, False], 2))
        self.assertEqual(bounds.settle([True, None, None], [False, True, False], bounds.RERUN),
                         ([True, True, False], 2))
        self.assertEqual(bounds.settle(["up", None], ["up", "down"], bounds.BOUND, "none"), (["up", "none"], 1))


class WidenTest(unittest.TestCase):
    def test_quantile_queries_get_both_ends(self):
        spec = {"queries": [{"id": "p99", "promql": "quantile_over_time(0.99, m[5m])"},
                            {"id": "n", "promql": "count_over_time(m[5m])"}]}
        out = bounds.with_bound_queries(spec, 0.05)
        self.assertEqual([(q["id"], q["promql"]) for q in out["queries"][2:]],
                         [("p99_lower_bound", "quantile_over_time(0.94, m[5m])"),
                          ("p99_upper_bound", "quantile_over_time(1, m[5m])")])

    def plan(self, metric):
        return {"sketches": [{}], "sketch_analytics": [{"readouts": [{"error": {"metric": metric}}]}]}

    def test_rank_reads_the_ends_and_relative_scales(self):
        answers = lambda v: [{"time": 1, "series": {"__name__": "m", "host": "a"}, "value": v}]
        approx = {"queries": [{"id": "p99", "plan": self.plan("rank"), "answers": answers(10.0)},
                              {"id": "p99_lower_bound", "plan": self.plan("rank"), "answers": answers(8.0)},
                              {"id": "p99_upper_bound", "plan": self.plan("rank"), "answers": answers(13.0)},
                              {"id": "d", "plan": self.plan("relative"), "answers": answers(100.0)},
                              {"id": "e", "plan": {"sketches": [], "sketch_analytics": []}, "answers": []},
                              {"id": "u", "plan": {"sketches": [{}], "sketch_analytics": [{"readouts": [{}]}]}}]}
        widen = bounds.Widen(approx, 0.25)
        self.assertEqual(widen.bounded_queries(), ["d", "p99"])
        labels = {"use_case": "x", "query": "p99", "arm": "approx", "host": "a"}
        b = widen("p99", 1, labels, 10.0)
        self.assertEqual((b.lo, b.hi), (8.0, 13.0))
        b = widen("d", 1, labels, 100.0)
        self.assertEqual((b.lo, b.hi), (80.0, 100.0 / 0.75))
        self.assertEqual(widen("e", 1, labels, 3.0), 3.0)
        self.assertEqual(widen("u", 1, labels, 3.0), 3.0)
        self.assertEqual(widen("p99", 2, labels, 10.0), 10.0)


if __name__ == "__main__":
    unittest.main()
