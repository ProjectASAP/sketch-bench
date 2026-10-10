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
    SCHEMA_TEMPLATES, SCHEMAS, SYNTHETIC_DEFAULT, SYNTHETIC_GRID, SYNTHETIC_TEMPLATE_SETS,
    all_subsets, boom_query, group_coverage, grouping_cardinality, label_shares, main, metric_queries, queries_per_instance,
    shared_queries, synthetic_plan, synthetic_queries)
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
        default = "synthetic-templatesall-shared1.json"
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
        qs = synthetic_queries(templates="classic")
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
        qs = {(q["query_id"], q["range"]): q for q in synthetic_queries(templates="classic")}
        # 1e4 series x 200 samples/s = 2e6 samples/s.
        self.assertEqual(qs[("quantile_by_job_p0.5", "1s")]["max_N"], 2e5)
        self.assertEqual(qs[("quantile_over_time_p0.5", "15m")]["max_N"], 200 * 900)
        top = qs[("topk32_sum_by_label0_rate", "24h")]
        self.assertEqual((top["K"], top["groups"], top["max_N"]), (10_000, 1, 2e6 * 86400))

    def test_every_template_issues_one_query_per_instance(self):
        self.assertEqual({queries_per_instance(q) for q in synthetic_queries(templates="all")},
                         {1})


COMMITTED_TABLES = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..",
                                "rqe-optimizer", "results", "autosketch-vs-asap-inputs",
                                "tables")


class SchemaTemplatesTest(unittest.TestCase):
    def test_classic_tables_are_main_s_all_tables_byte_for_byte(self):
        # The committed tables are the 10-template set's, from before "all"
        # grew: renamed, today's "classic" tables match them byte for byte.
        with tempfile.TemporaryDirectory() as out:
            main(["--synthetic", "--revision", "test", "--out", out])
            classic = [n for n in sorted(os.listdir(out)) if "templatesclassic" in n]
            self.assertEqual(len(classic), 4)
            for name in classic:
                with open(os.path.join(out, name)) as f:
                    ours = json.loads(f.read().replace("templates=classic", "templates=all"))
                with open(os.path.join(COMMITTED_TABLES,
                                       name.replace("templatesclassic", "templatesall"))) as f:
                    committed = json.load(f)
                del ours["sketch_bench_revision"], committed["sketch_bench_revision"]
                self.assertEqual(json.dumps(ours, sort_keys=True),
                                 json.dumps(committed, sort_keys=True), name)

    def test_all_is_classic_plus_the_schema_templates(self):
        classic = synthetic_queries(templates="classic", dataset="d")
        every = synthetic_queries(templates="all", dataset="d")
        self.assertEqual(every[:len(classic)], classic)
        new = every[len(classic):]
        # Per window (5m, 15m): 1 + 3 + 4 + 15 + 3 + 1 groupings.
        self.assertEqual(len(new), 2 * 27)
        self.assertEqual({q["stream"] for q in new},
                         {"flows/src_ip", "http/user_id", "http/latency"})
        self.assertEqual({q["range"] for q in new}, {"5m", "15m"})
        self.assertTrue(all("grouping" not in q for q in classic))

    def test_schema_cardinalities_count_a_child_with_its_parent(self):
        self.assertEqual(grouping_cardinality("http", ["region"]), 4)
        # An endpoint value names its service: 25 per service, 625 in all.
        self.assertEqual(grouping_cardinality("http", ["endpoint"]), 625)
        self.assertEqual(grouping_cardinality("http", ["service", "endpoint"]), 625)
        self.assertEqual(grouping_cardinality("http", all_subsets("http")[-1]), 10_000)
        self.assertEqual(grouping_cardinality("flows", ["dst_subnet", "dst_port"]), 10**6)

    def test_dropping_labels_never_adds_groups_and_rates_cover_every_group(self):
        for metric, schema in SCHEMAS.items():
            subsets = all_subsets(metric)
            for fine in subsets:
                for coarse in subsets:
                    if set(coarse) < set(fine):
                        self.assertLessEqual(grouping_cardinality(metric, coarse),
                                             grouping_cardinality(metric, fine))
        # Every grouping a template reads has a series per group (the
        # runner's full label set) and the curves' first N in a 5m window.
        for q in synthetic_queries(templates="all"):
            if "grouping" in q:
                self.assertLessEqual(q["groups"], q["arrival_rate"])
                self.assertGreaterEqual(q["max_N"], 1000)

    def test_template_groupings_are_ordered_subsets_of_the_schema(self):
        for n, _, metric, value, _, groupings, _ in SCHEMA_TEMPLATES:
            names = [label["name"] for label in SCHEMAS[metric]["labels"]]
            self.assertIn(value, SCHEMAS[metric]["values"])
            for g in groupings:
                self.assertTrue(g, n)
                self.assertEqual(g, [x for x in names if x in g], n)
            self.assertEqual(len(groupings), len({tuple(g) for g in groupings}), n)
        groupings = {n: g for n, *_, g, _ in SCHEMA_TEMPLATES}
        self.assertEqual(len(groupings[15]), 15)
        self.assertEqual(groupings[17], [["region", "service", "endpoint", "status"]])

    def test_rqe_entries_name_grouping_coverage_and_shape(self):
        with tempfile.TemporaryDirectory() as out:
            main(["--synthetic", "--revision", "test", "--out", out])
            with open(os.path.join(out, "synthetic-templatesall-shared1.json")) as f:
                workload = json.load(f)["workloads"][0]
        self.assertEqual(set(workload["schemas"]), {"http", "flows"})
        rqes = {r["query_id"] + "/" + r["range"]: r for r in workload["rqes"]}
        r = rqes["t14_distinct_users_by_region,service/5m"]
        self.assertEqual((r["grouping"], r["covers_share"], r["label_set"]["groups"]),
                         (["region", "service"], 0.01, 100))
        self.assertEqual(r["families"][0]["sketch"], "hll")
        self.assertEqual((r["families"][0]["grid_param"], r["families"][0]["grid_K"]),
                         (0.8, 1_000_000))
        self.assertIsNone(rqes["t17_distinct_users_by_region,service,endpoint,status/15m"]
                          ["covers_share"])
        kll, dd = rqes["t16_p99_latency_by_service,status/5m"]["families"]
        self.assertEqual((kll["grid_param"], kll["grid_K"]), (COST_PARETO_ALPHA, None))
        self.assertEqual(rqes["t11_distinct_src_by_dst_subnet/5m"]["covers_share"], 0.05)
        self.assertEqual((r["covered_groups"], r["min_covered_share"], r["covered_min_N"]),
                         group_coverage("http", ["region", "service"], 0.01)
                         + (2e6 * 300 * r["min_covered_share"],))

    def test_label_shares_are_skewed_and_the_burst_lifts_a_tail_subnet(self):
        for metric, schema in SCHEMAS.items():
            for label in schema["labels"]:
                self.assertAlmostEqual(sum(label_shares(metric, label)), 1.0)
        service = label_shares("http", SCHEMAS["http"]["labels"][1])
        self.assertGreater(service[0], 5 * service[-1])
        subnets = label_shares("flows", SCHEMAS["flows"]["labels"][0])
        self.assertGreaterEqual(subnets[999], 0.05)

    def test_coverage_is_the_groups_over_the_share_plus_the_largest(self):
        # Brute force over every group: shares are products of the labels'.
        http = {label["name"]: label for label in SCHEMAS["http"]["labels"]}
        region, status = (label_shares("http", http[n]) for n in ["region", "status"])
        shares = [r * s for r in region for s in status]
        for tau in [0.01, 0.05, 0.2]:
            covered = [x for x in shares if x >= tau]
            self.assertEqual(group_coverage("http", ["region", "status"], tau),
                             (len(covered), min(covered)))
        # No group holds 50%: the largest alone is covered.
        self.assertEqual(group_coverage("http", ["region", "status"], 0.5), (1, max(shares)))
        self.assertEqual(group_coverage("http", ["region", "status"], None),
                         (16, min(shares)))
        # The DDoS subnet is one of template 11's covered groups.
        n, smallest = group_coverage("flows", ["dst_subnet"], 0.05)
        self.assertGreater(n, 1)
        self.assertAlmostEqual(smallest, label_shares("flows", SCHEMAS["flows"]["labels"][0])[999])

    def test_every_covered_rqe_names_its_covered_groups_and_their_items(self):
        for q in synthetic_queries(templates="all"):
            if q.get("covers_share") is not None:
                self.assertGreaterEqual(q["covered_groups"], 1)
                self.assertEqual(q["covered_min_N"],
                                 q["arrival_rate"] * q["range_s"] * q["min_covered_share"])
                self.assertGreaterEqual(q["covered_min_N"], 1000)

    def test_metric_copies_name_their_schema_metrics(self):
        qs = metric_queries({**SYNTHETIC_DEFAULT, "metrics": 2})
        ids = [(q["query_id"], q["range"]) for q in qs]
        self.assertEqual(len(ids), len(set(ids)))
        self.assertEqual({q["metric"] for q in qs if "grouping" in q},
                         {"data_0/http", "data_0/flows", "data_1/http", "data_1/flows"})


