#!/usr/bin/env python3
"""Tests for merge_study_parts.py."""

import csv
import os
import subprocess
import sys
import tempfile
import unittest

SCRIPT = os.path.join(os.path.dirname(os.path.abspath(__file__)), "merge_study_parts.py")
KEY = "family,sketch,config,dist,param,cardinality"


def part(d, name, points, merge=False):
    path = os.path.join(d, name)
    os.makedirs(path)
    with open(os.path.join(path, "saturation.csv"), "w") as f:
        f.write(f"{KEY},n_sat\n" + "".join(f"{p},1000\n" for p in points))
    with open(os.path.join(path, "saturation_curve.csv"), "w") as f:
        f.write(f"{KEY},n,seed_mean_error,seed_se\n" + "".join(f"{p},1000,0.1,0\n" for p in points))
    if merge:
        with open(os.path.join(path, "saturation_merge_curve.csv"), "w") as f:
            f.write(f"{KEY},n,shards,seed_mean_error,seed_se\n"
                    + "".join(f"{p},1000,4,0.2,0\n" for p in points))
    return path


def rows(path):
    with open(path, newline="") as f:
        return list(csv.DictReader(f))


class MergeStudyPartsTest(unittest.TestCase):
    def test_concatenates_disjoint_parts(self):
        with tempfile.TemporaryDirectory() as d:
            a = part(d, "a", ["topk,cms,rows=3,zipf,1.1,10000"])
            b = part(d, "b", ["quantile,kll,k=200,pareto,2.0,"], merge=True)
            out = os.path.join(d, "out")
            subprocess.run([sys.executable, SCRIPT, out, a, b], check=True, capture_output=True)
            self.assertEqual(len(rows(os.path.join(out, "saturation.csv"))), 2)
            self.assertEqual(len(rows(os.path.join(out, "saturation_curve.csv"))), 2)
            # Only one part has merge curves; they come through.
            self.assertEqual(len(rows(os.path.join(out, "saturation_merge_curve.csv"))), 1)

    def test_a_point_in_two_parts_is_refused(self):
        with tempfile.TemporaryDirectory() as d:
            p = "topk,cms,rows=3,zipf,1.1,10000"
            a, b = part(d, "a", [p]), part(d, "b", [p])
            result = subprocess.run([sys.executable, SCRIPT, os.path.join(d, "out"), a, b],
                                    capture_output=True)
            self.assertNotEqual(result.returncode, 0)
            self.assertIn(b"is in both", result.stderr)


if __name__ == "__main__":
    unittest.main()
