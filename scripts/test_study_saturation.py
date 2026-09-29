#!/usr/bin/env python3
"""Unit tests for study_saturation's pure logic, and its cost phase against a
fake approxbench (so no real cost run happens).

Run: python3 scripts/test_study_saturation.py
"""

import csv
import os
import subprocess
import sys
import tempfile
import textwrap
import unittest

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

from study_saturation import checkpoints, n_saturation, n_star, read_curve  # noqa: E402

SCRIPT = os.path.join(os.path.dirname(os.path.abspath(__file__)), "study_saturation.py")

NS = [10, 20, 30, 40, 50, 60]


class NSaturationTest(unittest.TestCase):
    def test_seed_noise_widens_the_band(self):
        # A small plateau with seed noise wider than 10% of it: without the
        # standard-error term this never saturates.
        errors = [0.05, 0.019, 0.013, 0.018, 0.013, 0.017]
        self.assertIsNone(n_saturation(NS, errors, 0.10, 3))
        ses = [0.004, 0.002, 0.002, 0.002, 0.002, 0.002]
        self.assertEqual(n_saturation(NS, errors, 0.10, 3, ses), 20)

    def test_falling_error_saturates_where_it_enters_the_band(self):
        errors = [1.0, 0.5, 0.21, 0.2, 0.2, 0.19]
        self.assertEqual(n_saturation(NS, errors, 0.10, 3), 30)

    def test_a_flat_curve_saturates_at_the_first_checkpoint(self):
        self.assertEqual(n_saturation(NS, [0.3] * 6, 0.10, 3), 10)

    def test_rising_metric_works_the_same(self):
        # precision_at_k: higher is better; only stability matters.
        errors = [0.2, 0.5, 0.9, 0.95, 0.96, 0.97]
        self.assertEqual(n_saturation(NS, errors, 0.10, 3), 30)

    def test_an_excursion_after_the_band_moves_n_sat_past_it(self):
        errors = [0.2, 0.5, 0.2, 0.2, 0.2, 0.2]
        self.assertEqual(n_saturation(NS, errors, 0.10, 3), 30)

    def test_only_the_tail_in_band_is_not_saturated(self):
        errors = [1.0, 0.8, 0.6, 0.2, 0.2, 0.2]
        self.assertIsNone(n_saturation(NS, errors, 0.10, 3))

    def test_a_still_falling_tail_is_not_saturated(self):
        errors = [1.0, 0.5, 0.25, 0.12, 0.06, 0.03]
        self.assertIsNone(n_saturation(NS, errors, 0.10, 3))

    def test_too_few_checkpoints_is_not_saturated(self):
        self.assertIsNone(n_saturation(NS[:2], [0.1, 0.1], 0.10, 3))
        self.assertIsNone(n_saturation(NS[:3], [0.1, 0.1, 0.1], 0.10, 3))

    def test_zero_plateau_uses_the_absolute_fallback(self):
        errors = [0.5, 0.0, 0.0, 0.0, 0.0, 0.0]
        self.assertEqual(n_saturation(NS, errors, 0.10, 3), 20)


class CheckpointsTest(unittest.TestCase):
    def test_four_per_decade_spans_both_ends(self):
        ns = checkpoints(1e3, 1e5, 4)
        self.assertEqual(ns[0], 1000)
        self.assertEqual(ns[-1], 100000)
        self.assertEqual(len(ns), 9)
        self.assertEqual(ns[1], 1778)


class NStarTest(unittest.TestCase):
    def test_first_size_where_exact_reaches_the_factor(self):
        ns = [10, 100, 1000]
        self.assertEqual(n_star(ns, [5, 50, 500], [5, 5, 5], 10), 100)
        self.assertEqual(n_star(ns, [5, 50, 500], [5, 5, 5], 100), 1000)

    def test_equal_counts_as_reached(self):
        self.assertEqual(n_star([10], [50], [5], 10), 10)

    def test_never_reached_is_none(self):
        # Exact and sketch both linear in N: the ratio never grows.
        self.assertIsNone(n_star([10, 100], [20, 200], [10, 100], 10))


def write_csv(path, header, rows):
    with open(path, "w", newline="") as f:
        writer = csv.writer(f)
        writer.writerow(header)
        writer.writerows(rows)


CURVE_HEADER = ["family", "sketch", "config", "dist", "param", "cardinality", "n",
                "seed_mean_error", "seed_se"]


