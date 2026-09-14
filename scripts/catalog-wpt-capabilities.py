#!/usr/bin/env python3
"""Map the entire pinned Git tree, including paths absent from sparse checkout.

Source-file counts are NOT runnable test counts. Official WPT MANIFEST.json
enumeration is required before claiming a complete execution inventory.
"""
from __future__ import annotations

import argparse
from collections import Counter
import hashlib
import json
from pathlib import Path
import subprocess

ROOT = Path(__file__).resolve().parents[1]


def classify(path: str, policy: dict) -> tuple[str, bool]:
    matches = [(len(prefix), rule["tier"]) for rule in policy["rules"]
               for prefix in rule["prefixes"]
               if path == prefix or path.startswith(prefix + "/")]
    return (max(matches)[1], False) if matches else (policy["default_tier"], True)


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--checkout", type=Path, required=True)
    parser.add_argument("--policy", type=Path, default=ROOT / "benchmarks/conformance/wpt-capability-tiers-v1.json")
    parser.add_argument("--inventory", type=Path, default=ROOT / "benchmarks/conformance/agent-browser-conformance-v1-inventory.json")
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    policy = json.loads(args.policy.read_text())
    inventory = json.loads(args.inventory.read_text())
    revision = subprocess.check_output(["git", "-C", str(args.checkout), "rev-parse", "HEAD"], text=True).strip()
    if revision != policy["upstream_revision"] or inventory["upstream_revision"] != revision:
        raise SystemExit("revision mismatch")
    paths = subprocess.check_output(["git", "-C", str(args.checkout), "ls-tree", "-r", "--name-only", "HEAD"], text=True).splitlines()
    roots: dict[str, dict] = {}
    for path in paths:
        if "/" not in path:
            continue
        root = path.split("/", 1)[0]
        if root in policy["infrastructure_roots"]:
            continue
        row = roots.setdefault(root, {"tracked_source_files": 0, "source_files_by_tier": Counter(), "unclassified_source_files": 0, "initial_inventory_files": 0, "initial_inventory_executions": 0})
        tier, unclassified = classify(path, policy)
        row["tracked_source_files"] += 1
        row["source_files_by_tier"][tier] += 1
        row["unclassified_source_files"] += int(unclassified)
    tiers = {tier: {"files": 0, "executions": 0} for tier in policy["tiers"]}
    for test in inventory["tests"]:
        tier, _ = classify(test["path"], policy)
        executions = len(test["globals"]) * len(test["variants"])
        tiers[tier]["files"] += 1
        tiers[tier]["executions"] += executions
        row = roots[test["path"].split("/", 1)[0]]
        row["initial_inventory_files"] += 1
        row["initial_inventory_executions"] += executions
    result = {"schema_version": 1, "upstream_revision": revision,
              "policy_sha256": hashlib.sha256(args.policy.read_bytes()).hexdigest(),
              "initial_inventory_sha256": hashlib.sha256(args.inventory.read_bytes()).hexdigest(),
              "qualification": "Complete Git source-tree map, not a complete runnable WPT manifest or measured result. Initial inventory is a subset; source counts include support and reference files.",
              "initial_inventory_by_tier": tiers, "roots": dict(sorted(roots.items()))}
    args.output.write_text(json.dumps(result, indent=2, sort_keys=True) + "\n")
    print(json.dumps({"mapped_roots": len(roots), "initial_inventory_by_tier": tiers, "roots_absent_from_initial_inventory": sum(row["initial_inventory_files"] == 0 for row in roots.values())}, indent=2))


if __name__ == "__main__":
    main()
