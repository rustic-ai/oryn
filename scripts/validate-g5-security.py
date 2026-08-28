#!/usr/bin/env python3
"""Validate the static, reproducible inputs to the Oryn2 G5 security gate."""

from __future__ import annotations

import json
import pathlib
import re
import sys
from datetime import date

ROOT = pathlib.Path(__file__).resolve().parents[1]


def fail(message: str) -> None:
    raise SystemExit(f"G5 validation failed: {message}")


manifest = json.loads((ROOT / "benchmarks/conformance/g5-manifest-v1.json").read_text())
cases = manifest.get("cases", [])
ids = [case.get("id") for case in cases]
if manifest.get("gate") != "G5" or manifest.get("schema_version") != 1:
    fail("unexpected manifest identity")
if len(ids) != len(set(ids)) or not ids or not all(case.get("required") for case in cases):
    fail("manifest cases must be unique and required")

policy = json.loads((ROOT / "security/v8-update-policy.json").read_text())
lock = (ROOT / "Cargo.lock").read_text()
match = re.search(
    r'\[\[package\]\]\nname = "v8"\nversion = "([^"]+)"\n'
    r'source = "[^"]+"\nchecksum = "([^"]+)"',
    lock,
)
if not match:
    fail("Cargo.lock has no registry-pinned v8 package")
if match.group(1) != policy["expected_crate_version"]:
    fail(f"V8 crate drift: {match.group(1)}")
if match.group(2) != policy["expected_crate_checksum"]:
    fail("V8 checksum drift")
if policy.get("response_sla_hours") != {"critical": 24, "high": 72}:
    fail("V8 security response SLA changed")
if policy.get("rust_toolchain") != "1.98.0":
    fail("Rust toolchain policy drift")
if 'channel = "1.98.0"' not in (ROOT / "rust-toolchain.toml").read_text():
    fail("rust-toolchain.toml does not match the security policy")

exceptions = json.loads((ROOT / "security/dependency-exceptions.json").read_text())
if exceptions.get("schema_version") != 1:
    fail("unexpected dependency-exception schema")
exception_keys: set[tuple[str, str, str]] = set()
for item in exceptions.get("exceptions", []):
    for field in ("package", "version", "advisory", "classification", "scope", "risk", "resolution", "review_by"):
        if not item.get(field):
            fail(f"dependency exception is missing {field}")
    key = (item["package"], item["version"], item["advisory"])
    if key in exception_keys:
        fail(f"duplicate dependency exception: {key}")
    exception_keys.add(key)
    if date.fromisoformat(item["review_by"]) < date.today():
        fail(f"dependency exception review expired: {key}")

expected_exceptions = {
    ("bincode", "1.3.3", "RUSTSEC-2025-0141"),
    ("paste", "1.0.15", "RUSTSEC-2024-0436"),
    ("chacha20", "0.10.1", "yanked"),
    ("ttf-parser", "0.25.1", "RUSTSEC-2026-0192"),
}
if exception_keys != expected_exceptions:
    fail("dependency exceptions do not exactly match the reviewed lockfile warnings")
for required_command in (
    "cargo test --workspace --all-features",
    "cargo clippy --workspace --all-targets --all-features -- -D warnings",
    "scripts/run-g5-production-gate.sh",
):
    if required_command not in policy.get("qualification", []):
        fail(f"V8 qualification omits {required_command}")
if not (ROOT / "scripts/qualify-v8-update.sh").is_file():
    fail("V8 update qualification entrypoint is missing")

required_fuzz = {"html_tree_sink", "dom_mutation", "url_policy", "worker_protocol"}
expected_seeds = {
    "html_tree_sink": {"basic.html"},
    "dom_mutation": {"basic.bin"},
    "url_policy": {"public.txt", "private.txt"},
    "worker_protocol": {"init.json"},
}
fuzz_toml = (ROOT / "fuzz/Cargo.toml").read_text()
for target in required_fuzz:
    if f'name = "{target}"' not in fuzz_toml:
        fail(f"missing fuzz target {target}")
    if not (ROOT / f"fuzz/fuzz_targets/{target}.rs").is_file():
        fail(f"missing fuzz source {target}")
    seed_dir = ROOT / f"fuzz/seeds/{target}"
    seeds = {path.name for path in seed_dir.iterdir() if path.is_file()} if seed_dir.is_dir() else set()
    if seeds != expected_seeds[target]:
        fail(f"missing fuzz seed corpus {target}")

security_doc = (ROOT / "docs/oryn2/G5_SECURITY.md").read_text()
for heading in (
    "Threat model",
    "Action policy",
    "Secret handles",
    "Download quarantine",
    "Fuzzing",
    "V8 update train",
    "Release gate",
):
    if heading not in security_doc:
        fail(f"security policy is missing {heading!r}")

print(f"G5 static validation passed: {len(cases)} required controls, V8 {match.group(1)}")
