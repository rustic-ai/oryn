#!/usr/bin/env python3
"""Fail-closed helpers for assertion-bound capability evidence."""

from __future__ import annotations

import json
from pathlib import Path
from typing import Any


class EvidenceError(ValueError):
    """Raised when raw evidence cannot substantiate a result."""


def load_object(path: Path) -> dict[str, Any]:
    value = json.loads(path.read_text(encoding="utf-8"))
    if not isinstance(value, dict):
        raise EvidenceError(f"{path} must contain an object")
    return value


def assertion_bound_cases(
    manifest: dict[str, Any],
    assertion_document: dict[str, Any],
    artifact_ids: dict[str, str],
    *,
    include_layout_domains: bool = False,
) -> list[dict[str, Any]]:
    """Validate one executed assertion per case and derive the compact ledger."""

    if assertion_document.get("schema_version") != 1:
        raise EvidenceError("assertion ledger must use schema version 1")
    assertions = assertion_document.get("assertions")
    if not isinstance(assertions, list):
        raise EvidenceError("assertion ledger must contain an assertions array")

    by_case: dict[str, dict[str, Any]] = {}
    assertion_ids: set[str] = set()
    for assertion in assertions:
        if not isinstance(assertion, dict):
            raise EvidenceError("each assertion must be an object")
        assertion_id = assertion.get("id")
        case_id = assertion.get("case_id")
        if not isinstance(assertion_id, str) or not assertion_id:
            raise EvidenceError("assertion id must be a non-empty string")
        if assertion_id in assertion_ids:
            raise EvidenceError(f"duplicate assertion id: {assertion_id}")
        assertion_ids.add(assertion_id)
        if not isinstance(case_id, str) or not case_id:
            raise EvidenceError(f"assertion {assertion_id} has no case_id")
        if case_id in by_case:
            raise EvidenceError(f"case {case_id} has multiple assertions")

        checks = assertion.get("checks")
        if not isinstance(checks, list) or not checks:
            raise EvidenceError(f"assertion {assertion_id} has no executable checks")
        if any(
            not isinstance(check, dict)
            or not isinstance(check.get("name"), str)
            or not check.get("name")
            or not isinstance(check.get("artifact"), str)
            or not check.get("artifact")
            or check.get("passed") is not True
            for check in checks
        ):
            raise EvidenceError(f"assertion {assertion_id} contains a failed check")

        artifacts = assertion.get("artifacts")
        if not isinstance(artifacts, list) or not artifacts:
            raise EvidenceError(f"assertion {assertion_id} has no artifacts")
        check_artifacts = {check["artifact"] for check in checks}
        if not check_artifacts <= set(artifacts):
            raise EvidenceError(
                f"assertion {assertion_id} has checks without declared artifacts"
            )
        missing = [path for path in artifacts if path not in artifact_ids]
        if missing:
            raise EvidenceError(
                f"assertion {assertion_id} references missing artifacts: {missing}"
            )
        if assertion.get("status") not in {"passed", "expected_limitation"}:
            raise EvidenceError(f"assertion {assertion_id} did not pass")
        by_case[case_id] = assertion

    manifest_cases = manifest.get("cases")
    if not isinstance(manifest_cases, list):
        raise EvidenceError("manifest must contain a cases array")
    expected = {case.get("id") for case in manifest_cases}
    if None in expected or len(expected) != len(manifest_cases):
        raise EvidenceError("manifest contains missing or duplicate case ids")
    actual = set(by_case)
    if actual != expected:
        missing = sorted(expected - actual)
        unexpected = sorted(actual - expected)
        raise EvidenceError(
            f"assertion coverage mismatch; missing={missing}, unexpected={unexpected}"
        )

    result = []
    for case in manifest_cases:
        case_id = case["id"]
        assertion = by_case[case_id]
        limitation = case.get("disposition") == "declared_limitation"
        expected_status = "expected_limitation" if limitation else "passed"
        if assertion["status"] != expected_status:
            raise EvidenceError(
                f"case {case_id} expected {expected_status}, got {assertion['status']}"
            )
        item: dict[str, Any] = {
            "id": case_id,
            "category": case["category"],
            "status": expected_status,
            "assertion_id": assertion["id"],
            "evidence": [artifact_ids[path] for path in assertion["artifacts"]]
            + (
                [artifact_ids["assertions.json"]]
                if "assertions.json" in artifact_ids
                else []
            ),
        }
        if "disposition" in case:
            item["disposition"] = case["disposition"]
        if limitation:
            item["expected_capability"] = case["expected_capability"]
        if include_layout_domains:
            chromium = assertion.get("domains", {}).get(
                "chromium_differential", "not_applicable"
            )
            if chromium not in {"passed", "not_applicable"}:
                raise EvidenceError(f"case {case_id} has invalid Chromium status")
            item["domains"] = {
                "native": "passed",
                "chromium_differential": chromium,
            }
        result.append(item)
    return result


def verified_regressions(path: Path, required: set[str]) -> dict[str, str]:
    document = load_object(path)
    if document.get("schema_version") != 1:
        raise EvidenceError("regression ledger must use schema version 1")
    if not isinstance(document.get("suite"), str) or not document["suite"]:
        raise EvidenceError("regression ledger must name its suite")
    values = document.get("regressions")
    if not isinstance(values, dict) or set(values) != required:
        raise EvidenceError("regression ledger does not exactly match required keys")
    artifact_root = path.parent
    for name, value in values.items():
        if not isinstance(value, dict):
            raise EvidenceError(f"regression {name} is not an assertion record")
        if value.get("status") != "passed":
            raise EvidenceError(f"regression {name} did not pass")
        checks = value.get("checks")
        if not isinstance(checks, list) or not checks:
            raise EvidenceError(f"regression {name} has no executable checks")
        if any(
            not isinstance(check, dict)
            or not isinstance(check.get("name"), str)
            or not check.get("name")
            or check.get("passed") is not True
            for check in checks
        ):
            raise EvidenceError(f"regression {name} contains a failed check")
        artifacts = value.get("artifacts")
        if not isinstance(artifacts, list) or not artifacts:
            raise EvidenceError(f"regression {name} has no artifacts")
        missing = [
            artifact
            for artifact in artifacts
            if not (artifact_root / artifact).is_file()
        ]
        if missing:
            raise EvidenceError(
                f"regression {name} references missing artifacts: {missing}"
            )
    return {key: "passed" for key in sorted(required)}
