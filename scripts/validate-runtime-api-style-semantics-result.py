#!/usr/bin/env python3
"""Recompute the exhaustive WebIDL/style/semantics WPT gate from shards."""

from __future__ import annotations

import argparse
import hashlib
import json
import re
from wpt_evidence import completion_errors, resource_integrity_errors
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
INVENTORY = ROOT / "benchmarks/conformance/runtime-api-style-semantics-v2-inventory.json"
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


def validate_passed_assertion(assertion: dict[str, object], context: str, strict: bool = False) -> None:
    if assertion.get("status") != "passed":
        return
    result = assertion.get("result")
    require(isinstance(result, dict), f"{context}: passing assertion lacks testharness result")
    require(result.get("complete") is True, f"{context}: testharness completion unverified")
    require(result.get("harness") == 0, f"{context}: testharness did not succeed")
    tests = result.get("tests")
    require(isinstance(tests, list) and tests, f"{context}: no testharness assertions")
    require(
        all(isinstance(test, dict) and test.get("status") == 0 for test in tests),
        f"{context}: failing testharness assertion reported as passed",
    )
    require(
        assertion.get("result_transport") != "isolated_dom_probe",
        f"{context}: diagnostic probe cannot establish conformance",
    )
    if strict:
        execution = assertion.get('execution', {})
        shutdown = execution.get('parent_shutdown', {})
        require(not completion_errors(result) and result.get('completion_count') == 1,
                f'{context}: invalid structured completion')
        require(execution.get('completion_valid') is True and not execution.get('error')
                and not execution.get('cleanup_errors') and not execution.get('replacement_error')
                and shutdown.get('returncode') == 0 and shutdown.get('shutdown_acknowledged') is True
                and not shutdown.get('error'), f'{context}: execution integrity unverified')
        require(assertion.get('resource_evidence_complete') is True and not resource_integrity_errors(assertion),
                f'{context}: resource evidence unverified')
        require(assertion.get('qualification') == 'qualified',
                f'{context}: a hash-bound qualification audit is required')


def subtest_counts(assertion: dict[str, object]) -> tuple[int, int]:
    result = assertion.get("result")
    tests = result.get("tests") if isinstance(result, dict) else None
    if not isinstance(tests, list):
        return 0, 0
    statuses = [test.get("status") for test in tests if isinstance(test, dict)]
    return sum(status == 0 for status in statuses), len(statuses)


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
        require(shard.get("schema_version") == 3, f"{path}: current release evidence requires schema 3; older schemas are historical")
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
            validate_passed_assertion(assertion, f"{path}: {identifier}", strict=True)
            assertions[identifier] = assertion
    require(shard_count is not None, "no shards supplied")
    require(shard_indices == set(range(shard_count)), "shard set is incomplete")
    missing = expected.difference(assertions)
    require(not missing, f"missing {len(missing)} required assertions; first={next(iter(sorted(missing)), '')}")
    failed = sorted(identifier for identifier, assertion in assertions.items() if assertion["status"] != "passed")
    by_workstream: dict[str, dict[str, int]] = {}
    subtests = {"passed": 0, "observed": 0, "executions_without_observed_subtests": 0}
    for assertion in assertions.values():
        bucket = by_workstream.setdefault(
            str(assertion["classification"]),
            {
                "required": 0,
                "passed": 0,
                "subtests_passed": 0,
                "subtests_observed": 0,
                "executions_without_observed_subtests": 0,
            },
        )
        bucket["required"] += 1
        bucket["passed"] += int(assertion["status"] == "passed")
        passed_subtests, observed_subtests = subtest_counts(assertion)
        bucket["subtests_passed"] += passed_subtests
        bucket["subtests_observed"] += observed_subtests
        bucket["executions_without_observed_subtests"] += int(observed_subtests == 0)
        subtests["passed"] += passed_subtests
        subtests["observed"] += observed_subtests
        subtests["executions_without_observed_subtests"] += int(observed_subtests == 0)
    subtests["failed"] = subtests["observed"] - subtests["passed"]
    result = {
        "schema_version": 1,
        "inventory_sha256": inventory_sha,
        "upstream_revision": inventory["upstream_revision"],
        "required_total": len(expected),
        "required_passed": len(expected) - len(failed),
        "required_failed": len(failed),
        "by_workstream": dict(sorted(by_workstream.items())),
        "subtests": subtests,
        "outcome": {"status": "passed" if not failed else "failed"},
        "failures": failed,
    }
    if args.output:
        args.output.parent.mkdir(parents=True, exist_ok=True)
        args.output.write_text(json.dumps(result, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    print(
        json.dumps(
            {
                key: result[key]
                for key in ["required_total", "required_passed", "required_failed", "subtests", "outcome"]
            },
            sort_keys=True,
        )
    )
    return 0 if not failed else 1


if __name__ == "__main__":
    raise SystemExit(main())
