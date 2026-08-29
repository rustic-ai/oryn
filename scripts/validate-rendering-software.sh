#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
artifact_dir="${ORYN_RENDERING_ARTIFACT_DIR:-$repo_root/artifacts/rendering-v1/software}"
mkdir -p "$artifact_dir"

if [[ "${ORYN_RENDERING_SKIP_EXECUTION_REPLAY:-0}" != "1" ]]; then
  ORYN_EXECUTION_REPLAY_ARTIFACT_DIR="$artifact_dir/execution-replay" \
    "$repo_root/scripts/validate-execution-replay-software.sh" >"$artifact_dir/execution-replay.log" 2>&1
fi
cargo fmt --all -- --check >"$artifact_dir/fmt.log" 2>&1
cargo clippy --locked -p oryn-render --all-targets -- -D warnings >"$artifact_dir/clippy.log" 2>&1
cargo test --locked -p oryn-render >"$artifact_dir/render-tests.log" 2>&1
cargo test --locked -p oryn-native --features v8-host paint_snapshot_is_worker_neutral_and_broker_complete \
  >"$artifact_dir/snapshot-test.log" 2>&1
cargo check --locked -p oryn >"$artifact_dir/parent-default.log" 2>&1
cargo check --locked -p oryn --features render >"$artifact_dir/parent-render.log" 2>&1
"$repo_root/scripts/validate_rendering_architecture.py" >"$artifact_dir/architecture.log" 2>&1

jq -n '{schema_version:1,suite:"oryn-rendering-software",checks:{formatting:"passed",strict_clippy:"passed",renderer_tests:"passed",v8_snapshot:"passed",default_parent:"passed",optional_parent:"passed",architecture:"passed"},outcome:{status:"passed"}}' \
  >"$artifact_dir/summary.json"
jq . "$artifact_dir/summary.json"
