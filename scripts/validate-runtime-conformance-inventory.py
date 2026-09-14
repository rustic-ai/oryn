#!/usr/bin/env python3
"""Validate the frozen runtime API/style/semantics WPT inventory."""

from __future__ import annotations

import argparse
import hashlib
import json
import re
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
MANIFEST = ROOT / "benchmarks/conformance/runtime-api-style-semantics-v2.json"
HEX = re.compile(r"^[0-9a-f]{64}$")


def digest(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def require(condition: bool, message: str) -> None:
    if not condition:
        raise SystemExit(message)


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("inventory", type=Path)
    parser.add_argument("--manifest", type=Path, default=MANIFEST)
    args = parser.parse_args()
    manifest_path = args.manifest.resolve()
    manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
    inventory = json.loads(args.inventory.read_text(encoding="utf-8"))
    require(inventory.get("schema_version") == 1, "inventory schema_version must be 1")
    require(inventory.get("manifest_sha256") == digest(manifest_path), "manifest hash mismatch")
    require(inventory.get("upstream_revision") == manifest["upstream"]["revision"], "WPT revision mismatch")
    tests = inventory.get("tests")
    require(isinstance(tests, list) and tests, "tests must be a non-empty list")
    paths: set[str] = set()
    counts: dict[str, int] = {}
    global_counts: dict[str, int] = {}
    required_executions = 0
    allowed_workstreams = {entry["workstream"] for entry in manifest["roots"]}
    allowed_globals = set(
        manifest["selection"].get(
            "allowed_globals",
            ["window", "dedicatedworker", "sharedworker", "serviceworker"],
        )
    )
    for test in tests:
        path = test.get("path")
        require(isinstance(path, str) and path and path not in paths, f"duplicate or invalid path: {path}")
        paths.add(path)
        require(test.get("kind") in {"script", "html"}, f"invalid kind for {path}")
        require(test.get("workstream") in allowed_workstreams, f"invalid workstream for {path}")
        require(HEX.fullmatch(test.get("sha256", "")) is not None, f"invalid digest for {path}")
        require(isinstance(test.get("globals"), list) and test["globals"], f"missing globals for {path}")
        require(isinstance(test.get("variants"), list) and test["variants"], f"missing variants for {path}")
        counts[test["workstream"]] = counts.get(test["workstream"], 0) + 1
        for global_name in test["globals"]:
            require(
                global_name in allowed_globals,
                f"invalid global {global_name!r} for {path}",
            )
            variants = len(test["variants"])
            global_counts[global_name] = global_counts.get(global_name, 0) + variants
            required_executions += variants
    require(inventory.get("required_total") == len(tests), "required_total mismatch")
    require(inventory.get("required_by_workstream") == dict(sorted(counts.items())), "workstream counts mismatch")
    require(inventory.get("required_execution_total") == required_executions, "execution total mismatch")
    require(inventory.get("required_by_global") == dict(sorted(global_counts.items())), "global counts mismatch")
    print(
        json.dumps(
            {
                "required_files": len(tests),
                "required_executions": required_executions,
                "by_workstream": counts,
                "by_global": global_counts,
                "status": "valid",
            },
            sort_keys=True,
        )
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
