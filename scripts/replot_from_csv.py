#!/usr/bin/env python3
"""Regenerate boxplot PNGs from the tidy CSVs under
visualization/output/throughput_boxplot/, without re-running benches."""
from __future__ import annotations
import csv
from collections import OrderedDict
from pathlib import Path

import sys
sys.path.insert(0, str(Path(__file__).resolve().parent))
from throughput_boxplot import boxplot, boxplot_grouped, OUT_DIR

PANELS = [
    ("cms_2k",      "CMS @ 5×2048 (Zipf s=1.1, 10M items)",                 False),
    ("cs_2k",       "CountSketch @ 5×2048 (Zipf s=1.1, 10M items)",          False),
    ("cms_cs_32k",  "CMS vs CountSketch @ 5×32768 (Zipf s=1.1, 10M items)",  True),
    ("hll",         "HLL @ lg_k=14 (Zipf s=1.1, 10M items)",                 False),
    ("kll",         "KLL @ k=200 (Zipf s=1.1, 10M items)",                   False),
]

for tag, title, grouped in PANELS:
    csv_path = OUT_DIR / f"{tag}.csv"
    if not csv_path.exists():
        print(f"missing {csv_path}; skip")
        continue
    if grouped:
        data: "OrderedDict[tuple[str,str], list[float]]" = OrderedDict()
        with csv_path.open() as f:
            for row in csv.DictReader(f):
                key = (row["group"], row["impl"])
                data.setdefault(key, []).append(float(row["throughput_items_per_sec"]))
        boxplot_grouped(data, title, OUT_DIR / f"{tag}.png")
    else:
        data = OrderedDict()
        with csv_path.open() as f:
            for row in csv.DictReader(f):
                data.setdefault(row["impl"], []).append(float(row["throughput_items_per_sec"]))
        boxplot(data, title, OUT_DIR / f"{tag}.png")
