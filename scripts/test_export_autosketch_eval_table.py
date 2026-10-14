#!/usr/bin/env python3
"""Unit tests for export_autosketch_eval_table on tiny synthetic CSVs.

Run: python3 scripts/test_export_autosketch_eval_table.py
"""

import csv
import json
import math
import os
import sys
import tempfile
import unittest
from unittest import mock

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

from study_saturation import COST_KEYS, COST_PARETO_ALPHA, COST_THETA  # noqa: E402
from export_autosketch_eval_table import (  # noqa: E402
    SCHEMA_TEMPLATES, SCHEMAS, SYNTHETIC_DEFAULT, SYNTHETIC_GRID, SYNTHETIC_TEMPLATE_SETS,
    all_subsets, boom_query, e2e_block, group_coverage, grouping_cardinality, label_shares, main,
    metric_queries, queries_per_instance,
    shared_queries, synthetic_entry, synthetic_plan, synthetic_queries)
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
        top = rqes["topk32_sum_by_label0_sum_over_time/15m"]["families"][0]
        self.assertEqual((top["grid_param"], top["grid_K"]), (COST_THETA, COST_KEYS))
        kll, dd = rqes["quantile_over_time_p0.5/15m"]["families"]
        self.assertEqual((kll["sketch"], kll["grid_param"], kll["grid_K"]),
                         ("kll-percall", COST_PARETO_ALPHA, None))
        self.assertEqual(dd["sketch"], "dd")
        # Every table carries the e2e block; `all` has classic and schema metrics.
        with open(os.path.join(out, default)) as f:
            self.assertEqual({m: v["type"] for m, v in
                              json.load(f)["workloads"][0]["e2e"]["metrics"].items()},
                             {"data": "gauge", "data_total": "counter",
                              "http": "records", "flows": "records"})
        with open(os.path.join(out, "synthetic-templatesclassic-shared1.json")) as f:
            metrics = json.load(f)["workloads"][0]["e2e"]["metrics"]
        self.assertEqual({m: metrics[m]["type"] for m in metrics},
                         {"data": "gauge", "data_total": "counter"})
        self.assertEqual(grouping_cardinality_of(metrics["data"]["labels"]), 10_000)


