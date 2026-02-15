#!/usr/bin/env python3
"""Lightweight assertions that keep docs aligned with current unified CLI reality."""

from __future__ import annotations

import re
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]

FORBIDDEN_PATTERNS: list[tuple[str, str, str]] = [
    (
        "website/docs/guides/basic-navigation.md",
        r"After navigating to a site, you can use relative paths",
        "Relative-path navigation is not currently supported in unified `goto` translation.",
    ),
    (
        "website/docs/guides/basic-navigation.md",
        r"ok refresh \(cache cleared\)",
        "`refresh --hard` cache-clearing behavior is not currently wired end-to-end.",
    ),
    (
        "website/docs/reference/intent-commands.md",
        r"wait load\|idle\|navigation\|ready",
        "`wait ready` is parsed but not currently translated to a supported scanner condition.",
    ),
    (
        "website/docs/index.md",
        r"Built-in Intents \(8\)",
        "Homepage status count is stale for current unified CLI support.",
    ),
]

REQUIRED_SUBSTRINGS: list[tuple[str, str, str]] = [
    (
        "website/docs/reference/intent-commands.md",
        "`--headers` and `--timeout` parse but are currently not applied in unified translation.",
        "Missing `goto` option caveat.",
    ),
    (
        "website/docs/reference/intent-commands.md",
        "`--hard` is parsed, but current executor/backend wiring does not distinguish hard vs soft refresh.",
        "Missing `refresh --hard` caveat.",
    ),
    (
        "website/docs/reference/intent-commands.md",
        "`--ctrl/--shift/--alt` and `--timeout` are parsed, but currently not applied by translation/execution.",
        "Missing click option caveat.",
    ),
    (
        "website/docs/reference/intent-commands.md",
        "`--append` and `--timeout` parse but are currently not applied in unified translation.",
        "Missing type option caveat.",
    ),
    (
        "website/docs/reference/intent-commands.md",
        "`--no-submit`, `--wait`, and `--timeout` parse but are currently not applied in unified translation.",
        "Missing login option caveat.",
    ),
    (
        "website/docs/reference/intent-commands.md",
        "`--submit`, `--wait`, and `--timeout` parse but are currently not applied in unified translation.",
        "Missing search option caveat.",
    ),
    (
        "website/docs/reference/intent-commands.md",
        "`ready` is parsed in grammar but not currently mapped to a supported scanner wait condition.",
        "Missing wait/ready caveat.",
    ),
    (
        "website/docs/reference/truth-and-trust.md",
        "truth checks",
        "Truth & Trust reference page is missing required verification workflow details.",
    ),
]


def main() -> int:
    errors: list[str] = []

    for rel_path, pattern, message in FORBIDDEN_PATTERNS:
        path = ROOT / rel_path
        if not path.exists():
            errors.append(f"[MISSING] {rel_path}: file not found")
            continue

        text = path.read_text(encoding="utf-8")
        if re.search(pattern, text):
            errors.append(f"[FORBIDDEN] {rel_path}: {message}")

    for rel_path, needle, message in REQUIRED_SUBSTRINGS:
        path = ROOT / rel_path
        if not path.exists():
            errors.append(f"[MISSING] {rel_path}: file not found")
            continue

        text = path.read_text(encoding="utf-8")
        if needle not in text:
            errors.append(f"[REQUIRED] {rel_path}: {message}")

    if errors:
        print("Docs truth check failed:\n")
        for err in errors:
            print(f"- {err}")
        return 1

    print("Docs truth check passed.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
