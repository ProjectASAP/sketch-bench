#!/usr/bin/env python3
"""Unit tests for study_saturation.n_saturation and checkpoints.

Run: python3 scripts/test_study_saturation.py
"""

import os
import sys
import unittest

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

from study_saturation import checkpoints, n_saturation  # noqa: E402

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


if __name__ == "__main__":
    unittest.main()