class SyntheticPlanTest(unittest.TestCase):
    def test_plan_is_the_default_and_each_dimension_alone(self):
        plan = synthetic_plan()
        self.assertEqual(plan[0], SYNTHETIC_DEFAULT)
        for templates in SYNTHETIC_TEMPLATE_SETS:
            default = {**SYNTHETIC_DEFAULT, "templates": templates}
            for dim, values in SYNTHETIC_GRID.items():
                for v in values:
                    self.assertIn({**default, dim: v}, plan)
        self.assertEqual(len(plan), len(SYNTHETIC_TEMPLATE_SETS)
                         * (1 + sum(len(v) - 1 for v in SYNTHETIC_GRID.values())))


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
        sizes = [len(shared_queries({**SYNTHETIC_DEFAULT, "shared": r})) for r in (1, 8)]
        self.assertLess(sizes[0], sizes[1])

    def test_metric_copies_share_nothing_and_grow_linearly(self):
        point = {**SYNTHETIC_DEFAULT, "templates": "classic", "metrics": 8}
        qs = metric_queries(point)
        self.assertEqual(len(qs), 8 * 50)
        self.assertEqual(len({(q["query_id"], q["range"]) for q in qs}), len(qs))
        streams = {q["stream"].split("/")[0] for q in qs}
        self.assertEqual(streams, {f"data_{i}" for i in range(8)})
        self.assertTrue(all(q["dataset"].endswith("metrics=8") for q in qs))
        # The top-k k stays readable from the query id.
        self.assertTrue(all(q["query_id"].startswith("topk32_")
                            for q in qs if q["capability"] == "topk"))


if __name__ == "__main__":
    unittest.main()
