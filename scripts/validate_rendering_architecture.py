#!/usr/bin/env python3
"""Validate the frozen rendering manifest and optional-renderer dependency boundary."""

from __future__ import annotations

import json
import subprocess
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]


def require(condition: bool, message: str) -> None:
    if not condition:
        raise SystemExit(message)


def tree(package: str, features: str | None = None) -> set[str]:
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
        command.extend(["--features", features])
    return {
        line.split()[0]
        for line in subprocess.check_output(command, cwd=ROOT, text=True).splitlines()
        if line.strip()
    }


def main() -> int:
    manifest = json.loads(
        (ROOT / "benchmarks/conformance/rendering-manifest-v1.json").read_text()
    )
    cases = manifest.get("cases", [])
    require(manifest.get("gate") == "G8", "rendering manifest capability mismatch")
    require(
        manifest.get("frozen_before_validation") is True,
        "rendering manifest was not frozen",
    )
    require(manifest.get("locked") is True, "rendering manifest is not locked")
    require(
        len(cases) == 24 and len({case["id"] for case in cases}) == 24,
        "rendering must contain 24 unique cases",
    )
    require(
        all(case.get("required") is True for case in cases),
        "rendering contains an optional case",
    )

    # flate2 already belongs to the core HTTP decompression path; the remaining
    # packages are introduced solely by the optional raster/image layer.
    forbidden = {"oryn-render", "image", "fontdue", "png", "zune-jpeg"}
    native = tree("oryn-native")
    parent = tree("oryn")
    require(
        not native.intersection(forbidden),
        f"native core contains renderer dependencies: {native.intersection(forbidden)}",
    )
    require(
        not parent.intersection(forbidden),
        f"default parent contains renderer dependencies: {parent.intersection(forbidden)}",
    )
    optional = tree("oryn", "render")
    require(
        "oryn-render" in optional and "image" in optional and "fontdue" in optional,
        "optional render feature is incomplete",
    )
    renderer_source = (ROOT / "crates/oryn-render/src/lib.rs").read_text()
    require(
        "std::net" not in renderer_source and "reqwest" not in renderer_source,
        "renderer library has direct network authority",
    )
    require(
        "PaintSnapshot" in renderer_source and "PageState" not in renderer_source,
        "renderer does not honor the snapshot boundary",
    )
    print("rendering manifest and optional-renderer dependency boundary are valid")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
