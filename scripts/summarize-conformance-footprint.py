#!/usr/bin/env python3
"""Summarize a frozen WPT inventory against published competitor footprints."""

from __future__ import annotations

import argparse
import json
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
MANIFEST = ROOT / "benchmarks/conformance/agent-browser-conformance-v1.json"
INVENTORY = ROOT / "benchmarks/conformance/agent-browser-conformance-v1-inventory.json"


def matches_prefix(path: str, prefix: str) -> bool:
    return not prefix or path == prefix or path.startswith(prefix + "/")


def matches_profile(path: str, profile: dict[str, object]) -> bool:
    included = any(matches_prefix(path, prefix) for prefix in profile["include_prefixes"])
    excluded = any(matches_prefix(path, prefix) for prefix in profile["exclude_prefixes"])
    return included and not excluded


def execution_count(test: dict[str, object]) -> int:
    return len(test["globals"]) * len(test["variants"])


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--manifest", type=Path, default=MANIFEST)
    parser.add_argument("--inventory", type=Path, default=INVENTORY)
    args = parser.parse_args()
    manifest = json.loads(args.manifest.read_text(encoding="utf-8"))
    inventory = json.loads(args.inventory.read_text(encoding="utf-8"))
    profiles = {}
    for name, profile in manifest["published_comparison_profiles"].items():
        tests = [test for test in inventory["tests"] if matches_profile(test["path"], profile)]
        profiles[name] = {
            "files": len(tests),
            "executions": sum(execution_count(test) for test in tests),
            "reported_unit": profile["reported_unit"],
            "source": profile["source"],
        }
    result = {
        "suite": manifest["suite"],
        "upstream_revision": inventory["upstream_revision"],
        "files": inventory["required_total"],
        "executions": inventory["required_execution_total"],
        "by_workstream": inventory["required_by_workstream"],
        "profiles": profiles,
    }
    print(json.dumps(result, indent=2, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
