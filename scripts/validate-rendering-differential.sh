#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
native_png="$1"
output_dir="${2:-$repo_root/artifacts/rendering-v1/differential}"
node_bin="${ORYN_NODE_BIN:-$(command -v node || true)}"
if [[ -z "$node_bin" && -x "/Users/rohit/.cache/codex-runtimes/codex-primary-runtime/dependencies/node/bin/node" ]]; then
  node_bin="/Users/rohit/.cache/codex-runtimes/codex-primary-runtime/dependencies/node/bin/node"
fi
test -n "$node_bin"
mkdir -p "$output_dir"
"$node_bin" "$repo_root/test-harness/chromium-rendering.js" "$output_dir/chromium.png" \
  >"$output_dir/chromium.json"
cargo run --quiet --release --locked -p oryn-render --example rendering_comparison -- \
  "$native_png" "$output_dir/chromium.png" >"$output_dir/differential.json"
jq -e '.outcome.status == "passed"' "$output_dir/chromium.json" >/dev/null
jq -e '.outcome.status == "passed"' "$output_dir/differential.json" >/dev/null
jq . "$output_dir/differential.json"
