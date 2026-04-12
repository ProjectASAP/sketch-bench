#!/usr/bin/env python3
from __future__ import annotations

import argparse
import csv
import os
import subprocess
import sys
import time
from pathlib import Path


def read_proc_total_ticks() -> tuple[int, int]:
    with Path("/proc/stat").open("r", encoding="utf-8") as handle:
        first = handle.readline().strip().split()
    values = [int(value) for value in first[1:]]
    idle = values[3] + (values[4] if len(values) > 4 else 0)
    total = sum(values)
    return total, idle


def list_descendants(root_pid: int) -> set[int]:
    children_by_parent: dict[int, list[int]] = {}
    for entry in Path("/proc").iterdir():
        if not entry.name.isdigit():
            continue
        try:
            stat = entry.joinpath("stat").read_text(encoding="utf-8")
        except OSError:
            continue
        closing = stat.rfind(")")
        if closing == -1:
            continue
        fields = stat[closing + 2 :].split()
        if len(fields) < 2:
            continue
        try:
            ppid = int(fields[1])
        except ValueError:
            continue
        children_by_parent.setdefault(ppid, []).append(int(entry.name))

    result: set[int] = set()
    stack = [root_pid]
    while stack:
        pid = stack.pop()
        if pid in result:
            continue
        result.add(pid)
        stack.extend(children_by_parent.get(pid, []))
    return result


def read_process_tree_metrics(root_pid: int) -> tuple[int, int, int]:
    total_ticks = 0
    total_rss_kb = 0
    process_count = 0

    for pid in list_descendants(root_pid):
        stat_path = Path("/proc") / str(pid) / "stat"
        status_path = Path("/proc") / str(pid) / "status"
        try:
            stat = stat_path.read_text(encoding="utf-8")
        except OSError:
            continue

        closing = stat.rfind(")")
        if closing == -1:
            continue
        fields = stat[closing + 2 :].split()
        if len(fields) < 15:
            continue

        try:
            utime = int(fields[11])
            stime = int(fields[12])
        except ValueError:
            continue

        total_ticks += utime + stime
        process_count += 1

        try:
            for line in status_path.read_text(encoding="utf-8").splitlines():
                if line.startswith("VmRSS:"):
                    parts = line.split()
                    if len(parts) >= 2:
                        total_rss_kb += int(parts[1])
                    break
        except OSError:
            continue

    return total_ticks, total_rss_kb, process_count


