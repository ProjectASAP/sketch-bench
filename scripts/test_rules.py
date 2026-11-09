#!/usr/bin/env python3

import unittest

import ddos_rules
import rate_limit_rules
import rules
import scaling_rules
from bounds import Bounded
import video_rules
import video_spec
from approx_rows import row


def results(query, entity, arm, values, start=0):
    return [row(start + i, "query_result", "r1",
                {"use_case": "uc", "signal": "s", "query": query, "arm": arm, "host": entity}, "value", v, "u")
            for i, v in enumerate(values) if v is not None]


class BandTest(unittest.TestCase):
    def test_above_band_needs_full_history(self):
        values = [1.0, 1.0, 1.0, 1.0, 5.0]
        self.assertEqual(rules.above_band(values, 4, 3), [False, False, False, False, True])
        self.assertEqual(rules.above_band(values, 5, 3), [False] * 5)

    def test_outside_band_is_two_sided(self):
        values = [2.0, 4.0, 2.0, 4.0, 0.0, 6.0, 3.0]
        self.assertEqual(rules.outside_band(values, 4, 1.5)[4:], [True, True, False])

    def test_missing_values_neither_fire_nor_count(self):
        self.assertEqual(rules.above_band([1.0, None, 1.0, 9.0], 3, 3, 2), [False, False, False, True])

    def test_decayed_band_waits_for_minimum(self):
        flags = rules.above_decayed_band([1.0, 1.0, 1.0, 10.0, 1.0], 0.5, 3, 3)
        self.assertEqual(flags, [False, False, False, True, False])


class ChangeTest(unittest.TestCase):
    def test_above_mean_times(self):
        self.assertEqual(rules.above_mean_times([1.0, 1.0, 2.5, 1.5], 2, 2.0), [False, False, True, False])

    def test_cusum_fires_on_a_shift_and_falls_back(self):
        values = [1.0, 2.0] * 5 + [6.0] * 4 + [1.5] * 12
        flags = rules.cusum_above(values, 10, 0.5, 5)
        self.assertEqual(flags[:11], [False] * 10 + [True])
        self.assertFalse(flags[-1])

    def test_cusum_is_unsure_when_the_interval_straddles(self):
        values = [1.0, 2.0] * 5 + [Bounded(4.0, 1.0, 7.0)]
        self.assertEqual(rules.cusum_above(values, 10, 0.5, 5)[-1], None)


class HoldTest(unittest.TestCase):
    def test_for_consecutive(self):
        self.assertEqual(rules.for_consecutive(2)([True, False, True, True, True]),
                         [False, False, False, True, True])

    def test_x_of_last_y(self):
        self.assertEqual(rules.x_of_last_y(2, 3)([True, False, True, False, False]),
                         [False, False, True, False, False])

    def test_latch_extends_or_suppresses(self):
        flags = [True, False, True, False, False, False]
        self.assertEqual(rules.latch(2)(flags), [True, True, True, True, False, False])
        self.assertEqual(rules.latch(2, extend=False)(flags), [True, True, True, True, False, False])
        self.assertEqual(rules.latch(3, extend=False)(flags), [True, True, True, False, False, False])

    def test_latch_while_held(self):
        self.assertEqual(rules.latch(1, while_held=True)([True, True, True, False]), [True, True, True, False])


class DeriveTest(unittest.TestCase):
    def test_entities_compared_between_arms(self):
        rows = (results("q", "a", "exact", [1, 5, 5, 1]) + results("q", "a", "approx", [1, 1, 5, 1])
                + results("q", "b", "exact", [1, 1, 1, 1]) + results("q", "b", "approx", [1, 5, 1, 1]))
        rule = {"queries": ("q",), "condition": lambda v: rules.above(v["q"], 3), "entity": lambda l: l["host"]}
        out = rules.derive(rows, {"over": rule})
        summary = {r["metric"]: r["value"] for r in out
                   if r["kind"] == "fidelity_summary" and "entity" not in r["labels"]}
        self.assertEqual((summary["true_positive"], summary["false_positive"], summary["false_negative"],
                          summary["true_negative"]), (1, 1, 1, 5))
        self.assertEqual((summary["entities"], summary["entries"], summary["missed_entries"]), (2, 1, 0))
        self.assertEqual(summary["mean_onset_delay"], 1)
        states = [(r["labels"]["entity"], r["time"]) for r in out if r["kind"] == "app_state"
                  and r["metric"] == "state" and r["labels"]["arm"] == "approx" and r["value"] == rules.ACTIVE]
        self.assertEqual(sorted(states), [("a", 2), ("b", 1)])

    def test_entity_only_in_one_arm_counts_as_idle_in_the_other(self):
        rows = results("q", "a", "exact", [1, 1]) + results("q", "a", "approx", [1, 1]) + results(
            "q", "z", "approx", [9, 9])
        rule = {"queries": ("q",), "condition": lambda v: rules.above(v["q"], 3), "entity": lambda l: l["host"]}
        summary = {r["metric"]: r["value"] for r in rules.derive(rows, {"over": rule})
                   if r["kind"] == "fidelity_summary" and "entity" not in r["labels"]}
        self.assertEqual((summary["false_positive"], summary["true_negative"]), (2, 2))


