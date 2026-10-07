#!/usr/bin/env python3
"""Unit tests for recommend_config on tiny synthetic curve / summary CSVs.

Run: python3 scripts/test_recommend_config.py
"""

import csv
import os
import sys
import tempfile
import unittest

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

from recommend_config import (  # noqa: E402
    DEFAULT_TARGETS, FAMILIES, OUTPUT_COLUMNS, error_at, main, round_down, round_up,
)
from study_saturation import CURVE_COLUMNS, SUMMARY_COLUMNS  # noqa: E402

SUMMARY_HEADER = [
    "dataset", "query_id", "promql", "kind", "weight", "range", "range_s", "step_s",
    "worst_theta_cms", "worst_K", "min_N", "max_N", "worst_alpha_rank",
    "worst_alpha_memory", "target_are_top100", "target_precision_at_k",
    "target_hll_rel_err", "target_rank_err", "tail_class",
]
CMS = "cms-fastpath-vector2d"
SMALL, BIG = "rows=3 cols=256", "rows=3 cols=1024"


def write_csv(path, header, rows):
    with open(path, "w", newline="") as f:
        writer = csv.writer(f)
        writer.writerow(header)
        writer.writerows(rows)


def key_query(theta, K, min_N, max_N, target=""):
    return ["d", "q", "sum by (k) (x)", "keys", "value", "instant", "", "60",
            theta, K, min_N, max_N, "", "", target, "", "", "", ""]


class RoundingTest(unittest.TestCase):
    def test_ddsketch_uses_the_relative_value_error_default(self):
        self.assertEqual(FAMILIES["dd"][1], "target_relative_value_err")
        self.assertEqual(DEFAULT_TARGETS["target_relative_value_err"], 0.01)

    def test_theta_rounds_down_and_K_rounds_up(self):
        self.assertEqual(round_down(0.97, [0.5, 0.8, 1.0]), (0.8, ""))
        self.assertEqual(round_down(1.0, [0.5, 0.8, 1.0]), (1.0, ""))
        self.assertEqual(round_up(538, [1000, 100000]), (1000, ""))
        self.assertEqual(round_up(1000, [1000, 100000]), (1000, ""))

    def test_outside_the_grid_clamps_and_flags(self):
        value, flag = round_up(2e7, [1000, 10000000])
        self.assertEqual(value, 10000000)
        self.assertIn("beyond grid", flag)
        value, flag = round_down(0.3, [0.5, 1.0])
        self.assertEqual(value, 0.5)
        self.assertIn("below grid", flag)

    def test_error_interpolates_in_log_n_and_flags_extrapolation(self):
        curve = [(100, 0.1, 0), (10000, 0.3, 0)]
        self.assertAlmostEqual(error_at(curve, 1000)[0], 0.2)
        self.assertEqual(error_at(curve, 10 ** 6), (0.3, "extrapolated beyond N=10000"))


