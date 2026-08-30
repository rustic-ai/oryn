#!/usr/bin/env python3
"""Validate proposal-v1 runtime boundaries and cutover readiness."""

from __future__ import annotations

import argparse
import json
import os
import shutil
import subprocess
import tarfile
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
MANIFEST = ROOT / "benchmarks/conformance/runtime-architecture-v1.json"
SHADOW_MANIFEST = ROOT / "benchmarks/conformance/runtime-shadow-v1.json"
SHADOW_RESULT = ROOT / "benchmarks/evidence/runtime-shadow-v1.json"


def require(condition: bool, message: str) -> None:
    if not condition:
        raise SystemExit(message)


def validate_shadow(rerun: bool) -> None:
    manifest = json.loads(SHADOW_MANIFEST.read_text(encoding="utf-8"))
    result = json.loads(SHADOW_RESULT.read_text(encoding="utf-8"))
    source_tree_hash = subprocess.check_output(
        [str(ROOT / "scripts/source-tree-hash.sh")], cwd=ROOT, text=True
    ).strip()
    require(
        result.get("source_tree_sha256") == source_tree_hash,
        "shadow evidence is not bound to the current source tree",
    )
    manifest_ids = [fixture["id"] for fixture in manifest["fixtures"]]
    result_ids = [fixture["id"] for fixture in result["fixtures"]]
    require(result["manifest"] == str(SHADOW_MANIFEST.relative_to(ROOT)), "shadow manifest mismatch")
    require(result["side_effect_policy"] == "repository_fixtures_only", "shadow side effects are not bounded")
    require(result_ids == manifest_ids, "shadow fixture ledger does not match its manifest")
    require(
        all(item["legacy_passed"] and item["rust_owned_passed"] for item in result["fixtures"]),
        "shadow fixture failed",
    )
    require(result["outcome"] == {"status": "passed", "compared": len(manifest_ids), "divergences": 0}, "shadow aggregate mismatch")
    if not rerun:
        return
    command = [
        "cargo",
        "run",
        "-q",
        "-p",
        "oryn-native",
        "--example",
        "runtime_shadow",
        "--features",
        "v8-host in-process-probe",
    ]
    current = json.loads(subprocess.check_output(command, cwd=ROOT, text=True))
    require([item["id"] for item in current] == manifest_ids, "current shadow harness IDs differ")
    require(all(item["passed"] for item in current), "current shadow harness failed")
    with tempfile.TemporaryDirectory(prefix="oryn-shadow-legacy-") as directory:
        archive = subprocess.check_output(
            ["git", "archive", result["legacy_commit"]], cwd=ROOT
        )
        archive_path = Path(directory) / "source.tar"
        archive_path.write_bytes(archive)
        with tarfile.open(archive_path) as source:
            destination = Path(directory).resolve()
            for member in source.getmembers():
                target = (destination / member.name).resolve()
                require(
                    target == destination or destination in target.parents,
                    "legacy shadow archive contains an unsafe path",
                )
            source.extractall(directory)
        harness = Path(directory) / result["harness"]
        harness.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(ROOT / result["harness"], harness)
        environment = os.environ.copy()
        environment["CARGO_TARGET_DIR"] = str(ROOT / "target")
        legacy = json.loads(
            subprocess.check_output(command, cwd=directory, env=environment, text=True)
        )
    require([item["id"] for item in legacy] == manifest_ids, "legacy shadow harness IDs differ")
    require(all(item["passed"] for item in legacy), "legacy shadow harness failed")


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--require-cutover", action="store_true")
    args = parser.parse_args()
    value = json.loads(MANIFEST.read_text(encoding="utf-8"))
    cases = value.get("cases", [])
    require(value.get("schema_version") == 1, "runtime manifest schema mismatch")
    require(
        value.get("frozen_before_validation") is True,
        "runtime manifest must be frozen before validation",
    )
    require(
        value.get("shadow_side_effect_policy") == "fixtures_and_recorded_network_only",
        "shadow traffic policy permits duplicated live side effects",
    )
    ids = [case.get("id") for case in cases]
    require(len(ids) == len(set(ids)) and len(ids) >= 20, "runtime cases are incomplete")
    require(all(case.get("required") is True for case in cases), "optional runtime case found")
    validate_shadow(args.require_cutover)

    subprocess.run(
        [
            "cargo",
            "run",
            "-q",
            "-p",
            "oryn-webidl",
            "--",
            "check",
            "crates/oryn-webidl/idl/browser.webidl",
            "crates/oryn-js/src/generated/browser.rs",
        ],
        cwd=ROOT,
        check=True,
    )
    native_tree = subprocess.check_output(
        ["cargo", "tree", "--locked", "-p", "oryn-native", "--prefix", "none"],
        cwd=ROOT,
        text=True,
    )
    for package in ("oryn-dom", "oryn-js", "oryn-net"):
        require(package in native_tree, f"native orchestrator does not consume {package}")

    production_sources = [
        ROOT / "crates/oryn-native/src/rust_executable.rs",
        ROOT / "crates/oryn-native/src/host.rs",
        ROOT / "crates/oryn-js/src/v8_bindings.rs",
    ]
    require(all(path.exists() for path in production_sources), "production runtime source missing")
    executable = "\n".join(path.read_text(encoding="utf-8") for path in production_sources)
    legacy_markers = {
        "sync_dom": "full-DOM synchronization",
        "__oryn_export": "JavaScript DOM export",
        "__oryn_fetch_sync": "synchronous Fetch bridge",
        "_frame_documents": "isolate-per-frame document path",
    }
    remaining = [label for marker, label in legacy_markers.items() if marker in executable]
    pending = [
        case["id"]
        for case in cases
        if case.get("gate", "software") == "software" and case.get("status") != "passed"
    ]
    release_attestations = [
        case["id"]
        for case in cases
        if case.get("gate") == "release" and case.get("status") != "passed"
    ]
    if args.require_cutover:
        require(not remaining, f"production cutover still contains: {', '.join(remaining)}")
        require(not pending, f"runtime manifest is not complete: {', '.join(pending)}")
    print(
        json.dumps(
            {
                "manifest": str(MANIFEST.relative_to(ROOT)),
                "cases": len(cases),
                "pending": pending,
                "pending_release_attestations": release_attestations,
                "legacy_production_markers": remaining,
                "cutover_ready": not pending and not remaining,
            },
            indent=2,
            sort_keys=True,
        )
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