class DdosTest(unittest.TestCase):
    def test_for_15s_fires_at_the_fourth_evaluation(self):
        rule = ddos_rules.RULES["promsketch_for_15s"]
        v = {"entropy_10s": [9.0] * 5, "distinct_10s": [0] * 5, "distinct_5s": [0] * 5}
        self.assertEqual(rule["hold"](rule["condition"](v)), [False, False, False, True, True])

    def test_fastnetmon_bans_until_the_attack_ends(self):
        rule = ddos_rules.RULES["fastnetmon"]
        packets = [200000] * 400 + [5000] * 10
        held = rule["hold"](rule["condition"]({"packets_5s": packets}))
        self.assertEqual(held[:400], [True] * 400)
        self.assertEqual(held[-10:], [False] * 10)

    def test_any_threshold(self):
        v = {"entropy_10s": [9.0, 1.0, 1.0], "distinct_10s": [1, 3001, 1], "distinct_5s": [1, 1, 1]}
        self.assertEqual(ddos_rules.any_threshold(v), [True, True, False])


class RateLimitTest(unittest.TestCase):
    def test_a_client_outside_the_top_list_is_not_over(self):
        self.assertEqual(rate_limit_rules.aws_waf({"top10_clients": [2500, None, 1500]}), [True, False, False])

    def test_cloud_armor_ban_outlasts_the_burst(self):
        rule = rate_limit_rules.RULES["cloud_armor"]
        flags = rule["hold"](rule["condition"]({"top10_clients_1m": [600] + [None] * 11}))
        self.assertEqual(flags, [True] * 10 + [False] * 2)


class ScalingTest(unittest.TestCase):
    def test_vpa_resizes_outside_the_bounds_or_on_a_memory_peak(self):
        v = {"p50_cpu_1h": [0.5, 0.6, 0.1, 0.5], "p95_cpu_1h": [0.6, 0.7, 0.2, 0.6],
             "max_memory_1h": [1e9, 1e9, 1e9, 1.5e9]}
        self.assertEqual(scaling_rules.vpa(v), [False, True, True, True])

    def test_azure_advisor_needs_both_cpu_and_memory_small(self):
        v = {"p95_cpu_1h": [0.15, 0.15, 0.3], "p99_memory_1h": [5e8, 7e8, 5e8]}
        self.assertEqual(scaling_rules.azure_advisor(v), [True, False, False])


class VideoTest(unittest.TestCase):
    def test_every_rule_query_is_in_the_workload(self):
        ids = {q["id"] for q in video_spec.queries()}
        for name, rule in video_rules.RULES.items():
            self.assertLessEqual(set(rule["queries"]), ids, name)

    def test_incidents_hit_some_subpopulations(self):
        spec = video_spec.spec(10)
        hit = {g["labels"]["city"] for m in spec["metrics"] for g in m["series"] if g.get("phases")}
        self.assertEqual(hit, {"city-0", "city-1", "city-2", "city-3", "city-4", "city-5"})
        sessions = sum(60000 / g["sample_interval_ms"] for g in spec["metrics"][0]["series"])
        self.assertAlmostEqual(sessions / video_spec.SESSIONS_PER_MINUTE, 1, delta=0.05)

    def test_mux_needs_viewers(self):
        flags = video_rules.mux({"rebuffer_ratio": [0.1, 0.1, 0.01], "startup_p95": [1.0, 1.0, 5.0],
                                 "sessions": [50, 150, 150]})
        self.assertEqual(flags, [False, True, True])


if __name__ == "__main__":
    unittest.main()