class ReadCurveTest(unittest.TestCase):
    def test_recomputes_n_sat_and_matches_param_as_a_number(self):
        point = ("cardinality", "hll", "lg_k=12", "cardinality", "relative_error", [],
                 "zipf", 1.0, 1000)
        with tempfile.TemporaryDirectory() as d:
            path = os.path.join(d, "curve.csv")
            errors = [1.0, 0.5, 0.21, 0.2, 0.2, 0.19]
            write_csv(path, CURVE_HEADER,
                      [["cardinality", "hll", "lg_k=12", "zipf", "1", 1000, n, e, 0]
                       for n, e in zip(NS, errors)])
            [(p, n_sat, final)] = read_curve(path, [point], NS, 0.10, 3)
        self.assertIs(p, point)
        self.assertEqual(n_sat, 30)
        self.assertEqual(final, 0.19)

    def test_other_sizes_exit(self):
        point = ("cardinality", "hll", "lg_k=12", "cardinality", "relative_error", [],
                 "zipf", 1.0, 1000)
        with tempfile.TemporaryDirectory() as d:
            path = os.path.join(d, "curve.csv")
            write_csv(path, CURVE_HEADER,
                      [["cardinality", "hll", "lg_k=12", "zipf", 1.0, 1000, 10, 0.1, 0]])
            with self.assertRaises(SystemExit):
                read_curve(path, [point], NS, 0.10, 3)


# Stands in for approxbench: the sketch is 1000 bytes at 1 s insert + 0.5 s
# query at the final N; the exact baseline holds 8 bytes per item and spends
# 1e-6 s per item.
FAKE_APPROXBENCH = textwrap.dedent("""\
    import json, sys
    a = sys.argv
    n = int(a[a.index("--size") + 1])
    lib = a[a.index("--library") + 1]
    ops = a[a.index("--operations") + 1]
    cpu = lambda s: {"user_ms": {"mean": s * 1000}, "sys_ms": {"mean": 0}}
    if "accuracy" in a:
        r = {"bench": {"accuracy": {"relative_error": 0.01, "are_top100": 0.01}}}
    elif lib == "lib":
        r = {"memory_bytes": 1000, "insert_cpu_time_ms": cpu(1.0),
             "query_cpu_time_ms": cpu(0.5), "merge_cpu_time_ms": cpu(0.1)}
    elif ops == "prepare":
        r = {"memory_bytes": 8 * n, "prepare_cpu_time_ms": cpu(1e-6 * n)}
    else:
        r = {"memory_bytes": 8 * n, "insert_cpu_time_ms": cpu(0),
             "query_cpu_time_ms": cpu(0)}
    print(json.dumps(r))
""")


def fake_binary(d):
    binary = os.path.join(d, "approxbench")
    with open(binary, "w") as f:
        f.write("#!" + sys.executable + "\n" + FAKE_APPROXBENCH)
    os.chmod(binary, 0o755)
    return binary


class AccuracyPhaseTest(unittest.TestCase):
    def run_study(self, d, points_csv):
        return subprocess.run([
            sys.executable, SCRIPT, "--binary", fake_binary(d), "--out", d,
            "--phase", "accuracy", "--families", "cardinality", "--one-config",
            "--thetas", "0,1.0", "--cardinalities", "1000", "--n-max", "1e6",
            "--per-decade", "1", "--seeds", "2", "--points-from", points_csv,
        ], capture_output=True)

    def test_points_from_selects_points_and_cost_columns_stay_blank(self):
        with tempfile.TemporaryDirectory() as d:
            points = os.path.join(d, "points.csv")
            write_csv(points, ["sketch", "config", "dist", "param", "cardinality"],
                      [["hll", "lg_k=12", "zipf", "1", "1000"]])
            self.assertEqual(self.run_study(d, points).returncode, 0)
            with open(os.path.join(d, "saturation.csv"), newline="") as f:
                [summary] = list(csv.DictReader(f))
            self.assertFalse(os.path.exists(os.path.join(d, "crossover.csv")))
            self.assertFalse(os.path.exists(os.path.join(d, "saturation_cost.jsonl")))
        self.assertEqual((summary["param"], summary["n_sat"]), ("1.0", "1000"))
        self.assertEqual(summary["memory_bytes"], "")

    def test_points_outside_the_grid_exit(self):
        with tempfile.TemporaryDirectory() as d:
            points = os.path.join(d, "points.csv")
            write_csv(points, ["sketch", "config", "dist", "param", "cardinality"],
                      [["hll", "lg_k=12", "zipf", "2.0", "1000"]])
            self.assertNotEqual(self.run_study(d, points).returncode, 0)


