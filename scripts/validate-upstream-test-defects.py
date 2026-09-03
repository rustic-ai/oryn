#!/usr/bin/env python3
"""Validate a raw WPT result while narrowly accounting for pinned upstream defects."""

from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path
from typing import Any

ROOT = Path(__file__).resolve().parents[1]
LEDGER = ROOT / "benchmarks/conformance/upstream-test-defects-v1.json"


def require(condition: bool, message: str) -> None:
    if not condition:
        raise SystemExit(message)


def digest(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("result", type=Path)
    parser.add_argument("--checkout", type=Path, required=True)
    args = parser.parse_args()

    ledger = json.loads(LEDGER.read_text(encoding="utf-8"))
    result = json.loads(args.result.read_text(encoding="utf-8"))
    require(ledger.get("schema_version") == 1, "unsupported defect-ledger schema")
    revision = ledger["upstream"]["revision"]
    require(result.get("upstream_revision") == revision, "WPT revision mismatch")

    defects: dict[tuple[str, str], list[dict[str, Any]]] = {}
    expected: set[tuple[str, str, str]] = set()
    for defect in ledger["defects"]:
        source = args.checkout / defect["path"]
        require(source.is_file(), f"missing pinned source: {defect['path']}")
        require(digest(source) == defect["source_sha256"], f"source hash mismatch: {defect['id']}")
        source_text = source.read_text(encoding="utf-8", errors="strict")
        for predicate in defect["source_predicates"]:
            require(predicate in source_text, f"source predicate missing: {defect['id']}")
        for variant in defect["variants"]:
            defects.setdefault((defect["path"], variant), []).append(defect)
            expected.add((defect["id"], defect["path"], variant))

    seen: set[tuple[str, str, str]] = set()
    cases = conformant_cases = internal_total = internal_passed = excluded = 0
    for assertion in result.get("assertions", []):
        cases += 1
        path = assertion.get("path")
        variant = assertion.get("variant")
        payload = assertion.get("result", {})
        conformant = payload.get("complete") is True and payload.get("harness") in (0, None)
        tests = payload.get("tests", [])
        require(isinstance(tests, list) and tests, f"missing internal assertions: {path}|{variant}")
        for test in tests:
            internal_total += 1
            if test.get("status") == 0:
                internal_passed += 1
                continue
            matches = [
                defect
                for defect in defects.get((path, variant), [])
                if defect["assertion_name_contains"] in str(test.get("name", ""))
            ]
            require(len(matches) == 1, f"unexpected WPT failure: {path}|{variant}|{test.get('name')}")
            defect = matches[0]
            key = (defect["id"], path, variant)
            require(key not in seen, f"duplicate excluded assertion: {key}")
            seen.add(key)
            excluded += 1
        if conformant:
            conformant_cases += 1

    applicable_expected = {
        key for key in expected if any(item.get("path") == key[1] and item.get("variant") == key[2] for item in result.get("assertions", []))
    }
    require(seen == applicable_expected, "not every applicable pinned defect was observed exactly once")
    require(conformant_cases == cases, "one or more WPT cases had a harness-level failure")
    print(json.dumps({
        "cases": cases,
        "conformant_cases": conformant_cases,
        "internal_assertions": internal_total,
        "internal_passed": internal_passed,
        "excluded_upstream_defects": excluded,
        "status": "passed",
    }, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
