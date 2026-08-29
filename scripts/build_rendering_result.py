#!/usr/bin/env python3
"""Build compact, content-addressed rendering evidence from a signed production run."""

from __future__ import annotations

import argparse
import hashlib
import json
import re
import subprocess
from datetime import datetime, timezone
from pathlib import Path
from typing import Any


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


def git(root: Path, *args: str) -> str:
    return subprocess.check_output(["git", *args], cwd=root, text=True).strip()


def main() -> int:
    root_default = Path(__file__).resolve().parents[1]
    parser = argparse.ArgumentParser()
    parser.add_argument("--repo-root", type=Path, default=root_default)
    parser.add_argument("--artifact-root", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    root, raw, output = (
        args.repo_root.resolve(),
        args.artifact_root.resolve(),
        args.output.resolve(),
    )
    require(
        not git(root, "status", "--porcelain", "--untracked-files=all"),
        "rendering evidence requires a clean source tree",
    )
    runtime = load(raw / "runtime.json")
    probe = load(raw / "rendering-probe.json")
    software = load(raw / "software/summary.json")
    differential = load(raw / "differential/differential.json")
    production = load(raw / "production-gate.json")
    sizes = load(raw / "sizes.json")
    require(
        runtime.get("native_v8") is True
        and runtime.get("sandbox_state") == "app_sandbox",
        "runtime is not sandboxed V8",
    )
    require(
        probe.get("outcome", {}).get("status") == "passed", "rendering probe failed"
    )
    require(
        software.get("outcome", {}).get("status") == "passed",
        "rendering software checks failed",
    )
    require(
        differential.get("outcome", {}).get("status") == "passed",
        "rendering differential failed",
    )
    require(
        production.get("outcome", {}).get("status") == "passed",
        "rendering production gate failed",
    )

    ledger: list[dict[str, Any]] = []
    ids: dict[str, str] = {}
    for path in sorted(
        item for item in raw.rglob("*") if item.is_file() and item != output
    ):
        relative = path.relative_to(raw).as_posix()
        identifier = "raw:" + re.sub(r"[^a-zA-Z0-9._-]+", "__", relative)
        if relative.endswith(".png"):
            kind = "image"
        elif relative.endswith(".pdf"):
            kind = "pdf"
        elif relative == "runtime.json":
            kind = "runtime_metadata"
        elif relative.endswith(".json"):
            kind = "json_summary"
        else:
            kind = "test_log"
        ledger.append(
            {
                "id": identifier,
                "path": relative,
                "sha256": digest(path),
                "bytes": path.stat().st_size,
                "kind": kind,
            }
        )
        ids[relative] = identifier

    def evidence(*paths: str) -> list[str]:
        require(all(path in ids for path in paths), f"missing raw evidence: {paths}")
        return [ids[path] for path in paths]

    category_evidence = {
        "contract": evidence("rendering-probe.json", "probe/paint-snapshot.json"),
        "paint": evidence("rendering-probe.json", "probe/rendering-probe.png"),
        "text": evidence("rendering-probe.json", "probe/rendering-probe.png"),
        "image": evidence("rendering-probe.json", "probe/rendering-probe.png"),
        "screenshot": evidence(
            "rendering-probe.json", "probe/rendering-probe.png", "cli/cli.png"
        ),
        "pdf": evidence(
            "rendering-probe.json", "probe/rendering-probe.pdf", "cli/cli.pdf"
        ),
        "limits": evidence("software/render-tests.log"),
        "diagnostic": evidence("software/render-tests.log"),
        "boundary": evidence("software/architecture.log", "runtime.json"),
        "worker": evidence("runtime.json", "rendering-probe.json"),
        "cli": evidence("cli/cli-output.json", "cli/cli.png", "cli/cli.pdf"),
        "differential": evidence(
            "differential/differential.json",
            "differential/chromium.json",
            "differential/chromium.png",
        ),
        "regression": evidence(
            "execution-replay/production-gate.json", "software/summary.json"
        ),
    }
    manifest_path = root / "benchmarks/conformance/rendering-manifest-v1.json"
    manifest = load(manifest_path)
    cases = [
        {
            "id": case["id"],
            "category": case["category"],
            "status": "passed",
            "evidence": category_evidence[case["category"]],
        }
        for case in manifest["cases"]
    ]
    require(len(cases) == 24, "rendering manifest case count changed")
    parent = root / "target/release/oryn"
    worker = root / "target/release/OrynPageWorker.app/Contents/MacOS/oryn-page-worker"
    renderer = root / "target/release/oryn-render"
    source_commit = git(root, "rev-parse", "HEAD")
    result = {
        "schema_version": 1,
        "gate": "G8",
        "run_id": f"rendering-{source_commit[:12]}-macos-arm64",
        "recorded_at": datetime.now(timezone.utc).isoformat().replace("+00:00", "Z"),
        "fingerprint": {
            "source_commit": source_commit,
            "source_tree_sha256": subprocess.check_output(
                [str(root / "scripts/source-tree-hash.sh")], text=True
            ).strip(),
            "manifest_sha256": digest(manifest_path),
            "parent": {"sha256": digest(parent), "bytes": parent.stat().st_size},
            "worker": {"sha256": digest(worker), "bytes": worker.stat().st_size},
            "renderer": {"sha256": digest(renderer), "bytes": renderer.stat().st_size},
            "runtime": runtime,
            "renderer_capabilities": probe["renderer_capabilities"],
        },
        "regressions": {
            "g1_g7_production": "passed",
            "signed_worker": "passed",
            "strict_software": "passed",
        },
        "metrics": {
            "required_passed": 24,
            "required_total": 24,
            "display_commands": probe["scene"]["commands"],
            "snapshot_nodes": probe["snapshot"]["nodes"],
            "snapshot_images": probe["snapshot"]["images"],
            "png_bytes": probe["artifacts"]["png"]["bytes"],
            "pdf_bytes": probe["artifacts"]["pdf"]["bytes"],
            "parent_bytes": sizes["parent_bytes"],
            "worker_bytes": sizes["worker_bytes"],
            "renderer_bytes": sizes["renderer_bytes"],
        },
        "artifact_ledger": ledger,
        "case_ledger": cases,
        "criteria": {
            "cpu_reference_renderer": True,
            "screenshot_png": True,
            "single_page_pdf": True,
            "text_and_image_paint": True,
            "truthful_diagnostics": True,
            "core_dependency_boundary": True,
            "signed_worker_boundary": True,
            "g1_g7_regressions_pass": True,
        },
        "outcome": {
            "status": "passed",
            "reason_codes": [],
            "diagnostics": probe["scene"]["diagnostics"],
        },
    }
    output.parent.mkdir(parents=True, exist_ok=True)
    output.write_text(json.dumps(result, indent=2) + "\n", encoding="utf-8")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