def monitor_command(
    command: list[str],
    output_csv: Path,
    stage_name: str,
    cwd: Path,
    sample_interval: float,
) -> dict[str, float]:
    output_csv.parent.mkdir(parents=True, exist_ok=True)
    clock_ticks = os.sysconf("SC_CLK_TCK")
    cpu_count = os.cpu_count() or 1

    start_time = time.monotonic()
    process = subprocess.Popen(command, cwd=str(cwd))

    prev_wall = time.monotonic()
    prev_total, prev_idle = read_proc_total_ticks()
    prev_proc_ticks, prev_rss_kb, prev_process_count = read_process_tree_metrics(process.pid)

    rows: list[dict[str, float | int | str]] = []

    while True:
        time.sleep(sample_interval)
        now_wall = time.monotonic()
        total, idle = read_proc_total_ticks()
        proc_ticks, rss_kb, process_count = read_process_tree_metrics(process.pid)

        elapsed = max(now_wall - prev_wall, 1e-9)
        total_delta = max(total - prev_total, 1)
        idle_delta = max(idle - prev_idle, 0)
        proc_delta = max(proc_ticks - prev_proc_ticks, 0)

        system_cpu_percent = max(0.0, min(100.0, (1.0 - idle_delta / total_delta) * 100.0))
        process_tree_cpu_cores = proc_delta / clock_ticks / elapsed
        process_tree_cpu_percent_of_system = process_tree_cpu_cores / cpu_count * 100.0
        process_tree_cpu_percent_of_one_core = process_tree_cpu_cores * 100.0

        rows.append(
            {
                "stage": stage_name,
                "seconds_since_start": now_wall - start_time,
                "system_cpu_percent": system_cpu_percent,
                "process_tree_cpu_cores": process_tree_cpu_cores,
                "process_tree_cpu_percent_of_system": process_tree_cpu_percent_of_system,
                "process_tree_cpu_percent_of_one_core": process_tree_cpu_percent_of_one_core,
                "process_tree_rss_kb": rss_kb,
                "process_count": process_count,
            }
        )

        prev_wall = now_wall
        prev_total, prev_idle = total, idle
        prev_proc_ticks, prev_rss_kb, prev_process_count = proc_ticks, rss_kb, process_count

        if process.poll() is not None:
            break

    return_code = process.wait()
    if return_code != 0:
        raise subprocess.CalledProcessError(return_code, command)

    with output_csv.open("w", encoding="utf-8", newline="") as handle:
        writer = csv.DictWriter(
            handle,
            fieldnames=[
                "stage",
                "seconds_since_start",
                "system_cpu_percent",
                "process_tree_cpu_cores",
                "process_tree_cpu_percent_of_system",
                "process_tree_cpu_percent_of_one_core",
                "process_tree_rss_kb",
                "process_count",
            ],
        )
        writer.writeheader()
        writer.writerows(rows)

    if not rows:
        return {
            "samples": 0,
            "duration_seconds": time.monotonic() - start_time,
            "avg_system_cpu_percent": 0.0,
            "max_system_cpu_percent": 0.0,
            "avg_process_tree_cpu_cores": 0.0,
            "max_process_tree_cpu_cores": 0.0,
            "max_process_tree_rss_kb": 0.0,
        }

    return {
        "samples": float(len(rows)),
        "duration_seconds": rows[-1]["seconds_since_start"],
        "avg_system_cpu_percent": sum(row["system_cpu_percent"] for row in rows) / len(rows),
        "max_system_cpu_percent": max(row["system_cpu_percent"] for row in rows),
        "avg_process_tree_cpu_cores": sum(row["process_tree_cpu_cores"] for row in rows)
        / len(rows),
        "max_process_tree_cpu_cores": max(row["process_tree_cpu_cores"] for row in rows),
        "max_process_tree_rss_kb": max(row["process_tree_rss_kb"] for row in rows),
    }


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument(
        "--variant",
        default="cms",
        choices=["cms", "cs", "hll", "kll", "octo", "cms32k", "cs32k"],
    )
    parser.add_argument("--op", default="both", choices=["insert", "query", "both"])
    parser.add_argument("--sample-interval", type=float, default=0.5)
    args = parser.parse_args()

    repo_root = Path(__file__).resolve().parents[2]
    script_dir = Path(__file__).resolve().parent
    throughput_dir = repo_root / "throughput"
    output_dir = throughput_dir / args.variant / "output"
    output_dir.mkdir(parents=True, exist_ok=True)

    ops = ["insert", "query"] if args.op == "both" else [args.op]
    rust_only_variants = {"octo", "cs32k"}
    cpp_skip_query = {"cs"}  # cs has no C++ query binary
    octo_skip_query = {"octo"}  # octo has no query binary

    stages: list[tuple[str, list[str], Path]] = []
    for op in ops:
        if args.variant in octo_skip_query and op == "query":
            continue
        result_prefix = (
            f"{args.variant}_throughput_query" if op == "query" else f"{args.variant}_throughput"
        )
        rust_output = output_dir / f"{result_prefix}_results_rust.csv"
        stages.append(
            (
                f"rust_{op}",
                [
                    str(script_dir / "run_throughput_rust.sh"),
                    args.variant,
                    str(rust_output),
                    op,
                ],
                output_dir / f"{result_prefix}_cpu_rust.csv",
            )
        )
        if args.variant in rust_only_variants:
            continue
        if op == "query" and args.variant in cpp_skip_query:
            continue
        cpp_output = output_dir / f"{result_prefix}_results_cpp.csv"
        stages.append(
            (
                f"cpp_{op}",
                [
                    str(script_dir / "run_throughput_cpp.sh"),
                    args.variant,
                    str(cpp_output),
                    op,
                ],
                output_dir / f"{result_prefix}_cpu_cpp.csv",
            )
        )

    result_prefix = f"{args.variant}_throughput"

    summary_rows = []
    for stage_name, command, output_csv in stages:
        print(f"Monitoring CPU during {stage_name} throughput stage...", flush=True)
        summary = monitor_command(
            command=command,
            output_csv=output_csv,
            stage_name=stage_name,
            cwd=repo_root,
            sample_interval=args.sample_interval,
        )
        summary_rows.append(
            {
                "stage": stage_name,
                **summary,
                "sample_csv": str(output_csv.relative_to(repo_root)),
            }
        )
        print(f"Wrote {output_csv}", flush=True)

    summary_csv = output_dir / f"{result_prefix}_cpu_summary.csv"
    with summary_csv.open("w", encoding="utf-8", newline="") as handle:
        writer = csv.DictWriter(
            handle,
            fieldnames=[
                "stage",
                "samples",
                "duration_seconds",
                "avg_system_cpu_percent",
                "max_system_cpu_percent",
                "avg_process_tree_cpu_cores",
                "max_process_tree_cpu_cores",
                "max_process_tree_rss_kb",
                "sample_csv",
            ],
        )
        writer.writeheader()
        writer.writerows(summary_rows)

    print(f"Wrote {summary_csv}")


if __name__ == "__main__":
    try:
        main()
    except subprocess.CalledProcessError as exc:
        sys.exit(exc.returncode)
