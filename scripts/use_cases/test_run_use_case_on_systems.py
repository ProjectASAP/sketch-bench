#!/usr/bin/env python3

import unittest

import run_use_case
from run_use_case_on_systems import (backend_arms, cpu_seconds, describe_backend_plan, latency, merged_exact,
                                     truth_errors, workload)

SPEC = {
    "metrics": [
        {"name": "cpu", "sample_interval_ms": 500, "non_negative": True,
         "series": [{"count": 3, "labels": {}, "baseline": {}}]},
        {"name": "requests", "sample_interval_ms": 2, "planned_series_count": 100, "label_from_value": "client_id",
         "series": [{"labels": {}, "baseline": {}}]},
    ],
}
QUERY = {"id": "p95", "promql": "quantile_over_time(0.95, cpu[5m])", "interval_ms": 60000,
         "accuracy_metric": "rank"}


class WorkloadTest(unittest.TestCase):
    def test_sources_and_rate_follow_the_in_process_planner_workload(self):
        w = workload(SPEC, [QUERY], 0.05, 0.01)
        self.assertEqual(w["data_workload"]["sources"]["cpu"],
                         {"series_count": 3, "sample_interval": 500, "values_non_negative": True})
        self.assertEqual(w["data_workload"]["sources"]["requests"], {"series_count": 100, "sample_interval": 200})
        self.assertAlmostEqual(w["data_workload"]["arrival"]["ingesting"]["rows_per_second"], 6.0 + 500.0)
        query = w["query_workload"]["repeating_queries"][0]
        self.assertEqual(query["requirements"],
                         {"accuracy": {"max_error": 0.05, "failure_probability": 0.01, "accuracy_metric": "rank"}})
        self.assertEqual(query["demand"], {"fixed_interval": 60000})

    def test_no_target_is_the_exact_requirement(self):
        self.assertEqual(workload(SPEC, [QUERY], None, 0.01)["query_workload"]["repeating_queries"][0]
                         ["requirements"], {})


class ArmsTest(unittest.TestCase):
    def test_only_arms_that_keep_no_raw_samples(self):
        self.assertEqual(backend_arms(run_use_case.USE_CASES["2"]),
                         ["noraw-eps0.1", "noraw-eps0.05", "noraw-eps0.01", "noraw-eps0.005"])


class CpuSecondsTest(unittest.TestCase):
    def test_ps_time_formats(self):
        self.assertAlmostEqual(cpu_seconds("0:01.25"), 1.25)
        self.assertAlmostEqual(cpu_seconds("2:03.00"), 123.0)
        self.assertAlmostEqual(cpu_seconds("1:00:00"), 3600.0)


def answers(*values):
    return [{"time": 1000 * i, "series": {}, "value": v, "exact_plan": {"relative_error": e}}
            for i, (v, e) in enumerate(values)]


class ExactTest(unittest.TestCase):
    def test_truth_stands_in_for_a_query_victoriametrics_does_not_support(self):
        victoriametrics = {"use_case": "u", "queries": [
            {"id": "a", "answers": answers((1.0, 0.0)), "costs": [{"time": 0, "query_seconds": 0.1}]},
            {"id": "b", "unsupported": "unsupported function", "costs": []}]}
        truth = {"queries": [{"id": "a", "answers": answers((1.0, 0.0))},
                             {"id": "b", "answers": answers((2.0, 0.0)), "costs": [{"refresh_seconds": 1}]}]}
        exact, sources = merged_exact(victoriametrics, truth)
        self.assertEqual([q["id"] for q in exact["queries"]], ["a", "b"])
        self.assertEqual(exact["queries"][1]["answers"][0]["value"], 2.0)
        self.assertEqual(exact["queries"][1]["costs"], [])
        self.assertEqual(sources["a"], "VictoriaMetrics")
        self.assertIn("does not support", sources["b"])

    def test_victoriametrics_scored_against_truth(self):
        out = truth_errors({"queries": [{"id": "a", "answers": answers((1.0, 0.0), (1.1, 0.1))},
                                        {"id": "b", "unsupported": "x"}]})
        self.assertEqual(out, {"a": {"max relative error": 0.1, "answers differing": 1, "answers": 2}})


class LatencyTest(unittest.TestCase):
    def test_unsupported_queries_are_left_out(self):
        document = {"queries": [{"id": "a", "costs": [{"query_seconds": s} for s in (0.001, 0.002, 0.003)]},
                                {"id": "b", "unsupported": "x", "costs": [{"query_seconds": 9.0}]}]}
        out = latency(document)
        self.assertAlmostEqual(out["mean query ms"], 2.0)
        self.assertAlmostEqual(out["median query ms"], 2.0)
        self.assertEqual(out["evaluations"], 3)

    def test_failed_evaluations_are_left_out(self):
        document = {"queries": [{"id": "a", "costs": [{"query_seconds": 0.001, "series_returned": 2},
                                                      {"query_seconds": 5.0, "series_returned": None}]}]}
        self.assertEqual(latency(document)["evaluations"], 1)


class PlanTest(unittest.TestCase):
    def test_sketches_and_stored_outputs_below_the_query_root(self):
        kll = {"algorithm": "Kll", "params": {"k": 200}}
        plan = {"dag": {"dag": {
            "nodes": [{"id": 0, "payload": {}}, {"id": 1, "payload": {"Sketch": [kll, "x"]}},
                      {"id": 2, "payload": {"Sketch": [{"algorithm": "Hll", "params": {"p": 12}}, "y"]}},
                      {"id": 3, "payload": {}}],
            "edges": [{"producer": 0, "consumer": 1}, {"producer": 1, "consumer": 3}, {"producer": 0, "consumer": 2}]}},
            "queries": [{"query": "a", "root_node_id": 3}, {"query": "b", "root_node_id": 2}],
            "stored_outputs": [{"node_id": 1}, {"node_id": 2}]}
        self.assertEqual(describe_backend_plan(plan, "a"), 'Kll {"k":200} (1 stored outputs)')
        self.assertEqual(describe_backend_plan(plan, "b"), 'Hll {"p":12} (1 stored outputs)')


if __name__ == "__main__":
    unittest.main()
