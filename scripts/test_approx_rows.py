#!/usr/bin/env python3

import json
import os
import tempfile
import unittest

from approx_rows import KINDS, SCHEMA_VERSION, check, read_index, read_run, row, write_run


def sample(run_id="r1", time=1000, kind="query_result"):
    return row(time, kind, run_id, {"query": "q", "arm": "exact"}, "value", 1.5, "addresses")


class RowTest(unittest.TestCase):
    def test_row_carries_schema_version(self):
        self.assertEqual(sample()["schema_version"], SCHEMA_VERSION)

    def test_unknown_kind_rejected(self):
        with self.assertRaises(ValueError):
            sample(kind="dashboard")

    def test_other_schema_version_rejected(self):
        r = sample()
        r["schema_version"] = SCHEMA_VERSION + 1
        with self.assertRaises(ValueError):
            check(r)

    def test_extra_field_rejected(self):
        r = sample()
        r["note"] = "x"
        with self.assertRaises(ValueError):
            check(r)

    def test_non_string_label_rejected(self):
        with self.assertRaises(ValueError):
            row(1000, "query_result", "r1", {"window": 10}, "value", 1.0, "addresses")

    def test_float_time_rejected(self):
        with self.assertRaises(ValueError):
            row(1000.5, "query_result", "r1", {}, "value", 1.0, "addresses")

    def test_bool_value_rejected(self):
        with self.assertRaises(ValueError):
            row(1000, "app_state", "r1", {}, "alert", True, "state")


class WriteRunTest(unittest.TestCase):
    def test_round_trip_and_one_file_per_kind(self):
        with tempfile.TemporaryDirectory() as out:
            rows = [sample(time=2000), sample(time=1000), sample(time=1500, kind="low_level")]
            write_run(out, "r1", "uc", rows)
            self.assertEqual(sorted(os.listdir(os.path.join(out, "runs", "r1"))),
                             sorted(f"{k}.json" for k in KINDS))
            self.assertEqual([r["time"] for r in read_run(out, "r1", "query_result")], [1000, 2000])
            self.assertEqual(read_run(out, "r1", "app_state"), [])
            self.assertEqual(read_index(out),
                             [{"run_id": "r1", "use_case": "uc", "start": 1000, "end": 2000}])

    def test_rewrite_replaces_index_entry(self):
        with tempfile.TemporaryDirectory() as out:
            write_run(out, "r2", "uc", [sample("r2")])
            write_run(out, "r1", "uc", [sample("r1")])
            write_run(out, "r1", "uc", [sample("r1", time=5000)])
            self.assertEqual([(e["run_id"], e["end"]) for e in read_index(out)],
                             [("r1", 5000), ("r2", 1000)])

    def test_details_kept_in_index(self):
        with tempfile.TemporaryDirectory() as out:
            write_run(out, "r1", "uc", [sample()], details={"arm": "a"})
            self.assertEqual(read_index(out)[0]["details"], {"arm": "a"})

    def test_row_of_another_run_rejected(self):
        with tempfile.TemporaryDirectory() as out:
            with self.assertRaises(ValueError):
                write_run(out, "r1", "uc", [sample("r2")])

    def test_read_rejects_bad_rows(self):
        with tempfile.TemporaryDirectory() as out:
            write_run(out, "r1", "uc", [sample()])
            path = os.path.join(out, "runs", "r1", "query_result.json")
            with open(path) as f:
                rows = json.load(f)
            rows[0]["schema_version"] = 0
            with open(path, "w") as f:
                json.dump(rows, f)
            with self.assertRaises(ValueError):
                read_run(out, "r1", "query_result")


if __name__ == "__main__":
    unittest.main()
