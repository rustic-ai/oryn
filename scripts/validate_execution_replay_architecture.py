#!/usr/bin/env python3
"""Validate frozen execution/replay manifests and dependency boundaries."""

from __future__ import annotations

import json
import subprocess
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]


def require(condition: bool, message: str) -> None:
    if not condition:
        raise SystemExit(message)


def manifest(capability: str, legacy_label: str, count: int) -> None:
    path = ROOT / f"benchmarks/conformance/{capability}-manifest-v1.json"
    value = json.loads(path.read_text(encoding="utf-8"))
    cases = value.get("cases", [])
    require(
        value.get("gate") == legacy_label, f"{capability} manifest capability mismatch"
    )
    require(
        value.get("frozen_before_validation") is True,
        f"{capability} manifest is not frozen",
    )
    require(len(cases) == count, f"{capability} manifest must contain {count} cases")
    require(
        len({case["id"] for case in cases}) == count,
        f"{capability} case IDs are not unique",
    )
    require(
        all(case.get("required") is True for case in cases),
        f"{capability} has optional cases",
    )


def tree(package: str, *features: str) -> str:
    command = [
        "cargo",
        "tree",
        "--locked",
        "-p",
        package,
        "--edges",
        "normal",
        "--prefix",
        "none",
    ]
    if features:
        command.extend(["--features", ",".join(features)])
    return subprocess.check_output(command, cwd=ROOT, text=True)


def main() -> int:
    manifest("execution-domains", "G6", 25)
    manifest("replay", "G7", 26)
    forbidden_native = {
        "oryn-h",
        "oryn-e",
        "oryn-r",
        "oryn-domain",
        "chromiumoxide",
        "fantoccini",
    }
    native = set(tree("oryn-native").splitlines())
    require(
        not (native & forbidden_native),
        f"native tree contains compatibility packages: {native & forbidden_native}",
    )
    canonical = set(tree("oryn").splitlines())
    require(
        not (canonical & {"oryn-h", "oryn-e", "oryn-r", "oryn-domain", "oryn-engine"}),
        "default oryn tree contains compatibility backends",
    )
    source = (ROOT / "crates/oryn-native/src/runtime.rs").read_text(encoding="utf-8")
    require(
        "replay_task_trace" in source and "prepare_handoff" in source,
        "execution/replay runtime surface missing",
    )
    cli = (ROOT / "crates/oryn/src/main.rs").read_text(encoding="utf-8")
    for flag in (
        "trace_out",
        "checkpoint_out",
        "restore_checkpoint",
        "replay_trace",
        "failure_artifact_out",
        "replay_failure",
    ):
        require(flag in cli, f"CLI is missing {flag}")
    print("execution/replay manifests and dependency boundaries are valid")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