def grouping_cardinality_of(labels):
    """Series of a label list in SCHEMAS' format: a `child_of` label's
    cardinality is per parent value, so the full set is the product."""
    return math.prod(label["cardinality"] for label in labels)


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
    def test_classic_templates_give_42_distinct_rqes(self):
        qs = synthetic_queries(templates="classic")
        # 1 + 5 spatial; per window 1 + 1 + 5 + 1 + 1 temporal (templates 2,
        # 4, 5, 6, 8), and template 10's operands repeat template 5's.
        # Templates 7 and 9 (rates by job, top-k by rate) are dropped.
        self.assertEqual(len(qs), 6 + 9 * 4)
        streams = {(q["stream"], q["capability"], q["groups"]) for q in qs}
        self.assertEqual(streams, {
            ("job/value", "sum", 10), ("job/dist", "quantile", 10),
            ("series/value", "sum", 10_000), ("series/increment", "rate", 10_000),
            ("series/dist", "quantile", 10_000), ("label_0/value", "topk", 1)})
        self.assertFalse({"sum_by_job_rate", "topk32_sum_by_label0_rate"}
                         & {q["query_id"] for q in qs})

    def test_items_per_instance_follow_the_data_model(self):
        qs = {(q["query_id"], q["range"]): q for q in synthetic_queries(templates="classic")}
        # 1e4 series x 200 samples/s = 2e6 samples/s; an instant query reads
        # one scrape, one sample from each of a job's 1e3 series.
        q = qs[("quantile_by_job_p0.5", "1s")]
        self.assertEqual((q["range_s"], q["max_N"]), (0.005, 1e3))
        self.assertEqual(qs[("quantile_over_time_p0.5", "15m")]["max_N"], 200 * 900)
        top = qs[("topk32_sum_by_label0_sum_over_time", "24h")]
        self.assertEqual((top["K"], top["groups"], top["max_N"]), (10_000, 1, 2e6 * 86400))

    def test_every_template_issues_one_query_per_instance(self):
        self.assertEqual({queries_per_instance(q) for q in synthetic_queries(templates="all")},
                         {1})

    def test_classic_promql_reads_every_rqe_and_leaves_the_rqes_unchanged(self):
        e2e = []
        qs = synthetic_queries(templates="classic", e2e=e2e)
        self.assertEqual(qs, synthetic_queries(templates="classic"))
        ids = {synthetic_entry(q)["id"] for q in qs}
        read = {rqe for query in e2e for rqe in query["rqes"]}
        self.assertEqual(read, ids)
        # 6 spatial + 10 per window: template 5's five quantiles, 4 others
        # and template 10's ratio, which reads two RQEs.
        self.assertEqual(len(e2e), 6 + 10 * 4)
        self.assertFalse([q for q in e2e if "sum by (job) (rate(" in q["promql"]
                          or "(rate(" in q["promql"] and "topk" in q["promql"]])
        by_text = {query["promql"]: query for query in e2e}
        ratio = by_text["quantile_over_time(0.9, data[1h]) / quantile_over_time(0.5, data[1h])"]
        self.assertEqual([r.split("/")[-3] for r in ratio["rqes"]],
                         ["quantile_over_time_p0.9", "quantile_over_time_p0.5"])
        self.assertEqual(by_text["rate(data_total[15m])"]["interval_secs"], 60)
        # Template 2 ranks series: label_0 names one, so no `sum by (label_0)`.
        top = by_text["topk(32, sum_over_time(data[15m]))"]
        self.assertEqual([r.split("/")[-3] for r in top["rqes"]],
                         ["topk32_sum_by_label0_sum_over_time"])
        self.assertEqual(by_text["sum by (job) (data)"]["interval_secs"], 1)

    def test_schema_queries_read_every_rqe_with_their_fields(self):
        e2e = []
        qs = synthetic_queries(templates="multigroup", dataset="d", e2e=e2e)
        self.assertEqual(qs, synthetic_queries(templates="multigroup", dataset="d"))
        self.assertEqual([r for query in e2e for r in query["rqes"]],
                         [synthetic_entry(q)["id"] for q in qs])
        self.assertTrue(all("promql" not in query for query in e2e))
        # Template 16, p99 latency GROUP BY CUBE(region, service, endpoint):
        # 7 groupings per window.
        cube = [query for query in e2e if query["rqes"][0].startswith("d/t16_")]
        self.assertEqual(len(cube), 2 * 7)
        self.assertEqual(sorted(tuple(q["grouping"]) for q in cube if q["window_secs"] == 300),
                         sorted(tuple(g) for g in all_subsets(
                             "http", ["region", "service", "endpoint"])))
        self.assertEqual(cube[0], {
            "table": "http", "value": "latency", "capability": "quantile",
            "grouping": ["region"], "quantile": 0.99, "window_secs": 300,
            "interval_secs": 60, "rqes": ["d/t16_p99_latency_by_region/values/5m"],
            "sql": "SELECT region, QUANTILE(0.99, latency) FROM http WHERE ts BETWEEN "
                   "DATEADD(s, -300, NOW()) AND NOW() GROUP BY region",
            "expr": {"rqe": {"id": "d/t16_p99_latency_by_region/values/5m", "metric": "http",
                             "capability": "quantile", "lookback_ms": 300_000,
                             "grouping": ["region"], "phi": 0.99, "value": "latency"}}})
        distinct = next(q for q in e2e if q["capability"] == "cardinality")
        self.assertNotIn("quantile", distinct)
        self.assertEqual(distinct["expr"], {"rqe": {
            "id": distinct["rqes"][0], "metric": "flows", "capability": "cardinality",
            "lookback_ms": 300_000, "grouping": ["dst_subnet"], "value": "src_ip"}})
        self.assertEqual(distinct["sql"],
                         "SELECT dst_subnet, COUNT(DISTINCT src_ip) FROM flows WHERE ts "
                         "BETWEEN DATEADD(s, -300, NOW()) AND NOW() GROUP BY dst_subnet")

    def test_classic_queries_carry_their_rqe_expression_trees(self):
        e2e = []
        synthetic_queries(templates="classic", dataset="d", e2e=e2e)
        by_text = {query["promql"]: query for query in e2e}

        def leaf(query_id, capability, lookback_ms, grouping, **extra):
            return {"rqe": {"id": f"d/{query_id}", "metric": "data", "capability": capability,
                            "lookback_ms": lookback_ms, "grouping": grouping, **extra}}

        series = ["job", "label_0"]
        # Instant RQEs read one scrape: 5 ms at 200 samples/s.
        self.assertEqual(by_text["sum by (job) (data)"]["expr"],
                         leaf("sum_by_job/keys/1s", "sum", 5, ["job"]))
        self.assertEqual(by_text["quantile by (job) (0.75, data)"]["expr"],
                         leaf("quantile_by_job_p0.75/values/1s", "quantile", 5, ["job"],
                              phi=0.75))
        self.assertEqual(by_text["topk(32, sum_over_time(data[15m]))"]["expr"],
                         leaf("topk32_sum_by_label0_sum_over_time/keys/15m", "topk_by_value",
                              900_000, series, k=32))
        self.assertEqual(by_text["sum_over_time(data[1h])"]["expr"],
                         leaf("sum_over_time/keys/1h", "sum", 3_600_000, series))
        self.assertEqual(by_text["quantile_over_time(0.99, data[6h])"]["expr"],
                         leaf("quantile_over_time_p0.99/values/6h", "quantile", 21_600_000,
                              series, phi=0.99))
        self.assertEqual(by_text["sum by (job) (sum_over_time(data[24h]))"]["expr"],
                         leaf("sum_by_job_sum_over_time/keys/24h", "sum", 86_400_000, ["job"]))
        ratio = by_text["quantile_over_time(0.9, data[1h]) / quantile_over_time(0.5, data[1h])"]
        self.assertEqual(ratio["expr"], {
            "op": "/",
            "lhs": leaf("quantile_over_time_p0.9/values/1h", "quantile", 3_600_000, series,
                        phi=0.9),
            "rhs": leaf("quantile_over_time_p0.5/values/1h", "quantile", 3_600_000, series,
                        phi=0.5)})
        # A rate leaf has no capability on the RQE route.
        self.assertNotIn("expr", by_text["rate(data_total[15m])"])
        self.assertEqual([q["promql"] for q in e2e if "expr" not in q],
                         [f"rate(data_total[{w}])" for w in ["15m", "1h", "6h", "24h"]])

        def leaves(node):
            return [node["rqe"]] if "rqe" in node else leaves(node["lhs"]) + leaves(node["rhs"])

        # Every leaf names the query's RQEs, in order.
        for query in e2e:
            if "expr" in query:
                self.assertEqual([x["id"] for x in leaves(query["expr"])], query["rqes"])

    def test_metric_copies_name_their_metric_in_each_leaf(self):
        e2e = []
        metric_queries({**SYNTHETIC_DEFAULT, "templates": "all", "metrics": 2}, e2e)
        leaf = next(q for q in e2e if q.get("promql") == "sum_over_time(data_1[1h])")["expr"]
        self.assertEqual(leaf["rqe"]["metric"], "data_1")
        self.assertTrue(leaf["rqe"]["id"].endswith("/sum_over_time_data_1/keys/1h"))
        sql = next(q for q in e2e if q.get("table") == "data_1_http")["expr"]
        self.assertEqual(sql["rqe"]["metric"], "data_1_http")

    @mock.patch("export_autosketch_eval_table.KEEP_DUPLICATE_QUERIES", True)
    def test_schema_metrics_are_records_tables_named_by_metric(self):
        e2e = []
        qs = metric_queries({**SYNTHETIC_DEFAULT, "templates": "multigroup", "metrics": 2},
                            e2e)
        block = e2e_block(qs, e2e)
        self.assertEqual(sorted(block["metrics"]),
                         ["data_0_flows", "data_0_http", "data_1_flows", "data_1_http"])
        http = block["metrics"]["data_1_http"]
        self.assertEqual(http, {"type": "records", "labels": SCHEMAS["http"]["labels"],
                                "records_per_sec": SCHEMAS["http"]["rate"],
                                "values": SCHEMAS["http"]["values"]})
        self.assertEqual({q["table"] for q in block["queries"]}, set(block["metrics"]))
        # Each distinct query appears once and reads every RQE asking it:
        # templates 14, 15 and 17 share some groupings.
        read = [r for q in block["queries"] for r in q["rqes"]]
        self.assertEqual(sorted(read), sorted(synthetic_entry(q)["id"] for q in qs))
        fields = [json.dumps({k: v for k, v in q.items() if k not in ("rqes", "expr")},
                             sort_keys=True)
                  for q in block["queries"]]
        self.assertEqual(len(fields), len(set(fields)))
        # A query reading several RQEs has no single leaf.
        self.assertTrue(all(("expr" in q) == (len(q["rqes"]) == 1) for q in block["queries"]))
        self.assertTrue(all(q["rqes"] == sorted(q["rqes"]) for q in block["queries"]))
        by_service = [q for q in block["queries"] if q["table"] == "data_0_http"
                      and q["capability"] == "cardinality" and q["grouping"] == ["service"]
                      and q["window_secs"] == 300]
        self.assertEqual([[r.split("/")[-3] for r in q["rqes"]] for q in by_service],
                         [["t14_distinct_users_by_service_data_0",
                           "t15_distinct_users_by_service_data_0"]])

    def test_each_query_is_only_its_strictest_templates_rqe(self):
        # One query string can be served by one aggregation, so a query
        # templates 14, 15 and 17 share keeps the template covering the most
        # groups: 17 (every group), then 14 (1%), then 15 (5%).
        e2e = []
        synthetic_queries(templates="multigroup", dataset="d", e2e=e2e)
        block = e2e_block([], e2e)
        self.assertTrue(all(len(q["rqes"]) == 1 for q in block["queries"]))
        self.assertTrue(all(q["expr"]["rqe"]["id"] == q["rqes"][0] for q in block["queries"]))
        users = {(tuple(q["grouping"]), q["window_secs"]): q["rqes"][0].split("/")[1]
                 for q in block["queries"] if q["value"] == "user_id"}
        for window in [300, 900]:
            self.assertTrue(users[(("service", "endpoint"), window)].startswith("t17_"))
            for grouping in [("region",), ("service",), ("region", "service")]:
                self.assertTrue(users[(grouping, window)].startswith("t14_"))
            self.assertTrue(users[(("endpoint",), window)].startswith("t15_"))

    def test_a_strictest_copy_below_the_curves_fails_loudly(self):
        # Every group of the full label set: the smallest has ~350 items in 5m
        # (left out) and ~1060 in 15m.
        grouping = [all_subsets("http")[-1]]
        every = (17, "distinct_users", "http", "user_id", "cardinality", grouping, None)
        some = (15, "distinct_users", "http", "user_id", "cardinality", grouping, 0.05)
        with mock.patch("export_autosketch_eval_table.SKIP_BELOW_CURVES", True), \
                mock.patch("sys.stderr"):
            with mock.patch("export_autosketch_eval_table.SCHEMA_TEMPLATES", [every]):
                self.assertEqual([q["range"] for q in synthetic_queries(templates="multigroup")],
                                 ["15m"])
            # A weaker copy would fit, but would weaken the workload's target.
            with mock.patch("export_autosketch_eval_table.SCHEMA_TEMPLATES", [some, every]):
                with self.assertRaisesRegex(ValueError, "strictest of 2 templates"):
                    synthetic_queries(templates="multigroup")


