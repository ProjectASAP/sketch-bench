#!/usr/bin/env python3
"""Concatenate per-node study outputs into one run directory.

Usage: merge_study_parts.py OUT_DIR PART_DIR...

Each PART_DIR holds a study_saturation.py --phase accuracy output
(saturation.csv, saturation_curve.csv, optional saturation_merge_curve.csv)
for a disjoint set of points. OUT_DIR gets the three files with one header
each. Exits if two parts hold the same point.
"""

import csv
import os
import sys

FILES = ["saturation.csv", "saturation_curve.csv", "saturation_merge_curve.csv"]
KEY = ["sketch", "config", "dist", "param", "cardinality"]


def main(out, parts):
    os.makedirs(out, exist_ok=True)
    owner = {}
    for part in parts:
        with open(os.path.join(part, "saturation.csv"), newline="") as f:
            for r in csv.DictReader(f):
                key = tuple(r[k] for k in KEY)
                if key in owner:
                    sys.exit(f"{key} is in both {owner[key]} and {part}")
                owner[key] = part
    for name in FILES:
        header, rows = None, []
        for part in parts:
            path = os.path.join(part, name)
            if not os.path.exists(path):
                continue
            with open(path, newline="") as f:
                reader = csv.reader(f)
                h = next(reader)
                if header is None:
                    header = h
                elif h != header:
                    sys.exit(f"{path}: header {h} differs from {header}")
                rows += list(reader)
        if header is None:
            continue
        with open(os.path.join(out, name), "w", newline="") as f:
            writer = csv.writer(f)
            writer.writerow(header)
            writer.writerows(rows)
        print(f"{name}: {len(rows)} rows", file=sys.stderr)
    print(f"{len(owner)} points from {len(parts)} parts", file=sys.stderr)


if __name__ == "__main__":
    main(sys.argv[1], sys.argv[2:])
