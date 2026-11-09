#!/usr/bin/env python3
"""Convert downloaded public traces into the CSV tables planeval imports for use cases 1, 2, 4 and 5."""

import argparse
import csv
import gzip
import hashlib
import io
import os
import random
import re
import struct
import tarfile
from datetime import datetime

DATASETS = os.path.expanduser("~/ProjectASAP/datasets")
CAIDA = os.path.expanduser("~/ProjectASAP/equinix-chicago.dirA.20160121-125911.UTC.anon.pcap.gz")
MINUTE_NS = 60 * 10**9


def writer(name):
    path = os.path.join(DATASETS, "converted", name)
    os.makedirs(os.path.dirname(path), exist_ok=True)
    handle = gzip.open(path, "wt", compresslevel=1, newline="")
    return path, handle, csv.writer(handle)


def syn_flood():
    path, handle, out = writer("uc1/caida_syn_only.csv.gz")
    out.writerow(["timestamp", "vm", "src_ip"])
    rows, sources, first, last = 0, set(), None, None
    with gzip.open(CAIDA, "rb") as f:
        f.read(24)
        while True:
            header = f.read(16)
            if len(header) < 16:
                break
            seconds, micros, captured, _ = struct.unpack("<IIII", header)
            packet = f.read(captured)
            if len(packet) < 20 or packet[0] >> 4 != 4 or packet[9] != 6:
                continue
            ihl = (packet[0] & 15) * 4
            if len(packet) < ihl + 14:
                continue
            flags = packet[ihl + 13]
            if not flags & 2 or flags & 16:
                continue
            ns = seconds * 10**9 + micros * 1000
            first = ns if first is None else first
            last = ns
            source = struct.unpack(">I", packet[12:16])[0]
            sources.add(source)
            out.writerow([ns, "instance1", source])
            rows += 1
    handle.close()
    print(f"{path}: {rows} SYN-only packets from {len(sources)} sources over {(last - first) / 1e9:.1f} s")


def autoscaling(containers=200, seed=7, cores=16, memory_bytes=64e9):
    source = os.path.join(DATASETS, "google-cluster-2011/task_usage/part-00262-of-00500.csv.gz")
    records = {}
    with gzip.open(source, "rt") as f:
        for r in csv.reader(f):
            records.setdefault((r[2], r[3]), []).append((int(r[0]), float(r[5] or 0), float(r[6] or 0)))
    periods = {k: len({r[0] // 300_000_000 for r in v}) for k, v in records.items()}
    longest = max(periods.values())
    whole = sorted(k for k, n in periods.items() if n >= longest - 1)
    chosen = random.Random(seed).sample(whole, containers)
    paths = []
    for metric, column, scale in (("container_cpu", 1, cores), ("container_memory", 2, memory_bytes)):
        path, handle, out = writer(f"uc2/google_2011_{metric}.csv.gz")
        out.writerow(["timestamp", "container", "dimension", "value"])
        for job, task in chosen:
            for record in records[(job, task)]:
                out.writerow([record[0] * 1000, f"{job}-{task}", "used", record[column] * scale])
        handle.close()
        paths.append(path)
    print(f"{paths}: {containers} of {len(whole)} tasks present throughout ({len(records)} tasks in the part), "
          f"at least {longest - 1} of {longest} five-minute periods each; cpu_rate x {cores} cores, memory x {memory_bytes:.0f} bytes")


INCIDENTS = (
    (30, 60, 10, ("scale", 1.08)),
    (70, 110, 10, ("scale", 1.25)),
    (160, 190, 0, ("pareto", 0.05)),
    (200, 230, 0, ("fixed", 0.087)),
)


def intensity(minute, start, end, ramp):
    if minute < start or minute >= end:
        return 0.0
    if ramp == 0:
        return 1.0
    return min(1.0, (minute - start) / ramp, (end - minute) / ramp)


def canary_latency(minute, latency, rng):
    for start, end, ramp, (kind, amount) in INCIDENTS:
        w = intensity(minute, start, end, ramp)
        if w == 0:
            continue
        if kind == "scale":
            latency *= 1 + w * (amount - 1)
        elif kind == "pareto" and rng.random() < amount * w:
            latency = 2.0 / (1 - rng.random()) ** (1 / 3)
        elif kind == "fixed" and rng.random() < amount * w:
            latency = 1.0
    return latency


def canary(service="091794afdcf1abbaf62404337ff72a38df96c5d8af8bd0d1348b331e84ac89e5", stretch=50, share=0.05,
           seed=11):
    source = os.path.join(DATASETS, "alibaba-microservices-v2021/MSCallGraph/MSCallGraph_0.tar.gz")
    rng = random.Random(seed)
    calls = []
    with tarfile.open(source) as archive:
        member = next(m for m in archive.getmembers() if m.name.endswith(".csv"))
        reader = csv.reader(io.TextIOWrapper(archive.extractfile(member)))
        header = next(reader)
        ts, trace, rpc, dm, rt = (header.index(c) for c in ("timestamp", "traceid", "rpcid", "dm", "rt"))
        for r in reader:
            if r[dm] != service:
                continue
            try:
                latency_ms = float(r[rt])
            except ValueError:
                continue
            if latency_ms > 0:
                calls.append((int(r[ts]), r[trace] + r[rpc], latency_ms / 1000))
    calls.sort()
    first = calls[0][0]
    path, handle, out = writer("uc4/alibaba_2021_latency.csv.gz")
    out.writerow(["timestamp", "version", "value"])
    counts = {"stable": 0, "canary": 0}
    for ms, key, latency in calls:
        version = "canary" if int(hashlib.sha1(key.encode()).hexdigest()[:8], 16) < share * 2**32 else "stable"
        if version == "canary":
            latency = canary_latency((ms - first) * stretch / 60000, latency, rng)
        out.writerow([ms * 10**6, version, latency])
        counts[version] += 1
    handle.close()
    span = (calls[-1][0] - first) / 60000
    print(f"{path}: {len(calls)} caller-side calls with rt > 0 ms to one service over {span:.2f} min "
          f"({span * stretch:.0f} min stretched x{stretch}); {counts}; canary incidents as in the synthetic run")


LOG = re.compile(r'^(\S+) \S+ \S+ \[([^\]]+)\]')


def top_clients():
    source = os.path.join(DATASETS, "nasa-http/NASA_access_log_Jul95.gz")
    path, handle, out = writer("uc5/nasa_jul95_requests.csv.gz")
    out.writerow(["timestamp", "client_id"])
    ids, rows, first, last = {}, 0, None, None
    with gzip.open(source, "rt", encoding="latin-1") as f:
        for line in f:
            m = LOG.match(line)
            if not m:
                continue
            ns = int(datetime.strptime(m.group(2), "%d/%b/%Y:%H:%M:%S %z").timestamp()) * 10**9
            client = ids.setdefault(m.group(1), len(ids))
            first = ns if first is None else min(first, ns)
            last = ns if last is None else max(last, ns)
            out.writerow([ns, client])
            rows += 1
    handle.close()
    print(f"{path}: {rows} requests from {len(ids)} hosts over {(last - first) / 86400e9:.2f} days")


CASES = {"1": syn_flood, "2": autoscaling, "4": canary, "5": top_clients}


def main():
    parser = argparse.ArgumentParser(description="Convert public traces into planeval import tables")
    parser.add_argument("use_case", nargs="+", choices=sorted(CASES))
    for case in parser.parse_args().use_case:
        CASES[case]()


if __name__ == "__main__":
    main()