class ResumeTest(unittest.TestCase):
    def test_keeps_complete_curves_and_reruns_the_rest(self):
        with tempfile.TemporaryDirectory() as d:
            ns = checkpoints(1e3, 1e6, 1)
            curve = os.path.join(d, "saturation_curve.csv")
            # theta 0 is complete (error 0.5, which the fake would not give);
            # theta 1 was cut off after two sizes.
            write_csv(curve, CURVE_HEADER,
                      [["cardinality", "hll", "lg_k=12", "zipf", 0.0, 1000, n, 0.5, 0]
                       for n in ns]
                      + [["cardinality", "hll", "lg_k=12", "zipf", 1.0, 1000, n, 0.9, 0]
                         for n in ns[:2]])
            subprocess.run([
                sys.executable, SCRIPT, "--binary", fake_binary(d), "--out", d,
                "--phase", "accuracy", "--families", "cardinality", "--one-config",
                "--thetas", "0,1.0", "--cardinalities", "1000", "--n-max", "1e6",
                "--per-decade", "1", "--seeds", "2", "--resume",
            ], check=True, capture_output=True)
            with open(curve, newline="") as f:
                rows = list(csv.DictReader(f))
            with open(os.path.join(d, "saturation.csv"), newline="") as f:
                summary = list(csv.DictReader(f))
            with open(os.path.join(d, "saturation_accuracy.jsonl")) as f:
                raw = f.readlines()
        errors = {}
        for r in rows:
            errors.setdefault(r["param"], []).append(float(r["seed_mean_error"]))
        self.assertEqual(errors, {"0.0": [0.5] * 4, "1.0": [0.01] * 4})
        self.assertEqual([r["final_error"] for r in summary], ["0.5", "0.01"])
        # Only the rerun point was measured: 4 sizes x 2 seeds.
        self.assertEqual(len(raw), 8)


class CostPhaseTest(unittest.TestCase):
    def test_cost_rows_measures_only_that_rows_value(self):
        with tempfile.TemporaryDirectory() as d:
            ns = checkpoints(1e3, 1e6, 1)
            configs = ["rows=3 cols=256", "rows=5 cols=256"]
            write_csv(os.path.join(d, "saturation_curve.csv"), CURVE_HEADER,
                      [["frequency", "cms-fastpath-vector2d", c, "zipf", 1.0, 1000, n, 0.01, 0]
                       for c in configs for n in ns])
            points = os.path.join(d, "points.csv")
            write_csv(points, ["sketch", "config", "dist", "param", "cardinality"],
                      [["cms-fastpath-vector2d", c, "zipf", 1.0, 1000] for c in configs])
            subprocess.run([
                sys.executable, SCRIPT, "--binary", fake_binary(d), "--out", d,
                "--phase", "cost", "--families", "frequency", "--thetas", "1.0",
                "--cardinalities", "1000", "--n-max", "1e6", "--per-decade", "1",
                "--points-from", points, "--cost-rows", "3",
            ], check=True, capture_output=True)
            with open(os.path.join(d, "saturation.csv"), newline="") as f:
                summary = list(csv.DictReader(f))
            with open(os.path.join(d, "crossover.csv"), newline="") as f:
                crossover = list(csv.DictReader(f))
        self.assertEqual([(r["config"], r["memory_bytes"]) for r in summary],
                         [("rows=3 cols=256", "1000"), ("rows=5 cols=256", "")])
        self.assertEqual([r["config"] for r in crossover], ["rows=3 cols=256"])

    def test_cost_phase_writes_crossover_from_an_existing_curve(self):
        with tempfile.TemporaryDirectory() as d:
            binary = fake_binary(d)
            ns = checkpoints(1e3, 1e6, 1)
            write_csv(os.path.join(d, "saturation_curve.csv"), CURVE_HEADER,
                      [["cardinality", "hll", "lg_k=12", "zipf", 1.0, 1000, n, 0.01, 0]
                       for n in ns])
            subprocess.run([
                sys.executable, SCRIPT, "--binary", binary, "--out", d, "--phase", "cost",
                "--families", "cardinality", "--one-config", "--thetas", "1.0",
                "--cardinalities", "1000", "--n-max", "1e6", "--per-decade", "1",
            ], check=True, capture_output=True)
            with open(os.path.join(d, "crossover.csv"), newline="") as f:
                [row] = list(csv.DictReader(f))
            with open(os.path.join(d, "saturation.csv"), newline="") as f:
                [summary] = list(csv.DictReader(f))
        self.assertEqual(summary["n_sat"], "1000")
        self.assertEqual(summary["insert_cpu_secs"], "1.0")
        # 8N >= 10 * 1000 bytes first at N = 1e4; >= 100 * 1000 at N = 1e5.
        self.assertEqual(row["memory_n_star_10x"], "10000")
        self.assertEqual(row["memory_n_star_100x"], "100000")
        # Sketch CPU at N is 1 * N / 1e6 + 0.5; exact is 1e-6 * N, so it never
        # reaches 10x.
        self.assertEqual(row["cpu_n_star_10x"], "not_reached")
        self.assertEqual(row["sketch_cpu_secs"], "1.5")


if __name__ == "__main__":
    unittest.main()
