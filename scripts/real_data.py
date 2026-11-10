#!/usr/bin/env python3
"""Convert downloaded public traces into the CSV tables planeval imports for use cases 1, 2, 4 and 5."""

import argparse
import csv
import functools
import gzip
import hashlib
import io
import os
import random
import re
import struct
import sys
import tarfile
from datetime import datetime

import data_root

CAIDA = "equinix-chicago.dirA.20160121-125911.UTC.anon.pcap.gz"
TRACES = {
    "caida-2016": [CAIDA],
    "caida-2019": [f"caida-2019/equinix-nyc.dirA.20190117-{start}.UTC.anon.pcap.gz"
                   for start in ("125910", *(f"130{minute}00" for minute in range(9)))],
    "mawi-2019": ["mawi-2019/201901161400.pcap.gz"],
}
KEEP_EVERY = {"caida-2019": 48, "mawi-2019": 16}
RAW_IP, ETHERNET = 101, 1
VLAN = (0x8100, 0x88A8)
IPV4 = 0x0800
MINUTE_NS = 60 * 10**9


def writer(out_root, name):
    path = os.path.join(out_root, "converted", name)
    os.makedirs(os.path.dirname(path), exist_ok=True)
    handle = gzip.open(path, "wt", compresslevel=1, newline="")
    return path, handle, csv.writer(handle)


def ipv4_packets(root, files):
    for name in files:
        source = data_root.existing(os.path.join(root, name), root)
        with gzip.open(source, "rb") as f:
            header = f.read(24)
            magic = header[:4]
            endian = "<" if magic in (b"\xd4\xc3\xb2\xa1", b"\x4d\x3c\xb2\xa1") else ">"
            unit_ns = 1 if magic in (b"\x4d\x3c\xb2\xa1", b"\xa1\xb2\x3c\x4d") else 1000
            link = struct.unpack(endian + "I", header[20:24])[0]
            if link not in (RAW_IP, ETHERNET):
                sys.exit(f"{name}: link type {link} is neither raw IP ({RAW_IP}) nor Ethernet ({ETHERNET})")
            record = endian + "IIII"
            while True:
                head = f.read(16)
                if len(head) < 16:
                    break
                seconds, fraction, captured, _ = struct.unpack(record, head)
                packet = f.read(captured)
                if link == ETHERNET:
                    if len(packet) < 14:
                        continue
                    offset, ethertype = 14, int.from_bytes(packet[12:14], "big")
                    while ethertype in VLAN and len(packet) >= offset + 4:
                        ethertype = int.from_bytes(packet[offset + 2:offset + 4], "big")
                        offset += 4
                    if ethertype != IPV4:
                        continue
                    packet = packet[offset:]
                if len(packet) < 20 or packet[0] >> 4 != 4:
                    continue
                yield seconds * 10**9 + fraction * unit_ns, packet


def syn_only(packet):
    if packet[9] != 6:
        return False
    ihl = (packet[0] & 15) * 4
    if len(packet) < ihl + 14:
        return False
    flags = packet[ihl + 13]
    return bool(flags & 2) and not flags & 16


def syn_flood(root, out_root, trace="caida-2016"):
    name = "uc1/caida_syn_only.csv.gz" if trace == "caida-2016" else f"uc1/{trace.replace('-', '_')}_syn_only.csv.gz"
    path, handle, out = writer(out_root, name)
    out.writerow(["timestamp", "vm", "src_ip"])
    rows, sources, first, last = 0, set(), None, None
    for ns, packet in ipv4_packets(root, TRACES[trace]):
        if not syn_only(packet):
            continue
        first = ns if first is None else first
        last = ns
        source = struct.unpack(">I", packet[12:16])[0]
        sources.add(source)
        out.writerow([ns, "instance1", source])
        rows += 1
    handle.close()
    print(f"{path}: {rows} SYN-only packets from {len(sources)} sources over {(last - first) / 1e9:.1f} s of {trace}")


def autoscaling(root, out_root, containers=200, seed=7, cores=16, memory_bytes=64e9):
    source = data_root.existing(
        os.path.join(root, "google-cluster-2011/task_usage/part-00262-of-00500.csv.gz"), root)
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
        path, handle, out = writer(out_root, f"uc2/google_2011_{metric}.csv.gz")
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


