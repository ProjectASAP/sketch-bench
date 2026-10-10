#!/usr/bin/env python3
"""Unit tests for study_saturation's pure logic, and its cost phase against a
fake approxbench (so no real cost run happens).

Run: python3 scripts/test_study_saturation.py
"""

import csv
import json
import os
import subprocess
import sys
import tempfile
import textwrap
import unittest

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

from study_saturation import (  # noqa: E402
    HYDRA_COLUMNS, checkpoints, covered_max, hydra_groupings, hydra_labels, hydra_spec,
    n_saturation, n_star, read_curve, read_per_group)

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

    def test_must_stay_reached_to_the_largest_size(self):
        # A fixed exact-side overhead makes the ratio high at the smallest N,
        # dip below the factor, then grow again: N* is where it stays.
        ns = [10, 100, 1000, 10000]
        self.assertEqual(n_star(ns, [50, 20, 80, 900], [1, 10, 10, 10], 10), 10000)

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
    if "accuracy" in a and ops == "merge":
        e = 0.01 * int(a[a.index("--merge-shards") + 1])
        r = {"bench": {"accuracy": {"relative_error": e, "are_top100": e}}}
    elif "accuracy" in a:
        r = {"bench": {"accuracy": {"relative_error": 0.01, "are_top100": 0.01,
                                    "precision_at_k": 0.9}}}
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
            "--per-decade", "1", "--no-cost-shape", "--seeds", "2", "--points-from", points_csv,
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


