#!/usr/bin/env python3
"""Recompute and validate runtime-completion-v1 compact evidence."""

from __future__ import annotations

import argparse
import hashlib
import json
import re
import subprocess
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
MANIFEST = ROOT / "benchmarks/conformance/runtime-completion-v1.json"
HEX = re.compile(r"^[0-9a-f]{64}$")


def digest(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def source_tree_hash() -> str:
    return subprocess.check_output(
        [str(ROOT / "scripts/source-tree-hash.sh")], cwd=ROOT, text=True
    ).strip()


def require(condition: bool, message: str) -> None:
    if not condition:
        raise SystemExit(message)


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("result", type=Path)
    parser.add_argument("--allow-historical-tree", action="store_true")
    args = parser.parse_args()
    result = json.loads(args.result.read_text())
    require(result.get("schema_version") == 1, "schema_version must be 1")
    require(result.get("manifest_sha256") == digest(MANIFEST), "manifest hash mismatch")
    require(HEX.fullmatch(result.get("source_tree_sha256", "")) is not None, "invalid source tree hash")
    if not args.allow_historical_tree:
        require(result["source_tree_sha256"] == source_tree_hash(), "source tree hash mismatch")

    runtime = result.get("runtime", {})
    for field in [
        "parent_sha256", "worker_sha256", "icu_data_sha256", "snapshot_sha256",
        "webidl_manifest_sha256",
    ]:
        require(HEX.fullmatch(runtime.get(field, "")) is not None, f"invalid runtime.{field}")
    require(runtime.get("v8_heap_sandbox") is True, "V8 heap sandbox is not attested")
    require(runtime.get("runtime_profile") in {"core_full", "core_lite_english"}, "invalid runtime profile")

    assertions = result.get("assertions")
    require(isinstance(assertions, list) and assertions, "assertions must be non-empty")
    ids: set[str] = set()
    for assertion in assertions:
        identifier = assertion.get("id")
        require(isinstance(identifier, str) and identifier and identifier not in ids, "duplicate or invalid assertion id")
        ids.add(identifier)
        require(assertion.get("classification") in {"required", "informational", "excluded"}, f"invalid classification for {identifier}")
        require(assertion.get("status") in {"passed", "failed", "unsupported", "not_run"}, f"invalid status for {identifier}")

    required = [item for item in assertions if item["classification"] == "required"]
    passed = sum(item["status"] == "passed" for item in required)
    outcome = result.get("outcome", {})
    require(outcome.get("required_total") == len(required), "required_total was not recomputed correctly")
    require(outcome.get("required_passed") == passed, "required_passed was not recomputed correctly")
    expected = "passed" if required and passed == len(required) else "failed"
    require(outcome.get("status") == expected, f"outcome.status must be {expected}")
    print(json.dumps({"assertions": len(assertions), "required": len(required), "status": expected}, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
