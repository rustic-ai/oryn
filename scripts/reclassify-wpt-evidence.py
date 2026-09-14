#!/usr/bin/env python3
"""Project immutable WPT outcomes onto a new exact-ID coverage inventory.

This does not rerun tests or remove the original evidence's harness defects.
"""
import argparse
from collections import Counter
import hashlib
import json
from pathlib import Path


def project(coverage, assertions):
    results = {}
    for row in assertions:
        if row["id"] in results:
            raise ValueError("duplicate result ID: " + row["id"])
        results[row["id"]] = row
    counts = Counter()
    matched = set()
    for row in coverage:
        result = results.get(row.get("runner_id"))
        status = row["status"]
        if result is not None:
            status = result["status"]
            matched.add(row["runner_id"])
        counts[(row["tier"], status)] += 1
    if results.keys() - matched:
        raise ValueError("results absent from new inventory")
    return [{"tier": tier, "status": status, "count": count}
            for (tier, status), count in sorted(counts.items())]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--coverage", type=Path, required=True)
    parser.add_argument("--results", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    if args.output.exists():
        parser.error("output exists; choose a new evidence path")
    sources, assertions = [], []
    for path in sorted(args.results.glob("*.json")):
        data = json.loads(path.read_text())
        if "assertions" not in data:
            continue
        assertions.extend(data["assertions"])
        sources.append({"path": str(path.resolve()), "sha256": hashlib.sha256(path.read_bytes()).hexdigest()})
    coverage = [json.loads(line) for line in args.coverage.read_text().splitlines()]
    output = {
        "qualification": "Reclassification only, not a rerun. All original harness defects and evidence qualifications still apply.",
        "coverage_sha256": hashlib.sha256(args.coverage.read_bytes()).hexdigest(),
        "sources": sources, "matched_executions": len(assertions),
        "counts": project(coverage, assertions),
    }
    args.output.write_text(json.dumps(output, indent=2) + "\n")


if __name__ == "__main__":
    main()
