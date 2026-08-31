#!/usr/bin/env python3
"""Validate execution domains/replay compact evidence and optionally recompute raw artifact bindings."""

from __future__ import annotations

import argparse
import hashlib
import json
import subprocess
from pathlib import Path
from typing import Any

from jsonschema import Draft202012Validator, FormatChecker


def load(path: Path) -> dict[str, Any]:
    value = json.loads(path.read_text(encoding="utf-8"))
    if not isinstance(value, dict):
        raise ValueError(f"{path} must contain an object")
    return value


def digest(path: Path) -> str:
    value = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            value.update(chunk)
    return value.hexdigest()


def require(condition: bool, message: str) -> None:
    if not condition:
        raise ValueError(message)


def main() -> int:
    root_default = Path(__file__).resolve().parents[1]
    parser = argparse.ArgumentParser()
    parser.add_argument("result", type=Path)
    parser.add_argument("--repo-root", type=Path, default=root_default)
    parser.add_argument("--artifact-root", type=Path)
    parser.add_argument("--require-raw", action="store_true")
    args = parser.parse_args()
    root = args.repo_root.resolve()
    result = load(args.result.resolve())
    legacy_label = result.get("gate")
    schema_names = {"G6": "execution-domains", "G7": "replay"}
    probe_names = {"G6": "execution-domain-probe.json", "G7": "replay-probe.json"}
    require(
        legacy_label in schema_names,
        "result capability must be execution domains or replay",
    )
    schema = load(
        root / f"benchmarks/schema/{schema_names[legacy_label]}-result-v1.schema.json"
    )
    Draft202012Validator.check_schema(schema)
    errors = sorted(
        Draft202012Validator(schema, format_checker=FormatChecker()).iter_errors(
            result
        ),
        key=lambda error: list(error.path),
    )
    require(
        not errors,
        "schema validation failed: " + "; ".join(error.message for error in errors),
    )
    manifest_path = (
        root / f"benchmarks/conformance/{schema_names[legacy_label]}-manifest-v1.json"
    )
    manifest = load(manifest_path)
    require(
        result["fingerprint"]["manifest_sha256"] == digest(manifest_path),
        "manifest hash mismatch",
    )
    expected = {(case["id"], case["category"]) for case in manifest["cases"]}
    actual = {(case["id"], case["category"]) for case in result["case_ledger"]}
    require(actual == expected, "case ledger does not match frozen manifest")
    require(
        all(case["status"] == "passed" for case in result["case_ledger"]),
        "not every required case passed",
    )
    require(all(result["criteria"].values()), "one or more gate criteria failed")
    require(
        result["outcome"]["status"] == "passed",
        f"{schema_names[legacy_label]} is not passed",
    )
    if args.require_raw:
        require(args.artifact_root is not None, "--require-raw needs --artifact-root")
        raw = args.artifact_root.resolve()
        artifacts = {item["id"]: item for item in result["artifact_ledger"]}
        for item in artifacts.values():
            path = raw / item["path"]
            require(path.is_file(), f"missing raw artifact: {item['path']}")
            require(
                path.stat().st_size == item["bytes"],
                f"artifact size mismatch: {item['path']}",
            )
            require(
                digest(path) == item["sha256"],
                f"artifact hash mismatch: {item['path']}",
            )
        referenced = {
            identifier
            for case in result["case_ledger"]
            for identifier in case["evidence"]
        }
        require(
            referenced <= artifacts.keys(), "case ledger references unknown artifacts"
        )
        runtime = load(raw / "runtime.json")
        require(
            runtime.get("native_v8") is True
            and runtime.get("sandbox_state") == "app_sandbox",
            "raw runtime is not sandboxed V8",
        )
        require(
            load(raw / "security/security-result-v2.json")
            .get("outcome", {})
            .get("status")
            == "passed",
            "raw security regression failed",
        )
        require(
            load(raw / probe_names[legacy_label]).get("outcome", {}).get("status")
            == "passed",
            f"raw {schema_names[legacy_label]} probe failed",
        )
        current_tree = subprocess.check_output(
            [str(root / "scripts/source-tree-hash.sh")], text=True
        ).strip()
        require(
            current_tree == result["fingerprint"]["source_tree_sha256"],
            "source-tree hash mismatch",
        )
    print(
        f"{schema_names[legacy_label]} result is schema-valid and recomputes to passed"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
