#!/usr/bin/env python3
"""Validate security compact evidence and recompute claims from raw artifacts."""

from __future__ import annotations

import argparse
import hashlib
import json
import subprocess
from pathlib import Path
from typing import Any

from evidence_protocols import validate_protocols, validate_protocol_pair

from jsonschema import Draft202012Validator, FormatChecker

from native_protocol_versions import versions as native_protocol_versions

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
    parser.add_argument("--allow-historical", action="store_true")
    args = parser.parse_args()
    root = args.repo_root.resolve()
    result = load(args.result.resolve())
    validate_protocols(result.get("fingerprint", {}).get("worker"), root, fingerprint=True, historical=args.allow_historical)
    version = result.get("schema_version")
    require(version == 3 or (args.allow_historical and version == 2), "current security evidence requires schema 3")
    schema = load(root / f"benchmarks/schema/security-result-v{version}.schema.json")
    if not args.allow_historical:
        require(result.get("fingerprint", {}).get("worker", {}).get("protocol_version") == native_protocol_versions(root)["worker"], "native worker protocol mismatch")

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

    manifest_path = root / "benchmarks/conformance/security-manifest-v1.json"
    manifest = load(manifest_path)
    require(
        result["fingerprint"]["manifest_sha256"] == digest(manifest_path),
        "manifest hash mismatch",
    )
    if not args.allow_historical:
        require(
            result["fingerprint"]["v8_policy_sha256"]
            == digest(root / "security/v8-update-policy.json"),
            "V8 policy hash mismatch",
        )
    if not args.allow_historical:
        require(
            result["fingerprint"]["dependency_exceptions_sha256"]
            == digest(root / "security/dependency-exceptions.json"),
            "dependency exception hash mismatch",
        )
    expected = {(case["id"], case["category"]) for case in manifest["cases"]}
    actual = {(case["id"], case["category"]) for case in result["case_ledger"]}
    require(
        actual == expected, "case ledger does not exactly match the security manifest"
    )
    require(
        len(result["case_ledger"]) == 51
        and all(case["status"] == "passed" for case in result["case_ledger"]),
        "required security cases are not 51/51",
    )
    require(all(result["criteria"].values()), "one or more security criteria failed")
    require(result["outcome"]["status"] == "passed", "security result is not passed")

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
            manifest, load(raw / "assertions.json"), by_path
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
        validate_protocol_pair(runtime, result["fingerprint"]["worker"], root, historical=args.allow_historical)
        require(
            runtime.get("native_v8") is True
            and runtime.get("build_profile") == "release"
            and runtime.get("execution_mode") == "sandboxed_worker"
            and runtime.get("sandbox_state") == "app_sandbox"
            and runtime.get("worker_protocol_version") == (result["fingerprint"]["worker"]["protocol_version"] if args.allow_historical else native_protocol_versions(root)["worker"]),
            "raw runtime is not signed sandboxed V8",
        )
        require(
            runtime.get("worker_binary_sha256")
            == result["fingerprint"]["worker"]["sha256"],
            "worker runtime hash mismatch",
        )
        security = load(raw / "security-probe.json")
        require(
            security.get("outcome", {}).get("status") == "passed"
            and all(
                value is True
                for key, value in security.items()
                if key not in {"schema_version", "suite", "audit_records", "outcome"}
            ),
            "raw security probe did not pass every assertion",
        )
        require(security.get("audit_records", 0) > 0, "security audit is empty")
        fuzz = load(raw / "fuzz/summary.json")
        require(
            fuzz.get("crashes") == 0
            and len(fuzz.get("targets", [])) == 4
            and all(item.get("status") == "passed" for item in fuzz["targets"]),
            "raw fuzz summary failed",
        )
        audit = load(raw / "software/cargo-audit.json")
        require(
            audit.get("vulnerabilities") == {"found": False, "count": 0, "list": []},
            "raw dependency audit found vulnerabilities",
        )
        churn = load(raw / "churn.json")
        require(
            churn.get("status") == "passed"
            and churn.get("false_preservation", {}).get("numerator") == 0
            and churn.get("worker_replacement", {}).get("status") == "passed",
            "raw limit/churn evidence failed",
        )
        require(
            load(raw / "layout/layout-result-v2.json").get("outcome", {}).get("status")
            == "passed",
            "raw layout regression result failed",
        )
        current_tree = subprocess.check_output(
            [str(root / "scripts/source-tree-hash.sh")], text=True
        ).strip()
        require(
            current_tree == result["fingerprint"]["source_tree_sha256"],
            "source-tree hash mismatch",
        )

    print("security result is schema-valid and recomputes to passed")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
