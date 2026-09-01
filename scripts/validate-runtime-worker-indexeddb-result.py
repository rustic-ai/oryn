#!/usr/bin/env python3
"""Recompute the compact production-worker IndexedDB WPT result."""

from __future__ import annotations

import argparse
import hashlib
import json
import re
import subprocess
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
INVENTORY = ROOT / "benchmarks/conformance/runtime-api-style-semantics-v1-inventory.json"
HEX = re.compile(r"^[0-9a-f]{64}$")
TEST_PREFIX = "IndexedDB/reading-autoincrement"
GLOBALS = {"sharedworker", "serviceworker"}


def require(condition: bool, message: str) -> None:
    if not condition:
        raise SystemExit(message)


def digest(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("result", type=Path)
    args = parser.parse_args()
    result = json.loads(args.result.read_text(encoding="utf-8"))
    require(result.get("schema_version") == 1, "schema_version must be 1")

    source = result.get("source", {})
    commit = source.get("commit", "")
    require(re.fullmatch(r"[0-9a-f]{40}", commit) is not None, "invalid source commit")
    require(source.get("dirty") is False, "source candidate was dirty")
    require(HEX.fullmatch(source.get("source_tree_sha256", "")) is not None, "invalid source hash")
    subprocess.run(["git", "cat-file", "-e", f"{commit}^{{commit}}"], cwd=ROOT, check=True)

    inventory = json.loads(INVENTORY.read_text(encoding="utf-8"))
    upstream = result.get("upstream", {})
    require(upstream.get("revision") == inventory.get("upstream_revision"), "WPT revision mismatch")
    require(upstream.get("inventory_sha256") == digest(INVENTORY), "inventory hash mismatch")
    expected = {
        f"{record['path']}|{global_name}|{variant}"
        for record in inventory["tests"]
        if record["path"].startswith(TEST_PREFIX)
        for global_name in record["globals"]
        if global_name in GLOBALS
        for variant in record["variants"]
    }
    require(len(expected) == 8, f"inventory defines {len(expected)} worker assertions, expected 8")

    artifact = result.get("artifact", {})
    require(artifact.get("execution_mode") == "sandboxed_worker", "not production worker evidence")
    for field in ["parent_sha256", "worker_sha256", "worker_manifest_sha256", "entitlement_sha256"]:
        require(HEX.fullmatch(artifact.get(field, "")) is not None, f"invalid artifact.{field}")
    require(artifact.get("parent_bytes", 0) > 0 and artifact.get("worker_bytes", 0) > 0, "invalid sizes")
    require(artifact.get("bundle_id") == "ai.rustic.oryn.page-worker", "wrong bundle id")
    require(artifact.get("team_id") == "5HVA9VFF8K", "wrong Team ID")

    runtime = result.get("runtime", {})
    require(
        runtime.get("v8_heap_sandbox") is True
        and runtime.get("streaming_parser") is True
        and runtime.get("transport_mode") == "async_streaming_parent_broker"
        and runtime.get("style_engine_version") == "stylo-computed-values-0.20+taffy-grid-v1",
        "runtime fingerprint is not the completed production profile",
    )
    for field in ["webidl_manifest_sha256", "icu_data_sha256", "startup_snapshot_sha256", "external_references_sha256"]:
        require(HEX.fullmatch(runtime.get(field, "")) is not None, f"invalid runtime.{field}")

    sandbox = result.get("sandbox", {})
    require(sandbox.get("state") == "app_sandbox", "worker was not App Sandbox isolated")
    for field in ["filesystem_read_succeeded", "outgoing_connection_succeeded", "listener_succeeded", "secret_environment_present"]:
        require(sandbox.get(field) is False, f"sandbox denial failed: {field}")

    assertions = result.get("assertions", [])
    by_id = {item.get("id"): item for item in assertions}
    require(len(by_id) == len(assertions), "duplicate assertion id")
    require(set(by_id) == expected, "worker assertion set differs from immutable inventory")
    passed = sum(item.get("status") == "passed" for item in assertions)
    require(passed == len(assertions), "one or more worker assertions failed")
    outcome = result.get("outcome", {})
    require(outcome == {"required_total": 8, "required_passed": 8, "required_failed": 0, "status": "passed"}, "aggregate was not recomputed correctly")
    print(json.dumps({"required_total": 8, "required_passed": passed, "status": "passed"}, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
