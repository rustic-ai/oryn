#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
artifact_dir="${ORYN_G6_G7_ARTIFACT_DIR:-$repo_root/artifacts/g6-g7-v1/software}"
mkdir -p "$artifact_dir"

if [[ "${ORYN_G6_G7_SKIP_G5:-0}" != "1" ]]; then
  "$repo_root/scripts/validate-g5-software.sh" >"$artifact_dir/g1-g5-software.log" 2>&1
fi
cargo fmt --all -- --check >"$artifact_dir/fmt.log" 2>&1
cargo clippy --locked --workspace --all-targets --all-features -- -D warnings \
  >"$artifact_dir/clippy.log" 2>&1
cargo test --locked -p oryn-native --all-features \
  >"$artifact_dir/native.log" 2>&1
cargo test --locked -p oryn-domain --all-features \
  >"$artifact_dir/domain.log" 2>&1
cargo check --locked -p oryn --no-default-features \
  >"$artifact_dir/oryn-no-default.log" 2>&1
cargo check --locked -p oryn \
  >"$artifact_dir/oryn-default.log" 2>&1
cargo check --locked -p oryn --all-features \
  >"$artifact_dir/oryn-all-features.log" 2>&1
"$repo_root/scripts/validate-g6-g7-architecture.py" \
  >"$artifact_dir/architecture.log" 2>&1

jq -n \
  --arg recorded_at "$(date -u +%Y-%m-%dT%H:%M:%SZ)" \
  '{schema_version:1,suite:"oryn2-g6-g7-software",recorded_at:$recorded_at,
    checks:{formatting:"passed",strict_clippy:"passed",native:"passed",domain_adapters:"passed",
      oryn_no_default:"passed",oryn_default:"passed",oryn_all_features:"passed",architecture:"passed"},
    outcome:{status:"passed"}}' >"$artifact_dir/summary.json"
jq . "$artifact_dir/summary.json"