COMMITTED_TABLES = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..",
                                "rqe-optimizer", "results", "autosketch-vs-asap-inputs",
                                "tables")


# Classic templates 7 and 9, dropped since the committed tables.
DROPPED = ("sum_by_job_rate", "topk32_sum_by_label0_rate")


class SchemaTemplatesTest(unittest.TestCase):
    def test_classic_tables_are_main_s_all_tables_byte_for_byte(self):
        # The committed tables are the 10-template set's, from before "all"
        # grew: renamed, today's "classic" tables match them byte for byte
        # without templates 7 and 9's RQEs and with instant RQEs reading one
        # scrape, apart from the `e2e` block. Of that block the runner reads
        # only each RQE metric's samples_per_sec_per_series, its scrape rate.
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
                rqes = committed["workloads"][0]["rqes"]
                rqes[:] = [r for r in rqes if not r["query_id"].startswith(DROPPED)]
                for r in rqes:  # instant RQEs now read one scrape (5 ms)
                    if r["range"] == "1s":
                        r.update(lookback_secs=0.005, min_N=1e3, max_N=1e3)
                del ours["sketch_bench_revision"], committed["sketch_bench_revision"]
                workload = ours["workloads"][0]
                metrics = workload.pop("e2e")["metrics"]
                for metric in {r["metric"] for r in workload["rqes"]}:
                    self.assertEqual(metrics[metric]["samples_per_sec_per_series"], 200, name)
                self.assertEqual(json.dumps(ours, sort_keys=True),
                                 json.dumps(committed, sort_keys=True), name)

    def test_all_is_classic_plus_the_schema_templates(self):
        classic = synthetic_queries(templates="classic", dataset="d")
        every = synthetic_queries(templates="all", dataset="d")
        self.assertEqual(every[:len(classic)], classic)
        new = every[len(classic):]
        # Per window (5m, 15m): 1 + 3 + 4 + 15 + 7 + 1 groupings, of which 5
        # repeat another template's query.
        self.assertEqual(len(new), 2 * 26)
        with mock.patch("export_autosketch_eval_table.KEEP_DUPLICATE_QUERIES", True):
            self.assertEqual(len(synthetic_queries(templates="multigroup", dataset="d")),
                             2 * 31)
        self.assertEqual({q["stream"] for q in new},
                         {"flows/src_ip", "http/user_id", "http/latency"})
        self.assertEqual({q["range"] for q in new}, {"5m", "15m"})
        self.assertTrue(all("grouping" not in q for q in classic))
        # "multigroup" is the schema templates alone.
        self.assertEqual(synthetic_queries(templates="multigroup", dataset="d"), new)

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
        self.assertEqual(groupings[17], [["service", "endpoint"]])
        # Template 16 is CUBE(region, service, endpoint).
        self.assertEqual(len(groupings[16]), 7)

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
        self.assertIsNone(rqes["t17_distinct_users_by_service,endpoint/15m"]
                          ["covers_share"])
        kll, dd = rqes["t16_p99_latency_by_region,service,endpoint/5m"]["families"]
        self.assertEqual((kll["grid_param"], kll["grid_K"]), (COST_PARETO_ALPHA, None))
        self.assertEqual(rqes["t11_distinct_src_by_dst_subnet/5m"]["covers_share"], 0.05)
        self.assertEqual((r["covered_groups"], r["min_covered_share"], r["max_covered_share"],
                          r["covered_min_N"], r["covered_max_N"]),
                         group_coverage("http", ["region", "service"], 0.01)
                         + (2e6 * 300 * r["min_covered_share"],
                            2e6 * 300 * r["max_covered_share"]))

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
                             (len(covered), min(covered), max(shares)))
        # No group holds 50%: the largest alone is covered.
        self.assertEqual(group_coverage("http", ["region", "status"], 0.5), (1, max(shares), max(shares)))
        self.assertEqual(group_coverage("http", ["region", "status"], None),
                         (16, min(shares), max(shares)))
        # The DDoS subnet is one of template 11's covered groups.
        n, smallest, _ = group_coverage("flows", ["dst_subnet"], 0.05)
        self.assertGreater(n, 1)
        self.assertAlmostEqual(smallest, label_shares("flows", SCHEMAS["flows"]["labels"][0])[999])

    def test_every_covered_rqe_names_its_covered_groups_and_their_items(self):
        for q in synthetic_queries(templates="all"):
            if "grouping" in q:
                self.assertGreaterEqual(q["covered_groups"], 1)
                self.assertEqual(q["covered_min_N"],
                                 q["arrival_rate"] * q["range_s"] * q["min_covered_share"])
                self.assertGreaterEqual(q["covered_min_N"], 1000)
                self.assertEqual(q["covered_max_N"],
                                 q["arrival_rate"] * q["range_s"] * q["max_covered_share"])
                self.assertGreaterEqual(q["max_covered_share"], q["min_covered_share"])

    def test_a_covered_group_below_the_curves_first_n_fails_loudly(self):
        # Every group of the full label set: the smallest has ~350 items in 5m.
        full = (17, "distinct_users", "http", "user_id", "cardinality",
                [all_subsets("http")[-1]], None)
        with mock.patch("export_autosketch_eval_table.SCHEMA_TEMPLATES", [full]):
            with self.assertRaises(AssertionError):
                synthetic_queries(templates="all")

    def test_metric_copies_name_their_schema_metrics(self):
        qs = metric_queries({**SYNTHETIC_DEFAULT, "metrics": 2})
        ids = [(q["query_id"], q["range"]) for q in qs]
        self.assertEqual(len(ids), len(set(ids)))
        self.assertEqual({q["metric"] for q in qs if "grouping" in q},
                         {"data_0/http", "data_0/flows", "data_1/http", "data_1/flows"})


    def test_samples_per_sec_scales_rates_and_leaves_out_rqes_below_the_curves(self):
        with tempfile.TemporaryDirectory() as out, mock.patch("sys.stderr"):
            main(["--synthetic", "--samples-per-sec", "10", "--revision", "test",
                  "--out", out])
            with open(os.path.join(out, "synthetic-templatesmultigroup-shared1.json")) as f:
                workload = json.load(f)["workloads"][0]
        # At 10 samples/s the smallest (region, endpoint) group holds 403
        # items in 5m, below the curves' first N: those RQEs and their
        # queries are left out, the 15m ones kept.
        left_out = {f"t16_p99_latency_by_{g}/values/5m"
                    for g in ["region,endpoint", "region,service,endpoint"]}
        ids = {r["id"].split("/", 3)[3] for r in workload["rqes"]}
        e2e_ids = {r.split("/", 3)[3] for q in workload["e2e"]["queries"] for r in q["rqes"]}
        # 62 RQEs, less 10 duplicate queries and the 2 left out.
        self.assertEqual(len(ids), 50)
        self.assertFalse(left_out & (ids | e2e_ids))
        self.assertIn("t16_p99_latency_by_region,endpoint/values/15m", ids)
        self.assertEqual(workload["e2e"]["metrics"]["http"]["records_per_sec"], 100_000)
        self.assertEqual(workload["e2e"]["metrics"]["flows"]["records_per_sec"], 200_000)
        # The default rate is restored for the rest of the process.
        self.assertEqual(SCHEMAS["http"]["rate"], 2_000_000)

    def test_samples_per_sec_must_divide_1000(self):
        # An instant RQE's one-scrape lookback, 1000 / rate ms, must be whole.
        for rate in ["3", "0", "2000"]:
            with self.assertRaises(SystemExit), mock.patch("sys.stderr"):
                main(["--synthetic", "--samples-per-sec", rate, "--out", "unused"])


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
    def test_dashboard_has_15_distinct_rqes(self):
        qs = synthetic_queries(templates="dashboard")
        count = {}
        for q in qs:
            name = q["query_id"].split("_p")[0]
            count[name] = count.get(name, 0) + 1
        # D1 3 q x 4 windows, D2 3; D5 repeats D1. D3 and D4 (rates by job,
        # top-k by rate) are dropped with classic templates 7 and 9.
        self.assertEqual(count, {"quantile_over_time": 12, "quantile_by_job": 3})
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
        self.assertEqual(len(qs), 8 * 42)
        self.assertEqual(len({(q["query_id"], q["range"]) for q in qs}), len(qs))
        streams = {q["stream"].split("/")[0] for q in qs}
        self.assertEqual(streams, {f"data_{i}" for i in range(8)})
        self.assertTrue(all(q["dataset"].endswith("metrics=8") for q in qs))
        # The top-k k stays readable from the query id.
        self.assertTrue(all(q["query_id"].startswith("topk32_")
                            for q in qs if q["capability"] == "topk"))


if __name__ == "__main__":
    unittest.main()
