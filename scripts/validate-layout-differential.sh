#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
artifact_dir="${1:-$repo_root/artifacts/layout-v1/differential}"
node_bin="${ORYN_NODE_BIN:-$(command -v node || true)}"
if [[ -z "$node_bin" && -x "/Users/rohit/.cache/codex-runtimes/codex-primary-runtime/dependencies/node/bin/node" ]]; then
  node_bin="/Users/rohit/.cache/codex-runtimes/codex-primary-runtime/dependencies/node/bin/node"
fi
if [[ -z "$node_bin" ]]; then
  echo "Node.js is required for the Chromium differential harness" >&2
  exit 2
fi
mkdir -p "$artifact_dir"

cargo run --manifest-path "$repo_root/Cargo.toml" --quiet --release --locked \
  -p oryn-native \
  --example layout_probe >"$artifact_dir/native.json"
"$node_bin" "$repo_root/test-harness/chromium-layout.js" >"$artifact_dir/chromium.json"

jq -e '.outcome.status == "passed" and .domain == "native"' \
  "$artifact_dir/native.json" >/dev/null
jq -e '.outcome.status == "passed" and .domain == "chromium" and
  (.chromium_version | length > 0)' "$artifact_dir/chromium.json" >/dev/null

native_relations="$(jq -S '.relations | del(.table_geometry_unavailable, .block_border_width)' "$artifact_dir/native.json")"
chromium_relations="$(jq -S '.relations | del(.block_border_width)' "$artifact_dir/chromium.json")"
test "$native_relations" = "$chromium_relations"
jq -e '.relations.table_geometry_unavailable == true' "$artifact_dir/native.json" >/dev/null
jq -e --slurp '
  (.[0].relations.block_border_width - .[1].relations.block_border_width) as $difference |
  (($difference | if . < 0 then -. else . end) <= 1.0)
' "$artifact_dir/native.json" "$artifact_dir/chromium.json" >/dev/null

echo "layout native/Chromium supported-scope relational differential passed"
