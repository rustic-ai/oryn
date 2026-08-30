#!/usr/bin/env python3
"""Validate layout compact evidence and recompute its claims from raw artifacts."""

from __future__ import annotations

import argparse
import hashlib
import json
import subprocess
from pathlib import Path
from typing import Any

from jsonschema import Draft202012Validator, FormatChecker

from evidence_assertions import assertion_bound_cases, verified_regressions


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
    schema = load(root / "benchmarks/schema/layout-result-v1.schema.json")
    require(
        "$defs" in schema and "$defs" not in schema.get("properties", {}),
        "schema definitions must be top-level",
    )
    Draft202012Validator.check_schema(schema)
    errors = sorted(
        Draft202012Validator(schema, format_checker=FormatChecker()).iter_errors(
            result
        ),
        key=lambda error: list(error.path),
    )
    if errors:
        raise ValueError(
            "schema validation failed: " + "; ".join(error.message for error in errors)
        )

    manifest_path = root / "benchmarks/conformance/layout-manifest-v2.json"
    wpt_path = root / "benchmarks/layout/wpt-subset-v1.json"
    manifest = load(manifest_path)
    require(
        result["fingerprint"]["manifest_sha256"] == digest(manifest_path),
        "manifest hash mismatch",
    )
    require(
        result["fingerprint"]["wpt_subset_sha256"] == digest(wpt_path),
        "WPT subset hash mismatch",
    )
    expected = {
        (case["id"], case["category"], case["disposition"])
        for case in manifest["cases"]
    }
    actual = {
        (case["id"], case["category"], case["disposition"])
        for case in result["case_ledger"]
    }
    require(
        actual == expected,
        "case ledger does not exactly match the locked layout manifest",
    )
    required = [
        case for case in result["case_ledger"] if case["disposition"] == "required"
    ]
    limited = [
        case
        for case in result["case_ledger"]
        if case["disposition"] == "declared_limitation"
    ]
    require(
        len(required) == 28 and all(case["status"] == "passed" for case in required),
        "required layout cases are not 28/28",
    )
    require(
        len(limited) == 5
        and all(case["status"] == "expected_limitation" for case in limited),
        "layout limitations are not 5/5 explicit",
    )
    require(
        result["metrics"]["false_semantic_reference_preservations"] == 0,
        "false reference preservation is non-zero",
    )
    require(all(result["criteria"].values()), "one or more layout criteria failed")
    require(result["outcome"]["status"] == "passed", "layout result is not passed")

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
        by_path = {item["path"]: item["id"] for item in artifacts.values()}
        recomputed_cases = assertion_bound_cases(
            manifest,
            load(raw / "assertions.json"),
            by_path,
            include_layout_domains=True,
        )
        require(
            recomputed_cases == result["case_ledger"],
            "compact cases do not match assertion-bound raw evidence",
        )
        recomputed_regressions = verified_regressions(
            raw / "regressions.json", set(result["regressions"])
        )
        require(
            recomputed_regressions == result["regressions"],
            "compact regressions do not match assertion-bound raw evidence",
        )
        runtime = load(raw / "runtime.json")
        require(
            runtime.get("contract_version") == 4
            and runtime.get("worker_protocol_version") == 6,
            "raw runtime is not v4",
        )
        require(
            runtime.get("native_v8") is True
            and runtime.get("sandbox_state") == "app_sandbox",
            "raw runtime is not sandboxed V8",
        )
        require(
            runtime.get("worker_binary_sha256")
            == result["fingerprint"]["worker"]["sha256"],
            "worker runtime hash mismatch",
        )
        limitations = [
            json.loads(line)
            for line in (raw / "limitations/capabilities.jsonl")
            .read_text()
            .splitlines()
            if line.strip()
        ]
        unsupported = {
            cap["capability"]
            for row in limitations
            if row.get("kind") in {"observation", "geometry_observation"}
            for cap in row["observation"].get("capabilities", [])
            if cap.get("support") == "unsupported"
        }
        expected_caps = {
            case["expected_capability"]
            for case in manifest["cases"]
            if case["disposition"] == "declared_limitation"
        }
        require(
            expected_caps <= unsupported,
            "raw capabilities omit a declared layout limitation",
        )
        chromium = load(raw / "differential/chromium.json")
        native = load(raw / "differential/native.json")
        require(
            chromium.get("outcome", {}).get("status") == "passed"
            and native.get("outcome", {}).get("status") == "passed",
            "raw differential failed",
        )
        require(
            chromium.get("chromium_version")
            == result["fingerprint"]["chromium_version"],
            "Chromium version mismatch",
        )
        require(
            load(raw / "miniwob/aggregate.json").get("native")
            == {"completed": 24, "passed": 24},
            "raw native MiniWoB is not 24/24",
        )
        require(
            load(raw / "churn.json").get("false_preservation", {}).get("numerator")
            == 0,
            "raw churn false preservation is non-zero",
        )
        require(
            load(raw / "benchmarks/layout-benchmarks-v1.json")
            .get("outcome", {})
            .get("status")
            == "passed",
            "raw benchmark failed",
        )
        current_tree = subprocess.check_output(
            [str(root / "scripts/source-tree-hash.sh")], text=True
        ).strip()
        require(
            current_tree == result["fingerprint"]["source_tree_sha256"],
            "source-tree hash mismatch",
        )
    print("layout result is schema-valid and recomputes to passed")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
