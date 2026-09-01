#!/usr/bin/env python3
"""Recompute the exhaustive WebIDL/style/semantics WPT gate from shards."""

from __future__ import annotations

import argparse
import hashlib
import json
import re
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
INVENTORY = ROOT / "benchmarks/conformance/runtime-api-style-semantics-v1-inventory.json"
HEX = re.compile(r"^[0-9a-f]{64}$")


def digest(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def require(condition: bool, message: str) -> None:
    if not condition:
        raise SystemExit(message)


def expected_assertions(inventory: dict[str, object]) -> set[str]:
    expected: set[str] = set()
    for test in inventory["tests"]:
        for global_name in test["globals"]:
            for variant in test["variants"]:
                expected.add(f"{test['path']}|{global_name}|{variant}")
    return expected


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("shards", nargs="+", type=Path)
    parser.add_argument("--output", type=Path)
    parser.add_argument("--inventory", type=Path, default=INVENTORY)
    args = parser.parse_args()
    inventory_path = args.inventory.resolve()
    inventory = json.loads(inventory_path.read_text(encoding="utf-8"))
    inventory_sha = digest(inventory_path)
    expected = expected_assertions(inventory)
    assertions: dict[str, dict[str, object]] = {}
    shard_count: int | None = None
    shard_indices: set[int] = set()
    release_fingerprint: tuple[object, ...] | None = None
    for path in args.shards:
        shard = json.loads(path.read_text(encoding="utf-8"))
        require(shard.get("schema_version") == 2, f"{path}: schema_version must be 2")
        require(
            shard.get("execution_mode") == "sandboxed_worker",
            f"{path}: release evidence must use the sandboxed worker",
        )
        binary = shard.get("binary", {})
        require(
            HEX.fullmatch(str(binary.get("sha256", ""))) is not None
            and isinstance(binary.get("size_bytes"), int)
            and binary["size_bytes"] > 0,
            f"{path}: invalid binary fingerprint",
        )
        runtime = shard.get("runtime_metadata")
        require(isinstance(runtime, dict), f"{path}: missing runtime metadata")
        require(
            runtime.get("native_v8") is True
            and runtime.get("sandboxed") is True
            and runtime.get("v8_heap_sandbox") is True
            and runtime.get("streaming_parser") is True
            and runtime.get("transport_mode") == "async_streaming_parent_broker"
            and isinstance(runtime.get("worker_binary_sha256"), str),
            f"{path}: runtime metadata does not describe the hardened production engine",
        )
        source = shard.get("source", {})
        require(
            isinstance(source.get("commit"), str) and not source.get("dirty", True),
            f"{path}: evidence source tree must be clean",
        )
        fingerprint = (
            binary.get("sha256"),
            binary.get("size_bytes"),
            json.dumps(shard["runtime_metadata"], sort_keys=True),
            source.get("commit"),
        )
        if release_fingerprint is None:
            release_fingerprint = fingerprint
        require(fingerprint == release_fingerprint, f"{path}: release fingerprint differs across shards")
        require(shard.get("inventory_sha256") == inventory_sha, f"{path}: inventory hash mismatch")
        descriptor = shard.get("shard", {})
        count = descriptor.get("count")
        index = descriptor.get("index")
        require(isinstance(count, int) and count > 0, f"{path}: invalid shard count")
        require(isinstance(index, int) and 0 <= index < count, f"{path}: invalid shard index")
        if shard_count is None:
            shard_count = count
        require(count == shard_count, f"{path}: inconsistent shard count")
        require(index not in shard_indices, f"{path}: duplicate shard index {index}")
        shard_indices.add(index)
        for assertion in shard.get("assertions", []):
            identifier = assertion.get("id")
            require(identifier in expected, f"{path}: unexpected assertion {identifier}")
            require(identifier not in assertions, f"{path}: duplicate assertion {identifier}")
            require(assertion.get("status") in {"passed", "failed"}, f"{path}: invalid status for {identifier}")
            assertions[identifier] = assertion
    require(shard_count is not None, "no shards supplied")
    require(shard_indices == set(range(shard_count)), "shard set is incomplete")
    missing = expected.difference(assertions)
    require(not missing, f"missing {len(missing)} required assertions; first={next(iter(sorted(missing)), '')}")
    failed = sorted(identifier for identifier, assertion in assertions.items() if assertion["status"] != "passed")
    by_workstream: dict[str, dict[str, int]] = {}
    for assertion in assertions.values():
        bucket = by_workstream.setdefault(str(assertion["classification"]), {"required": 0, "passed": 0})
        bucket["required"] += 1
        bucket["passed"] += int(assertion["status"] == "passed")
    result = {
        "schema_version": 1,
        "inventory_sha256": inventory_sha,
        "upstream_revision": inventory["upstream_revision"],
        "required_total": len(expected),
        "required_passed": len(expected) - len(failed),
        "required_failed": len(failed),
        "by_workstream": dict(sorted(by_workstream.items())),
        "outcome": {"status": "passed" if not failed else "failed"},
        "failures": failed,
    }
    if args.output:
        args.output.parent.mkdir(parents=True, exist_ok=True)
        args.output.write_text(json.dumps(result, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    print(json.dumps({key: result[key] for key in ["required_total", "required_passed", "required_failed", "outcome"]}, sort_keys=True))
    return 0 if not failed else 1


if __name__ == "__main__":
    raise SystemExit(main())
