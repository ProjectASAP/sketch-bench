#!/usr/bin/env python3
"""Snapshot on-demand EC2 prices for the rqe-optimizer machine families.

Reads the public price file behind aws.amazon.com/ec2/pricing/on-demand
(us-east-1, Linux, shared tenancy) and writes the three instance types the
optimizer prices plans with. Never edit the output by hand; rerun this.

Usage:
    scripts/fetch_ec2_pricing.py rqe-optimizer/data/ec2-pricing-$(date +%F).json
"""
import datetime
import gzip
import json
import sys
import urllib.request

SOURCE = (
    "https://b0.p.awsstatic.com/pricing/2.0/meteredUnitMaps/ec2/USD/current/"
    "ec2-ondemand-without-sec-sel/US%20East%20(N.%20Virginia)/Linux/index.json"
)
REGION = "US East (N. Virginia)"
# One size per family: plans are priced in fractional instances, so the size
# within a family does not change the result.
INSTANCE_TYPES = {
    "compute_optimized": "c7i.xlarge",
    "general_purpose": "m7i.xlarge",
    "memory_optimized": "r7i.xlarge",
}


def main(out_path):
    with urllib.request.urlopen(SOURCE) as response:
        body = response.read()
    if body[:2] == b"\x1f\x8b":
        body = gzip.decompress(body)
    data = json.loads(body)
    rows = {row["Instance Type"]: row for row in data["regions"][REGION].values()}

    families = []
    for family, instance_type in INSTANCE_TYPES.items():
        row = rows[instance_type]
        memory, unit = row["Memory"].split()
        assert unit == "GiB", row["Memory"]
        families.append(
            {
                "family": family,
                "instance_type": instance_type,
                "vcpu": int(row["vCPU"]),
                "memory_gib": float(memory),
                "usd_per_hour": float(row["price"]),
                "rate_code": row["rateCode"],
            }
        )

    snapshot = {
        "source": SOURCE,
        "region": REGION,
        "operating_system": "Linux",
        "pricing": "on-demand",
        "publication_date": data["manifest"]["hawkFilePublicationDate"],
        "fetched": datetime.date.today().isoformat(),
        "families": families,
    }
    with open(out_path, "w") as out:
        json.dump(snapshot, out, indent=2)
        out.write("\n")


if __name__ == "__main__":
    main(sys.argv[1])
