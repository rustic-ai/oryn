#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
oryn_bin="${ORYN_BIN:-$repo_root/target/debug/oryn}"
node_bin="${ORYN_NODE_BIN:-$(command -v node || true)}"
if [[ -z "$node_bin" && -x "/Users/rohit/.cache/codex-runtimes/codex-primary-runtime/dependencies/node/bin/node" ]]; then
  node_bin="/Users/rohit/.cache/codex-runtimes/codex-primary-runtime/dependencies/node/bin/node"
fi
port="${ORYN_APP_REGRESSION_PORT:-3000}"
artifact_dir="${ORYN_APP_COMPATIBILITY_ARTIFACT_DIR:-$repo_root/artifacts/app-compatibility/controlled}"
server_pid=""

cleanup() {
  if [[ -n "$server_pid" ]]; then
    kill "$server_pid" 2>/dev/null || true
    wait "$server_pid" 2>/dev/null || true
  fi
}
trap cleanup EXIT

if [[ -z "$node_bin" ]]; then
  echo "Node.js is required to start the controlled regression server" >&2
  exit 2
fi
if [[ ! -x "$oryn_bin" ]]; then
  echo "Oryn binary is not executable: $oryn_bin" >&2
  exit 2
fi

(
  cd "$repo_root/test-harness"
  PORT="$port" "$node_bin" server.js
) >"${TMPDIR:-/tmp}/oryn-app-compatibility-controlled-$port.log" 2>&1 &
server_pid="$!"
for _ in $(seq 1 50); do
  curl --fail --silent "http://127.0.0.1:$port/ping" >/dev/null 2>&1 && break
  sleep 0.1
done
curl --fail --silent "http://127.0.0.1:$port/ping" >/dev/null

mkdir -p "$artifact_dir"
scripts=(
  01_static
  02_forms
  03_ecommerce
  04_interactivity
  05_dynamic
  06_edge_cases
  07_intents_builtin
  08_multipage_flows
  09_target_resolution
  10_react_spa
  11_angular_spa
  12_popup_detection
)

for name in "${scripts[@]}"; do
  source="$repo_root/test-harness/scripts/$name.oil"
  log="$artifact_dir/$name.jsonl"
  "$oryn_bin" --file "$source" native --allow-loopback >"$log"
  jq -s -e '
    length > 0 and
    any(.[]; .kind == "observation") and
    all(.[]; .kind != "unsupported")
  ' "$log" >/dev/null
  echo "application compatibility controlled regression passed: $name"
done

echo "application compatibility controlled regression corpus passed: 12/12"