def canary(root, out_root, service="091794afdcf1abbaf62404337ff72a38df96c5d8af8bd0d1348b331e84ac89e5", files=24, timeline=250,
           share=0.05, seed=11):
    rng = random.Random(seed)
    calls = []
    for part in range(files):
        source = data_root.existing(
            os.path.join(root, f"alibaba-microservices-v2021/MSCallGraph/MSCallGraph_{part}.tar.gz"), root)
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
                    key = (r[trace] + r[rpc]).encode()
                    is_canary = int(hashlib.sha1(key).hexdigest()[:8], 16) < share * 2**32
                    calls.append((int(r[ts]), is_canary, latency_ms / 1000))
    calls.sort()
    first = calls[0][0]
    span = (calls[-1][0] - first) / 60000
    stretch = timeline / span
    path, handle, out = writer(out_root, "uc4/alibaba_2021_latency.csv.gz")
    out.writerow(["timestamp", "version", "value"])
    counts = {"stable": 0, "canary": 0}
    for ms, is_canary, latency in calls:
        version = "canary" if is_canary else "stable"
        if is_canary:
            latency = canary_latency((ms - first) * stretch / 60000, latency, rng)
        out.writerow([ms * 10**6, version, latency])
        counts[version] += 1
    handle.close()
    print(f"{path}: {len(calls)} caller-side calls with rt > 0 ms to one service in {files} files over {span:.2f} min "
          f"(stretch x{stretch:.3f} to {timeline} min); {counts}; canary incidents as in the synthetic run")


LOG = re.compile(r'^(\S+) \S+ \S+ \[([^\]]+)\]')


def top_clients(root, out_root):
    source = data_root.existing(os.path.join(root, "nasa-http/NASA_access_log_Jul95.gz"), root)
    path, handle, out = writer(out_root, "uc5/nasa_jul95_requests.csv.gz")
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


def top_clients_packets(root, out_root, trace="caida-2016", keep_every=3):
    name = "uc5/caida_src_ip.csv.gz" if trace == "caida-2016" else f"uc5/{trace.replace('-', '_')}_src_ip.csv.gz"
    path, handle, out = writer(out_root, name)
    out.writerow(["timestamp", "client_id"])
    index, rows, sources, first, last = 0, 0, set(), None, None
    for ns, packet in ipv4_packets(root, TRACES[trace]):
        index += 1
        if index % keep_every:
            continue
        first = ns if first is None else first
        last = ns
        source = struct.unpack(">I", packet[12:16])[0]
        sources.add(source)
        out.writerow([ns, source])
        rows += 1
    handle.close()
    print(f"{path}: every {keep_every}th of {index} IPv4 packets of {trace}, {rows} rows from {len(sources)} sources "
          f"over {(last - first) / 1e9:.1f} s")


CASES = {
    "1": syn_flood,
    "1-caida-2019": functools.partial(syn_flood, trace="caida-2019"),
    "1-mawi-2019": functools.partial(syn_flood, trace="mawi-2019"),
    "2": autoscaling,
    "4": canary,
    "5": top_clients,
    "5-caida": top_clients_packets,
    "5-caida-2019": functools.partial(top_clients_packets, trace="caida-2019", keep_every=KEEP_EVERY["caida-2019"]),
    "5-mawi-2019": functools.partial(top_clients_packets, trace="mawi-2019", keep_every=KEEP_EVERY["mawi-2019"]),
}


def main():
    parser = argparse.ArgumentParser(description="Convert public traces into planeval import tables")
    parser.add_argument("use_case", nargs="+", choices=sorted(CASES))
    parser.add_argument("--data-root", default=data_root.default(),
                        help=f"where the downloaded traces are; default ${data_root.ENV}, else datasets/ in the repository")
    parser.add_argument("--out-root", help="where converted/ is written; default --data-root")
    args = parser.parse_args()
    for case in args.use_case:
        CASES[case](args.data_root, args.out_root or args.data_root)


if __name__ == "__main__":
    main()
