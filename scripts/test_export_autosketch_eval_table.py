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

from study_saturation import COST_KEYS, COST_PARETO_ALPHA, COST_THETA  # noqa: E402
from export_autosketch_eval_table import (  # noqa: E402
    SYNTHETIC_DEFAULT, SYNTHETIC_GRID, boom_query, main, queries_per_instance, shared_queries,
    synthetic_plan, synthetic_queries)
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

    def test_synthetic_mode_writes_one_table_per_grid_point_and_a_plan(self):
        out = self.path("synthetic")
        main(["--synthetic", "--revision", "test", "--out", out])
        with open(os.path.join(out, "plan.tsv")) as f:
            plan = [line.rstrip("\n").split("\t") for line in f]
        self.assertEqual(len(plan), len(synthetic_plan()))
        self.assertEqual(len(os.listdir(out)), len({row[0] for row in plan}) + 1)
        default = "synthetic-templatesdashboard-shared1.json"
        self.assertIn([default, "p95", default[:-5] + "-tp95.json"], plan)
        with open(os.path.join(out, "synthetic-templatesall-shared1.json")) as f:
            rqes = {r["query_id"] + "/" + r["range"]: r
                    for r in json.load(f)["workloads"][0]["rqes"]}
        # The tables hold the workload only: sums name the exact accumulator
        # with no data shape; sketches carry the cost table's shape.
        family = rqes["sum_by_job/1s"]["families"][0]
        self.assertEqual((family["sketch"], family["target"], family["grid_param"]),
                         ("exact-sum", 0.0, None))
        self.assertNotIn("configs", family)
        self.assertEqual(rqes["sum_by_job/1s"]["label_set"]["groups"], 10)
        top = rqes["topk32_sum_by_label0_rate/15m"]["families"][0]
        self.assertEqual((top["grid_param"], top["grid_K"]), (COST_THETA, COST_KEYS))
        kll, dd = rqes["quantile_over_time_p0.5/15m"]["families"]
        self.assertEqual((kll["sketch"], kll["grid_param"], kll["grid_K"]),
                         ("kll-percall", COST_PARETO_ALPHA, None))
        self.assertEqual(dd["sketch"], "dd")


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


class SyntheticQueriesTest(unittest.TestCase):
    def test_ten_templates_give_50_distinct_rqes(self):
        qs = synthetic_queries(templates="all")
        # 1 + 5 spatial; per window 1 + 1 + 5 + 1 + 1 + 1 + 1 temporal, and
        # template 10's operands repeat template 5's.
        self.assertEqual(len(qs), 6 + 11 * 4)
        streams = {(q["stream"], q["capability"], q["groups"]) for q in qs}
        self.assertEqual(streams, {
            ("job/value", "sum", 10), ("job/increment", "rate", 10),
            ("job/dist", "quantile", 10), ("series/value", "sum", 10_000),
            ("series/increment", "rate", 10_000), ("series/dist", "quantile", 10_000),
            ("label_0/value", "topk", 1), ("label_0/increment", "topk", 1)})

    def test_items_per_instance_follow_the_data_model(self):
        qs = {(q["query_id"], q["range"]): q for q in synthetic_queries(templates="all")}
        # 1e4 series x 200 samples/s = 2e6 samples/s.
        self.assertEqual(qs[("quantile_by_job_p0.5", "1s")]["max_N"], 2e5)
        self.assertEqual(qs[("quantile_over_time_p0.5", "15m")]["max_N"], 200 * 900)
        top = qs[("topk32_sum_by_label0_rate", "24h")]
        self.assertEqual((top["K"], top["groups"], top["max_N"]), (10_000, 1, 2e6 * 86400))

    def test_every_template_issues_one_query_per_instance(self):
        self.assertEqual({queries_per_instance(q) for q in synthetic_queries(templates="all")},
                         {1})


class SyntheticPlanTest(unittest.TestCase):
    def test_plan_is_the_default_and_each_dimension_alone(self):
        plan = synthetic_plan()
        self.assertEqual(plan[0], SYNTHETIC_DEFAULT)
        for dim, values in SYNTHETIC_GRID.items():
            for v in values:
                self.assertIn({**SYNTHETIC_DEFAULT, dim: v}, plan)
        self.assertEqual(len(plan), 1 + sum(len(v) - 1 for v in SYNTHETIC_GRID.values()))


class DashboardAndSharedTest(unittest.TestCase):
    def test_dashboard_has_21_distinct_rqes(self):
        qs = synthetic_queries(templates="dashboard")
        count = {}
        for q in qs:
            name = q["query_id"].split("_p")[0]
            count[name] = count.get(name, 0) + 1
        # D1 3 q x 4 windows, D2 3, D3 4, D4 2; D5 repeats D1.
        self.assertEqual(count, {"quantile_over_time": 12, "quantile_by_job": 3,
                                 "sum_by_job_rate": 4, "topk32_sum_by_label0_rate": 2})
        self.assertTrue(all(q["step_s"] == 60 for q in qs if q["range"] != "1s"))

    def test_shared_replicas_read_one_stream_and_dedupe(self):
        point = {**SYNTHETIC_DEFAULT, "shared": 4}
        qs = shared_queries(point)
        self.assertEqual(qs, shared_queries(point))  # seeded
        ids = [(q["query_id"], q["range"]) for q in qs]
        self.assertEqual(len(ids), len(set(ids)))
        self.assertTrue(all(q["dataset"].endswith("shared=4") for q in qs))
        self.assertTrue(all("@" in q["range"] for q in qs if q["range"] != "1s"))
        sizes = [len(shared_queries({**SYNTHETIC_DEFAULT, "shared": r})) for r in (1, 8, 64)]
        self.assertLess(sizes[0], sizes[1])
        self.assertLess(sizes[2], 8 * sizes[1])


if __name__ == "__main__":
    unittest.main()
