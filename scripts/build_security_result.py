#!/usr/bin/env python3
"""Build compact, content-addressed security evidence from a production gate run."""

from __future__ import annotations

import argparse
import hashlib
import json
import re
import subprocess
from datetime import datetime, timezone
from pathlib import Path
from typing import Any

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


def git(root: Path, *args: str) -> str:
    return subprocess.check_output(["git", *args], cwd=root, text=True).strip()


def artifact_id(relative: str) -> str:
    return "raw:" + re.sub(r"[^a-zA-Z0-9._-]+", "__", relative)


def artifact_kind(relative: str) -> str:
    if relative == "runtime.json":
        return "runtime_metadata"
    if relative.endswith(".json"):
        return "benchmark" if "benchmark" in relative else "json_summary"
    return "test_log"


def require(condition: bool, message: str) -> None:
    if not condition:
        raise ValueError(message)


def main() -> int:
    root_default = Path(__file__).resolve().parents[1]
    parser = argparse.ArgumentParser()
    parser.add_argument("--repo-root", type=Path, default=root_default)
    parser.add_argument("--artifact-root", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    root = args.repo_root.resolve()
    raw = args.artifact_root.resolve()
    output = args.output.resolve()

    require(
        not git(root, "status", "--porcelain", "--untracked-files=all"),
        "passed security evidence requires a clean source tree",
    )
    production = load(raw / "production-gate.json")
    runtime = load(raw / "runtime.json")
    security = load(raw / "security-probe.json")
    churn = load(raw / "churn.json")
    fuzz = load(raw / "fuzz/summary.json")
    audit = load(raw / "software/cargo-audit.json")
    benchmarks = load(raw / "benchmarks/result.json")
    layout_result = load(raw / "layout/layout-result-v2.json")

    require(
        production.get("outcome", {}).get("status") == "passed",
        "production gate failed",
    )
    require(
        layout_result.get("outcome", {}).get("status") == "passed",
        "layout regression failed",
    )
    require(
        security.get("outcome", {}).get("status") == "passed", "security probe failed"
    )
    require(churn.get("status") == "passed", "limit/churn suite failed")
    require(
        churn.get("false_preservation", {}).get("numerator") == 0,
        "reference churn found false preservation",
    )
    require(
        fuzz.get("outcome", {}).get("status") == "passed" and fuzz.get("crashes") == 0,
        "fuzz smoke failed",
    )
    require(
        audit.get("vulnerabilities", {}).get("count") == 0,
        "dependency vulnerabilities found",
    )
    require(
        benchmarks.get("outcome", {}).get("status") == "passed",
        "production benchmarks failed",
    )
    require(
        runtime.get("native_v8") is True
        and runtime.get("sandbox_state") == "app_sandbox"
        and runtime.get("worker_protocol_version") == 7,
        "runtime is not the signed sandboxed V8 worker",
    )

    ledger: list[dict[str, Any]] = []
    by_path: dict[str, str] = {}
    for path in sorted(item for item in raw.rglob("*") if item.is_file()):
        if path == output:
            continue
        relative = path.relative_to(raw).as_posix()
        identifier = artifact_id(relative)
        ledger.append(
            {
                "id": identifier,
                "path": relative,
                "sha256": digest(path),
                "bytes": path.stat().st_size,
                "kind": artifact_kind(relative),
            }
        )
        by_path[relative] = identifier

    def evidence(*paths: str) -> list[str]:
        missing = [path for path in paths if path not in by_path]
        if missing:
            raise ValueError(f"missing raw evidence: {missing}")
        return [by_path[path] for path in paths]

    manifest_path = root / "benchmarks/conformance/security-manifest-v1.json"
    manifest = load(manifest_path)
    cases = assertion_bound_cases(manifest, load(raw / "assertions.json"), by_path)
    require(len(cases) == 51, "security manifest is not 51 required cases")
    required_regressions = {
        "g1",
        "g2_controlled",
        "g3",
        "g4",
        "native_miniwob_24",
        "rust_workspace",
        "strict_clippy",
        "python_sdk",
        "intentgym",
        "scanner",
        "extension",
        "signed_production_boundary",
        "reference_churn",
    }
    regressions = verified_regressions(raw / "regressions.json", required_regressions)
    reviewed_warnings = sum(
        len(items)
        for items in (audit.get("warnings") or {}).values()
        if isinstance(items, list)
    )

    parent = root / "target/release/oryn"
    worker = root / "target/release/OrynPageWorker.app/Contents/MacOS/oryn-page-worker"
    source_commit = git(root, "rev-parse", "HEAD")
    source_tree = subprocess.check_output(
        [str(root / "scripts/source-tree-hash.sh")], text=True
    ).strip()
    process = benchmarks["artifacts"]
    required_passed = sum(case["status"] == "passed" for case in cases)
    fuzz_targets = fuzz.get("targets") or []
    fuzz_passed = sum(target.get("status") == "passed" for target in fuzz_targets)
    vulnerabilities = int(audit.get("vulnerabilities", {}).get("count", -1))
    fuzz_crashes = int(fuzz.get("crashes", -1))
    false_preservations = int(churn.get("false_preservation", {}).get("numerator", -1))
    signed_boundary = (
        runtime.get("native_v8") is True
        and runtime.get("execution_mode") == "sandboxed_worker"
        and runtime.get("sandboxed") is True
        and runtime.get("sandbox_state") == "app_sandbox"
    )
    criteria = {
        "all_threat_model_cases_pass": required_passed == 51,
        "signed_worker_isolation_passes": signed_boundary,
        "network_action_secret_download_controls_pass": required_passed == 51,
        "resource_limits_and_replacement_pass": churn.get("status") == "passed"
        and churn.get("worker_replacement", {}).get("status") == "passed",
        "fuzz_targets_pass_without_crash": len(fuzz_targets) == 4
        and fuzz_passed == 4
        and fuzz_crashes == 0
        and fuzz.get("outcome", {}).get("status") == "passed",
        "dependency_vulnerabilities_are_zero": vulnerabilities == 0,
        "dependency_warnings_are_reviewed": reviewed_warnings
        == len(
            load(root / "security/dependency-exceptions.json").get("exceptions") or []
        ),
        "existing_regressions_pass": all(
            value == "passed" for value in regressions.values()
        ),
        "false_reference_preservation_is_zero": false_preservations == 0,
        "production_benchmarks_published": benchmarks.get("outcome", {}).get("status")
        == "passed",
    }
    reason_codes = [name for name, passed in criteria.items() if not passed]
    result = {
        "schema_version": 2,
        "profile": "security-production",
        "run_id": f"security-{source_commit[:12]}-macos-arm64",
        "recorded_at": datetime.now(timezone.utc).isoformat().replace("+00:00", "Z"),
        "fingerprint": {
            "source_commit": source_commit,
            "source_dirty": False,
            "source_tree_sha256": source_tree,
            "manifest_sha256": digest(manifest_path),
            "v8_policy_sha256": digest(root / "security/v8-update-policy.json"),
            "dependency_exceptions_sha256": digest(
                root / "security/dependency-exceptions.json"
            ),
            "parent": {
                "sha256": digest(parent),
                "bytes": parent.stat().st_size,
                "build_profile": "release",
            },
            "worker": {
                "sha256": digest(worker),
                "bytes": worker.stat().st_size,
                "protocol_version": runtime["worker_protocol_version"],
                "contract_version": runtime["contract_version"],
                "native_v8": runtime["native_v8"],
                "v8_version": runtime["v8_version"],
                "execution_mode": runtime["execution_mode"],
                "sandboxed": runtime["sandboxed"],
                "sandbox_state": runtime["sandbox_state"],
                "bundle_id": runtime["worker_bundle_id"],
                "team_id": runtime["worker_team_id"],
                "signing_cert_sha1": runtime["worker_signing_cert_sha1"],
                "entitlement_sha256": process["worker"]["entitlements_sha256"],
                "policy_sha256": runtime["policy_digest"],
                "limits": runtime["limits"],
            },
        },
        "corpus": {"manifest_cases": 51, "required_cases": 51},
        "regressions": regressions,
        "metrics": {
            "required_passed": required_passed,
            "required_total": 51,
            "vulnerabilities": vulnerabilities,
            "reviewed_dependency_warnings": reviewed_warnings,
            "fuzz_targets_passed": fuzz_passed,
            "fuzz_targets_total": len(fuzz_targets),
            "fuzz_crashes": fuzz_crashes,
            "false_semantic_reference_preservations": false_preservations,
        },
        "benchmarks": benchmarks,
        "artifact_ledger": ledger,
        "case_ledger": cases,
        "criteria": criteria,
        "outcome": {
            "status": "passed" if not reason_codes else "failed",
            "reason_codes": reason_codes,
            "diagnostics": (
                [f"unsatisfied security evidence criteria: {', '.join(reason_codes)}"]
                if reason_codes
                else []
            ),
        },
    }
    output.parent.mkdir(parents=True, exist_ok=True)
    output.write_text(json.dumps(result, indent=2) + "\n", encoding="utf-8")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
