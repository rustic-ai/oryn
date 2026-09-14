#!/usr/bin/env python3
"""Build the immutable automated-WPT inventory for the runtime cutover."""

from __future__ import annotations

import argparse
import hashlib
import json
import re
import subprocess
from html.parser import HTMLParser
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
MANIFEST = ROOT / "benchmarks/conformance/runtime-api-style-semantics-v2.json"
META_GLOBAL = re.compile(r"^//\s*META:\s*global=(.+)$", re.MULTILINE)
META_VARIANT = re.compile(r"^//\s*META:\s*variant=(.*)$", re.MULTILINE)


class HtmlVariantParser(HTMLParser):
    def __init__(self) -> None:
        super().__init__()
        self.variants: list[str] = []

    def handle_starttag(self, tag: str, attrs: list[tuple[str, str | None]]) -> None:
        if tag.lower() != "meta":
            return
        values = {name.lower(): value or "" for name, value in attrs}
        if values.get("name", "").lower() == "variant":
            self.variants.append(values.get("content", ""))


def html_variants(source: str) -> list[str]:
    parser = HtmlVariantParser()
    parser.feed(source)
    return parser.variants or [""]


def sha256(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def classify(path: Path, roots: list[dict[str, str]]) -> dict[str, str] | None:
    candidates = [entry for entry in roots if path == Path(entry["path"]) or Path(entry["path"]) in path.parents]
    return max(candidates, key=lambda entry: len(Path(entry["path"]).parts), default=None)


def normalize_global(value: str) -> str:
    # WPT's historical `worker` global names the dedicated-worker wrapper;
    # shared and service workers are declared explicitly.
    return "dedicatedworker" if value == "worker" else value


def is_excluded_path(path: Path, prefixes: list[Path]) -> bool:
    return any(path == prefix or prefix in path.parents for prefix in prefixes)


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--checkout", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--manifest", type=Path, default=MANIFEST)
    args = parser.parse_args()

    manifest_path = args.manifest.resolve()
    manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
    checkout = args.checkout.resolve()
    actual = subprocess.check_output(
        ["git", "-C", str(checkout), "rev-parse", "HEAD"], text=True
    ).strip()
    expected = manifest["upstream"]["revision"]
    if actual != expected:
        raise SystemExit(f"WPT revision mismatch: expected {expected}, found {actual}")

    selected: dict[str, dict[str, object]] = {}
    roots = manifest["roots"]
    excluded = manifest["selection"]["excluded_filename_markers"]
    excluded_segments = set(manifest["selection"]["excluded_path_segments"])
    excluded_prefixes = [Path(value) for value in manifest["selection"].get("excluded_path_prefixes", [])]
    excluded_globals = set(manifest["selection"].get("excluded_globals", []))
    for entry in roots:
        directory = checkout / entry["path"]
        if not directory.is_dir():
            continue
        for source in directory.rglob("*"):
            if (
                not source.is_file()
                or any(marker in source.name for marker in excluded)
                or excluded_segments.intersection(source.relative_to(checkout).parts)
            ):
                continue
            relative = source.relative_to(checkout)
            if is_excluded_path(relative, excluded_prefixes):
                continue
            kind: str | None = None
            globals_: list[str] = []
            variants: list[str] = []
            text = ""
            if source.name.endswith((".any.js", ".window.js")):
                kind = "script"
                text = source.read_text(encoding="utf-8", errors="replace")
                declared = META_GLOBAL.findall(text)
                globals_ = sorted(
                    {
                        normalize_global(item.strip())
                        for value in declared
                        for item in value.split(",")
                        if item.strip()
                    }
                )
                if not globals_:
                    globals_ = ["window"] if source.name.endswith(".window.js") else ["window", "dedicatedworker"]
                globals_ = [value for value in globals_ if value not in excluded_globals]
                if not globals_:
                    continue
                variants = META_VARIANT.findall(text) or [""]
            elif source.suffix in {".html", ".htm"}:
                text = source.read_text(encoding="utf-8", errors="replace")
                if "testharness.js" not in text.lower():
                    continue
                kind = "html"
                globals_ = ["window"]
                variants = html_variants(text)
            if kind is None:
                continue
            classification = classify(relative, roots)
            if classification is None:
                continue
            selected[relative.as_posix()] = {
                "path": relative.as_posix(),
                "workstream": classification["workstream"],
                "root": classification["path"],
                "kind": kind,
                "globals": globals_,
                "variants": variants,
                "sha256": sha256(source),
            }

    tests = [selected[key] for key in sorted(selected)]
    by_workstream: dict[str, int] = {}
    by_global: dict[str, int] = {}
    required_executions = 0
    for test in tests:
        workstream = str(test["workstream"])
        by_workstream[workstream] = by_workstream.get(workstream, 0) + 1
        variants = len(test["variants"])
        for global_name in test["globals"]:
            by_global[global_name] = by_global.get(global_name, 0) + variants
            required_executions += variants
    output = {
        "schema_version": 1,
        "manifest_sha256": sha256(manifest_path),
        "upstream_revision": expected,
        "required_total": len(tests),
        "required_execution_total": required_executions,
        "required_by_workstream": dict(sorted(by_workstream.items())),
        "required_by_global": dict(sorted(by_global.items())),
        "tests": tests,
    }
    args.output.parent.mkdir(parents=True, exist_ok=True)
    # Keep one test per line so the large immutable inventory remains
    # reviewable without a 40k-line pretty-printed diff.
    header = {key: value for key, value in output.items() if key != "tests"}
    lines = [json.dumps(header, sort_keys=True)[:-1] + ', "tests": [']
    for index, test in enumerate(tests):
        suffix = "," if index + 1 < len(tests) else ""
        lines.append("  " + json.dumps(test, sort_keys=True, separators=(",", ":")) + suffix)
    lines.append("]}\n")
    args.output.write_text("\n".join(lines), encoding="utf-8")
    print(
        json.dumps(
            {
                "required_files": len(tests),
                "required_executions": required_executions,
                "by_workstream": by_workstream,
                "by_global": by_global,
            },
            sort_keys=True,
        )
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