class RecommendTest(unittest.TestCase):
    def setUp(self):
        self.dir = tempfile.mkdtemp()
        self.curves = os.path.join(self.dir, "curve.csv")
        # theta 0.5 is harder than 1.0; the small config meets 0.05 only at 1.0.
        rows = []
        for theta, (small, big) in {0.5: (0.2, 0.04), 1.0: (0.03, 0.01)}.items():
            for config, err in ((SMALL, small), (BIG, big)):
                for n in (1000, 100000):
                    rows.append(["frequency", CMS, config, "zipf", theta, 1000, n, err, 0])
        write_csv(self.curves, CURVE_COLUMNS, rows)
        self.saturation = os.path.join(self.dir, "saturation.csv")
        self.write_saturation(n_sat=1000, memory="")

    def write_saturation(self, n_sat, memory):
        rows = []
        for theta in (0.5, 1.0):
            for config in (SMALL, BIG):
                mem = "" if memory == "" else memory[config]
                cost = ["", "", ""] if memory == "" else [0.5, 0.15, 0.001]
                rows.append(["frequency", CMS, config, "zipf", theta, 1000, n_sat, 0,
                             "are_top100", *cost, mem])
        write_csv(self.saturation, SUMMARY_COLUMNS, rows)

    def run_cms(self, query):
        summary = os.path.join(self.dir, "summary.csv")
        write_csv(summary, SUMMARY_HEADER, [query])
        out = os.path.join(self.dir, "rec.csv")
        main(["--summary", summary, "--curves", self.curves,
              "--curves-big", os.path.join(self.dir, "none.csv"),
              "--saturation", self.saturation,
              "--saturation-big", os.path.join(self.dir, "none.csv"), "--out", out])
        with open(out, newline="") as f:
            rows = list(csv.DictReader(f))
        self.assertEqual(list(rows[0]), OUTPUT_COLUMNS)
        return next(r for r in rows if r["family"] == "cms")

    def test_theta_between_grid_points_uses_the_lower_one(self):
        row = self.run_cms(key_query(0.97, 538, 5000, 5000))
        self.assertEqual(row["grid_param"], "0.5")
        self.assertEqual(row["grid_K"], "1000")
        self.assertEqual(row["config"], BIG)

    def test_smallest_config_meeting_the_target_wins(self):
        row = self.run_cms(key_query(1.0, 1000, 5000, 5000))
        self.assertEqual(row["config"], SMALL)
        self.assertEqual(row["meets_target"], "True")

    def test_no_config_meets_the_target_reports_the_best(self):
        row = self.run_cms(key_query(0.5, 1000, 5000, 5000, target="0.001"))
        self.assertEqual(row["config"], BIG)
        self.assertEqual(row["meets_target"], "False")
        self.assertIn("target not met", row["flags"])

    def test_missing_cost_leaves_columns_empty_and_orders_by_nominal_size(self):
        row = self.run_cms(key_query(0.5, 1000, 5000, 5000, target="0.5"))
        self.assertEqual(row["config"], SMALL)
        for col in ("memory_bytes", "insert_ns_per_item", "merge_us_per_fold", "query_phase_us"):
            self.assertEqual(row[col], "")

    def test_present_cost_orders_by_memory_and_converts_units(self):
        # The nominally bigger config is the smaller one in memory.
        self.write_saturation(n_sat=1000, memory={SMALL: 9000, BIG: 5000})
        row = self.run_cms(key_query(0.5, 1000, 5000, 5000, target="0.5"))
        self.assertEqual(row["config"], BIG)
        self.assertEqual(float(row["memory_bytes"]), 5000)
        self.assertAlmostEqual(float(row["insert_ns_per_item"]), 0.5 * 1e9 / 100000)
        self.assertAlmostEqual(float(row["merge_us_per_fold"]), 0.15 * 1e6 / 15)
        self.assertAlmostEqual(float(row["query_phase_us"]), 1000)

    def test_min_N_below_n_sat_is_flagged(self):
        self.write_saturation(n_sat=100000, memory="")
        row = self.run_cms(key_query(1.0, 1000, 5000, 5000))
        self.assertIn("not saturated at min_N", row["flags"])
        row = self.run_cms(key_query(1.0, 1000, 200000, 200000))
        self.assertNotIn("not saturated", row["flags"])
        self.assertIn("extrapolated", row["flags"])

    def test_never_saturated_is_flagged(self):
        self.write_saturation(n_sat="not_saturated", memory="")
        row = self.run_cms(key_query(1.0, 1000, 5000, 5000))
        self.assertIn("not saturated", row["flags"])


class MergeCurveTest(unittest.TestCase):
    def test_top_k_uses_the_nearest_shard_count(self):
        d = tempfile.mkdtemp()
        topk = "cms-heap-topk-fastpath-vector2d"
        curves = os.path.join(d, "curve.csv")
        write_csv(curves, CURVE_COLUMNS, [
            ["topk", topk, SMALL, "zipf", 1.0, 1000, n, 0.99, 0] for n in (1000, 100000)])
        merge = os.path.join(d, "merge.csv")
        write_csv(merge, CURVE_COLUMNS[:-2] + ["shards", "seed_mean_error", "seed_se"], [
            ["topk", topk, SMALL, "zipf", 1.0, 1000, n, m, err, 0]
            for m, err in ((4, 0.9), (64, 0.5)) for n in (1000, 100000)])
        summary = os.path.join(d, "summary.csv")
        query = key_query(1.0, 1000, 5000, 5000)
        query[5:7] = ["1h", "3600"]  # range 1h at step 60: m = 60, nearest 64
        write_csv(summary, SUMMARY_HEADER, [query])
        out = os.path.join(d, "rec.csv")
        base = ["--summary", summary, "--curves", curves, "--curves-big", "none",
                "--saturation", "none", "--saturation-big", "none", "--out", out]
        main(base + ["--merge-curves", merge])
        with open(out, newline="") as f:
            row = next(csv.DictReader(f))
        self.assertAlmostEqual(float(row["est_error"]), 0.5)
        self.assertIn("merged curve at m=64", row["flags"])
        self.assertEqual(row["shards"], "60")
        main(base)
        with open(out, newline="") as f:
            row = next(csv.DictReader(f))
        self.assertAlmostEqual(float(row["est_error"]), 0.99)
        self.assertIn("single-sketch curve optimistic", row["flags"])


if __name__ == "__main__":
    unittest.main()