class MergeCurveTest(unittest.TestCase):
    def test_one_row_per_shard_count_and_one_is_the_plain_query(self):
        with tempfile.TemporaryDirectory() as d:
            subprocess.run([
                sys.executable, SCRIPT, "--binary", fake_binary(d), "--out", d,
                "--phase", "accuracy", "--families", "cardinality", "--one-config",
                "--thetas", "1.0", "--cardinalities", "1000", "--n-max", "1e4",
                "--per-decade", "1", "--no-cost-shape", "--seeds", "2", "--merge-shards-list", "4,1",
            ], check=True, capture_output=True)
            with open(os.path.join(d, "saturation_merge_curve.csv"), newline="") as f:
                rows = list(csv.DictReader(f))
        self.assertEqual(
            [(r["n"], r["shards"], float(r["seed_mean_error"])) for r in rows],
            [("1000", "4", 0.04), ("1000", "1", 0.01),
             ("10000", "4", 0.04), ("10000", "1", 0.01)])

    def test_without_the_flag_there_is_no_merge_curve(self):
        with tempfile.TemporaryDirectory() as d:
            subprocess.run([
                sys.executable, SCRIPT, "--binary", fake_binary(d), "--out", d,
                "--phase", "accuracy", "--families", "cardinality", "--one-config",
                "--thetas", "1.0", "--cardinalities", "1000", "--n-max", "1e4",
                "--per-decade", "1", "--no-cost-shape", "--seeds", "1",
            ], check=True, capture_output=True)
            self.assertFalse(os.path.exists(os.path.join(d, "saturation_merge_curve.csv")))


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
                         for n in ns[:2]]
                      # theta 2 is outside this resume's grid and must survive.
                      + [["cardinality", "hll", "lg_k=12", "zipf", 2.0, 1000, n, 0.7, 0]
                         for n in ns])
            subprocess.run([
                sys.executable, SCRIPT, "--binary", fake_binary(d), "--out", d,
                "--phase", "accuracy", "--families", "cardinality", "--one-config",
                "--thetas", "0,1.0", "--cardinalities", "1000", "--n-max", "1e6",
                "--per-decade", "1", "--no-cost-shape", "--seeds", "2", "--resume",
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
        self.assertEqual(errors, {"0.0": [0.5] * 4, "1.0": [0.01] * 4, "2.0": [0.7] * 4})
        self.assertEqual([r["final_error"] for r in summary], ["0.5", "0.01"])
        # Only the rerun point was measured: 4 sizes x 2 seeds.
        self.assertEqual(len(raw), 8)

    def test_a_point_without_its_merge_curve_is_rerun_without_duplicates(self):
        with tempfile.TemporaryDirectory() as d:
            ns = checkpoints(1e3, 1e6, 1)
            row = ["cardinality", "hll", "lg_k=12", "zipf"]
            # Both curves are complete, but theta 1's merge rows were cut off
            # after one size and that size was written twice.
            write_csv(os.path.join(d, "saturation_curve.csv"), CURVE_HEADER,
                      [row + [t, 1000, n, 0.5, 0] for t in (0.0, 1.0) for n in ns])
            write_csv(os.path.join(d, "saturation_merge_curve.csv"),
                      CURVE_HEADER[:-2] + ["shards", "seed_mean_error", "seed_se"],
                      [row + [0.0, 1000, n, m, 0.5, 0] for n in ns for m in (1, 4)]
                      + [row + [1.0, 1000, ns[0], 4, 0.9, 0]] * 2)
            subprocess.run([
                sys.executable, SCRIPT, "--binary", fake_binary(d), "--out", d,
                "--phase", "accuracy", "--families", "cardinality", "--one-config",
                "--thetas", "0,1.0", "--cardinalities", "1000", "--n-max", "1e6",
                "--per-decade", "1", "--no-cost-shape", "--seeds", "2", "--merge-shards-list", "1,4",
                "--resume",
            ], check=True, capture_output=True)
            with open(os.path.join(d, "saturation_merge_curve.csv"), newline="") as f:
                rows = list(csv.DictReader(f))
        errors = {}
        for r in rows:
            errors.setdefault((r["param"], r["shards"]), []).append(float(r["seed_mean_error"]))
        self.assertEqual(errors, {("0.0", "1"): [0.5] * 4, ("0.0", "4"): [0.5] * 4,
                                  ("1.0", "1"): [0.01] * 4, ("1.0", "4"): [0.04] * 4})


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
                "--cardinalities", "1000", "--n-max", "1e6", "--per-decade", "1", "--no-cost-shape",
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
                "--cardinalities", "1000", "--n-max", "1e6", "--per-decade", "1", "--no-cost-shape",
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
        # Sketch CPU at N is 1 * N / 1e6 plus one query; the fake reports no
        # query throughput, so one query costs 0. Exact is 1e-6 * N, so it
        # never reaches 10x.
        self.assertEqual(row["cpu_n_star_10x"], "not_reached")
        self.assertEqual(row["sketch_cpu_secs"], "1.0")

    def test_crossover_phase_rebuilds_from_saved_records(self):
        with tempfile.TemporaryDirectory() as d:
            binary = fake_binary(d)
            ns = checkpoints(1e3, 1e6, 1)
            write_csv(os.path.join(d, "saturation_curve.csv"), CURVE_HEADER,
                      [["cardinality", "hll", "lg_k=12", "zipf", 1.0, 1000, n, 0.01, 0]
                       for n in ns])
            args = [sys.executable, SCRIPT, "--binary", binary, "--out", d,
                    "--families", "cardinality", "--one-config", "--thetas", "1.0",
                    "--cardinalities", "1000", "--n-max", "1e6", "--per-decade", "1", "--no-cost-shape"]
            subprocess.run(args + ["--phase", "cost"], check=True, capture_output=True)
            path = os.path.join(d, "crossover.csv")
            with open(path) as f:
                measured = f.read()
            os.remove(path)
            # No binary needed: the rebuild reads only the saved JSONL.
            subprocess.run(args[:3] + ["/nonexistent"] + args[4:] + ["--phase", "crossover"],
                           check=True, capture_output=True)
            with open(path) as f:
                rebuilt = f.read()
        self.assertEqual(rebuilt, measured)

    def test_crossover_phase_matches_saved_records_by_key(self):
        with tempfile.TemporaryDirectory() as d:
            ns = checkpoints(1e3, 1e6, 1)
            configs = ["rows=3 cols=256", "rows=5 cols=256"]
            write_csv(os.path.join(d, "saturation_curve.csv"), CURVE_HEADER,
                      [["frequency", "cms-fastpath-vector2d", c, "zipf", 1.0, 1000, n, 0.01, 0]
                       for c in configs for n in ns])
            points = os.path.join(d, "points.csv")
            write_csv(points, ["sketch", "config", "dist", "param", "cardinality"],
                      [["cms-fastpath-vector2d", c, "zipf", 1.0, 1000] for c in configs])
            args = [sys.executable, SCRIPT, "--binary", fake_binary(d), "--out", d,
                    "--families", "frequency", "--thetas", "1.0", "--cardinalities",
                    "1000", "--n-max", "1e6", "--per-decade", "1", "--no-cost-shape", "--points-from", points]
            subprocess.run(args + ["--phase", "cost"], check=True, capture_output=True)
            # Tell the rows=5 record apart, and store it first.
            cost = os.path.join(d, "saturation_cost.jsonl")
            with open(cost) as f:
                records = [json.loads(line) for line in f]
            records[1]["memory_bytes"] = 5000
            with open(cost, "w") as f:
                f.writelines(json.dumps(r) + "\n" for r in reversed(records))
            subprocess.run(args + ["--phase", "crossover", "--cost-rows", "3"],
                           check=True, capture_output=True)
            with open(os.path.join(d, "crossover.csv"), newline="") as f:
                [row] = list(csv.DictReader(f))
            # A point the cost phase never measured exits.
            with open(cost, "w") as f:
                f.write(json.dumps(records[0]) + "\n")
            missing = subprocess.run(args + ["--phase", "crossover"], capture_output=True)
        self.assertEqual((row["config"], row["sketch_memory_bytes"]), ("rows=3 cols=256", "1000"))
        self.assertNotEqual(missing.returncode, 0)
        self.assertIn(b"has no record", missing.stderr)


class ResumeSummaryTest(unittest.TestCase):
    def test_a_narrower_resume_keeps_the_other_points_summary_rows(self):
        with tempfile.TemporaryDirectory() as d:
            base = [sys.executable, SCRIPT, "--binary", fake_binary(d), "--out", d,
                    "--phase", "accuracy", "--one-config", "--thetas", "1.0",
                    "--cardinalities", "1000", "--n-max", "1e4", "--per-decade", "1",
                    "--seeds", "1", "--no-cost-shape"]
            subprocess.run(base + ["--families", "cardinality,frequency"], check=True,
                           capture_output=True)
            subprocess.run(base + ["--families", "cardinality", "--resume"], check=True,
                           capture_output=True)
            with open(os.path.join(d, "saturation.csv"), newline="") as f:
                sketches = sorted(r["sketch"] for r in csv.DictReader(f))
        self.assertEqual(sketches, ["cms-fastpath-vector2d", "countsketch-fastpath-vector2d",
                                    "hll"])


class TopkKTest(unittest.TestCase):
    def test_topk_curves_run_at_each_k(self):
        with tempfile.TemporaryDirectory() as d:
            subprocess.run([
                sys.executable, SCRIPT, "--binary", fake_binary(d), "--out", d,
                "--phase", "accuracy", "--families", "topk", "--one-config",
                "--thetas", "1.0", "--cardinalities", "1000", "--n-max", "1e4",
                "--per-decade", "1", "--seeds", "1", "--no-cost-shape", "--topk-ks", "10,32,100",
            ], check=True, capture_output=True)
            with open(os.path.join(d, "saturation_curve.csv"), newline="") as f:
                configs = {r["config"] for r in csv.DictReader(f)}
        # k = 32 keeps the old key; the others carry topk_k.
        self.assertEqual(configs, {"rows=3 cols=256", "rows=3 cols=256 topk_k=10",
                                   "rows=3 cols=256 topk_k=100"})


class TopkKsValidationTest(unittest.TestCase):
    def test_topk_ks_must_include_32(self):
        with tempfile.TemporaryDirectory() as d:
            result = subprocess.run([
                sys.executable, SCRIPT, "--binary", fake_binary(d), "--out", d,
                "--phase", "accuracy", "--families", "topk", "--topk-ks", "10,100",
            ], capture_output=True)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn(b"must include 32", result.stderr)


class CostShapeTest(unittest.TestCase):
    def test_the_grid_also_runs_the_optimizer_cost_shape(self):
        with tempfile.TemporaryDirectory() as d:
            subprocess.run([
                sys.executable, SCRIPT, "--binary", fake_binary(d), "--out", d,
                "--phase", "accuracy", "--families", "cardinality", "--one-config",
                "--thetas", "1.0", "--cardinalities", "1000", "--n-max", "1e4",
                "--per-decade", "1", "--seeds", "1",
            ], check=True, capture_output=True)
            with open(os.path.join(d, "saturation_curve.csv"), newline="") as f:
                shapes = {(r["param"], r["cardinality"]) for r in csv.DictReader(f)}
        # A full row and column: θ = 1.1 at every K, K = 1e4 at every θ.
        self.assertEqual(shapes, {("1.0", "1000"), ("1.0", "10000"),
                                  ("1.1", "1000"), ("1.1", "10000")})

    def test_a_grid_that_already_has_the_cost_shape_adds_nothing(self):
        with tempfile.TemporaryDirectory() as d:
            subprocess.run([
                sys.executable, SCRIPT, "--binary", fake_binary(d), "--out", d,
                "--phase", "accuracy", "--families", "cardinality", "--one-config",
                "--thetas", "1.1", "--cardinalities", "10000", "--n-max", "1e4",
                "--per-decade", "1", "--seeds", "1",
            ], check=True, capture_output=True)
            with open(os.path.join(d, "saturation.csv"), newline="") as f:
                shapes = [(r["param"], r["cardinality"]) for r in csv.DictReader(f)]
        self.assertEqual(shapes, [("1.1", "10000")])


# Logs every call; atomic-costs reports `--rows` rows kept, 0 skipped.
FAKE_COST_APPROXBENCH = textwrap.dedent("""\
    import json, os, sys
    a = sys.argv
    value = lambda flag: a[a.index(flag) + 1] if flag in a else None
    with open(os.environ["FAKE_LOG"], "a") as f:
        f.write(json.dumps(a[1:]) + "\\n")
    if a[1] == "atomic-costs":
        # One row per logged accuracy pass, scoring 0.5 in its metric.
        metrics = dict(m.split("=") for m in a[3:] if "=" in m and "/" not in m)
        rows = []
        for line in open(os.environ["FAKE_LOG"]):
            c = json.loads(line)
            if c[0] == "sketchbench" and "--report" in c and "accuracy" in c:
                v = c[c.index("--variant") + 1]
                cfg = c[c.index("--config") + 1] if "--config" in c else ""
                params = {k: float(x) for k, x in (kv.split("=") for kv in cfg.split())}
                # The reducer reads a Hydra row's width off its scores.
                at = {}
                if v.startswith("hydra-"):
                    labels = open(c[c.index("--spec") + 1]).read().split("column_label: [")[1]
                    at = {"schema_width": labels.split("]")[0].count(",")}
                rows.append({"sketch": v, "sketch_config": {"params": params},
                             "query_accuracy": {metrics[v]: 0.5}, "accuracy_metric": metrics[v],
                             "measured_at": at})
        json.dump(rows, open(value("--output"), "w"))
        kept = int(os.environ["FAKE_KEPT"])
        sys.stderr.write(f"approxbench atomic-costs: {kept} row(s), 0 skipped\\n")
    elif a[1] == "sketchbench" and "--report" not in a:
        # A seed run: error = seed / 100 in every metric.
        e = int(value("--seed")) / 100
        print(json.dumps({"bench": {"accuracy": {m: e for m in (
            "precision_at_k", "mean_rank_err", "mean_relative_value_error")}}}))
""")


class OptimizerCostTest(unittest.TestCase):
    def run_phase(self, d, kept, families="topk,quantile"):
        binary = os.path.join(d, "approxbench")
        with open(binary, "w") as f:
            f.write("#!" + sys.executable + "\n" + FAKE_COST_APPROXBENCH)
        os.chmod(binary, 0o755)
        log = os.path.join(d, "log.jsonl")
        result = subprocess.run([
            sys.executable, SCRIPT, "--binary", binary, "--out", os.path.join(d, "out"),
            "--phase", "optimizer-cost", "--families", families, "--one-config",
            "--seeds", "3",
        ], capture_output=True, env={**os.environ, "FAKE_LOG": log, "FAKE_KEPT": str(kept)})
        with open(log) as f:
            return result, [json.loads(line) for line in f]

    def test_one_cost_and_one_accuracy_pass_per_config_at_one_shape(self):
        with tempfile.TemporaryDirectory() as d:
            # topk one config at 4 heaps, quantile (kll, dd) one config
            # each, plus 5 exact.
            result, calls = self.run_phase(d, 11)
        self.assertEqual(result.returncode, 0, result.stderr)
        bench = [c for c in calls if c[0] == "sketchbench" and "--report" in c]
        self.assertEqual(len(bench), 22)
        value = lambda c, flag: c[c.index(flag) + 1]
        topk, kll = bench[0], bench[8]
        self.assertEqual([value(c, "--config") for c in bench[0:8:2]],
                         [f"rows=3 cols=256 heap={h}" for h in (32, 128, 512, 2048)])
        self.assertEqual((value(topk, "--zipf-s"), value(topk, "--cardinality"),
                          value(topk, "--size")), ("1.1", "10000", "1000000"))
        self.assertEqual(value(kll, "--pareto-alpha"), "2.0")
        self.assertEqual((value(topk, "--runs"), value(topk, "--warmup-runs")), ("5", "3"))
        self.assertEqual(value(bench[1], "--metrics"), "accuracy")
        self.assertEqual(value(bench[-1], "--library"), "exact")
        [reduce] = [c for c in calls if c[0] == "atomic-costs"]
        self.assertIn("cms-heap-topk-fastpath-vector2d=precision_at_k", reduce)
        self.assertIn("kll-percall=mean_rank_err", reduce)
        self.assertIn("exact-sum=relative_error", reduce)
        self.assertNotIn("--merge-accuracy", reduce)
        # Then 3 seed runs per sketch config at heap k: top-k, KLL, DD.
        seeds = [c for c in calls if c[0] == "sketchbench" and "--report" not in c]
        self.assertEqual(len(seeds), 9)
        self.assertEqual({c[c.index("--seed") + 1] for c in seeds}, {"1", "2", "3"})
        self.assertTrue(all("heap=32" in c[c.index("--config") + 1] for c in seeds
                            if "topk" in c[c.index("--variant") + 1]))

    def test_sketch_rows_take_the_seed_mean_accuracy(self):
        with tempfile.TemporaryDirectory() as d:
            result, _ = self.run_phase(d, 11)
            with open(os.path.join(d, "out", "rqe_atomic_costs.json")) as f:
                rows = json.load(f)
        self.assertEqual(result.returncode, 0, result.stderr)
        # Seeds 1..3 score 0.01, 0.02, 0.03: every sketch row (every heap)
        # reads their mean; exact rows keep the accuracy pass's.
        sketches = [r for r in rows if not r["sketch"].startswith("exact-")]
        self.assertEqual(len(sketches), 6)
        for row in sketches:
            self.assertAlmostEqual(row["query_accuracy"][row["accuracy_metric"]], 0.02)
        for row in rows:
            if row["sketch"].startswith("exact-"):
                self.assertEqual(row["query_accuracy"][row["accuracy_metric"]], 0.5)

    def test_hydra_rows_on_the_eval_datasets_name_their_dataset(self):
        with tempfile.TemporaryDirectory() as d:
            # hydra-hll and hydra-univmon-cardinality on http and flows,
            # hydra-kll on http_latency, at the first W; plus 5 exact.
            result, calls = self.run_phase(d, 10, families="hydra")
            with open(os.path.join(d, "out", "rqe_atomic_costs.json")) as f:
                rows = json.load(f)
        self.assertEqual(result.returncode, 0, result.stderr)
        value = lambda c, flag: c[c.index(flag) + 1]
        hydra = [c for c in calls if c[0] == "sketchbench" and "--report" in c
                 and value(c, "--variant").startswith("hydra-")]
        self.assertEqual(len(hydra), 10)
        cost = hydra[0]
        self.assertEqual((value(cost, "--runs"), value(cost, "--warmup-runs"),
                          value(cost, "--merge-shards")), ("3", "1", "16"))
        self.assertEqual(value(cost, "--config"), "rows=3 cols=1024")
        self.assertTrue(value(cost, "--spec").endswith("hydra_http_n1000000.yaml"))
        self.assertNotIn("--comparator", hydra[1])
        # No seed runs: Hydra accuracy comes from hydra_saturation.csv.
        self.assertFalse([c for c in calls if c[0] == "sketchbench" and "--report" not in c])
        at = {(r["sketch"], r["measured_at"]["schema_width"]): r["measured_at"]["dataset"]
              for r in rows if r["sketch"].startswith("hydra-")}
        self.assertEqual(at, {
            ("hydra-hll", 4): "hydra_http", ("hydra-hll", 3): "hydra_flows",
            ("hydra-univmon-cardinality", 4): "hydra_http",
            ("hydra-univmon-cardinality", 3): "hydra_flows",
            ("hydra-kll", 4): "hydra_http_latency"})

    def test_a_skipped_row_fails(self):
        with tempfile.TemporaryDirectory() as d:
            result, _ = self.run_phase(d, 10)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn(b"did not keep all 11 rows", result.stderr)


class CoveredMaxTest(unittest.TestCase):
    # (n_q, error) of 100 records: shares 0.6, 0.3, 0.06, 0.04.
    GROUPS = [(60, 0.1), (30, 0.2), (6, 0.5), (4, 0.9)]

    def test_groups_at_or_above_the_share_count(self):
        self.assertEqual(covered_max(self.GROUPS, 100, 0.05), 0.5)
        self.assertEqual(covered_max(self.GROUPS, 100, 0.04), 0.9)
        self.assertEqual(covered_max(self.GROUPS, 100, 0.3), 0.2)

    def test_the_largest_group_is_always_covered(self):
        self.assertEqual(covered_max(self.GROUPS, 100, 0.9), 0.1)
        # Wherever it sits in the file.
        self.assertEqual(covered_max(self.GROUPS[::-1], 100, 0.9), 0.1)

    def test_unscored_groups_have_no_error_but_can_be_the_largest(self):
        groups = [(60, None), (30, 0.2), (6, 0.5)]
        self.assertEqual(covered_max(groups, 100, 0.05), 0.5)
        # The largest is unscored, so nothing covered has an error.
        self.assertEqual(covered_max(groups, 100, 0.9), "")
        # An unscored group does not hand "largest" to the next one.
        self.assertEqual(covered_max([(60, None), (30, 0.2)], 100, 0.5), "")

    def test_no_groups(self):
        self.assertEqual(covered_max([], 100, 0.01), "")


SPEC = textwrap.dedent("""\
    column_num: 3
    column_label: [region, service, user_id]
    row_num: 200000
    column_spec:
      - data_type: string
        distribution:
          kind: uniform
          seed: 1
      - data_type: i64
        distribution: {kind: zipf, skewness: 0.8, population_size: 10, seed: 5}
""")


class HydraSpecTest(unittest.TestCase):
    def test_rows_and_seeds(self):
        self.assertEqual(hydra_spec(SPEC, 1000, 1).replace("row_num: 1000", "row_num: 200000"),
                         SPEC)
        text = hydra_spec(SPEC, 1000, 3)
        self.assertIn("row_num: 1000\n", text)
        self.assertIn("      seed: 2001\n", text)
        # A flow mapping's seed is not a line of its own; it stays.
        self.assertIn("seed: 5}", text)

    def test_labels_and_groupings_in_schema_order(self):
        self.assertEqual(hydra_labels(SPEC), ["region", "service"])
        self.assertEqual(hydra_groupings(["a", "b", "c"]),
                         [["a"], ["b"], ["c"], ["a", "b"], ["a", "c"], ["b", "c"],
                          ["a", "b", "c"]])

    def test_the_eval_specs_match_their_schemas(self):
        root = os.path.join(os.path.dirname(SCRIPT), "..", "configs", "datagen")

        def read(stem):
            with open(os.path.join(root, f"{stem}.yaml")) as f:
                return f.read()
        http = ["region", "service", "endpoint", "status"]
        self.assertEqual(hydra_labels(read("hydra_http")), http)
        self.assertEqual(hydra_labels(read("hydra_http_latency")), http)
        self.assertEqual(hydra_labels(read("hydra_flows")), ["dst_subnet", "dst_port", "proto"])
        self.assertEqual(len(hydra_groupings(http)), 15)


# A Hydra run: writes a fixed per-group file for the grouping asked, and a
# record of N = 100 records over 2 labels. Merged runs score 0.1 worse.
FAKE_HYDRA_APPROXBENCH = textwrap.dedent("""\
    import json, os, sys
    a = sys.argv
    value = lambda flag: a[a.index(flag) + 1] if flag in a else None
    with open(os.environ["FAKE_LOG"], "a") as f:
        f.write(json.dumps(a[1:]) + "\\n")
    if value("--variant") == "hydra-cms" and value("--group-columns") == "1":
        sys.exit("boom")
    m = int(value("--merge-shards") or 1)
    worse = 0.1 if m > 1 else 0.0
    groups = [(60, 0.1 + worse), (30, None), (6, 0.5 + worse), (4, 0.9 + worse)]
    with open(value("--per-group-out"), "w") as f:
        f.write("group_key,n_q,error\\n")
        for i, (n, e) in enumerate(groups):
            f.write(f'"label0:g{i},x",{n},{"" if e is None else e}\\n')
    errs = sorted(e for _, e in groups if e is not None)
    bench = {"accuracy": {"err_mean": sum(errs) / 3, "err_p50": errs[1], "err_p90": errs[2],
                          "err_max": errs[2], "groups_scored": 3.0, "schema_width": 2.0,
                          "records": 100.0, "fanned_mass": 300.0}}
    if m > 1:
        bench.update(merge_shards=m, merge_split=value("--merge-split"))
    print(json.dumps({"bench": bench}))
""")


class HydraPhaseTest(unittest.TestCase):
    def run_phase(self, d, *extra, out="out"):
        binary = os.path.join(d, "approxbench")
        with open(binary, "w") as f:
            f.write("#!" + sys.executable + "\n" + FAKE_HYDRA_APPROXBENCH)
        os.chmod(binary, 0o755)
        datagen = os.path.join(d, "datagen")
        os.makedirs(datagen, exist_ok=True)
        for stem in ("hydra_hier_d2", "hydra_hier_d3", "hydra_hier"):
            with open(os.path.join(datagen, f"{stem}.yaml"), "w") as f:
                f.write(SPEC)
        log = os.path.join(d, "log.jsonl")
        result = subprocess.run([
            sys.executable, SCRIPT, "--binary", binary, "--out", os.path.join(d, out),
            "--phase", "hydra", "--datagen", datagen, "--hydra-sweeps", "generic",
            "--hydra-datasets", "hydra_hier_d2", "--hydra-variants", "hydra-hll,hydra-cms",
            "--hydra-ws", "1024", "--hydra-ns", "100", "--hydra-seeds", "1", "--jobs", "2",
            *extra,
        ], capture_output=True, env={**os.environ, "FAKE_LOG": log})
        table = os.path.join(d, out, "hydra_saturation.csv")
        with open(table, newline="") as f:
            header = next(csv.reader(f))
        with open(table, newline="") as f:
            rows = list(csv.DictReader(f))
        with open(log) as f:
            return result, rows, header, [json.loads(line) for line in f]

    def test_one_row_per_run_with_coverage_columns(self):
        with tempfile.TemporaryDirectory() as d:
            result, rows, header, calls = self.run_phase(d)
        # 2 variants x 3 groupings x shards {1, 4}; hydra-cms at {service}
        # fails (2 runs), is reported, and the phase exits 1.
        self.assertEqual(result.returncode, 1)
        self.assertIn(b"2 run(s) failed", result.stderr)
        self.assertEqual(len(calls), 12)
        self.assertEqual(header, HYDRA_COLUMNS)
        self.assertEqual(len(rows), 10)
        row = {(r["variant"], r["group_columns"], r["merge_shards"]): r for r in rows}
        query = row[("hydra-hll", "region,service", "1")]
        self.assertEqual(
            {k: query[k] for k in ("statistic", "config", "R", "W", "dataset", "schema_width",
                                   "records", "fanned_mass", "merge_split", "seed",
                                   "err_max", "groups_scored")},
            {"statistic": "cardinality", "config": "rows=3 cols=1024", "R": "3",
             "W": "1024", "dataset": "hydra_hier_d2", "schema_width": "2", "records": "100",
             "fanned_mass": "300", "merge_split": "", "seed": "1", "err_max": "0.9",
             "groups_scored": "3"})
        # Shares 0.6, 0.3 (unscored), 0.06, 0.04.
        self.assertEqual((query["err_max_cov_0.01"], query["err_max_cov_0.05"]), ("0.9", "0.5"))
        merged = row[("hydra-hll", "region,service", "4")]
        self.assertEqual((merged["merge_split"], merged["err_max_cov_0.05"]),
                         ("interleaved", "0.6"))
        # Groupings are asked by column index, in schema order.
        hll = [c for c in calls if c[c.index("--variant") + 1] == "hydra-hll"]
        self.assertEqual(sorted({c[c.index("--group-columns") + 1] for c in hll}),
                         ["0", "0,1", "1"])
        spec = hll[0][hll[0].index("--spec") + 1]
        self.assertTrue(spec.endswith("hydra_hier_d2_n100_s1.yaml"))

    def test_shards_split_the_runs_and_resume_reruns_only_the_missing(self):
        with tempfile.TemporaryDirectory() as d:
            _, first, _, _ = self.run_phase(d, "--hydra-shard", "0/2")
            os.remove(os.path.join(d, "log.jsonl"))
            _, other, _, _ = self.run_phase(d, "--hydra-shard", "1/2", out="other")
            os.remove(os.path.join(d, "log.jsonl"))
            _, rows, _, calls = self.run_phase(d, "--resume")
        self.assertEqual(len(first) + len(other), 10)
        key = lambda r: (r["variant"], r["group_columns"], r["merge_shards"])
        self.assertFalse({key(r) for r in first} & {key(r) for r in other})
        # The rerun runs the 6 runs shard 0 lacked; the 2 failures fail again.
        self.assertEqual(len(calls), 12 - len(first))
        self.assertEqual(len(rows), 10)


if __name__ == "__main__":
    unittest.main()
