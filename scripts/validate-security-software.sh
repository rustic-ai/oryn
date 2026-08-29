#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
artifact_dir="${ORYN_SECURITY_ARTIFACT_DIR:-$repo_root/artifacts/security-v1/software}"
mkdir -p "$artifact_dir"

if [[ "${ORYN_SECURITY_SKIP_LAYOUT:-0}" != "1" ]]; then
  "$repo_root/scripts/validate-layout-software.sh" >"$artifact_dir/core-layout-software.log" 2>&1
fi
cargo test --locked -p oryn-native --no-default-features --features in-process-probe \
  >"$artifact_dir/native-security-protocol.log" 2>&1
cargo check --locked --manifest-path "$repo_root/fuzz/Cargo.toml" \
  >"$artifact_dir/fuzz-check.log" 2>&1
"$repo_root/scripts/validate_security_controls.py" >"$artifact_dir/security-static.log" 2>&1
cargo audit --no-fetch --json >"$artifact_dir/cargo-audit.json"
"$repo_root/scripts/validate_security_dependencies.py" "$artifact_dir/cargo-audit.json" \
  >"$artifact_dir/dependency-audit.log" 2>&1

printf '%s\n' "security software, threat-model unit, fuzz-build, and dependency checks passed"
