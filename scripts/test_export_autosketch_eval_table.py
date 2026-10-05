#!/usr/bin/env python3
"""Unit tests for export_autosketch_eval_table on tiny synthetic CSVs.

Run: python3 scripts/test_export_autosketch_eval_table.py
"""

import csv
import json
import os
import sys
import tempfile
import unittest

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

from export_autosketch_eval_table import boom_query, main  # noqa: E402
from study_saturation import CURVE_COLUMNS, SUMMARY_COLUMNS  # noqa: E402

SUMMARY_HEADER = [
    "dataset", "query_id", "promql", "kind", "weight", "range", "range_s", "step_s",
    "K_total", "rows_total", "worst_theta_cms", "worst_K", "min_N", "max_N",
    "worst_alpha_rank", "worst_alpha_memory", "target_are_top100",
    "target_precision_at_k", "target_hll_rel_err", "target_rank_err", "tail_class",
]
CMS, KLL = "cms-fastpath-vector2d", "kll-percall"
MERGE_HEADER = CURVE_COLUMNS[:7] + ["shards"] + CURVE_COLUMNS[7:]


def write_csv(path, header, rows):
    with open(path, "w", newline="") as f:
        writer = csv.writer(f)
        writer.writerow(header)
        writer.writerows(rows)


class ExportTest(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        d = self.tmp.name
        self.path = lambda name: os.path.join(d, name)
        # One key query (CMS) and one value query (KLL), 1h lookback, 60 s step.
        write_csv(self.path("summary.csv"), SUMMARY_HEADER, [
            ["d", "freq", "sum by (k) (x)", "keys", "count", "1h", "3600", "60", "", "",
             "1.0", "1000", "500", "5000", "", "", "", "", "", "", ""],
            ["d", "p99", "quantile(0.99, x)", "values", "", "1h", "3600", "60", "", "",
             "", "", "500", "5000", "2.0", "2.0", "", "", "", "", ""],
        ])
        curve = [(100, 0.5), (1000, 0.2), (10000, 0.1)]
        write_csv(self.path("curve.csv"), CURVE_COLUMNS, [
            *[["frequency", CMS, "rows=3 cols=256", "zipf", "1.0", "1000", n, e, 0]
              for n, e in curve],
            *[["quantile", KLL, "k=50", "pareto", "2.0", "", n, e, 0] for n, e in curve],
        ])
        # CMS saturates at 1000 <= max_N 5000; KLL saturates at 10000 > 5000.
        write_csv(self.path("sat.csv"), SUMMARY_COLUMNS, [
            ["frequency", CMS, "rows=3 cols=256", "zipf", "1.0", "1000", "1000", "0.1",
             "are_top100", "0.1", "0.001", "0.002", "3072"],
            ["quantile", KLL, "k=50", "pareto", "2.0", "", "10000", "0.1",
             "mean_rank_err", "0.1", "0.001", "0.002", "800"],
        ])
        # Merged KLL curve only at m=4: doubles the error.
        write_csv(self.path("merge.csv"), MERGE_HEADER, [
            ["quantile", KLL, "k=50", "pareto", "2.0", "", n, 4, 2 * e, 0] for n, e in curve])

    def tearDown(self):
        self.tmp.cleanup()

    def export(self):
        out = self.path("table.json")
        main(["--summary", self.path("summary.csv"), "--curves", self.path("curve.csv"),
              "--saturation", self.path("sat.csv"), "--merge-curves", self.path("merge.csv"),
              "--revision", "test", "--out", out])
        with open(out) as f:
            table = json.load(f)
        rqes = {r["query_id"]: r for w in table["workloads"] for r in w["rqes"]}
        return {q: r["families"][0]["configs"][0] for q, r in rqes.items()}, rqes

    def test_exact_merge_reads_the_single_sketch_value_for_every_m(self):
        configs, _ = self.export()
        cms = configs["freq"]
        self.assertEqual(set(cms["asap_error_by_m"].values()), {cms["autosketch_error"]})

    def test_inexact_merge_reads_the_merged_curve_or_flags_its_absence(self):
        configs, _ = self.export()
        kll = configs["p99"]
        self.assertAlmostEqual(kll["asap_error_by_m"]["4"], 2 * kll["asap_error_by_m"]["1"])
        self.assertEqual(kll["asap_error_by_m"]["16"], kll["autosketch_error"])
        self.assertIn("single-sketch curve optimistic", kll["asap_flag_by_m"]["16"])

    def test_saturated_compares_max_N_with_n_sat(self):
        configs, _ = self.export()
        self.assertTrue(configs["freq"]["saturated"])
        self.assertFalse(configs["p99"]["saturated"])

    def test_autosketch_reads_the_curve_even_when_unsaturated(self):
        configs, _ = self.export()
        # max_N 5000 lies between 1000 (0.2) and 10000 (0.1), log-linear: 0.130.
        self.assertAlmostEqual(configs["p99"]["autosketch_error"], 0.2 - 0.1 * 0.69897, 4)

    def test_rqe_carries_window_and_label_set(self):
        _, rqes = self.export()
        r = rqes["freq"]
        self.assertEqual((r["lookback_secs"], r["interval_secs"]), (3600, 60))
        self.assertEqual(r["label_set"]["groups"], 1)
        self.assertAlmostEqual(r["label_set"]["arrival_rate_per_sec"], 5000 / 3600)


class BoomTest(unittest.TestCase):
    def test_chunk_defines_n_and_window(self):
        row = dict.fromkeys(SUMMARY_HEADER, "")
        row.update(dataset="boom", query_id="target_tail[ds-1-5T]", kind="values",
                   K_total="10", rows_total="2000")
        q = boom_query(row)
        self.assertEqual(q["max_N"], 100)
        # 2000 rows / 10 variates / 20 chunks = 10 steps of 300 s.
        self.assertEqual((q["range_s"], q["step_s"]), (3000, 3000))
        self.assertEqual(q["alpha_rank"], float("inf"))


if __name__ == "__main__":
    unittest.main()
