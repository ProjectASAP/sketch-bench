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
            write_run(out, "r1", "uc", rows, split={"query_result": "container"})
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

    def test_split_writes_one_file_per_entity(self):
        with tempfile.TemporaryDirectory() as out:
            rows = [row(t, "app_state", "r1", labels, "state", "on", "state")
                    for t, labels in ((2000, {"container": "c1"}), (1000, {"container": "c1"}),
                                      (1000, {"container": "c2"}), (1000, {}))]
            write_run(out, "r1", "uc", rows + [sample()], split={"app_state": "container"})
            path = os.path.join(out, "runs", "r1")
            self.assertEqual(sorted(os.listdir(os.path.join(path, "app_state"))), ["c1.json", "c2.json"])
            with open(os.path.join(path, "app_state", "c1.json")) as f:
                self.assertEqual([r["time"] for r in json.load(f)], [1000, 2000])
            with open(os.path.join(path, "app_state.json")) as f:
                self.assertEqual([r["labels"] for r in json.load(f)], [{}])
            self.assertEqual(len(read_run(out, "r1", "app_state")), 4)
            write_run(out, "r1", "uc", rows)
            self.assertFalse(os.path.exists(os.path.join(path, "app_state")))
            self.assertEqual(len(read_run(out, "r1", "app_state")), 4)

    def test_split_rejects_entity_that_cannot_name_a_file(self):
        with tempfile.TemporaryDirectory() as out:
            r = row(1000, "app_state", "r1", {"container": "../c1"}, "state", "on", "state")
            with self.assertRaises(ValueError):
                write_run(out, "r1", "uc", [r], split={"app_state": "container"})

    def test_split_rejects_entities_differing_only_in_case(self):
        with tempfile.TemporaryDirectory() as out:
            rows = [row(1000, "app_state", "r1", {"container": c}, "state", "on", "state") for c in ("c1", "C1")]
            with self.assertRaises(ValueError):
                write_run(out, "r1", "uc", rows, split={"app_state": "container"})
            self.assertFalse(os.path.exists(os.path.join(out, "runs")))

    def test_read_skips_files_that_are_not_json(self):
        with tempfile.TemporaryDirectory() as out:
            rows = [row(1000, "app_state", "r1", {"container": "c1"}, "state", "on", "state")]
            write_run(out, "r1", "uc", rows, split={"app_state": "container"})
            with open(os.path.join(out, "runs", "r1", "app_state", ".DS_Store"), "wb") as f:
                f.write(b"\x00\x00\x00\x01Bud1")
            self.assertEqual(read_run(out, "r1", "app_state"), rows)

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
