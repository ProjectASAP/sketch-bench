#!/usr/bin/env python3
"""Run one external workload spec over complete tumbling time windows.

Preparation happens independently in each approxbench process and is excluded
from the measured sketch operation. The output report is append-only, so all
windows can share one JSONL destination.
"""

import argparse
import datetime as dt
import subprocess
import sys


def parse_time(value: str) -> dt.datetime:
    value = value.replace("Z", "+00:00")
    parsed = dt.datetime.fromisoformat(value)
    if parsed.tzinfo is None:
        parsed = parsed.replace(tzinfo=dt.timezone.utc)
    return parsed.astimezone(dt.timezone.utc)


def wire_time(value: dt.datetime) -> str:
    return value.isoformat(timespec="microseconds").replace("+00:00", "Z")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", default="approxbench")
    parser.add_argument("--workload-spec", required=True)
    parser.add_argument("--data-root", default=".")
    parser.add_argument("--start", required=True, help="UTC/RFC3339 sweep start")
    parser.add_argument("--end", required=True, help="UTC/RFC3339 sweep end")
    parser.add_argument("--window-seconds", type=int, default=60)
    parser.add_argument("--step-seconds", type=int)
    parser.add_argument("benchmark_args", nargs=argparse.REMAINDER)
    args = parser.parse_args()

    if args.window_seconds <= 0:
        parser.error("--window-seconds must be positive")
    step = args.step_seconds or args.window_seconds
    if step <= 0:
        parser.error("--step-seconds must be positive")
    start = parse_time(args.start)
    end = parse_time(args.end)
    if start >= end:
        parser.error("--start must be before --end")
    benchmark_args = list(args.benchmark_args)
    if benchmark_args[:1] == ["--"]:
        benchmark_args.pop(0)

    cursor = start
    windows = 0
    while cursor + dt.timedelta(seconds=args.window_seconds) <= end:
        window_end = cursor + dt.timedelta(seconds=args.window_seconds)
        command = [
            args.binary,
            "sketchbench",
            "--workload-spec",
            args.workload_spec,
            "--data-root",
            args.data_root,
            "--window-start",
            wire_time(cursor),
            "--window-end",
            wire_time(window_end),
        ] + benchmark_args
        subprocess.run(command, check=True)
        windows += 1
        cursor += dt.timedelta(seconds=step)

    if windows == 0:
        parser.error("sweep range contains no complete windows")
    return 0


if __name__ == "__main__":
    sys.exit(main())
