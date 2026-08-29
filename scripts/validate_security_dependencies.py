#!/usr/bin/env python3
"""Validate cargo-audit JSON against the reviewed security dependency inventory."""

from __future__ import annotations

import argparse
import json
from datetime import date
from pathlib import Path
from typing import Any


def load(path: Path) -> dict[str, Any]:
    value = json.loads(path.read_text(encoding="utf-8"))
    if not isinstance(value, dict):
        raise ValueError(f"{path} must contain a JSON object")
    return value


def require(condition: bool, message: str) -> None:
    if not condition:
        raise ValueError(message)


def main() -> int:
    root = Path(__file__).resolve().parents[1]
    parser = argparse.ArgumentParser()
    parser.add_argument("audit_json", type=Path)
    args = parser.parse_args()

    audit = load(args.audit_json)
    inventory = load(root / "security/dependency-exceptions.json")
    require(
        audit.get("vulnerabilities") == {"found": False, "count": 0, "list": []},
        "cargo audit reported one or more vulnerabilities",
    )
    require(
        audit.get("database", {}).get("advisory-count", 0) > 0,
        "cargo audit used no advisory database",
    )

    actual: set[tuple[str, str, str]] = set()
    for category, warnings in audit.get("warnings", {}).items():
        for warning in warnings:
            package = warning["package"]
            advisory = warning["advisory"]
            advisory_id = advisory["id"] if advisory is not None else category
            actual.add((package["name"], package["version"], advisory_id))
            if advisory is not None:
                require(
                    advisory.get("informational") == category,
                    f"warning category mismatch for {package['name']}",
                )

    reviewed = {
        (item["package"], item["version"], item["advisory"])
        for item in inventory["exceptions"]
    }
    require(actual == reviewed, "cargo-audit warnings do not match reviewed exceptions")
    for item in inventory["exceptions"]:
        require(
            date.fromisoformat(item["review_by"]) >= date.today(),
            f"dependency exception expired: {item['package']} {item['version']}",
        )

    lock = (root / "Cargo.lock").read_text(encoding="utf-8")
    for item in inventory["exceptions"]:
        require(
            f'name = "{item["package"]}"\nversion = "{item["version"]}"' in lock,
            f"reviewed dependency is absent or changed: {item['package']} {item['version']}",
        )

    print(
        "security dependency audit passed: "
        f"{audit['lockfile']['dependency-count']} locked packages, "
        "0 vulnerabilities, "
        f"{len(inventory['exceptions'])} reviewed informational warnings"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
