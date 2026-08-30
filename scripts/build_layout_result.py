#!/usr/bin/env python3
"""Build compact, content-addressed layout evidence from a production gate run."""

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
    if relative.endswith(".jsonl"):
        return "jsonl"
    if relative == "runtime.json":
        return "runtime_metadata"
    if relative.endswith(".json"):
        return "benchmark" if "benchmark" in relative else "json_summary"
    return "test_log"


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
    if git(root, "status", "--porcelain", "--untracked-files=all"):
        raise ValueError("passed layout evidence requires a clean source tree")

    production = load(raw / "production-gate.json")
    runtime = load(raw / "runtime.json")
    churn = load(raw / "churn.json")
    miniwob = load(raw / "miniwob/aggregate.json")
    differential = load(raw / "differential/chromium.json")
    benchmarks = load(raw / "benchmarks/layout-benchmarks-v1.json")
    if production.get("outcome", {}).get("status") != "passed":
        raise ValueError("layout production gate did not pass")
    if (
        runtime.get("contract_version") != 4
        or runtime.get("worker_protocol_version") != 6
    ):
        raise ValueError("production runtime is not contract v4 / worker protocol v6")
    if miniwob.get("native") != {"completed": 24, "passed": 24}:
        raise ValueError("native MiniWoB is not 24/24")
    if churn.get("false_preservation", {}).get("numerator") != 0:
        raise ValueError("reference churn found false preservation")
    if benchmarks.get("outcome", {}).get("status") != "passed":
        raise ValueError("layout production benchmarks failed")

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

    manifest_path = root / "benchmarks/conformance/layout-manifest-v2.json"
    wpt_path = root / "benchmarks/layout/wpt-subset-v1.json"
    manifest = load(manifest_path)
    case_ledger = assertion_bound_cases(
        manifest,
        load(raw / "assertions.json"),
        by_path,
        include_layout_domains=True,
    )
    required_regressions = {
        "rust_workspace",
        "rust_v8",
        "strict_clippy_default",
        "strict_clippy_v8",
        "python_sdk",
        "intentgym",
        "scanner",
        "extension",
        "g1",
        "g2_controlled",
        "native_miniwob_24",
        "g3_frameworks_native",
        "g3_oauth_native",
        "reference_churn",
        "signed_production_boundary",
    }
    regressions = verified_regressions(raw / "regressions.json", required_regressions)
    required_passed = sum(
        case["status"] == "passed" and case["disposition"] == "required"
        for case in case_ledger
    )
    limitations_reported = sum(
        case["status"] == "expected_limitation" for case in case_ledger
    )

    parent = root / "target/release/oryn"
    worker = root / "target/release/OrynPageWorker.app/Contents/MacOS/oryn-page-worker"
    process = benchmarks["process"]
    source_commit = git(root, "rev-parse", "HEAD")
    source_tree = subprocess.check_output(
        [str(root / "scripts/source-tree-hash.sh")], text=True
    ).strip()
    false_preservations = int(churn.get("false_preservation", {}).get("numerator", -1))
    signed_boundary = (
        runtime.get("native_v8") is True
        and runtime.get("execution_mode") == "sandboxed_worker"
        and runtime.get("sandboxed") is True
        and runtime.get("sandbox_state") == "app_sandbox"
    )
    differential_cases = [
        case
        for case in case_ledger
        if case["domains"]["chromium_differential"] != "not_applicable"
    ]
    criteria = {
        "required_native_cases_pass": required_passed == 29,
        "declared_limitations_are_explicit": limitations_reported == 4,
        "native_chromium_relations_match": bool(differential_cases)
        and all(
            case["domains"]["chromium_differential"] == "passed"
            for case in differential_cases
        ),
        "existing_regressions_pass": all(
            value == "passed" for value in regressions.values()
        ),
        "false_reference_preservation_is_zero": false_preservations == 0,
        "signed_process_boundary_passes": signed_boundary,
        "production_benchmarks_published": benchmarks.get("outcome", {}).get("status")
        == "passed",
    }
    reason_codes = [name for name, passed in criteria.items() if not passed]
    result = {
        "schema_version": 2,
        "profile": "layout-production",
        "run_id": f"layout-{source_commit[:12]}-macos-arm64",
        "recorded_at": datetime.now(timezone.utc).isoformat().replace("+00:00", "Z"),
        "fingerprint": {
            "source_commit": source_commit,
            "source_dirty": False,
            "source_tree_sha256": source_tree,
            "manifest_sha256": digest(manifest_path),
            "wpt_subset_sha256": digest(wpt_path),
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
                "entitlement_sha256": process["artifacts"]["worker"][
                    "entitlements_sha256"
                ],
                "style_engine_version": runtime["style_engine_version"],
                "box_engine_version": runtime["box_engine_version"],
                "inline_engine_version": runtime["inline_engine_version"],
                "embedded_font_sha256": runtime["embedded_font_sha256"],
            },
            "chromium_version": differential["chromium_version"],
        },
        "corpus": {
            "manifest_cases": 33,
            "required_cases": 28,
            "declared_limitations": 5,
        },
        "regressions": regressions,
        "metrics": {
            "required_passed": required_passed,
            "required_total": 29,
            "limitations_reported": limitations_reported,
            "limitations_total": 4,
            "false_semantic_reference_preservations": false_preservations,
        },
        "benchmarks": benchmarks,
        "artifact_ledger": ledger,
        "case_ledger": case_ledger,
        "criteria": criteria,
        "outcome": {
            "status": "passed" if not reason_codes else "failed",
            "reason_codes": reason_codes,
            "diagnostics": (
                [f"unsatisfied layout evidence criteria: {', '.join(reason_codes)}"]
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
